//! Assistant en langage naturel (voir src/assistant.rs).

use super::*;

async fn contexte_page(state: &AppState, session: &SessionData) -> Map<String, Value> {
    let pool = &state.pool;
    let mut ctx = context(session);
    ctx.insert("cle_enregistree".into(), json!(crate::proxmox::parametre(pool, "assistant_cle").await.is_some()));
    ctx.insert("modele".into(), json!(crate::proxmox::parametre(pool, "assistant_modele").await));
    ctx.insert("modele_defaut".into(), json!(crate::assistant::MODELE_DEFAUT));
    ctx
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let mut ctx = contexte_page(&state, &session).await;
    ctx.insert("resultat".into(), json!(query.get("resultat")));
    render(&state, "assistant.html", Value::Object(ctx))
}

pub(super) async fn demander(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    verify_csrf(&session, &form)?;
    let question = form_text(&form, "question").unwrap_or_default();
    let mut ctx = contexte_page(&state, &session).await;
    if !question.is_empty() {
        match crate::assistant::demander(&state.pool, &state.config.db_path, &question).await {
            Ok(r) => {
                ctx.insert("reponse".into(), json!(r));
            }
            Err(e) => {
                ctx.insert("erreur".into(), json!(e));
            }
        }
    }
    ctx.insert("question".into(), json!(question));
    render(&state, "assistant.html", Value::Object(ctx))
}

pub(super) async fn reglages(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    if !session.est_admin() {
        return Err(AppError::Forbidden);
    }
    verify_csrf(&session, &form)?;
    let pool = &state.pool;
    // Clé laissée vide : on garde celle déjà enregistrée.
    if let Some(cle) = form_text(&form, "cle") {
        crate::sms_free::ecrire_parametre(pool, "assistant_cle", &cle).await?;
    }
    crate::sms_free::ecrire_parametre(pool, "assistant_modele", &form_text(&form, "modele").unwrap_or_default()).await?;
    Ok(Redirect::to("/equipements/assistant?resultat=enregistre").into_response())
}
