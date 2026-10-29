//! Demandes de modification envoyées à l'IA depuis une note du plan de
//! l'élevage. Le logiciel ne fait que les enregistrer : le programme
//! serveur `eo-demande-ia` (minuterie systemd) les confie une par une à
//! Claude, qui prépare et teste la modification sur une branche git, puis
//! l'installe quand l'administrateur clique sur « Installer ».
//!
//! Réservé à l'administrateur : l'IA agit sur le serveur.

use super::*;

fn require_admin(session: &SessionData) -> AppResult<()> {
    if session.est_admin() {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

/// Demandes liées à une note (`?repere=`), ou toutes.
pub(super) async fn liste_json(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<Response> {
    require_admin(&session)?;
    let repere = query.get("repere").and_then(|v| v.parse::<i64>().ok());
    let lignes = sqlx::query(
        "SELECT id, repere_id, texte, auteur, cree_le, statut, compte_rendu, version, maj_le FROM demande_ia \
         WHERE ?1 IS NULL OR repere_id=?1 ORDER BY id DESC LIMIT 50",
    )
    .bind(repere)
    .fetch_all(&state.pool)
    .await?;
    Ok(axum::Json(json!({ "demandes": rows_to_json(lignes)? })).into_response())
}

pub(super) async fn envoyer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_admin(&session)?;
    verify_csrf(&session, &form)?;
    let texte = form_text(&form, "texte").ok_or_else(|| AppError::Invalid("Écrivez ou dictez la demande d'abord".into()))?;
    if texte.chars().count() > 4000 {
        return Err(AppError::Invalid("Demande trop longue (4 000 caractères au plus)".into()));
    }
    let id = sqlx::query("INSERT INTO demande_ia(repere_id, texte, auteur) VALUES(?,?,?)")
        .bind(form_i64(&form, "repere_id"))
        .bind(&texte)
        .bind(&session.nom)
        .execute(&state.pool)
        .await?
        .last_insert_rowid();
    db::journal(&state.pool, &session.nom, "envoyer", "demande IA", &id.to_string(), "/ambiance/plan").await;
    Ok(axum::Json(json!({ "id": id })).into_response())
}

/// Passe une demande à l'état voulu, depuis l'état attendu seulement.
async fn changer(state: &AppState, session: &SessionData, id: i64, de: &[&str], vers: &str) -> AppResult<Response> {
    let liste = de.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(",");
    let fait = sqlx::query(&format!(
        "UPDATE demande_ia SET statut=?, maj_le=CURRENT_TIMESTAMP WHERE id=? AND statut IN ({liste})"
    ))
    .bind(vers)
    .bind(id)
    .execute(&state.pool)
    .await?;
    if fait.rows_affected() == 0 {
        return Err(AppError::Invalid("Cette demande n'est plus dans cet état : rechargez".into()));
    }
    db::journal(&state.pool, &session.nom, vers, "demande IA", &id.to_string(), "/ambiance/plan").await;
    Ok(axum::Json(json!({ "ok": true })).into_response())
}

pub(super) async fn installer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_admin(&session)?;
    verify_csrf(&session, &form)?;
    changer(&state, &session, id, &["prete"], "installation").await
}

pub(super) async fn abandonner(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_admin(&session)?;
    verify_csrf(&session, &form)?;
    changer(&state, &session, id, &["attente", "prete", "echec"], "annulee").await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn le_schema_se_cree_deux_fois() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        for _ in 0..2 {
            sqlx::raw_sql(include_str!("../../migrations/0032_demande_ia.sql")).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO demande_ia(texte) VALUES('x')").execute(&pool).await.unwrap();
        let statut: String = sqlx::query_scalar("SELECT statut FROM demande_ia").fetch_one(&pool).await.unwrap();
        assert_eq!(statut, "attente");
    }
}
