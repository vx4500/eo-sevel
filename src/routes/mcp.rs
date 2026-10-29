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
        {"name": "declarer_mort",
         "description": "Déclare des porcs morts dans une salle (engraissement, post-sevrage…, pas la maternité). Case facultative si la salle n'en a qu'une. Deux temps comme deplacer_truie.",
         "input_schema": {"type": "object", "properties": {
             "salle": {"type": "string"}, "case": {"type": "string", "description": "numéro de vanne ou nom de la case (facultatif)"},
             "nombre": {"type": "integer"}, "cause": {"type": "string", "description": "une cause configurée (la liste est renvoyée si elle est inconnue)"},
             "stade": {"type": "string", "description": "facultatif : déduit de la salle"},
             "date": {"type": "string", "description": "AAAA-MM-JJ, aujourd'hui par défaut"},
             "confirme": confirme}, "required": ["salle", "nombre", "cause"]}},
        {"name": "transferer_porcs",
         "description": "Transfère des porcs d'une salle (ou case) vers une autre ; les porcs gardent leur bande. Deux temps comme deplacer_truie.",
         "input_schema": {"type": "object", "properties": {
             "salle_source": {"type": "string"}, "case_source": {"type": "string", "description": "facultatif"},
             "salle_destination": {"type": "string"}, "case_destination": {"type": "string", "description": "facultatif si la salle n'a qu'une case"},
             "nombre": {"type": "integer"}, "bande": {"type": "string", "description": "facultatif : vérifie que les porcs sont de cette bande"},
             "date": {"type": "string", "description": "AAAA-MM-JJ, aujourd'hui par défaut"},
             "confirme": confirme}, "required": ["salle_source", "salle_destination", "nombre"]}},
        {"name": "sevrer_salle",
         "description": "Sevre toute une salle de maternité d'un coup (ex. « sèvre la Mater 1, porcelets en PS1, truies en verraterie ») : chaque portée ouverte de la salle est sevrée avec ses porcelets présents, les porcelets vont dans la case de post-sevrage indiquée et les truies dans la salle indiquée (verraterie par défaut, sans case précise). Deux temps comme deplacer_truie.",
         "input_schema": {"type": "object", "properties": {
             "salle": {"type": "string", "description": "salle de maternité, ex. « Mater 1 »"},
             "destination": {"type": "string", "description": "case(s) de post-sevrage pour les porcelets, ex. « PS1 » ou « PS1, PS2 » : les portées entières remplissent la première, puis la suivante"},
             "salle_truies": {"type": "string", "description": "salle où vont les truies, « Verraterie » par défaut"},
             "date": {"type": "string", "description": "AAAA-MM-JJ, aujourd'hui par défaut"},
             "confirme": confirme}, "required": ["salle", "destination"]}},
        {"name": "enregistrer_achat",
         "description": "Enregistre un achat (facture) dans l'économie, comme la saisie à la main de la page Économie : semence, vétérinaire, génétique ou aliment. Ex. « ajoute 510 € de semence LB-CIA à la B3.14 ». Deux temps comme deplacer_truie.",
         "input_schema": {"type": "object", "properties": {
             "categorie": {"type": "string", "enum": ["semence", "veto", "genetique", "aliment"]},
             "montant_ht": {"type": "number", "description": "montant hors taxes en euros"},
             "fournisseur": {"type": "string"},
             "designation": {"type": "string", "description": "produit ou désignation de la facture"},
             "quantite": {"type": "number", "description": "doses (semence), quantité (veto), animaux (génétique) ou tonnes (aliment)"},
             "bande": {"type": "string", "description": "code de la bande concernée, ex. « B3.14 » (facultatif)"},
             "date": {"type": "string", "description": "AAAA-MM-JJ, date de la facture ; aujourd'hui par défaut"},
             "num_facture": {"type": "string"}, "note": {"type": "string"},
             "confirme": confirme}, "required": ["categorie", "montant_ht"]}},
        {"name": "depart_porcs",
         "description": "Enregistre un départ de porcs à l'abattoir depuis une salle (pris dans les cases les plus pleines). Deux temps comme deplacer_truie.",
         "input_schema": {"type": "object", "properties": {
             "salle": {"type": "string"}, "case": {"type": "string", "description": "facultatif"},
             "nombre": {"type": "integer"},
             "date": {"type": "string", "description": "AAAA-MM-JJ, aujourd'hui par défaut"},
             "confirme": confirme}, "required": ["salle", "nombre"]}},
        {"name": "rappels",
         "description": "Rappels et tâches non faits, par échéance.",
         "input_schema": {"type": "object", "properties": {}}},
        {"name": "memoire",
         "description": "Ce que l'éleveur a demandé de retenir (habitudes, vocabulaire, consignes). Déjà fournies dans les consignes ; sert à retrouver un numéro pour oublier.",
         "input_schema": {"type": "object", "properties": {}}},
        {"name": "retenir",
         "description": "Retient une information pour les prochaines conversations (ex. « la vanne 12 de la Verraterie est en panne », « dire IA pour insémination »). À utiliser quand l'éleveur dit « retiens », « souviens-toi », ou explique une habitude durable.",
         "input_schema": {"type": "object", "properties": {"texte": {"type": "string"}}, "required": ["texte"]}},
        {"name": "oublier",
         "description": "Supprime une information retenue (numéro donné par memoire).",
         "input_schema": {"type": "object", "properties": {"numero": {"type": "integer"}}, "required": ["numero"]}},
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
N'invente jamais un numéro, une date ou une case : vérifie avec les outils. Pour une date relative aux IA, utilise calendrier_ia. \
Ce qui a été retenu est listé ci-dessous (pas besoin d'appeler memoire). Quand l'éleveur explique quelque chose de durable, retiens-le en une phrase courte (outil retenir).";

async fn table_memoire(pool: &SqlitePool) -> Result<(), String> {
    sqlx::query("CREATE TABLE IF NOT EXISTS assistant_memoire(id INTEGER PRIMARY KEY, texte TEXT NOT NULL, cree_le TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP)")
        .execute(pool).await.map(|_| ()).map_err(|e| e.to_string())
}

async fn notes_memoire(pool: &SqlitePool) -> Result<Vec<(i64, String, String)>, String> {
    table_memoire(pool).await?;
    sqlx::query_as("SELECT id,texte,substr(cree_le,1,10) FROM assistant_memoire ORDER BY id")
        .fetch_all(pool).await.map_err(|e| e.to_string())
}

fn notes_en_texte(notes: &[(i64, String, String)]) -> String {
    notes.iter().map(|(i, t, d)| format!("{i}. {t} (le {d})")).collect::<Vec<_>>().join("\n")
}

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

/// Porcs présents par case d'une salle (case facultative), la plus pleine
/// d'abord : (case_id, salle_id, salle, case, effectif). La maternité est
/// refusée : ses porcelets se comptent par portée, saisie dans EO_Sevel.
async fn cases_porcs(pool: &SqlitePool, salle: &str, case: &str) -> Result<Vec<(i64, i64, String, String, i64)>, String> {
    let lignes: Vec<(i64, i64, String, String, Option<String>)> = sqlx::query_as(
        "SELECT c.id,c.salle_id,s.nom,c.nom,s.type FROM casesalle c JOIN salle s ON s.id=c.salle_id \
         WHERE lower(s.nom) LIKE '%'||lower(?1)||'%' AND (?2='' OR c.num_vanne=?2 OR lower(c.nom)=lower(?2) OR c.nom LIKE ?2||' %') ORDER BY c.id",
    )
    .bind(salle).bind(case.trim_start_matches(['v', 'V']).trim())
    .fetch_all(pool).await.map_err(|e| e.to_string())?;
    if lignes.is_empty() {
        return Err(format!("Pas de salle « {salle} »{}. Utilise cases_salle ou lister les salles avec requete.",
            if case.is_empty() { String::new() } else { format!(" avec la case « {case} »") }));
    }
    let salles: std::collections::BTreeSet<&str> = lignes.iter().map(|l| l.2.as_str()).collect();
    if salles.len() > 1 {
        return Err(format!("Plusieurs salles correspondent : {}. Laquelle ?", salles.into_iter().collect::<Vec<_>>().join(", ")));
    }
    if lignes.iter().any(|l| stade_pour_type_salle(l.4.as_deref().unwrap_or("")).as_deref() == Some("Maternité")) {
        return Err("Salle de maternité : les porcelets se saisissent par portée dans EO_Sevel.".into());
    }
    let mut sortie = Vec::new();
    for (id, salle_id, s, c, _) in lignes {
        let n = case_pig_count(pool, id).await.map_err(|e| e.to_string())?;
        sortie.push((id, salle_id, s, c, n));
    }
    sortie.sort_by(|a, b| b.4.cmp(&a.4));
    Ok(sortie)
}

/// Répartit `nombre` porcs sur les cases (les plus pleines d'abord).
fn prelever(cases: &[(i64, i64, String, String, i64)], nombre: i64) -> Result<Vec<(i64, i64, i64)>, String> {
    let present: i64 = cases.iter().map(|c| c.4).sum();
    if nombre <= 0 {
        return Err("Le nombre doit être d'au moins 1.".into());
    }
    if nombre > present {
        return Err(format!("{} : {nombre} porcs demandés, {present} présents.", cases[0].2));
    }
    let mut reste = nombre;
    let mut pris = Vec::new();
    for (case_id, salle_id, _, _, n) in cases {
        if reste == 0 { break; }
        let p = reste.min(*n);
        if p > 0 { pris.push((*case_id, *salle_id, p)); reste -= p; }
    }
    Ok(pris)
}

fn date_ou_aujourdhui(a: &Value) -> Result<String, String> {
    let auj = chrono::Local::now().date_naive();
    let d = match arg(a, "date") {
        "" => auj,
        t => chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d").map_err(|_| "Date attendue au format AAAA-MM-JJ".to_string())?,
    };
    if d > auj { return Err("Date dans le futur.".into()); }
    Ok(d.to_string())
}

async fn code_bande(pool: &SqlitePool, bande_id: Option<i64>) -> Option<String> {
    sqlx::query_scalar("SELECT code FROM bande WHERE id=?").bind(bande_id?).fetch_optional(pool).await.ok().flatten()
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
        "sevrer_salle" => {
            let date = date_ou_aujourdhui(a)?;
            let salles: Vec<(i64, String)> = sqlx::query_as(
                "SELECT id,nom FROM salle WHERE lower(nom) LIKE '%'||lower(?1)||'%' AND lower(COALESCE(type,'')||' '||nom) LIKE '%mater%'",
            ).bind(arg(a, "salle")).fetch_all(pool).await.map_err(|e| e.to_string())?;
            let (salle_id, salle) = match salles.as_slice() {
                [une] => une.clone(),
                [] => return Err(format!("Pas de maternité « {} ».", arg(a, "salle"))),
                _ => return Err(format!("Plusieurs maternités correspondent : {}. Laquelle ?", salles.iter().map(|s| s.1.as_str()).collect::<Vec<_>>().join(", "))),
            };
            // Cases de post-sevrage, dans l'ordre dit : par leur nom (« PS1 », « PS 2 »)
            // ou par leur salle quand elle n'a qu'une case.
            let mut cases: Vec<(i64, String, i64, Option<i64>)> = Vec::new();
            for nom in arg(a, "destination").replace(" et ", ",").replace('+', ",").split(',').map(str::trim).filter(|n| !n.is_empty()) {
                let d = nom.to_lowercase().replace(' ', "");
                let trouvees: Vec<(i64, String, Option<i64>)> = sqlx::query_as(
                    "SELECT c.id,c.nom,c.nb_max_porcs FROM casesalle c JOIN salle s ON s.id=c.salle_id \
                     WHERE (lower(COALESCE(s.type,'')) LIKE '%sevr%' OR lower(s.nom) LIKE '%sevr%') \
                     AND (replace(lower(c.nom),' ','')=?1 OR (replace(lower(s.nom),' ','') LIKE '%'||?1||'%' AND (SELECT COUNT(*) FROM casesalle x WHERE x.salle_id=s.id)=1))",
                ).bind(&d).fetch_all(pool).await.map_err(|e| e.to_string())?;
                let (id, case, max) = match trouvees.as_slice() {
                    [une] => une.clone(),
                    [] => return Err(format!("Pas de case de post-sevrage « {nom} ».")),
                    _ => return Err(format!("Plusieurs cases correspondent à « {nom} » : {}. Laquelle ?", trouvees.iter().map(|c| c.1.as_str()).collect::<Vec<_>>().join(", "))),
                };
                let deja = case_pig_count(pool, id).await.map_err(|e| e.to_string())?.max(0);
                cases.push((id, case, deja, max.filter(|m| *m > 0)));
            }
            if cases.is_empty() {
                return Err("Dans quelle case de post-sevrage vont les porcelets ?".into());
            }
            let st = match arg(a, "salle_truies") { "" => "verrat", s => s };
            let (salle_truies_id, salle_truies): (i64, String) = sqlx::query_as(
                "SELECT id,nom FROM salle WHERE lower(COALESCE(type,'')||' '||nom) LIKE '%'||lower(?1)||'%' ORDER BY ordre LIMIT 1",
            ).bind(st).fetch_optional(pool).await.map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Pas de salle « {st} » pour les truies."))?;
            // Portées ouvertes de la maternité, par bande.
            let portees: Vec<(i64, i64, String, i64)> = sqlx::query_as(
                "SELECT pe.bande_id,t.id,t.num_travail,pe.presents FROM portee_effectif pe JOIN truie t ON t.id=pe.truie_id \
                 JOIN evenement e ON e.id=pe.id JOIN casesalle c ON c.id=COALESCE(e.case_id,t.case_id) \
                 WHERE c.salle_id=? AND pe.cloturee=0 AND pe.date<=? ORDER BY pe.bande_id,CAST(t.num_travail AS INTEGER)",
            ).bind(salle_id).bind(&date).fetch_all(pool).await.map_err(|e| e.to_string())?;
            if portees.is_empty() {
                return Err(format!("Aucune portée à sevrer en {salle}."));
            }
            // Chaque portée entière va dans la première case qui a encore la place.
            let mut remplissage: Vec<i64> = cases.iter().map(|c| c.2).collect();
            let mut affectation = Vec::new();
            for p in &portees {
                let i = (0..cases.len()).find(|&i| cases[i].3.is_none_or(|m| remplissage[i] + p.3 <= m))
                    .ok_or_else(|| format!("Plus de place pour la portée de {} ({} porcelets) : {}. Ajoute une case de post-sevrage.",
                        p.2, p.3, cases.iter().zip(&remplissage).map(|(c, r)| format!("{} {r}/{}", c.1, c.3.map(|m| m.to_string()).unwrap_or("?".into()))).collect::<Vec<_>>().join(", ")))?;
                remplissage[i] += p.3;
                affectation.push(i);
            }
            let total: i64 = portees.iter().map(|p| p.3).sum();
            let repartition = cases.iter().enumerate().filter(|(i, _)| affectation.contains(i)).map(|(i, c)| format!("{} : {} porcelet(s), {}{} après",
                c.1, remplissage[i] - c.2, remplissage[i], c.3.map(|m| format!("/{m}")).unwrap_or_default())).collect::<Vec<_>>().join(" ; ");
            let resume = format!(
                "Sevrage de {salle} le {date} : {} truie(s) ({}), {total} porcelet(s) → {repartition} ; truies → {salle_truies}.",
                portees.len(), portees.iter().map(|p| format!("{} : {}", p.2, p.3)).collect::<Vec<_>>().join(", "));
            if a.get("confirme").and_then(Value::as_bool) != Some(true) {
                return Ok(format!("À CONFIRMER (rien n'est enregistré) : {resume}"));
            }
            let mut bandes: Vec<i64> = portees.iter().map(|p| p.0).collect();
            bandes.dedup();
            for bande in bandes {
                let mut form: HashMap<String, String> = HashMap::from([
                    ("date".into(), date.clone()),
                    ("salle_truies".into(), salle_truies_id.to_string()),
                ]);
                for (p, i) in portees.iter().zip(&affectation).filter(|(p, _)| p.0 == bande) {
                    form.insert(format!("sel_{}", p.1), "on".into());
                    form.insert(format!("nb_{}", p.1), p.3.to_string());
                    form.insert(format!("case_{}", p.1), cases[*i].0.to_string());
                }
                super::sevrage::sevrer(pool, AUTEUR, bande, &form).await.map_err(|e| e.to_string())?;
            }
            journaliser(pool, "sevrer_salle", &salle, &resume).await;
            Ok(format!("Enregistré : {resume}"))
        }
        "enregistrer_achat" => {
            let categorie = arg(a, "categorie");
            let montant = a.get("montant_ht").and_then(Value::as_f64).filter(|m| *m > 0.0)
                .ok_or("Montant HT manquant.")?;
            let date = date_ou_aujourdhui(a)?;
            let quantite = a.get("quantite").and_then(Value::as_f64);
            let texte_ou = |k: &str| Some(arg(a, k).to_string()).filter(|t| !t.is_empty());
            let (fournisseur, designation, num, note) = (texte_ou("fournisseur"), texte_ou("designation"), texte_ou("num_facture"), texte_ou("note"));
            let bande: Option<(i64, String)> = match arg(a, "bande") {
                "" => None,
                b => Some(sqlx::query_as("SELECT id,code FROM bande WHERE lower(replace(code,' ',''))=lower(replace(?,' ',''))")
                    .bind(b).fetch_optional(pool).await.map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("Pas de bande « {b} ».") )?),
            };
            let libelle = match categorie {
                "semence" => "semence", "veto" => "vétérinaire", "genetique" => "génétique", "aliment" => "aliment",
                _ => return Err("Catégorie : semence, veto, genetique ou aliment.".into()),
            };
            let resume = format!("Achat {libelle} du {date} : {montant:.2} € HT{}{}{}{}.",
                fournisseur.as_deref().map(|f| format!(", {f}")).unwrap_or_default(),
                designation.as_deref().map(|d| format!(", {d}")).unwrap_or_default(),
                quantite.map(|q| format!(", quantité {q}")).unwrap_or_default(),
                bande.as_ref().map(|b| format!(", bande {}", b.1)).unwrap_or_else(|| ", sans bande".into()));
            if a.get("confirme").and_then(Value::as_bool) != Some(true) {
                return Ok(format!("À CONFIRMER (rien n'est enregistré) : {resume}"));
            }
            let bande_id = bande.as_ref().map(|b| b.0);
            let requete = match categorie {
                "semence" => sqlx::query("INSERT INTO achatsemence(date,num_facture,fournisseur,designation,nb_doses,montant_ht,bande_id,note) VALUES(?,?,?,?,?,?,?,?)")
                    .bind(&date).bind(&num).bind(&fournisseur).bind(&designation).bind(quantite.map(|q| q.round() as i64)).bind(montant).bind(bande_id).bind(&note),
                "veto" => sqlx::query("INSERT INTO achatveto(date,produit,quantite,montant_ht,num_facture,fournisseur,bande_id) VALUES(?,?,?,?,?,?,?)")
                    .bind(&date).bind(&designation).bind(quantite).bind(montant).bind(&num).bind(&fournisseur).bind(bande_id),
                "genetique" => sqlx::query("INSERT INTO achatgenetique(date,num_facture,fournisseur,designation,nb_animaux,montant_ht,bande_code,note) VALUES(?,?,?,?,?,?,?,?)")
                    .bind(&date).bind(&num).bind(&fournisseur).bind(&designation).bind(quantite.map(|q| q.round() as i64)).bind(montant).bind(bande.as_ref().map(|b| b.1.clone())).bind(&note),
                _ => sqlx::query("INSERT INTO livraisonaliment(date,fournisseur,produit,tonnage,montant_ht,num_facture,bande_id) VALUES(?,?,?,?,?,?,?)")
                    .bind(&date).bind(&fournisseur).bind(&designation).bind(quantite).bind(montant).bind(&num).bind(bande_id),
            };
            requete.execute(pool).await.map_err(|e| e.to_string())?;
            crate::db::auto_assign_economic_invoices(pool).await.map_err(|e| e.to_string())?;
            journaliser(pool, "enregistrer_achat", libelle, &resume).await;
            Ok(format!("Enregistré : {resume}"))
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
        "declarer_mort" => {
            let date = date_ou_aujourdhui(a)?;
            let nombre = a.get("nombre").and_then(Value::as_i64).unwrap_or(0);
            let cases = cases_porcs(pool, arg(a, "salle"), arg(a, "case")).await?;
            if cases.len() > 1 && arg(a, "case").is_empty() && nombre > 0 && cases[0].4 < nombre {
                return Err(format!("Précise la case : {}.", cases.iter().map(|c| format!("{} ({} porcs)", c.3, c.4)).collect::<Vec<_>>().join(", ")));
            }
            let (case_id, _, salle, case, present) = cases[0].clone();
            prelever(&cases[..1], nombre)?;
            let causes: Vec<String> = sqlx::query_scalar("SELECT libelle FROM causeperte ORDER BY libelle")
                .fetch_all(pool).await.map_err(|e| e.to_string())?;
            let cause = causes.iter().find(|c| c.to_lowercase() == arg(a, "cause").to_lowercase())
                .ok_or_else(|| format!("Cause inconnue « {} ». Causes possibles : {}.", arg(a, "cause"), causes.join(", ")))?.clone();
            let stade = Some(arg(a, "stade").to_string()).filter(|s| !s.is_empty())
                .or(stade_from_case(pool, case_id).await.map_err(|e| e.to_string())?)
                .map(|s| normalize_death_stage(&s));
            let bande = code_bande(pool, bande_de_la_case(pool, case_id, &date).await.map_err(|e| e.to_string())?).await;
            let resume = format!("{nombre} porc(s) mort(s) le {date} en {salle}, case {case} ({}), cause {cause}{}. Reste {} porcs dans la case.",
                stade.as_deref().unwrap_or("stade inconnu"), bande.as_deref().map(|b| format!(", bande {b}")).unwrap_or_default(), present - nombre);
            if a.get("confirme").and_then(Value::as_bool) != Some(true) {
                return Ok(format!("À CONFIRMER (rien n'est enregistré) : {resume}"));
            }
            sqlx::query("INSERT INTO declarationmort(horodatage,bande_code,date,stade,case_id,cause,nombre,declare_par) VALUES(CURRENT_TIMESTAMP,?,?,?,?,?,?,?)")
                .bind(&bande).bind(&date).bind(&stade).bind(case_id).bind(&cause).bind(nombre).bind(AUTEUR)
                .execute(pool).await.map_err(|e| e.to_string())?;
            journaliser(pool, "declarer_mort", &salle, &resume).await;
            Ok(format!("Enregistré : {resume}"))
        }
        "transferer_porcs" => {
            let date = date_ou_aujourdhui(a)?;
            let nombre = a.get("nombre").and_then(Value::as_i64).unwrap_or(0);
            let sources = cases_porcs(pool, arg(a, "salle_source"), arg(a, "case_source")).await?;
            let dest = cases_porcs(pool, arg(a, "salle_destination"), arg(a, "case_destination")).await?;
            if dest.len() > 1 {
                return Err(format!("Précise la case d'arrivée : {}.", dest.iter().map(|c| c.3.as_str()).collect::<Vec<_>>().join(", ")));
            }
            let (dest_case, dest_salle, dest_nom, dest_case_nom, dest_n) = dest[0].clone();
            let sources: Vec<_> = sources.into_iter().filter(|c| c.0 != dest_case).collect();
            if sources.is_empty() {
                return Err("La source et la destination sont identiques.".into());
            }
            let pris = prelever(&sources, nombre)?;
            let mut lignes = Vec::new();
            for (case_id, salle_id, n) in pris {
                let bande_id = bande_de_la_case(pool, case_id, &date).await.map_err(|e| e.to_string())?;
                let code = code_bande(pool, bande_id).await;
                let voulue = arg(a, "bande");
                if !voulue.is_empty() && code.as_deref().map(|c| c.to_lowercase()) != Some(voulue.to_lowercase()) {
                    return Err(format!("Les porcs de {} sont de la bande {}, pas {voulue}.", sources[0].2, code.as_deref().unwrap_or("inconnue")));
                }
                lignes.push((case_id, salle_id, n, bande_id, code));
            }
            let bandes: std::collections::BTreeSet<String> = lignes.iter().filter_map(|l| l.4.clone()).collect();
            let resume = format!("{nombre} porc(s) de {} vers {dest_nom}, case {dest_case_nom}, le {date}{}. Après : {} porcs à l'arrivée.",
                sources[0].2, if bandes.is_empty() { String::new() } else { format!(", bande {}", bandes.into_iter().collect::<Vec<_>>().join(", ")) }, dest_n + nombre);
            if a.get("confirme").and_then(Value::as_bool) != Some(true) {
                return Ok(format!("À CONFIRMER (rien n'est enregistré) : {resume}"));
            }
            let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
            for (case_id, salle_id, n, bande_id, _) in lignes {
                sqlx::query("INSERT INTO transfert(date,espece,bande_id,salle_source_id,salle_dest_id,case_source_id,case_dest_id,nombre,note) VALUES(?,'porc',?,?,?,?,?,?,?)")
                    .bind(&date).bind(bande_id).bind(salle_id).bind(dest_salle).bind(case_id).bind(dest_case).bind(n).bind(AUTEUR)
                    .execute(&mut *tx).await.map_err(|e| e.to_string())?;
            }
            tx.commit().await.map_err(|e| e.to_string())?;
            journaliser(pool, "transferer_porcs", &dest_nom, &resume).await;
            Ok(format!("Enregistré : {resume}"))
        }
        "depart_porcs" => {
            let date = date_ou_aujourdhui(a)?;
            let nombre = a.get("nombre").and_then(Value::as_i64).unwrap_or(0);
            let cases = cases_porcs(pool, arg(a, "salle"), arg(a, "case")).await?;
            let pris = prelever(&cases, nombre)?;
            let present: i64 = cases.iter().map(|c| c.4).sum();
            let resume = format!("Départ abattoir le {date} : {nombre} porc(s) de {}. Reste {} porcs.", cases[0].2, present - nombre);
            if a.get("confirme").and_then(Value::as_bool) != Some(true) {
                return Ok(format!("À CONFIRMER (rien n'est enregistré) : {resume}"));
            }
            // Même forme que la confirmation d'un départ prévu (/departs) :
            // sortie sans case d'arrivée, note « Départ du … ».
            let note = format!("Départ du {} ({AUTEUR})", chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").map(|d| d.format("%d/%m").to_string()).unwrap_or_default());
            let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
            for (case_id, salle_id, n) in pris {
                let bande_id = bande_de_la_case(pool, case_id, &date).await.map_err(|e| e.to_string())?;
                sqlx::query("INSERT INTO transfert(date,espece,bande_id,salle_source_id,case_source_id,nombre,note) VALUES(?,'porc',?,?,?,?,?)")
                    .bind(&date).bind(bande_id).bind(salle_id).bind(case_id).bind(n).bind(&note)
                    .execute(&mut *tx).await.map_err(|e| e.to_string())?;
            }
            tx.commit().await.map_err(|e| e.to_string())?;
            journaliser(pool, "depart_porcs", &cases[0].2, &resume).await;
            Ok(format!("Enregistré : {resume}"))
        }
        "rappels" => {
            let l: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as("SELECT titre,echeance,note FROM tache WHERE fait=0 ORDER BY echeance IS NULL,echeance LIMIT 50")
                .fetch_all(pool).await.map_err(|e| e.to_string())?;
            Ok(json!(l.into_iter().map(|(t, e, n)| json!({"titre": t, "echeance": e, "note": n})).collect::<Vec<_>>()).to_string())
        }
        "memoire" => {
            let notes = notes_memoire(pool).await?;
            Ok(if notes.is_empty() { "Rien n'a encore été retenu.".into() } else { notes_en_texte(&notes) })
        }
        "retenir" => {
            let t = arg(a, "texte");
            if t.is_empty() { return Err("Texte vide.".into()); }
            if t.chars().count() > 200 { return Err("Trop long : une phrase courte (200 caractères au plus).".into()); }
            if notes_memoire(pool).await?.len() >= 40 { return Err("Déjà 40 informations retenues : fais-en oublier une d'abord (memoire puis oublier).".into()); }
            table_memoire(pool).await?;
            sqlx::query("INSERT INTO assistant_memoire(texte) VALUES(?)").bind(t)
                .execute(pool).await.map_err(|e| e.to_string())?;
            journaliser(pool, "retenir", "mémoire", t).await;
            Ok(format!("Retenu : {t}"))
        }
        "oublier" => {
            let n = a.get("numero").and_then(Value::as_i64).ok_or("Numéro manquant.")?;
            table_memoire(pool).await?;
            let r = sqlx::query("DELETE FROM assistant_memoire WHERE id=?").bind(n)
                .execute(pool).await.map_err(|e| e.to_string())?;
            if r.rows_affected() == 0 { return Err(format!("Pas d'information n° {n}.")); }
            journaliser(pool, "oublier", "mémoire", &n.to_string()).await;
            Ok(format!("Information n° {n} oubliée."))
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
            let notes = notes_memoire(&state.pool).await.unwrap_or_default();
            let consignes = if notes.is_empty() { CONSIGNES.to_string() }
                else { format!("{CONSIGNES}\n\nDéjà retenu :\n{}", notes_en_texte(&notes)) };
            Ok(json!({"protocolVersion": version, "capabilities": {"tools": {}},
                      "serverInfo": {"name": "eo-sevel", "version": env!("CARGO_PKG_VERSION")},
                      "instructions": consignes}))
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
        let (e, _) = outil(&state, &jeton, "retenir", json!({"texte":"La vanne 12 de la Verraterie est en panne"})).await;
        assert!(!e);
        let (_, t) = outil(&state, &jeton, "memoire", json!({})).await;
        assert!(t.contains("vanne 12"));
        let (_, v) = rpc(&state, &jeton, json!({"jsonrpc":"2.0","id":3,"method":"initialize","params":{}})).await;
        assert!(v["result"]["instructions"].as_str().unwrap().contains("vanne 12"));
        let (e, _) = outil(&state, &jeton, "requete", json!({"sql":"DELETE FROM truie"})).await;
        assert!(e);
        let (e, _) = outil(&state, &jeton, "deplacer_truie", json!({"numero":"999999","salle":"Verraterie","vanne":"47","confirme":true})).await;
        assert!(e);

        // Porcs : morts, transferts, départs (deux temps, contrôles d'effectif).
        let morts = |p: &SqlitePool| { let p = p.clone(); async move { sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM declarationmort").fetch_one(&p).await.unwrap() } };
        let transferts = |p: &SqlitePool| { let p = p.clone(); async move { sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM transfert").fetch_one(&p).await.unwrap() } };
        let (m0, t0) = (morts(&pool).await, transferts(&pool).await);
        let (e, t) = outil(&state, &jeton, "declarer_mort", json!({"salle":"Engraissement 5","nombre":1,"cause":"respiratoire"})).await;
        assert!(!e && t.starts_with("À CONFIRMER"));
        assert_eq!(morts(&pool).await, m0);
        let (e, t) = outil(&state, &jeton, "declarer_mort", json!({"salle":"Engraissement 5","nombre":1,"cause":"respiratoire","confirme":true})).await;
        assert!(!e && t.starts_with("Enregistré"));
        assert_eq!(morts(&pool).await, m0 + 1);
        for args in [json!({"salle":"Engraissement 5","nombre":0,"cause":"Respiratoire","confirme":true}),
                     json!({"salle":"Engraissement 5","nombre":10000,"cause":"Respiratoire","confirme":true}),
                     json!({"salle":"Engraissement 5","nombre":1,"cause":"grippe","confirme":true}),
                     json!({"salle":"Nulle part","nombre":1,"cause":"Respiratoire","confirme":true}),
                     json!({"salle":"Mater 1","nombre":1,"cause":"Respiratoire","confirme":true}),
                     json!({"salle":"Engraissement","nombre":1,"cause":"Respiratoire","confirme":true}),
                     json!({"salle":"Engraissement 5","nombre":1,"cause":"Respiratoire","date":"2099-01-01","confirme":true})] {
            let (e, _) = outil(&state, &jeton, "declarer_mort", args).await;
            assert!(e);
        }
        assert_eq!(morts(&pool).await, m0 + 1);
        let (e, t) = outil(&state, &jeton, "transferer_porcs", json!({"salle_source":"Engraissement 5","salle_destination":"Engraissement 4","nombre":5})).await;
        assert!(!e && t.starts_with("À CONFIRMER"));
        let (e, _) = outil(&state, &jeton, "transferer_porcs", json!({"salle_source":"Engraissement 5","salle_destination":"Engraissement 4","nombre":5,"bande":"ZZZ","confirme":true})).await;
        assert!(e);
        let (e, _) = outil(&state, &jeton, "transferer_porcs", json!({"salle_source":"Engraissement 5","salle_destination":"Engraissement 5","nombre":5,"confirme":true})).await;
        assert!(e);
        let (e, _) = outil(&state, &jeton, "transferer_porcs", json!({"salle_source":"Engraissement 5","salle_destination":"35G6K","nombre":5,"confirme":true})).await;
        assert!(e, "deux cases d'arrivée : il faut préciser");
        assert_eq!(transferts(&pool).await, t0);
        let (e, t) = outil(&state, &jeton, "transferer_porcs", json!({"salle_source":"Engraissement 5","salle_destination":"Engraissement 4","nombre":5,"confirme":true})).await;
        assert!(!e && t.starts_with("Enregistré"));
        assert_eq!(transferts(&pool).await, t0 + 1);
        let (e, t) = outil(&state, &jeton, "depart_porcs", json!({"salle":"35G6K","nombre":10})).await;
        assert!(!e && t.starts_with("À CONFIRMER"));
        let (e, _) = outil(&state, &jeton, "depart_porcs", json!({"salle":"35G6K","nombre":100000,"confirme":true})).await;
        assert!(e);
        let (e, t) = outil(&state, &jeton, "depart_porcs", json!({"salle":"35G6K","nombre":10,"confirme":true})).await;
        assert!(!e && t.starts_with("Enregistré"));
        let n: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(nombre),0) FROM transfert WHERE case_dest_id IS NULL AND note LIKE 'Départ du%' AND id>(SELECT MAX(id)-5 FROM transfert)").fetch_one(&pool).await?;
        assert_eq!(n, 10);
        Ok(())
    }

    /// `EO_TEST_BASE=/var/tmp/copie.db cargo test sevrage_salle_sur_copie -- --ignored --nocapture`
    /// (la copie est modifiée : la Mater 1 est sevrée).
    #[tokio::test]
    #[ignore]
    async fn sevrage_salle_sur_copie() -> anyhow::Result<()> {
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
        let ouvertes = |p: &SqlitePool| { let p = p.clone(); async move {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM portee_effectif pe JOIN truie t ON t.id=pe.truie_id JOIN casesalle c ON c.id=t.case_id WHERE c.salle_id=7 AND pe.cloturee=0").fetch_one(&p).await.unwrap() } };
        let avant = ouvertes(&pool).await;
        assert!(avant > 0);
        // Comme la semaine du sevrage : PS1 et PS 2 viennent d'être vidées.
        sqlx::query("INSERT INTO inventairecase(case_id,date,nombre) SELECT c.id,date('now','localtime'),0 FROM casesalle c WHERE c.nom IN ('PS1','PS 2')")
            .execute(&pool).await?;
        let (e, _) = outil(&state, &jeton, "sevrer_salle", json!({"salle":"Mater 9","destination":"PS1"})).await;
        assert!(e);
        let (e, _) = outil(&state, &jeton, "sevrer_salle", json!({"salle":"Mater 1","destination":"Engraissement 1"})).await;
        assert!(e, "la destination doit être du post-sevrage");
        let (e, t) = outil(&state, &jeton, "sevrer_salle", json!({"salle":"mater 1","destination":"PS1 et PS 2"})).await;
        assert!(!e && t.starts_with("À CONFIRMER") && t.contains("Verraterie"));
        assert_eq!(ouvertes(&pool).await, avant);
        let (e, t) = outil(&state, &jeton, "sevrer_salle", json!({"salle":"mater 1","destination":"PS1 et PS 2","confirme":true})).await;
        assert!(!e && t.starts_with("Enregistré"), "{t}");
        assert_eq!(ouvertes(&pool).await, 0);

        // Achat à la voix : semence affectée à une bande.
        let (e, t) = outil(&state, &jeton, "enregistrer_achat", json!({"categorie":"semence","montant_ht":510,"fournisseur":"LB-CIA Piétrain","quantite":80,"bande":"b3.14","date":"2025-12-08"})).await;
        assert!(!e && t.starts_with("À CONFIRMER") && t.contains("B3.14"));
        let (e, _) = outil(&state, &jeton, "enregistrer_achat", json!({"categorie":"semence","montant_ht":510,"bande":"B9.99","confirme":true})).await;
        assert!(e);
        let (e, t) = outil(&state, &jeton, "enregistrer_achat", json!({"categorie":"semence","montant_ht":510,"fournisseur":"LB-CIA Piétrain","quantite":80,"bande":"B3.14","date":"2025-12-08","confirme":true})).await;
        assert!(!e && t.starts_with("Enregistré"), "{t}");
        let affecte: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM achatsemence s JOIN affectationfacturebande a ON a.categorie='semence' AND a.facture_id=s.id JOIN bande b ON b.id=a.bande_id WHERE b.code='B3.14' AND s.montant_ht=510").fetch_one(&pool).await?;
        assert!(affecte >= 1);
        let en_mater: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM truie t JOIN casesalle c ON c.id=t.case_id WHERE c.salle_id=7 AND t.reformee=0").fetch_one(&pool).await?;
        println!("truies encore rangées dans une case de la Mater 1 : {en_mater}");
        Ok(())
    }
}
