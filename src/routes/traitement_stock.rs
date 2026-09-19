use super::*;

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

pub(super) async fn appliquer(conn: &mut sqlx::SqliteConnection, id: i64) -> AppResult<()> {
    let event: Option<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT e.produit,e.date,e.date_fin,b.code FROM evenement e LEFT JOIN bande b ON b.id=e.bande_id WHERE e.id=? AND e.type='traitement' AND e.date_fin IS NOT NULL AND e.produit IS NOT NULL",
    ).bind(id).fetch_optional(&mut *conn).await?;
    let Some((product, start, end, band)) = event else {
        return Ok(());
    };
    let start_day = parse_stored_date(&start)
        .ok_or_else(|| AppError::Invalid("Date de traitement invalide".into()))?;
    let end_day =
        parse_stored_date(&end).ok_or_else(|| AppError::Invalid("Date de fin invalide".into()))?;
    if end_day < start_day {
        return Err(AppError::Invalid(
            "La fin du traitement précède son début".into(),
        ));
    }
    let quantity = ((end_day - start_day).num_days() + 1) as f64;
    sqlx::query("INSERT INTO produitpharmacie(produit,stock_actuel,unite,maj) SELECT ?,0,'doses',CURRENT_TIMESTAMP WHERE NOT EXISTS(SELECT 1 FROM produitpharmacie WHERE lower(produit)=lower(?))")
        .bind(&product).bind(&product).execute(&mut *conn).await?;
    let stock: f64 = sqlx::query_scalar("SELECT CAST(COALESCE(stock_actuel,0) AS REAL) FROM produitpharmacie WHERE lower(produit)=lower(?) LIMIT 1")
        .bind(&product).fetch_one(&mut *conn).await?;
    let actual = quantity.min(stock.max(0.0));
    sqlx::query("UPDATE produitpharmacie SET stock_actuel=?,maj=CURRENT_TIMESTAMP WHERE lower(produit)=lower(?)")
        .bind((stock-actual).max(0.0)).bind(&product).execute(&mut *conn).await?;
    let movement = sqlx::query("INSERT INTO mouvementpharmacie(produit,date,type,quantite,note,bande_code) VALUES(?,?,'sortie',?,?,?)")
        .bind(&product).bind(&start).bind(actual)
        .bind(format!("Traitement #{id} du {start} au {end} : {quantity} doses prévues, {actual} déduites"))
        .bind(band).execute(&mut *conn).await?.last_insert_rowid();
    sqlx::query("INSERT INTO traitementstock(evenement_id,produit,quantite_reelle,mouvement_id) VALUES(?,?,?,?)")
        .bind(id).bind(product).bind(actual).bind(movement).execute(&mut *conn).await?;
    Ok(())
}
