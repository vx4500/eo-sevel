//! Mes équipements → Serveur Proxmox (voir src/proxmox.rs).

use super::*;

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let pool = &state.pool;
    let mut ctx = context(&session);
    let configure = crate::proxmox::parametre(pool, "proxmox_secret").await.is_some();
    ctx.insert("url".into(), json!(crate::proxmox::parametre(pool, "proxmox_url").await));
    ctx.insert("jeton".into(), json!(crate::proxmox::parametre(pool, "proxmox_jeton").await));
    ctx.insert("secret_enregistre".into(), json!(configure));
    ctx.insert("alertes".into(), json!(crate::proxmox::parametre(pool, "proxmox_alertes").await.as_deref() == Some("1")));
    ctx.insert("suivis".into(), json!(crate::proxmox::invites_suivis(pool).await));
    if configure {
        match crate::proxmox::lire(pool).await {
            Ok(etat) => {
                let p = crate::proxmox::problemes(&etat, &crate::proxmox::invites_suivis(pool).await);
                ctx.insert("problemes".into(), json!(p.into_iter().map(|(_, t)| t).collect::<Vec<_>>()));
                ctx.insert("etat".into(), etat);
            }
            Err(e) => {
                ctx.insert("erreur".into(), json!(e));
            }
        }
    }
    ctx.insert("resultat".into(), json!(query.get("resultat")));
    render(&state, "proxmox.html", Value::Object(ctx))
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
    let url = form_text(&form, "url").map(|u| crate::proxmox::normaliser_adresse(&u)).unwrap_or_default();
    crate::sms_free::ecrire_parametre(pool, "proxmox_url", &url).await?;
    let jeton = form_text(&form, "jeton").unwrap_or_default();
    if !jeton.is_empty() && !(jeton.contains('@') && jeton.contains('!')) {
        return Err(AppError::Invalid("Identifiant du jeton attendu sous la forme utilisateur@pam!nom".into()));
    }
    crate::sms_free::ecrire_parametre(pool, "proxmox_jeton", &jeton).await?;
    // Secret laissé vide : on garde celui déjà enregistré.
    if let Some(secret) = form_text(&form, "secret") {
        crate::sms_free::ecrire_parametre(pool, "proxmox_secret", &secret).await?;
    }
    crate::sms_free::ecrire_parametre(pool, "proxmox_alertes", if form.contains_key("alertes") { "1" } else { "0" }).await?;
    // Machines surveillées : cases cochées ; à la première connexion, toutes
    // celles qui tournent.
    let coches: Vec<String> = form
        .keys()
        .filter_map(|k| k.strip_prefix("suivi_"))
        .map(str::to_string)
        .collect();
    let suivis = if form.contains_key("choix_suivis") {
        coches.join(",")
    } else {
        match crate::proxmox::lire(pool).await {
            Ok(etat) => etat["noeuds"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|n| n["invites"].as_array().cloned().unwrap_or_default())
                .filter(|g| g["marche"].as_bool().unwrap_or(false))
                .filter_map(|g| g["vmid"].as_i64())
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(","),
            Err(_) => String::new(),
        }
    };
    crate::sms_free::ecrire_parametre(pool, "proxmox_suivis", &suivis).await?;
    Ok(Redirect::to("/equipements/proxmox?resultat=enregistre").into_response())
}
