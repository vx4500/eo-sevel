//! Verrats (/verrats, menu Animaux) : liste, case où loge chaque verrat,
//! statut (actif, réformé, mort) et actions. La case rattache le verrat à
//! sa salle : il apparaît alors dans la fiche de la salle et sur la feuille
//! d'inventaire, à sa vraie place.

use super::*;

/// Statuts possibles ; seul `actif` garde `verrat.actif=1`.
const STATUTS: [&str; 3] = ["actif", "reforme", "mort"];

pub(super) async fn liste(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
) -> AppResult<Html<String>> {
    let verrats = generic_rows(
        &state.pool,
        "SELECT v.id, v.code, v.race, v.note, v.case_id, COALESCE(v.statut, CASE WHEN v.actif=1 THEN 'actif' ELSE 'reforme' END) AS statut, \
           v.date_entree, v.date_sortie, v.motif_sortie, c.nom AS case_nom, s.nom AS salle_nom, s.id AS salle_id \
         FROM verrat v LEFT JOIN casesalle c ON c.id=v.case_id LEFT JOIN salle s ON s.id=c.salle_id \
         ORDER BY v.actif DESC, v.code",
    )
    .await?;
    // Cases proposées : celles des verrateries et salles de gestantes d'abord.
    let cases = generic_rows(
        &state.pool,
        "SELECT c.id, c.nom, s.nom AS salle FROM casesalle c JOIN salle s ON s.id=c.salle_id \
         ORDER BY CASE WHEN lower(COALESCE(s.type,'')||' '||s.nom) LIKE '%verrat%' THEN 0 ELSE 1 END, s.ordre, s.nom, c.id",
    )
    .await?;
    let mut ctx = context(&session);
    ctx.insert("verrats".into(), Value::Array(verrats));
    ctx.insert("cases".into(), Value::Array(cases));
    ctx.insert("aujourdhui".into(), json!(today_iso()));
    render(&state, "verrats.html", Value::Object(ctx))
}

/// Case choisie dans le formulaire : vide = aucune, sinon une case existante.
async fn case_du_formulaire(pool: &SqlitePool, form: &HashMap<String, String>) -> AppResult<Option<i64>> {
    let Some(brut) = form_text(form, "case_id") else { return Ok(None) };
    let id: i64 = brut.parse().map_err(|_| AppError::Invalid("Case inconnue".into()))?;
    let existe: Option<i64> = sqlx::query_scalar("SELECT id FROM casesalle WHERE id=?").bind(id).fetch_optional(pool).await?;
    existe.map(Some).ok_or_else(|| AppError::Invalid("Case inconnue".into()))
}

pub(super) async fn ajouter(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let code = form_text(&form, "code").ok_or_else(|| AppError::Invalid("Numéro du verrat obligatoire".into()))?;
    let case_id = case_du_formulaire(&state.pool, &form).await?;
    let existe: Option<i64> = sqlx::query_scalar("SELECT id FROM verrat WHERE lower(code)=lower(?)").bind(&code).fetch_optional(&state.pool).await?;
    if existe.is_some() {
        return Err(AppError::Invalid(format!("Le verrat {code} existe déjà")));
    }
    sqlx::query("INSERT INTO verrat(code,race,note,actif,statut,case_id,date_entree) VALUES(?,?,?,1,'actif',?,?)")
        .bind(&code)
        .bind(form_text(&form, "race"))
        .bind(form_text(&form, "note"))
        .bind(case_id)
        .bind(form_date(&form, "date_entree")?)
        .execute(&state.pool)
        .await?;
    db::journal(&state.pool, &session.nom, "ajouter", "verrat", &code, "/verrats").await;
    Ok(Redirect::to("/verrats").into_response())
}

pub(super) async fn modifier(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let case_id = case_du_formulaire(&state.pool, &form).await?;
    let statut = form_text(&form, "statut").unwrap_or_else(|| "actif".into());
    if !STATUTS.contains(&statut.as_str()) {
        return Err(AppError::Invalid("Statut inconnu".into()));
    }
    let actif = statut == "actif";
    // Un verrat sorti n'occupe plus de case ; la date de sortie est celle
    // saisie, ou aujourd'hui.
    let (date_sortie, motif) = if actif {
        (None, None)
    } else {
        (Some(form_date_or_today(&form, "date_sortie")?), form_text(&form, "motif_sortie"))
    };
    let fait = sqlx::query(
        "UPDATE verrat SET race=?, note=?, statut=?, actif=?, case_id=?, date_entree=?, date_sortie=?, motif_sortie=? WHERE id=?",
    )
    .bind(form_text(&form, "race"))
    .bind(form_text(&form, "note"))
    .bind(&statut)
    .bind(actif)
    .bind(if actif { case_id } else { None })
    .bind(form_date(&form, "date_entree")?)
    .bind(date_sortie)
    .bind(motif)
    .bind(id)
    .execute(&state.pool)
    .await?;
    if fait.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    db::journal(&state.pool, &session.nom, "modifier", "verrat", &id.to_string(), "/verrats").await;
    Ok(Redirect::to("/verrats").into_response())
}

#[cfg(test)]
mod tests {
    #[test]
    fn la_page_se_rend() {
        let env = crate::templates::build().expect("gabarits");
        let session = serde_json::json!({"csrf": "x", "peut_modifier": true, "pages": [], "identifiant": "test"});
        env.get_template("verrats.html").unwrap().render(serde_json::json!({
            "session": session, "aujourdhui": "2026-10-28",
            "cases": [{"id": 118, "nom": "42 Verra", "salle": "Verraterie"}],
            "verrats": [
                {"id": 1, "code": "510310", "race": "NU4V1", "note": null, "case_id": 118, "statut": "actif",
                 "date_entree": null, "date_sortie": null, "motif_sortie": null, "case_nom": "42 Verra", "salle_nom": "Verraterie", "salle_id": 1},
                {"id": 2, "code": "PIE", "race": null, "note": "vieux", "case_id": null, "statut": "reforme",
                 "date_entree": "2024-01-01", "date_sortie": "2026-10-01", "motif_sortie": "Âge", "case_nom": null, "salle_nom": null, "salle_id": null}
            ]
        })).expect("verrats.html");
    }
}
