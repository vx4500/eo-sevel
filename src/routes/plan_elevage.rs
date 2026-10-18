//! Plan de l'élevage en un coup d'œil (/ambiance/plan).
//!
//! Une image du plan (plan_reglage, 0015) sur laquelle on pose des repères
//! (plan_repere, 0029) : carte de salle, température, hygrométrie, % de
//! ventilation, mode de ventilation, éclairage. Un clic sur un repère ouvre
//! les réglages courants (consigne, ventilation mini/maxi, forçage de la
//! lumière) sans passer par la page complète de chaque module.
//!
//! Les valeurs viennent des mêmes sources que /ambiance et
//! /ambiance/eclairage : rien n'est lu directement sur l'automate ici.

use super::*;

const TYPES: [&str; 7] = [
    "salle",
    "temperature",
    "humidite",
    "ventil",
    "mode",
    "eclairage",
    "texte",
];

async fn reperes(pool: &SqlitePool) -> AppResult<Vec<Value>> {
    generic_rows(
        pool,
        "SELECT id,type,ref_id,libelle,x,y FROM plan_repere ORDER BY id",
    )
    .await
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
) -> AppResult<Html<String>> {
    let image: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM plan_reglage WHERE id=1 AND image_data IS NOT NULL",
    )
    .fetch_one(&state.pool)
    .await?;
    let salles = generic_rows(
        &state.pool,
        "SELECT s.id, s.nom FROM salle s JOIN site si ON si.id=s.site_id \
         ORDER BY si.code, s.ordre, s.nom",
    )
    .await?;
    let programmes = generic_rows(
        &state.pool,
        "SELECT id, nom FROM eclairage_programme ORDER BY nom",
    )
    .await?;
    let mut contexte = context(&session);
    contexte.insert("image".into(), json!(image > 0));
    contexte.insert("salles".into(), Value::Array(salles));
    contexte.insert("programmes".into(), Value::Array(programmes));
    render(&state, "plan_elevage.html", Value::Object(contexte))
}

/// Tout ce que la page affiche, relu toutes les 30 secondes.
pub(super) async fn etat_json(State(state): State<AppState>) -> AppResult<Response> {
    let programmes: Vec<Value> = super::eclairage::charger_programmes(&state.pool, None)
        .await?
        .into_iter()
        .map(|p| {
            let allume = p
                .get("sorties")
                .and_then(Value::as_array)
                .map(|s| s.iter().any(|s| s.get("allume") == Some(&json!(true))))
                .unwrap_or(false);
            json!({
                "id": p.get("id"),
                "nom": p.get("nom"),
                "actif": p.get("actif"),
                "allume": allume,
                "forcage": p.get("forcage"),
            })
        })
        .collect();
    Ok(axum::Json(json!({
        "demon": super::ambiance::etat_demon(&state.pool).await,
        "salles": super::ambiance::salles(&state.pool).await?,
        "programmes": programmes,
        "reperes": reperes(&state.pool).await?,
    }))
    .into_response())
}

pub(super) async fn image(State(state): State<AppState>) -> AppResult<Response> {
    let image: Option<(Vec<u8>, String)> = sqlx::query_as(
        "SELECT image_data,image_mime FROM plan_reglage \
         WHERE id=1 AND image_data IS NOT NULL AND image_mime IS NOT NULL",
    )
    .fetch_optional(&state.pool)
    .await?;
    let Some((bytes, mime)) = image else {
        return Err(AppError::NotFound);
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&mime).map_err(|_| AppError::Invalid("Image invalide".into()))?,
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok((headers, bytes).into_response())
}

pub(super) async fn image_maj(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    multipart: Multipart,
) -> AppResult<Response> {
    require_writer(&session)?;
    let (form, file, _) = parity::multipart_fields(multipart, "image").await?;
    verify_csrf(&session, &form)?;
    sqlx::query("INSERT OR IGNORE INTO plan_reglage(id) VALUES(1)")
        .execute(&state.pool)
        .await?;
    if form.contains_key("supprimer") {
        sqlx::query("UPDATE plan_reglage SET image_data=NULL,image_mime=NULL WHERE id=1")
            .execute(&state.pool)
            .await?;
    } else {
        let bytes = file.ok_or_else(|| AppError::Invalid("Image du plan obligatoire".into()))?;
        let mime = detect_image_mime(&bytes)?;
        sqlx::query("UPDATE plan_reglage SET image_data=?,image_mime=? WHERE id=1")
            .bind(bytes)
            .bind(mime)
            .execute(&state.pool)
            .await?;
    }
    Ok(Redirect::to("/ambiance/plan?travaux=1").into_response())
}

/// Crée un repère (sans `id`) ou le déplace (avec `id`). Répond le repère.
pub(super) async fn repere(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let x = form_f64(&form, "x").unwrap_or(50.0).clamp(0.0, 100.0);
    let y = form_f64(&form, "y").unwrap_or(50.0).clamp(0.0, 100.0);
    let id = if let Some(id) = form_i64(&form, "id") {
        sqlx::query("UPDATE plan_repere SET x=?,y=? WHERE id=?")
            .bind(x)
            .bind(y)
            .bind(id)
            .execute(&state.pool)
            .await?;
        id
    } else {
        let type_repere = form_text(&form, "type")
            .filter(|t| TYPES.contains(&t.as_str()))
            .ok_or_else(|| AppError::Invalid("Type de repère inconnu".into()))?;
        let ref_id = form_i64(&form, "ref_id");
        let libelle = form_text(&form, "libelle");
        if type_repere == "texte" && libelle.is_none() {
            return Err(AppError::Invalid("Écrivez le texte du repère".into()));
        }
        if type_repere != "texte" && ref_id.is_none() {
            return Err(AppError::Invalid(
                if type_repere == "eclairage" {
                    "Choisissez le programme d'éclairage"
                } else {
                    "Choisissez la salle"
                }
                .into(),
            ));
        }
        sqlx::query("INSERT INTO plan_repere(type,ref_id,libelle,x,y) VALUES(?,?,?,?,?)")
            .bind(&type_repere)
            .bind(ref_id)
            .bind(libelle)
            .bind(x)
            .bind(y)
            .execute(&state.pool)
            .await?
            .last_insert_rowid()
    };
    Ok(axum::Json(json!({ "id": id })).into_response())
}

pub(super) async fn repere_supprimer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    sqlx::query("DELETE FROM plan_repere WHERE id=?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(axum::Json(json!({ "ok": true })).into_response())
}

/// Réglages courants d'une salle depuis le plan : seulement les champs
/// envoyés sont modifiés, le reste de la configuration (/ambiance) est gardé.
pub(super) async fn salle_reglage(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(salle_id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let actuel: Option<(f64, f64, f64, f64)> = sqlx::query_as(
        "SELECT t_cible_fixe,plage,ventil_min,ventil_max FROM ambiance_salle WHERE salle_id=?",
    )
    .bind(salle_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((cible, plage, vmin, vmax)) = actuel else {
        return Err(AppError::Invalid(
            "Cette salle n'est pas encore raccordée : réglez-la d'abord dans Températures et ventilation"
                .into(),
        ));
    };
    let cible = form_f64(&form, "t_cible_fixe").unwrap_or(cible);
    if !(0.0..=45.0).contains(&cible) {
        return Err(AppError::Invalid(
            "La température de consigne doit rester entre 0 et 45 °C".into(),
        ));
    }
    let plage = form_f64(&form, "plage").unwrap_or(plage);
    if plage <= 0.0 {
        return Err(AppError::Invalid(
            "La plage de régulation doit être supérieure à zéro".into(),
        ));
    }
    let vmin = form_f64(&form, "ventil_min").unwrap_or(vmin).clamp(0.0, 100.0);
    let vmax = form_f64(&form, "ventil_max").unwrap_or(vmax).clamp(vmin, 100.0);
    sqlx::query(
        "UPDATE ambiance_salle SET t_cible_fixe=?,plage=?,ventil_min=?,ventil_max=? WHERE salle_id=?",
    )
    .bind(cible)
    .bind(plage)
    .bind(vmin)
    .bind(vmax)
    .bind(salle_id)
    .execute(&state.pool)
    .await?;
    Ok(axum::Json(json!({ "ok": true })).into_response())
}

#[cfg(test)]
mod tests {
    #[test]
    fn le_gabarit_se_compile() {
        let mut env = minijinja::Environment::new();
        env.add_template("base.html", "{% block content %}{% endblock %}").unwrap();
        env.add_template(
            "plan_elevage.html",
            include_str!("../../templates/plan_elevage.html"),
        )
        .unwrap();
        let rendu = env
            .get_template("plan_elevage.html")
            .unwrap()
            .render(minijinja::context! {
                session => minijinja::context! { peut_modifier => true, csrf => "x" },
                image => false,
                salles => vec![minijinja::context! { id => 1, nom => "Mater 1" }],
                programmes => Vec::<i32>::new(),
            })
            .unwrap();
        assert!(rendu.contains("Mater 1"));
    }
}
