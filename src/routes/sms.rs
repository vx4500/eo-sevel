//! Mes équipements → Free SMS : identifiants de l'option « Notifications par
//! SMS » de Free Mobile, niveau d'alerte envoyé, SMS de test et historique.

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
    let pool = &state.pool;
    let mut ctx = context(&session);
    ctx.insert("user".into(), json!(valeur(pool, "sms_free_user").await));
    ctx.insert("cle_enregistree".into(), json!(valeur(pool, "sms_free_pass").await.is_some()));
    ctx.insert("actif".into(), json!(valeur(pool, "sms_free_actif").await.as_deref() == Some("1")));
    ctx.insert("niveau".into(), json!(valeur(pool, "sms_free_niveau").await.unwrap_or_else(|| "critique".into())));
    let battement = valeur(pool, "sms_battement").await.and_then(|v| v.parse::<i64>().ok());
    ctx.insert("battement_age".into(), json!(battement.map(|b| chrono::Utc::now().timestamp() - b)));
    ctx.insert(
        "envois".into(),
        Value::Array(generic_rows(pool, "SELECT horodatage,origine,message,statut FROM sms_envoi ORDER BY id DESC LIMIT 30").await?),
    );
    ctx.insert("resultat".into(), json!(query.get("resultat")));
    render(&state, "sms_free.html", Value::Object(ctx))
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
    let user = form_text(&form, "user").unwrap_or_default();
    if !user.is_empty() && !user.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::Invalid("L'identifiant Free ne contient que des chiffres".into()));
    }
    crate::sms_free::ecrire_parametre(pool, "sms_free_user", &user).await?;
    // Clé laissée vide : on garde celle déjà enregistrée.
    if let Some(cle) = form_text(&form, "pass") {
        crate::sms_free::ecrire_parametre(pool, "sms_free_pass", &cle).await?;
    }
    crate::sms_free::ecrire_parametre(pool, "sms_free_actif", if form.contains_key("actif") { "1" } else { "0" }).await?;
    let niveau = if form.get("niveau").map(String::as_str) == Some("alerte") { "alerte" } else { "critique" };
    crate::sms_free::ecrire_parametre(pool, "sms_free_niveau", niveau).await?;
    Ok(Redirect::to("/equipements/sms?resultat=enregistre").into_response())
}

pub(super) async fn test(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    if !session.est_admin() {
        return Err(AppError::Forbidden);
    }
    verify_csrf(&session, &form)?;
    let texte = format!("EO_Sevel : SMS de test envoyé le {}. Les alertes arriveront ainsi.", Local::now().format("%d/%m à %H:%M"));
    let resultat = match crate::sms_free::envoyer(&state.pool, &texte, "test").await {
        Ok(()) => "test_ok".to_string(),
        Err(e) => format!("Échec : {e}"),
    };
    Ok(Redirect::to(&format!("/equipements/sms?resultat={}", urlencoding(&resultat))).into_response())
}

fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}
