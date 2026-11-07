use super::*;

/// Sortie de stock pharmacie liée à un soin. Le stock peut passer sous zéro :
/// la consommation reste comptée et le produit s'affiche en rouge, signe qu'un
/// inventaire est à faire (auparavant la sortie était plafonnée au stock et
/// la consommation se perdait sans rien dire).
pub(super) async fn sortie(
    conn: &mut sqlx::SqliteConnection,
    produit: &str,
    date: &str,
    quantite: f64,
    note: &str,
    bande_code: Option<&str>,
) -> AppResult<i64> {
    sqlx::query("INSERT INTO produitpharmacie(produit,stock_actuel,unite,maj) SELECT ?,0,'doses',CURRENT_TIMESTAMP WHERE NOT EXISTS(SELECT 1 FROM produitpharmacie WHERE lower(produit)=lower(?))")
        .bind(produit).bind(produit).execute(&mut *conn).await?;
    sqlx::query("UPDATE produitpharmacie SET stock_actuel=COALESCE(stock_actuel,0)-?,maj=CURRENT_TIMESTAMP WHERE lower(produit)=lower(?)")
        .bind(quantite).bind(produit).execute(&mut *conn).await?;
    Ok(sqlx::query("INSERT INTO mouvementpharmacie(produit,date,type,quantite,note,bande_code) VALUES(?,?,'sortie',?,?,?)")
        .bind(produit).bind(date).bind(quantite).bind(note).bind(bande_code)
        .execute(&mut *conn).await?.last_insert_rowid())
}

/// Annule une sortie faite par [`sortie`] : remet les doses en stock et
/// supprime le mouvement, sauf si un inventaire a été fait depuis.
pub(super) async fn restituer(conn: &mut sqlx::SqliteConnection, mouvement: i64) -> AppResult<()> {
    let old: Option<(String, f64)> = sqlx::query_as(
        "SELECT produit,CAST(COALESCE(quantite,0) AS REAL) FROM mouvementpharmacie WHERE id=? AND type='sortie'",
    )
    .bind(mouvement)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((product, quantity)) = old else {
        return Ok(());
    };
    let inventory: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mouvementpharmacie WHERE lower(produit)=lower(?) AND type='inventaire' AND id>?")
        .bind(&product).bind(mouvement).fetch_one(&mut *conn).await?;
    if inventory > 0 {
        return Err(AppError::Invalid("Un inventaire pharmacie a été réalisé depuis ce soin. La correction automatique est impossible ; corrigez le stock à la main".into()));
    }
    sqlx::query("UPDATE produitpharmacie SET stock_actuel=COALESCE(stock_actuel,0)+?,maj=CURRENT_TIMESTAMP WHERE lower(produit)=lower(?)")
        .bind(quantity).bind(&product).execute(&mut *conn).await?;
    sqlx::query("DELETE FROM mouvementpharmacie WHERE id=?")
        .bind(mouvement)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Le soin est-il déjà relié à une sortie de stock ?
pub(super) async fn lie(conn: &mut sqlx::SqliteConnection, id: i64) -> AppResult<bool> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM traitementstock WHERE evenement_id=?")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(n > 0)
}

pub(super) async fn annuler(conn: &mut sqlx::SqliteConnection, id: i64) -> AppResult<()> {
    let old: Option<(String, f64, i64)> = sqlx::query_as(
        "SELECT produit,quantite_reelle,mouvement_id FROM traitementstock WHERE evenement_id=?",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some((product, quantity, movement)) = old {
        let inventory: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mouvementpharmacie WHERE lower(produit)=lower(?) AND type='inventaire' AND id>?")
            .bind(&product).bind(movement).fetch_one(&mut *conn).await?;
        if inventory > 0 {
            return Err(AppError::Invalid("Un inventaire pharmacie a été réalisé depuis ce traitement. La correction automatique est impossible ; utilisez la correction manuelle du stock".into()));
        }
        sqlx::query("UPDATE produitpharmacie SET stock_actuel=COALESCE(stock_actuel,0)+?,maj=CURRENT_TIMESTAMP WHERE lower(produit)=lower(?)")
            .bind(quantity).bind(&product).execute(&mut *conn).await?;
        sqlx::query("DELETE FROM traitementstock WHERE evenement_id=?")
            .bind(id)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM mouvementpharmacie WHERE id=?")
            .bind(movement)
            .execute(&mut *conn)
            .await?;
    } else {
        let legacy: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM evenement WHERE id=? AND type='traitement' AND date_fin IS NOT NULL AND produit IS NOT NULL")
            .bind(id).fetch_one(&mut *conn).await?;
        if legacy > 0 {
            return Err(AppError::Invalid("Ce traitement ancien n'est pas lié à un mouvement pharmacie. Utilisez la correction manuelle du stock".into()));
        }
    }
    Ok(())
}

/// Sort du stock les doses d'un soin : doses par jour (`nb_doses`, 1 par
/// défaut) × nombre de jours (du début à `date_fin`, 1 jour sans date de fin).
pub(super) async fn appliquer(conn: &mut sqlx::SqliteConnection, id: i64) -> AppResult<()> {
    let event: Option<(String, String, Option<String>, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT e.produit,e.date,e.date_fin,e.nb_doses,b.code FROM evenement e LEFT JOIN bande b ON b.id=e.bande_id WHERE e.id=? AND e.type='traitement' AND e.produit IS NOT NULL",
    ).bind(id).fetch_optional(&mut *conn).await?;
    let Some((product, start, end, doses, band)) = event else {
        return Ok(());
    };
    let start_day = parse_stored_date(&start)
        .ok_or_else(|| AppError::Invalid("Date de traitement invalide".into()))?;
    let end_day = match end.as_deref() {
        Some(end) => {
            parse_stored_date(end).ok_or_else(|| AppError::Invalid("Date de fin invalide".into()))?
        }
        None => start_day,
    };
    if end_day < start_day {
        return Err(AppError::Invalid(
            "La fin du traitement précède son début".into(),
        ));
    }
    let days = (end_day - start_day).num_days() + 1;
    let per_day = doses.filter(|d| *d > 0).unwrap_or(1);
    let quantity = (days * per_day) as f64;
    let note = match end.as_deref() {
        Some(end) if days > 1 => format!("Traitement #{id} du {start} au {end} : {per_day} dose(s) par jour pendant {days} jours"),
        _ => format!("Traitement #{id} du {start} : {per_day} dose(s)"),
    };
    let movement = sortie(conn, &product, &start, quantity, &note, band.as_deref()).await?;
    sqlx::query("INSERT INTO traitementstock(evenement_id,produit,quantite_reelle,mouvement_id) VALUES(?,?,?,?)")
        .bind(id).bind(product).bind(quantity).bind(movement).execute(&mut *conn).await?;
    Ok(())
}
