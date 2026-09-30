//! Truies improductives : truies actives sans bande en cours, en attente de
//! remise en bande (après sevrage, écho vide…). C'est normal un temps ; au-delà
//! du délai réglé (`improductive_max_jours`, 60 jours par défaut) elles sont
//! signalées « à déclarer stérile ».

use super::*;

pub(super) const DELAI_DEFAUT: i64 = 60;

/// Truies actives hors bande en cours. `depuis` = dernier événement de
/// reproduction (sevrage, écho, IA, mise-bas, avortement), sinon l'entrée.
const SQL: &str = "SELECT t.id,t.num_travail,t.rang,s.nom AS salle, \
 COALESCE((SELECT MAX(e.date) FROM evenement e WHERE e.truie_id=t.id AND e.type IN ('sevrage','echo','echographie','ia','mise_bas','avortement')),t.date_entree,date(t.created_at)) AS depuis, \
 (SELECT e.type FROM evenement e WHERE e.truie_id=t.id AND e.type IN ('sevrage','echo','echographie','ia','mise_bas','avortement') ORDER BY e.date DESC,e.id DESC LIMIT 1) AS dernier, \
 (SELECT e.resultat FROM evenement e WHERE e.truie_id=t.id AND e.type IN ('echo','echographie') ORDER BY e.date DESC,e.id DESC LIMIT 1) AS dernier_echo, \
 (SELECT MAX(e.date) FROM evenement e WHERE e.truie_id=t.id AND e.type='sterile') AS signalee \
 FROM truie t LEFT JOIN salle s ON s.id=t.salle_id \
 WHERE t.reformee=0 AND NOT EXISTS(SELECT 1 FROM bande b WHERE b.active=1 AND b.code=t.bande_code)";

pub(super) async fn delai(pool: &SqlitePool) -> AppResult<i64> {
    let v: Option<String> = sqlx::query_scalar("SELECT valeur FROM parametre WHERE cle='improductive_max_jours'")
        .fetch_optional(pool)
        .await?
        .flatten();
    Ok(v.and_then(|v| v.trim().parse().ok()).filter(|j: &i64| *j > 0).unwrap_or(DELAI_DEFAUT))
}

async fn liste(pool: &SqlitePool) -> AppResult<(Vec<Value>, i64)> {
    let delai = delai(pool).await?;
    let today = Local::now().date_naive();
    let mut truies = generic_rows(pool, SQL).await?;
    for t in truies.iter_mut() {
        let jours = t["depuis"].as_str().and_then(parse_stored_date).map(|d| (today - d).num_days());
        t["jours"] = json!(jours);
        t["trop_long"] = json!(jours.is_some_and(|j| j > delai));
    }
    truies.sort_by_key(|t| std::cmp::Reverse(t["jours"].as_i64().unwrap_or(0)));
    Ok((truies, delai))
}

/// Truies au-delà du délai et pas encore signalées stériles (alertes).
pub(super) async fn a_signaler(pool: &SqlitePool) -> AppResult<i64> {
    let (truies, _) = liste(pool).await?;
    Ok(truies.iter().filter(|t| t["trop_long"] == true && t["signalee"].is_null()).count() as i64)
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
) -> AppResult<Html<String>> {
    let (truies, delai) = liste(&state.pool).await?;
    let mut ctx = context(&session);
    ctx.insert("truies".into(), Value::Array(truies));
    ctx.insert("delai".into(), json!(delai));
    render(&state, "truies_improductives.html", Value::Object(ctx))
}

pub(super) async fn signaler(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let existe: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM truie WHERE id=? AND reformee=0")
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    if existe == 0 {
        return Err(AppError::NotFound);
    }
    sqlx::query("INSERT INTO evenement(type,date,truie_id,note,suivi_actif) VALUES('sterile',date('now','localtime'),?,?,0)")
        .bind(id)
        .bind(form_text(&form, "note").unwrap_or_else(|| "Signalée stérile : improductive depuis plus de 2 mois".into()))
        .execute(&state.pool)
        .await?;
    db::journal(&state.pool, &session.nom, "signaler stérile", "truie", &id.to_string(), "/truies/improductives").await;
    Ok(Redirect::to("/truies/improductives").into_response())
}

pub(super) async fn regler(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    if !session.est_admin() {
        return Err(AppError::Forbidden);
    }
    verify_csrf(&session, &form)?;
    let jours = form_i64(&form, "jours").filter(|j| (7..=365).contains(j)).ok_or_else(|| AppError::Invalid("Délai entre 7 et 365 jours".into()))?;
    sqlx::query("INSERT INTO parametre(cle,valeur) VALUES('improductive_max_jours',?) ON CONFLICT(cle) DO UPDATE SET valeur=excluded.valeur")
        .bind(jours.to_string())
        .execute(&state.pool)
        .await?;
    Ok(Redirect::to("/truies/improductives").into_response())
}
