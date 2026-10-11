//! Mes équipements → Machine à soupe (module facultatif `module_machine_soupe`,
//! activé dans Paramètres). Suivi en écoute seule, voir src/soupe.rs.

use super::*;

async fn actif(pool: &SqlitePool) -> AppResult<bool> {
    module_actif(pool, "module_machine_soupe", false).await
}

/// Jeton partagé avec le service d'écoute de l'hôte Proxmox, créé au besoin.
async fn jeton(pool: &SqlitePool) -> AppResult<String> {
    let existant: Option<String> = sqlx::query_scalar("SELECT valeur FROM parametre WHERE cle='soupe_jeton' AND length(valeur)>=32")
        .fetch_optional(pool)
        .await?;
    if let Some(j) = existant {
        return Ok(j);
    }
    let nouveau = uuid::Uuid::new_v4().simple().to_string();
    sqlx::query("INSERT INTO parametre(cle,valeur) VALUES('soupe_jeton',?) ON CONFLICT(cle) DO UPDATE SET valeur=excluded.valeur")
        .bind(&nouveau)
        .execute(pool)
        .await?;
    Ok(nouveau)
}

async fn jeton_valide(pool: &SqlitePool, recu: Option<&str>) -> bool {
    let attendu: Option<String> = sqlx::query_scalar("SELECT valeur FROM parametre WHERE cle='soupe_jeton' AND length(valeur)>=32")
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    match (attendu, recu) {
        (Some(a), Some(r)) => subtle::ConstantTimeEq::ct_eq(a.as_bytes(), r.as_bytes()).into(),
        _ => false,
    }
}

fn adresse(headers: &HeaderMap) -> String {
    let hote = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("192.168.1.186:8080");
    format!("http://{hote}")
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    headers: HeaderMap,
) -> AppResult<Html<String>> {
    let pool = &state.pool;
    let mut ctx = context(&session);
    let actif = actif(pool).await?;
    ctx.insert("actif".into(), json!(actif));
    if actif {
        let etat = generic_rows(pool, "SELECT * FROM soupe_etat WHERE id=1").await?.into_iter().next();
        if let Some(Value::Object(mut e)) = etat {
            let ts = e.get("ts").and_then(Value::as_i64).unwrap_or(0);
            let phase = e.get("phase").and_then(Value::as_i64).unwrap_or(0);
            e.insert("libelle".into(), json!(crate::soupe::libelle_phase(phase)));
            e.insert("age_s".into(), json!(chrono::Utc::now().timestamp() - ts));
            e.insert("heure".into(), json!(chrono::DateTime::from_timestamp(ts, 0).map(|d| d.with_timezone(&Local).format("%d/%m %H:%M:%S").to_string())));
            ctx.insert("etat".into(), Value::Object(e));
        }
        ctx.insert(
            "distributions".into(),
            Value::Array(
                generic_rows(
                    pool,
                    "SELECT jour, strftime('%H:%M', ts, 'unixepoch', 'localtime') AS heure, sequence, formule, passage, vanne, consigne, distribue \
                     FROM soupe_distribution WHERE jour >= date('now','localtime','-1 day') ORDER BY ts DESC LIMIT 200",
                )
                .await?,
            ),
        );
        ctx.insert(
            "totaux".into(),
            Value::Array(
                generic_rows(
                    pool,
                    "SELECT jour, sequence, formule, COUNT(*) AS vannes, round(SUM(prevu),1) AS consigne, round(SUM(servi),1) AS distribue, \
                     strftime('%H:%M', MIN(debut), 'unixepoch', 'localtime') AS debut, strftime('%H:%M', MAX(fin), 'unixepoch', 'localtime') AS fin FROM \
                     (SELECT jour, sequence, formule, vanne, MAX(consigne) AS prevu, SUM(distribue) AS servi, MIN(ts) AS debut, MAX(ts) AS fin \
                      FROM soupe_distribution WHERE jour >= date('now','localtime','-7 day') GROUP BY jour, sequence, formule, vanne) \
                     GROUP BY jour, sequence, formule ORDER BY MIN(debut) DESC LIMIT 60",
                )
                .await?,
            ),
        );
        ctx.insert(
            "evenements".into(),
            Value::Array(
                generic_rows(
                    pool,
                    "SELECT strftime('%d/%m %H:%M:%S', ts, 'unixepoch', 'localtime') AS heure, phase, message FROM soupe_evenement ORDER BY id DESC LIMIT 60",
                )
                .await?,
            ),
        );
        let produits = crate::soupe::produits(pool).await;
        if let Some(Value::Object(e)) = ctx.get_mut("etat") {
            let n = e.get("produit").and_then(Value::as_u64).unwrap_or(0) as usize;
            if let Some(nom) = produits.get(n) {
                e.insert("produit_nom".into(), json!(nom));
            }
        }
        ctx.insert("produits".into(), json!(produits.iter().enumerate().take(16).filter(|(_, p)| p.chars().any(char::is_alphabetic) && p.as_str() != "Nc" && !p.starts_with("Produit_")).map(|(i, p)| json!({"n": i, "nom": p})).collect::<Vec<_>>()));
        ctx.insert(
            "synchro".into(),
            json!(sqlx::query_scalar::<_, Option<String>>("SELECT strftime('%d/%m/%Y %H:%M', MAX(ts), 'unixepoch', 'localtime') FROM soupe_donnee")
                .fetch_one(pool)
                .await?),
        );
        if session.est_admin() {
            ctx.insert("jeton".into(), json!(jeton(pool).await?));
            ctx.insert("adresse".into(), json!(adresse(&headers)));
        }
    }
    render(&state, "machine_soupe.html", Value::Object(ctx))
}

/// Trames envoyées par le service d'écoute (une par ligne).
pub(super) async fn trames(State(state): State<AppState>, headers: HeaderMap, corps: String) -> Response {
    let recu = headers.get("x-jeton").and_then(|v| v.to_str().ok());
    if !jeton_valide(&state.pool, recu).await {
        return (StatusCode::FORBIDDEN, "jeton invalide").into_response();
    }
    if !actif(&state.pool).await.unwrap_or(false) {
        return (StatusCode::OK, "module désactivé").into_response();
    }
    match crate::soupe::recevoir(&state.pool, &corps).await {
        Ok(n) => (StatusCode::OK, format!("{n}")).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "trames de la machine à soupe refusées");
            (StatusCode::INTERNAL_SERVER_ERROR, "erreur").into_response()
        }
    }
}

/// Programme d'écoute pour l'hôte Proxmox, jeton et adresse inclus.
pub(super) async fn script(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let recu = query.get("jeton").map(String::as_str);
    if !jeton_valide(&state.pool, recu).await {
        return (StatusCode::FORBIDDEN, "jeton invalide").into_response();
    }
    let texte = include_str!("../../scripts/soupe-ecoute.py")
        .replace("__URL__", &adresse(&headers))
        .replace("__JETON__", recu.unwrap_or_default());
    ([(header::CONTENT_TYPE, "text/x-python; charset=utf-8")], texte).into_response()
}

/// Tableau des consommations : soupe servie (par jour, vanne, formule) et
/// quantités dosées dans la cuve par produit (eau et aliments).
pub(super) async fn consommations(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(q): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let pool = &state.pool;
    let jours: i64 = q.get("jours").and_then(|v| v.parse().ok()).filter(|j| [7, 30, 90, 365].contains(j)).unwrap_or(30);
    let depuis = format!("-{} day", jours - 1);
    let produits = crate::soupe::produits(pool).await;
    let nom_produit = |n: i64| -> String {
        if n == 0 {
            return "Eau".into();
        }
        produits.get(n as usize).filter(|p| p.chars().any(char::is_alphabetic) && !p.starts_with("Produit_")).cloned().unwrap_or_else(|| format!("Produit {n}"))
    };
    // Produits réellement dosés sur la période : une colonne chacun, l'eau d'abord.
    let codes: Vec<i64> = sqlx::query_scalar("SELECT DISTINCT produit FROM soupe_dosage WHERE jour>=date('now','localtime',?) ORDER BY produit")
        .bind(&depuis).fetch_all(pool).await?;
    let dosages: Vec<(String, i64, f64)> = sqlx::query_as("SELECT jour, produit, SUM(kg) FROM soupe_dosage WHERE jour>=date('now','localtime',?) GROUP BY jour, produit")
        .bind(&depuis).fetch_all(pool).await?;
    let servis: Vec<(String, f64, f64, i64, i64)> = sqlx::query_as(
        "SELECT jour, COALESCE(SUM(distribue),0), COALESCE(SUM(consigne),0), COUNT(DISTINCT vanne), COUNT(DISTINCT sequence||'-'||formule) \
         FROM soupe_distribution WHERE jour>=date('now','localtime',?) GROUP BY jour",
    ).bind(&depuis).fetch_all(pool).await?;
    let mut lignes = Vec::new();
    let mut totaux = vec![0.0; codes.len()];
    let (mut t_servi, mut t_prevu) = (0.0, 0.0);
    for i in 0..jours {
        let jour = (Local::now().date_naive() - Duration::days(i)).format("%Y-%m-%d").to_string();
        let s = servis.iter().find(|s| s.0 == jour);
        let par_produit: Vec<Option<f64>> = codes.iter().enumerate().map(|(k, c)| {
            let v = dosages.iter().filter(|d| d.0 == jour && d.1 == *c).map(|d| d.2).sum::<f64>();
            totaux[k] += v;
            (v > 0.0).then_some(v)
        }).collect();
        if s.is_none() && par_produit.iter().all(Option::is_none) {
            continue;
        }
        let (servi, prevu, vannes, repas) = s.map(|s| (s.1, s.2, s.3, s.4)).unwrap_or((0.0, 0.0, 0, 0));
        t_servi += servi;
        t_prevu += prevu;
        lignes.push(json!({"jour": jour, "servi": servi, "prevu": prevu, "ecart": (prevu > 0.0).then(|| (servi - prevu) / prevu * 100.0),
            "vannes": vannes, "repas": repas, "produits": par_produit}));
    }
    let par_vanne = generic_rows_bind_text(pool,
        "SELECT vanne, COUNT(*) AS passages, round(SUM(distribue),1) AS servi, round(SUM(consigne),1) AS prevu, round(SUM(distribue)/COUNT(DISTINCT jour),1) AS moyenne \
         FROM soupe_distribution WHERE jour>=date('now','localtime',?) GROUP BY vanne ORDER BY vanne", &depuis).await?;
    let par_formule = generic_rows_bind_text(pool,
        "SELECT formule, COUNT(*) AS passages, round(SUM(distribue),1) AS servi, round(SUM(consigne),1) AS prevu \
         FROM soupe_distribution WHERE jour>=date('now','localtime',?) GROUP BY formule ORDER BY formule", &depuis).await?;
    let debut_dosage: Option<String> = sqlx::query_scalar("SELECT MIN(jour) FROM soupe_dosage").fetch_one(pool).await?;
    let mut ctx = context(&session);
    ctx.insert("jours".into(), json!(jours));
    ctx.insert("colonnes".into(), json!(codes.iter().map(|c| nom_produit(*c)).collect::<Vec<_>>()));
    ctx.insert("lignes".into(), Value::Array(lignes));
    ctx.insert("totaux".into(), json!({"servi": t_servi, "prevu": t_prevu, "produits": totaux}));
    ctx.insert("par_vanne".into(), Value::Array(par_vanne));
    ctx.insert("par_formule".into(), Value::Array(par_formule));
    ctx.insert("debut_dosage".into(), json!(debut_dosage));
    render(&state, "soupe_consommations.html", Value::Object(ctx))
}

async fn generic_rows_bind_text(pool: &SqlitePool, sql: &str, valeur: &str) -> AppResult<Vec<Value>> {
    let lignes = sqlx::query(sql).bind(valeur).fetch_all(pool).await?;
    Ok(lignes.iter().map(|l| {
        let mut m = Map::new();
        for c in l.columns() {
            let v: Value = l.try_get::<i64, _>(c.ordinal()).map(Value::from)
                .or_else(|_| l.try_get::<f64, _>(c.ordinal()).map(Value::from))
                .unwrap_or(Value::Null);
            m.insert(c.name().to_string(), v);
        }
        Value::Object(m)
    }).collect())
}
