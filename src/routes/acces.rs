//! Mes équipements → Accès à distance : jeton du tunnel Cloudflare, marche
//! ou arrêt, état et journal de cloudflared (voir `crate::tunnel`).

use super::*;

async fn valeur(pool: &SqlitePool, cle: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>("SELECT valeur FROM parametre WHERE cle=?")
        .bind(cle)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .flatten()
        .filter(|v| !v.trim().is_empty())
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    if !session.est_admin() {
        return Err(AppError::Forbidden);
    }
    let pool = &state.pool;
    let e = crate::tunnel::instantane();
    let mut ctx = context(&session);
    ctx.insert("jeton_enregistre".into(), json!(valeur(pool, "cf_token").await.is_some()));
    ctx.insert("actif".into(), json!(valeur(pool, "cf_actif").await.as_deref() == Some("1")));
    ctx.insert("adresse".into(), json!(valeur(pool, "cf_adresse").await));
    ctx.insert("installe".into(), json!(std::path::Path::new(crate::tunnel::BINAIRE).exists()));
    ctx.insert(
        "etat".into(),
        json!({
            "en_marche": e.en_marche,
            "connexions": e.connexions,
            "depuis": e.depuis.map(|d| super::salles::quand(d, chrono::Utc::now().timestamp())),
            "redemarrages": e.redemarrages,
            "erreur": e.derniere_erreur,
            "journal": e.journal.iter().rev().cloned().collect::<Vec<_>>(),
        }),
    );
    ctx.insert("resultat".into(), json!(query.get("resultat")));
    render(&state, "acces_distance.html", Value::Object(ctx))
}

pub(super) async fn enregistrer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    if !session.est_admin() {
        return Err(AppError::Forbidden);
    }
    verify_csrf(&session, &form)?;
    let pool = &state.pool;
    // On accepte la commande complète copiée depuis Cloudflare
    // (« cloudflared service install eyJ… ») : le jeton est le dernier mot.
    if let Some(saisie) = form_text(&form, "jeton") {
        let jeton = saisie.split_whitespace().last().unwrap_or_default().to_string();
        if jeton.len() < 50 || !jeton.starts_with("ey") {
            return Err(AppError::Invalid("Ce n'est pas un jeton de tunnel Cloudflare (il commence par « ey… »)".into()));
        }
        crate::sms_free::ecrire_parametre(pool, "cf_token", &jeton).await?;
    }
    let adresse = form_text(&form, "adresse").unwrap_or_default();
    let adresse = adresse.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_string();
    crate::sms_free::ecrire_parametre(pool, "cf_adresse", &adresse).await?;
    crate::sms_free::ecrire_parametre(pool, "cf_actif", if form.contains_key("actif") { "1" } else { "0" }).await?;
    crate::tunnel::redemarrer();
    Ok(Redirect::to("/equipements/acces-distance?resultat=enregistre").into_response())
}

pub(super) async fn relancer(
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    if !session.est_admin() {
        return Err(AppError::Forbidden);
    }
    verify_csrf(&session, &form)?;
    crate::tunnel::redemarrer();
    Ok(Redirect::to("/equipements/acces-distance?resultat=relance").into_response())
}
