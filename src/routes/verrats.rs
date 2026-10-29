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
        "SELECT v.id, v.code, v.race, v.pere, v.mere, v.note, v.case_id, COALESCE(v.statut, CASE WHEN v.actif=1 THEN 'actif' ELSE 'reforme' END) AS statut, \
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
    sqlx::query("INSERT INTO verrat(code,race,pere,mere,note,actif,statut,case_id,date_entree) VALUES(?,?,?,?,?,1,'actif',?,?)")
        .bind(&code)
        .bind(form_text(&form, "race"))
        .bind(form_text(&form, "pere"))
        .bind(form_text(&form, "mere"))
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
        "UPDATE verrat SET race=?, pere=?, mere=?, note=?, statut=?, actif=?, case_id=?, date_entree=?, date_sortie=?, motif_sortie=? WHERE id=?",
    )
    .bind(form_text(&form, "race"))
    .bind(form_text(&form, "pere"))
    .bind(form_text(&form, "mere"))
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

/// Fiche d'un verrat : identité, case, soins et vaccins, actes des
/// protocoles sanitaires, IA faites avec lui.
pub(super) async fn fiche(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
) -> AppResult<Html<String>> {
    let mut verrat = generic_rows_bind_id(
        &state.pool,
        "SELECT v.id, v.code, v.race, v.pere, v.mere, v.note, v.case_id, COALESCE(v.statut, CASE WHEN v.actif=1 THEN 'actif' ELSE 'reforme' END) AS statut, \
           v.date_entree, v.date_sortie, v.motif_sortie, c.nom AS case_nom, s.nom AS salle_nom, s.id AS salle_id \
         FROM verrat v LEFT JOIN casesalle c ON c.id=v.case_id LEFT JOIN salle s ON s.id=c.salle_id WHERE v.id=?",
        id,
    )
    .await?;
    let verrat = verrat.pop().ok_or(AppError::NotFound)?;
    // Soins et vaccins saisis sur le verrat, avec la fin du délai d'attente.
    let soins = generic_rows_bind_id(
        &state.pool,
        "SELECT id, date, date_fin, produit, motif, delai_attente, note, \
           CASE WHEN delai_attente>0 THEN date(COALESCE(date_fin,date), printf('+%d day', delai_attente)) END AS fin_attente \
         FROM evenement WHERE verrat_id=? AND truie_id IS NULL AND type='traitement' ORDER BY date DESC, id DESC",
        id,
    )
    .await?;
    let actes = generic_rows_bind_id(
        &state.pool,
        "SELECT r.id, r.date_realise AS date, a.libelle, a.produit, r.note FROM acterealiseverrat r \
         JOIN acteprotocole a ON a.id=r.acte_id WHERE r.verrat_id=? ORDER BY r.date_realise DESC, r.id DESC",
        id,
    )
    .await?;
    let ia = generic_rows_bind_id(
        &state.pool,
        "SELECT e.date, t.id AS truie_id, t.num_travail, b.code AS bande FROM evenement e \
         JOIN truie t ON t.id=e.truie_id LEFT JOIN bande b ON b.id=e.bande_id \
         WHERE e.verrat_id=? AND e.type='ia' ORDER BY e.date DESC LIMIT 100",
        id,
    )
    .await?;
    let protocoles = generic_rows(
        &state.pool,
        "SELECT id, libelle, produit FROM acteprotocole WHERE actif=1 ORDER BY libelle",
    )
    .await?;
    let produits = generic_rows(&state.pool, "SELECT produit FROM produitpharmacie ORDER BY produit").await?;
    let aujourdhui = today_iso();
    let en_attente = soins.iter().filter_map(|s| s["fin_attente"].as_str()).filter(|f| *f >= aujourdhui.as_str()).max().map(str::to_string);
    let mut ctx = context(&session);
    ctx.insert("verrat".into(), verrat);
    ctx.insert("soins".into(), Value::Array(soins));
    ctx.insert("actes".into(), Value::Array(actes));
    ctx.insert("ia".into(), Value::Array(ia));
    ctx.insert("protocoles".into(), Value::Array(protocoles));
    ctx.insert("produits".into(), Value::Array(produits));
    ctx.insert("en_attente".into(), json!(en_attente));
    ctx.insert("aujourdhui".into(), json!(aujourdhui));
    render(&state, "verrat.html", Value::Object(ctx))
}

async fn generic_rows_bind_id(pool: &SqlitePool, sql: &str, id: i64) -> AppResult<Vec<Value>> {
    let rows = sqlx::query(sql).bind(id).fetch_all(pool).await?;
    rows_to_json(rows)
}

async fn verrat_existe(pool: &SqlitePool, id: i64) -> AppResult<()> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM verrat WHERE id=?").bind(id).fetch_one(pool).await?;
    if n == 0 { Err(AppError::NotFound) } else { Ok(()) }
}

/// Soin ou vaccin : un événement `traitement` porté par le verrat (sans
/// truie). Sur plusieurs jours, une dose par jour sort du stock pharmacie,
/// comme pour les truies.
pub(super) async fn soin(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    verrat_existe(&state.pool, id).await?;
    let produit = form_text(&form, "produit").ok_or_else(|| AppError::Invalid("Produit obligatoire".into()))?;
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let evenement = sqlx::query(
        "INSERT INTO evenement(type,date,date_fin,verrat_id,produit,motif,delai_attente,note,suivi_actif,created_at) \
         VALUES('traitement',?,?,?,?,?,?,?,0,CURRENT_TIMESTAMP)",
    )
    .bind(form_date_or_today(&form, "date")?)
    .bind(form_date(&form, "date_fin")?)
    .bind(id)
    .bind(&produit)
    .bind(form_text(&form, "motif"))
    .bind(form_i64(&form, "delai_attente"))
    .bind(form_text(&form, "note"))
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();
    super::traitement_stock::appliquer(&mut *tx, evenement).await?;
    tx.commit().await?;
    db::journal(&state.pool, &session.nom, "ajouter", "soin verrat", &id.to_string(), &format!("/verrat/{id}")).await;
    Ok(Redirect::to(&format!("/verrat/{id}")).into_response())
}

pub(super) async fn soin_supprimer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path((id, soin)): Path<(i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM evenement WHERE id=? AND verrat_id=? AND truie_id IS NULL AND type='traitement'")
        .bind(soin).bind(id).fetch_one(&mut *tx).await?;
    if n == 0 {
        return Err(AppError::NotFound);
    }
    super::traitement_stock::annuler(&mut *tx, soin).await?;
    sqlx::query("DELETE FROM evenement WHERE id=?").bind(soin).execute(&mut *tx).await?;
    tx.commit().await?;
    db::journal(&state.pool, &session.nom, "supprimer", "soin verrat", &soin.to_string(), &format!("/verrat/{id}")).await;
    Ok(Redirect::to(&format!("/verrat/{id}")).into_response())
}

/// Acte d'un protocole sanitaire (vaccin du plan sanitaire) fait sur le verrat.
pub(super) async fn acte(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    verrat_existe(&state.pool, id).await?;
    let acte = form_i64(&form, "acte_id").ok_or_else(|| AppError::Invalid("Acte manquant".into()))?;
    let fait = sqlx::query("INSERT INTO acterealiseverrat(acte_id,verrat_id,date_realise,note) SELECT ?,?,?,? WHERE EXISTS(SELECT 1 FROM acteprotocole WHERE id=? AND actif=1)")
        .bind(acte).bind(id).bind(form_date_or_today(&form, "date")?).bind(form_text(&form, "note")).bind(acte)
        .execute(&state.pool).await?;
    if fait.rows_affected() == 0 {
        return Err(AppError::Invalid("Acte inconnu".into()));
    }
    Ok(Redirect::to(&format!("/verrat/{id}")).into_response())
}

pub(super) async fn acte_supprimer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path((id, acte)): Path<(i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    sqlx::query("DELETE FROM acterealiseverrat WHERE id=? AND verrat_id=?").bind(acte).bind(id).execute(&state.pool).await?;
    Ok(Redirect::to(&format!("/verrat/{id}")).into_response())
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
        env.get_template("verrat.html").unwrap().render(serde_json::json!({
            "session": session, "aujourdhui": "2026-10-29", "en_attente": "2026-11-05",
            "verrat": {"id": 1, "code": "510310", "race": "NU4V1", "pere": "FR35AB123", "mere": "FR35AB456", "note": null, "case_id": 118, "statut": "actif",
                 "date_entree": null, "date_sortie": null, "motif_sortie": null, "case_nom": "42 Verra", "salle_nom": "Verraterie", "salle_id": 1},
            "soins": [{"id": 5, "date": "2026-10-29", "date_fin": null, "produit": "Eryseng Parvo", "motif": "Vaccin", "delai_attente": 7, "note": null, "fin_attente": "2026-11-05"}],
            "actes": [{"id": 2, "date": "2026-10-01", "libelle": "Eryseng Parvo", "produit": "ERYSENG", "note": null}],
            "ia": [{"date": "2026-03-16", "truie_id": 1, "num_travail": "409353", "bande": "B2.12"}],
            "protocoles": [{"id": 2, "libelle": "Eryseng Parvo", "produit": "ERYSENG"}],
            "produits": [{"produit": "ERYSENG PARVO FL 50D"}]
        })).expect("verrat.html");
    }
}
