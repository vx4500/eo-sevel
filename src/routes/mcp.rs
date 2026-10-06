//! Connecteur Claude (MCP, transport HTTP « streamable », réponses JSON).
//!
//! L'application Claude du téléphone parle avec l'éleveur — à la voix — et
//! appelle ces outils pour lire ou enregistrer. Adresse : `/mcp/{jeton}`, le
//! jeton (paramètre `mcp_jeton`) tient lieu de mot de passe ; il se crée et
//! se change dans Mes équipements → Assistant.
//!
//! Toute écriture se fait en deux temps : sans `confirme`, l'outil ne fait
//! que décrire ce qu'il va faire ; Claude le relit à l'éleveur et ne rappelle
//! avec `confirme: true` qu'après son accord. Chaque écriture est tracée dans
//! `journal`.

use super::*;
use crate::assistant;
use axum::Json;
use chrono::Datelike;

const PROTOCOLES: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const AUTEUR: &str = "Claude (connecteur)";

fn outils() -> Value {
    let confirme = json!({"type": "boolean", "description": "false ou absent : décrit seulement l'action. true : enregistre — uniquement après accord explicite de l'éleveur."});
    json!([
        {"name": "chercher_truie",
         "description": "Fiche d'une truie par son numéro de travail (ou fin de numéro) : statut, rang, bande, salle, case/vanne et derniers événements.",
         "input_schema": {"type": "object", "properties": {"numero": {"type": "string"}}, "required": ["numero"]}},
        {"name": "cases_salle",
         "description": "Cases d'une salle (ex. « Verraterie ») avec numéro de vanne, places et truies présentes.",
         "input_schema": {"type": "object", "properties": {"salle": {"type": "string"}}, "required": ["salle"]}},
        {"name": "calendrier_ia",
         "description": "Dates d'IA prévues par bande active : IA de la bande (mise-bas − gestation) et IA des truies sevrées (lundi suivant le sevrage). Sert à calculer un rappel « X jours avant les IA ».",
         "input_schema": {"type": "object", "properties": {}}},
        {"name": "deplacer_truie",
         "description": "Place une truie dans une case. La case se désigne par la salle et le numéro de vanne ou le nom de case. Deux temps : d'abord sans confirme, lire le résumé à l'éleveur, puis confirme=true après son accord.",
         "input_schema": {"type": "object", "properties": {
             "numero": {"type": "string"}, "salle": {"type": "string"},
             "vanne": {"type": "string", "description": "numéro de vanne ou nom de la case"},
             "confirme": confirme}, "required": ["numero", "salle", "vanne"]}},
        {"name": "creer_rappel",
         "description": "Crée un rappel (tâche avec échéance) affiché dans EO_Sevel. Deux temps comme deplacer_truie.",
         "input_schema": {"type": "object", "properties": {
             "titre": {"type": "string", "description": "ex. « Truie 400042 : mettre sous œstrogène »"},
             "date": {"type": "string", "description": "AAAA-MM-JJ"},
             "note": {"type": "string"}, "bande": {"type": "string"},
             "confirme": confirme}, "required": ["titre", "date"]}},
        {"name": "rappels",
         "description": "Rappels et tâches non faits, par échéance.",
         "input_schema": {"type": "object", "properties": {}}},
        {"name": "lister_tables",
         "description": "Pour les autres questions : tables de la base de l'élevage (lecture seule).",
         "input_schema": {"type": "object", "properties": {}}},
        {"name": "decrire_tables",
         "description": "Schéma des tables demandées.",
         "input_schema": {"type": "object", "properties": {"tables": {"type": "array", "items": {"type": "string"}}}, "required": ["tables"]}},
        {"name": "requete",
         "description": "UNE requête SQLite SELECT/WITH en lecture seule, 200 lignes au plus.",
         "input_schema": {"type": "object", "properties": {"sql": {"type": "string"}}, "required": ["sql"]}}
    ])
}

const CONSIGNES: &str = "Outils de l'élevage EO_Sevel. Réponds brièvement, en français parlé (l'éleveur est souvent à la voix, dans le bâtiment). \
Avant toute écriture, appelle l'outil sans confirme, répète en une phrase ce qui va être fait et attends un « oui » ; seulement ensuite rappelle avec confirme=true. \
N'invente jamais un numéro, une date ou une case : vérifie avec les outils. Pour une date relative aux IA, utilise calendrier_ia.";

fn texte(t: impl Into<String>, erreur: bool) -> Value {
    json!({"content": [{"type": "text", "text": t.into()}], "isError": erreur})
}

fn arg<'a>(a: &'a Value, k: &str) -> &'a str {
    a.get(k).and_then(Value::as_str).map(str::trim).unwrap_or("")
}

async fn journaliser(pool: &SqlitePool, action: &str, objet: &str, detail: &str) {
    let _ = sqlx::query("INSERT INTO journal(horodatage,utilisateur,action,objet,detail,chemin) VALUES(CURRENT_TIMESTAMP,?,?,?,?,'/mcp')")
        .bind(AUTEUR).bind(action).bind(objet).bind(detail)
        .execute(pool).await;
}

async fn truie_par_numero(pool: &SqlitePool, numero: &str) -> Result<(i64, String), String> {
    let n = numero.replace([' ', '.'], "");
    let lignes: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id,num_travail FROM truie WHERE reformee=0 AND (num_travail=?1 OR replace(num_travail,' ','')=?1 OR num_national=?1 OR num_travail LIKE '%'||?1) ORDER BY num_travail=?1 DESC LIMIT 5",
    )
    .bind(&n)
    .fetch_all(pool).await.map_err(|e| e.to_string())?;
    match lignes.as_slice() {
        [] => Err(format!("Aucune truie présente ne porte le numéro {numero}.")),
        [une] => Ok(une.clone()),
        [une, ..] if une.1 == n => Ok(une.clone()),
        plusieurs => Err(format!("Plusieurs truies correspondent : {}. Lequel ?", plusieurs.iter().map(|l| l.1.as_str()).collect::<Vec<_>>().join(", "))),
    }
}

async fn case_par_vanne(pool: &SqlitePool, salle: &str, vanne: &str) -> Result<(i64, i64, String, String, Option<i64>), String> {
    let v = vanne.trim_start_matches(['v', 'V']).trim().to_string();
    let lignes: Vec<(i64, i64, String, String, Option<i64>)> = sqlx::query_as(
        "SELECT c.id,c.salle_id,s.nom,c.nom,COALESCE(c.nb_max_truies,c.nb_max_porcs) FROM casesalle c JOIN salle s ON s.id=c.salle_id \
         WHERE lower(s.nom) LIKE '%'||lower(?1)||'%' AND (c.num_vanne=?2 OR lower(c.nom)=lower(?2) OR c.nom LIKE ?2||' %')",
    )
    .bind(salle).bind(&v)
    .fetch_all(pool).await.map_err(|e| e.to_string())?;
    match lignes.len() {
        0 => Err(format!("Pas de case « {vanne} » dans la salle « {salle} ». Utilise cases_salle pour voir les cases.")),
        1 => Ok(lignes.into_iter().next().unwrap()),
        _ => Err(format!("Plusieurs cases correspondent : {}.", lignes.iter().map(|l| format!("{} / {}", l.2, l.3)).collect::<Vec<_>>().join(", "))),
    }
}

async fn appeler(state: &AppState, nom: &str, a: &Value) -> Result<String, String> {
    let pool = &state.pool;
    match nom {
        "chercher_truie" => {
            let (id, _) = truie_par_numero(pool, arg(a, "numero")).await?;
            let fiche: Option<(String, String, i64, Option<String>, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
                "SELECT t.num_travail,t.statut,t.rang,t.bande_code,s.nom,c.nom,c.num_vanne FROM truie t \
                 LEFT JOIN salle s ON s.id=t.salle_id LEFT JOIN casesalle c ON c.id=t.case_id WHERE t.id=?",
            ).bind(id).fetch_optional(pool).await.map_err(|e| e.to_string())?;
            let (num, statut, rang, bande, salle, case, vanne) = fiche.ok_or("Truie introuvable")?;
            let evts: Vec<(String, String)> = sqlx::query_as("SELECT date,type FROM evenement WHERE truie_id=? ORDER BY date DESC LIMIT 6")
                .bind(id).fetch_all(pool).await.unwrap_or_default();
            Ok(json!({"numero": num, "statut": statut, "rang": rang, "bande": bande, "salle": salle, "case": case, "vanne": vanne,
                      "derniers_evenements": evts.into_iter().map(|(d, t)| format!("{d} {t}")).collect::<Vec<_>>()}).to_string())
        }
        "cases_salle" => {
            let lignes: Vec<(String, String, Option<String>, Option<i64>, Option<String>)> = sqlx::query_as(
                "SELECT s.nom,c.nom,c.num_vanne,COALESCE(c.nb_max_truies,c.nb_max_porcs), \
                 (SELECT group_concat(t.num_travail,', ') FROM truie t WHERE t.case_id=c.id AND t.reformee=0) \
                 FROM casesalle c JOIN salle s ON s.id=c.salle_id WHERE lower(s.nom) LIKE '%'||lower(?)||'%' ORDER BY s.nom,c.id",
            ).bind(arg(a, "salle")).fetch_all(pool).await.map_err(|e| e.to_string())?;
            if lignes.is_empty() {
                return Err(format!("Aucune salle « {} ».", arg(a, "salle")));
            }
            Ok(Value::Array(lignes.into_iter().map(|(s, c, v, p, t)| {
                let truies: Vec<&str> = t.as_deref().map(|t| t.split(", ").collect()).unwrap_or_default();
                json!({"salle": s, "case": c, "vanne": v, "places": p, "occupees": truies.len(), "truies": truies})
            }).collect()).to_string())
        }
        "calendrier_ia" => {
            let c = load_band_schedule(pool).await.map_err(|e| e.to_string())?;
            let bandes: Vec<(String, String)> = sqlx::query_as("SELECT code,date_mb FROM bande WHERE active=1 AND date_mb IS NOT NULL ORDER BY date_mb")
                .fetch_all(pool).await.map_err(|e| e.to_string())?;
            let auj = chrono::Local::now().date_naive();
            let mut sortie = Vec::new();
            for (code, mb) in bandes {
                let Ok(mb) = chrono::NaiveDate::parse_from_str(&mb, "%Y-%m-%d") else { continue };
                let ia = mb - chrono::Duration::days(c.gestation);
                let sevrage = mb + chrono::Duration::days(c.weaning);
                let ia_sevrees = sevrage + chrono::Duration::days(7 - sevrage.weekday().num_days_from_monday() as i64);
                sortie.push(json!({"bande": code, "mise_bas": mb.to_string(), "ia_de_la_bande": ia.to_string(),
                                   "sevrage": sevrage.to_string(), "ia_des_sevrees": ia_sevrees.to_string(),
                                   "ia_des_sevrees_a_venir": ia_sevrees >= auj}));
            }
            Ok(json!({"aujourdhui": auj.to_string(), "gestation_jours": c.gestation, "sevrage_jours": c.weaning, "bandes": sortie}).to_string())
        }
        "deplacer_truie" => {
            let (truie_id, num) = truie_par_numero(pool, arg(a, "numero")).await?;
            let (case_id, salle_id, salle, case, places) = case_par_vanne(pool, arg(a, "salle"), arg(a, "vanne")).await?;
            let deja: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM truie WHERE case_id=? AND reformee=0 AND id<>?")
                .bind(case_id).bind(truie_id).fetch_one(pool).await.map_err(|e| e.to_string())?;
            let occupation = match places {
                Some(p) if deja >= p => format!(" ATTENTION : case déjà pleine ({deja}/{p})."),
                Some(p) => format!(" Après : {}/{p} places.", deja + 1),
                None => String::new(),
            };
            let resume = format!("Truie {num} → {salle}, case {case}.{occupation}");
            if a.get("confirme").and_then(Value::as_bool) != Some(true) {
                return Ok(format!("À CONFIRMER (rien n'est enregistré) : {resume}"));
            }
            sqlx::query("UPDATE truie SET case_id=?,salle_id=?,updated_at=CURRENT_TIMESTAMP WHERE id=?")
                .bind(case_id).bind(salle_id).bind(truie_id).execute(pool).await.map_err(|e| e.to_string())?;
            journaliser(pool, "deplacer_truie", &num, &resume).await;
            Ok(format!("Enregistré : {resume}"))
        }
        "creer_rappel" => {
            let titre = arg(a, "titre");
            let date = chrono::NaiveDate::parse_from_str(arg(a, "date"), "%Y-%m-%d").map_err(|_| "Date attendue au format AAAA-MM-JJ".to_string())?;
            if titre.is_empty() {
                return Err("Titre du rappel manquant.".into());
            }
            let bande = Some(arg(a, "bande")).filter(|b| !b.is_empty());
            let note = Some(arg(a, "note")).filter(|n| !n.is_empty());
            let resume = format!("Rappel « {titre} » le {}", date.format("%d/%m/%Y"));
            if a.get("confirme").and_then(Value::as_bool) != Some(true) {
                return Ok(format!("À CONFIRMER (rien n'est enregistré) : {resume}"));
            }
            sqlx::query("INSERT INTO tache(titre,type,bande_code,echeance,fait,note,cree_le) VALUES(?,'Rappel',?,?,0,?,CURRENT_TIMESTAMP)")
                .bind(titre).bind(bande).bind(date.to_string()).bind(note)
                .execute(pool).await.map_err(|e| e.to_string())?;
            journaliser(pool, "creer_rappel", titre, &resume).await;
            Ok(format!("Enregistré : {resume}"))
        }
        "rappels" => {
            let l: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as("SELECT titre,echeance,note FROM tache WHERE fait=0 ORDER BY echeance IS NULL,echeance LIMIT 50")
                .fetch_all(pool).await.map_err(|e| e.to_string())?;
            Ok(json!(l.into_iter().map(|(t, e, n)| json!({"titre": t, "echeance": e, "note": n})).collect::<Vec<_>>()).to_string())
        }
        "lister_tables" | "decrire_tables" | "requete" => {
            let mut cnx = assistant::connexion(&state.config.db_path).await?;
            let (_, r) = assistant::executer(&mut cnx, nom, a).await;
            if r.starts_with("ERREUR") { Err(r) } else { Ok(r) }
        }
        _ => Err(format!("Outil inconnu : {nom}")),
    }
}

/// Jeton valide ? Comparaison sur toute la longueur, jeton d'au moins 24 caractères.
async fn jeton_ok(pool: &SqlitePool, jeton: &str) -> bool {
    match crate::proxmox::parametre(pool, "mcp_jeton").await {
        Some(attendu) if attendu.len() >= 24 && attendu.len() == jeton.len() => {
            attendu.bytes().zip(jeton.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
        }
        _ => false,
    }
}

pub(super) async fn point_entree(
    State(state): State<AppState>,
    Path(jeton): Path<String>,
    Json(req): Json<Value>,
) -> Response {
    if !jeton_ok(&state.pool, &jeton).await {
        return StatusCode::NOT_FOUND.into_response();
    }
    let id = req.get("id").cloned();
    let methode = req["method"].as_str().unwrap_or("");
    let Some(id) = id else {
        // Notification (initialized, cancelled…) : rien à répondre.
        return StatusCode::ACCEPTED.into_response();
    };
    let resultat = match methode {
        "initialize" => {
            let demande = req["params"]["protocolVersion"].as_str().unwrap_or("");
            let version = PROTOCOLES.iter().find(|v| **v == demande).copied().unwrap_or(PROTOCOLES[0]);
            Ok(json!({"protocolVersion": version, "capabilities": {"tools": {}},
                      "serverInfo": {"name": "eo-sevel", "version": env!("CARGO_PKG_VERSION")},
                      "instructions": CONSIGNES}))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": outils()})),
        "tools/call" => {
            let nom = req["params"]["name"].as_str().unwrap_or("");
            let args = req["params"].get("arguments").cloned().unwrap_or(json!({}));
            let r = appeler(&state, nom, &args).await;
            tracing::info!(outil = nom, ok = r.is_ok(), "connecteur Claude");
            Ok(match r { Ok(t) => texte(t, false), Err(e) => texte(e, true) })
        }
        autre => Err(json!({"code": -32601, "message": format!("Méthode inconnue : {autre}")})),
    };
    let corps = match resultat {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
    };
    Json(corps).into_response()
}

pub(super) async fn get_refuse() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

/// `EO_TEST_BASE=/var/tmp/copie.db cargo test connecteur_sur_copie -- --ignored --nocapture`
/// (la copie est modifiée : truie 409353 déplacée, un rappel ajouté).
#[cfg(test)]
mod tests {
    use super::*;

    async fn rpc(state: &AppState, jeton: &str, req: Value) -> (StatusCode, Value) {
        let r = point_entree(State(state.clone()), Path(jeton.to_string()), Json(req)).await;
        let statut = r.status();
        let octets = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
        (statut, serde_json::from_slice(&octets).unwrap_or(Value::Null))
    }

    async fn outil(state: &AppState, jeton: &str, nom: &str, args: Value) -> (bool, String) {
        let (_, v) = rpc(state, jeton, json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":nom,"arguments":args}})).await;
        let t = v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string();
        println!("--- {nom} {args}\n{t}\n");
        (v["result"]["isError"] == true, t)
    }

    #[tokio::test]
    #[ignore]
    async fn connecteur_sur_copie() -> anyhow::Result<()> {
        let chemin = std::env::var("EO_TEST_BASE")?;
        let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&chemin).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(2).connect_with(options).await?;
        crate::db::init(&pool).await?;
        let jeton = "a".repeat(64);
        crate::sms_free::ecrire_parametre(&pool, "mcp_jeton", &jeton).await?;
        let state = AppState::new(
            Config { bind: "127.0.0.1:8080".parse()?, db_path: chemin.into(), secure_cookies: false },
            pool.clone(),
            crate::templates::build()?,
        );

        let (s, _) = rpc(&state, "mauvais", json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (_, v) = rpc(&state, &jeton, json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).await;
        assert_eq!(v["result"]["protocolVersion"], "2025-06-18");
        let (s, _) = rpc(&state, &jeton, json!({"jsonrpc":"2.0","method":"notifications/initialized"})).await;
        assert_eq!(s, StatusCode::ACCEPTED);
        let (_, v) = rpc(&state, &jeton, json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).await;
        assert!(v["result"]["tools"].as_array().unwrap().len() >= 9);

        let (e, _) = outil(&state, &jeton, "chercher_truie", json!({"numero":"409353"})).await;
        assert!(!e);
        let (e, _) = outil(&state, &jeton, "cases_salle", json!({"salle":"verraterie"})).await;
        assert!(!e);
        let (e, _) = outil(&state, &jeton, "calendrier_ia", json!({})).await;
        assert!(!e);
        let (e, t) = outil(&state, &jeton, "deplacer_truie", json!({"numero":"409353","salle":"Verraterie","vanne":"47"})).await;
        assert!(!e && t.starts_with("À CONFIRMER"));
        let avant: Option<i64> = sqlx::query_scalar("SELECT case_id FROM truie WHERE num_travail='409353'").fetch_one(&pool).await?;
        assert_ne!(avant, Some(123));
        let (e, t) = outil(&state, &jeton, "deplacer_truie", json!({"numero":"409353","salle":"Verraterie","vanne":"47","confirme":true})).await;
        assert!(!e && t.starts_with("Enregistré"));
        let apres: Option<i64> = sqlx::query_scalar("SELECT case_id FROM truie WHERE num_travail='409353'").fetch_one(&pool).await?;
        assert_eq!(apres, Some(123));
        let (e, _) = outil(&state, &jeton, "creer_rappel", json!({"titre":"Truie 409353 : mettre sous œstrogène","date":"2026-10-29","confirme":true})).await;
        assert!(!e);
        let (_, t) = outil(&state, &jeton, "rappels", json!({})).await;
        assert!(t.contains("œstrogène"));
        let (e, _) = outil(&state, &jeton, "requete", json!({"sql":"DELETE FROM truie"})).await;
        assert!(e);
        let (e, _) = outil(&state, &jeton, "deplacer_truie", json!({"numero":"999999","salle":"Verraterie","vanne":"47","confirme":true})).await;
        assert!(e);
        Ok(())
    }
}
