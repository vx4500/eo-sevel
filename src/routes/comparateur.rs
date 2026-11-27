//! Comparateur de prix (/comparateur) : pour un produit à acheter, l'éleveur
//! note chaque offre (fournisseur, contenance d'une unité, nombre d'unités,
//! prix à l'unité ou pour le lot). La page ramène tout au même prix par
//! unité de mesure (€/kg, €/L…) et classe les offres de la moins chère à la
//! plus chère.

use super::*;

const UNITES: [&str; 6] = ["kg", "L", "dose", "pièce", "m", "t"];

/// Prix d'une unité achetée et prix par unité de mesure (si la contenance
/// est connue), à partir du prix total payé pour `nombre` unités.
pub(super) fn prix_ramenes(contenance: Option<f64>, nombre: f64, prix_total: f64) -> (f64, Option<f64>) {
    let par_unite = prix_total / nombre;
    (par_unite, contenance.filter(|c| *c > 0.0).map(|c| par_unite / c))
}

/// « 0,452 € » sous 10 €, « 12,40 € » au-dessus : les petits prix au kg ont
/// besoin d'un chiffre de plus pour se départager.
fn prix_lisible(x: f64) -> String {
    let s = if x.abs() < 10.0 { format!("{x:.3}") } else { format!("{x:.2}") };
    format!("{} €", s.replace('.', ","))
}

fn nombre_lisible(x: f64) -> String {
    let s = format!("{x:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.replace('.', ",")
}

pub(super) async fn liste(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
) -> AppResult<Html<String>> {
    let comparatifs = generic_rows(
        &state.pool,
        "SELECT c.id,c.nom,c.unite,c.note,substr(c.cree_le,1,10) AS cree_le,\
         (SELECT COUNT(*) FROM comparatif_offre o WHERE o.comparatif_id=c.id) AS offres \
         FROM comparatif c ORDER BY c.id DESC",
    )
    .await?;
    let mut ctx = context(&session);
    ctx.insert("comparatifs".into(), Value::Array(comparatifs));
    ctx.insert("unites".into(), json!(UNITES));
    render(&state, "comparateur.html", Value::Object(ctx))
}

pub(super) async fn creer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let nom = form_text(&form, "nom").ok_or_else(|| AppError::Invalid("Nom du produit à comparer obligatoire".into()))?;
    let unite = form_text(&form, "unite").unwrap_or_else(|| "kg".into());
    let id: i64 = sqlx::query_scalar("INSERT INTO comparatif(nom,unite,note) VALUES(?,?,?) RETURNING id")
        .bind(&nom)
        .bind(&unite)
        .bind(form_text(&form, "note"))
        .fetch_one(&state.pool)
        .await?;
    db::journal(&state.pool, &session.nom, "créer", "comparatif", &nom, &format!("/comparateur/{id}")).await;
    Ok(Redirect::to(&format!("/comparateur/{id}")).into_response())
}

/// Offres d'un comparatif, calculées et classées. Toutes les offres ont une
/// contenance : classement au prix par unité de mesure ; sinon au prix d'une
/// unité achetée (on ne mélange pas les deux).
pub(super) async fn offres(pool: &SqlitePool, id: i64) -> AppResult<(Vec<Value>, bool)> {
    type Ligne = (i64, String, Option<String>, Option<f64>, f64, f64, String, Option<String>);
    let lignes: Vec<Ligne> = sqlx::query_as(
        "SELECT id,produit,fournisseur,contenance,nombre,prix_total,prix_pour,note FROM comparatif_offre WHERE comparatif_id=? ORDER BY id",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let par_mesure = !lignes.is_empty() && lignes.iter().all(|l| l.3.is_some_and(|c| c > 0.0));
    let mut calc: Vec<(f64, Value)> = lignes
        .into_iter()
        .map(|(oid, produit, fournisseur, contenance, nombre, total, pour, note)| {
            let (unite, mesure) = prix_ramenes(contenance, nombre, total);
            let cle = if par_mesure { mesure.unwrap_or(unite) } else { unite };
            (
                cle,
                json!({"id": oid, "produit": produit, "fournisseur": fournisseur,
                       "contenance": contenance.map(nombre_lisible), "nombre": nombre_lisible(nombre),
                       "lot": nombre != 1.0, "prix_pour": pour, "total": total,
                       "par_unite": prix_lisible(unite), "par_mesure": mesure.map(prix_lisible),
                       "quantite_totale": contenance.map(|c| nombre_lisible(c * nombre)),
                       "note": note}),
            )
        })
        .collect();
    calc.sort_by(|a, b| a.0.total_cmp(&b.0));
    let meilleur = calc.first().map(|c| c.0);
    let resultat = calc
        .into_iter()
        .enumerate()
        .map(|(rang, (cle, mut v))| {
            if let (Some(m), Some(obj)) = (meilleur, v.as_object_mut()) {
                obj.insert("rang".into(), json!(rang + 1));
                obj.insert("meilleur".into(), json!(rang == 0));
                let ecart = if m > 0.0 { (cle / m - 1.0) * 100.0 } else { 0.0 };
                obj.insert("ecart".into(), json!((ecart * 10.0).round() / 10.0));
            }
            v
        })
        .collect();
    Ok((resultat, par_mesure))
}

pub(super) async fn fiche(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
) -> AppResult<Html<String>> {
    let comp: Option<(i64, String, String, Option<String>)> =
        sqlx::query_as("SELECT id,nom,unite,note FROM comparatif WHERE id=?")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?;
    let (id, nom, unite, note) = comp.ok_or(AppError::NotFound)?;
    let (liste, par_mesure) = offres(&state.pool, id).await?;
    let mut ctx = context(&session);
    ctx.insert("comp".into(), json!({"id": id, "nom": nom, "unite": unite, "note": note}));
    ctx.insert("offres".into(), Value::Array(liste));
    ctx.insert("par_mesure".into(), json!(par_mesure));
    ctx.insert("unites".into(), json!(UNITES));
    render(&state, "comparateur.html", Value::Object(ctx))
}

pub(super) async fn modifier(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let nom = form_text(&form, "nom").ok_or_else(|| AppError::Invalid("Nom obligatoire".into()))?;
    let r = sqlx::query("UPDATE comparatif SET nom=?,unite=?,note=? WHERE id=?")
        .bind(nom)
        .bind(form_text(&form, "unite").unwrap_or_else(|| "kg".into()))
        .bind(form_text(&form, "note"))
        .bind(id)
        .execute(&state.pool)
        .await?;
    if r.rows_affected() != 1 {
        return Err(AppError::NotFound);
    }
    Ok(Redirect::to(&format!("/comparateur/{id}")).into_response())
}

pub(super) async fn ajouter_offre(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let existe: Option<i64> = sqlx::query_scalar("SELECT id FROM comparatif WHERE id=?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?;
    existe.ok_or(AppError::NotFound)?;
    let produit = form_text(&form, "produit").ok_or_else(|| AppError::Invalid("Nom du produit obligatoire".into()))?;
    let contenance = form_f64(&form, "contenance");
    if contenance.is_some_and(|c| c <= 0.0) {
        return Err(AppError::Invalid("La quantité dans une unité doit être positive".into()));
    }
    let nombre = form_f64(&form, "nombre").unwrap_or(1.0);
    if nombre <= 0.0 {
        return Err(AppError::Invalid("Le nombre d'unités doit être positif".into()));
    }
    let prix = form_f64(&form, "prix")
        .filter(|p| *p >= 0.0)
        .ok_or_else(|| AppError::Invalid("Prix obligatoire".into()))?;
    let pour = if form.get("prix_pour").map(String::as_str) == Some("lot") { "lot" } else { "unite" };
    let total = if pour == "lot" { prix } else { prix * nombre };
    sqlx::query(
        "INSERT INTO comparatif_offre(comparatif_id,produit,fournisseur,contenance,nombre,prix_total,prix_pour,note) VALUES(?,?,?,?,?,?,?,?)",
    )
    .bind(id)
    .bind(&produit)
    .bind(form_text(&form, "fournisseur"))
    .bind(contenance)
    .bind(nombre)
    .bind(total)
    .bind(pour)
    .bind(form_text(&form, "note"))
    .execute(&state.pool)
    .await?;
    db::journal(&state.pool, &session.nom, "ajouter", "offre comparateur", &produit, &format!("/comparateur/{id}")).await;
    Ok(Redirect::to(&format!("/comparateur/{id}")).into_response())
}

pub(super) async fn supprimer_offre(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path((id, offre)): Path<(i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    sqlx::query("DELETE FROM comparatif_offre WHERE id=? AND comparatif_id=?")
        .bind(offre)
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(Redirect::to(&format!("/comparateur/{id}")).into_response())
}

pub(super) async fn supprimer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM comparatif_offre WHERE comparatif_id=?").bind(id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM comparatif WHERE id=?").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    db::journal(&state.pool, &session.nom, "supprimer", "comparatif", &id.to_string(), "/comparateur").await;
    Ok(Redirect::to("/comparateur").into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prix_ramenes_a_l_unite_et_au_kg() {
        // Sac de 25 kg à 12,50 € : 12,50 €/sac et 0,50 €/kg.
        assert_eq!(prix_ramenes(Some(25.0), 1.0, 12.5), (12.5, Some(0.5)));
        // Lot de 40 sacs de 25 kg pour 400 € : 10 €/sac et 0,40 €/kg.
        assert_eq!(prix_ramenes(Some(25.0), 40.0, 400.0), (10.0, Some(0.4)));
        // Sans contenance : seulement le prix par unité.
        assert_eq!(prix_ramenes(None, 4.0, 10.0), (2.5, None));
        assert_eq!(prix_lisible(0.4), "0,400 €");
        assert_eq!(prix_lisible(12.5), "12,50 €");
        assert_eq!(nombre_lisible(25.0), "25");
        assert_eq!(nombre_lisible(0.75), "0,75");
    }

    #[tokio::test]
    async fn offres_classees_du_moins_cher_au_plus_cher() {
        let state = crate::routes::documents::tests::state().await;
        let p = &state.pool;
        sqlx::raw_sql(
            "INSERT INTO comparatif(id,nom,unite) VALUES(1,'Chaux','kg');\
             INSERT INTO comparatif_offre(comparatif_id,produit,fournisseur,contenance,nombre,prix_total,prix_pour) VALUES\
             (1,'Sac 25 kg','A',25,1,12.5,'unite'),\
             (1,'Palette 40 x 25 kg','A',25,40,400,'lot'),\
             (1,'Big bag 500 kg','B',500,1,225,'unite');",
        )
        .execute(p)
        .await
        .unwrap();
        let (liste, par_mesure) = offres(p, 1).await.unwrap();
        assert!(par_mesure);
        let produits: Vec<&str> = liste.iter().map(|o| o["produit"].as_str().unwrap()).collect();
        assert_eq!(produits, ["Palette 40 x 25 kg", "Big bag 500 kg", "Sac 25 kg"]);
        assert_eq!(liste[0]["meilleur"], json!(true));
        assert_eq!(liste[0]["par_mesure"], json!("0,400 €"));
        assert_eq!(liste[1]["ecart"], json!(12.5));
        assert_eq!(liste[2]["ecart"], json!(25.0));
        // Une offre sans contenance : on compare au prix par unité achetée.
        sqlx::query("INSERT INTO comparatif_offre(comparatif_id,produit,nombre,prix_total,prix_pour) VALUES(1,'Seau',1,5,'unite')")
            .execute(p)
            .await
            .unwrap();
        let (liste, par_mesure) = offres(p, 1).await.unwrap();
        assert!(!par_mesure);
        assert_eq!(liste[0]["produit"], json!("Seau"));
    }
}
