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
