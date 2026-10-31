//! Entretien des séries d'ambiance : repliement des mesures et purge.
//!
//! Le démon écrit des mesures sur variation, ce qui est déjà économe. Ce
//! module fait le reste du travail : il replie le brut en moyennes horaires,
//! puis les heures en journées, et supprime ce qui a été replié.
//!
//! Ordre de grandeur visé pour treize salles suivies en continu : une centaine
//! de mégaoctets pour plusieurs années, contre plusieurs gigaoctets par an si
//! l'on gardait tout le détail. Le détail à la minute ne sert que sur les jours
//! récents ; ce qui compte à long terme, c'est de pouvoir croiser l'ambiance
//! d'une salle avec les performances de la bande qui l'occupait.

use sqlx::SqlitePool;

/// Rétention du détail brut, en jours.
const RETENTION_BRUT_DEFAUT: i64 = 30;
/// Rétention des moyennes horaires, en jours. Les moyennes journalières, elles,
/// ne sont jamais purgées.
const RETENTION_HEURE_DEFAUT: i64 = 730;

async fn parametre_i64(pool: &SqlitePool, cle: &str, defaut: i64) -> i64 {
    let valeur: Option<Option<String>> =
        sqlx::query_scalar("SELECT valeur FROM parametre WHERE cle=?")
            .bind(cle)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    valeur
        .flatten()
        .and_then(|texte| texte.trim().parse::<i64>().ok())
        .filter(|valeur| *valeur > 0)
        .unwrap_or(defaut)
}

/// Replie le brut en heures, les heures en jours, puis purge ce qui est
/// devenu redondant. Idempotent : le relancer ne fausse rien.
pub async fn replier(pool: &SqlitePool) -> anyhow::Result<()> {
    let maintenant = chrono::Utc::now().timestamp();
    // Seules les heures révolues sont repliées : agréger l'heure en cours
    // donnerait une moyenne partielle qu'il faudrait corriger au coup d'après.
    let heure_courante = (maintenant / 3600) * 3600;
    // On ne retraite que les deux derniers jours : au-delà, le travail a déjà
    // été fait lors d'un passage précédent.
    let depuis = heure_courante - 48 * 3600;

    sqlx::query(
        "INSERT INTO ambiance_heure(salle_id,grandeur,heure,moy,mini,maxi,n,bande_id) \
         SELECT salle_id, grandeur, (ts/3600)*3600, \
                CAST(ROUND(AVG(valeur)) AS INTEGER), MIN(valeur), MAX(valeur), COUNT(*), \
                MAX(bande_id) \
         FROM ambiance_mesure WHERE ts >= ? AND ts < ? \
         GROUP BY salle_id, grandeur, (ts/3600)*3600 \
         ON CONFLICT(salle_id,grandeur,heure) DO UPDATE SET \
           moy=excluded.moy, mini=excluded.mini, maxi=excluded.maxi, \
           n=excluded.n, bande_id=excluded.bande_id",
    )
    .bind(depuis)
    .bind(heure_courante)
    .execute(pool)
    .await?;

    // Journalier, pondéré par le nombre de points de chaque heure : une heure
    // où la sonde a beaucoup bougé pèse plus qu'une heure calme, ce qui est le
    // comportement attendu d'une moyenne.
    let depuis_jour = heure_courante - 7 * 24 * 3600;
    sqlx::query(
        "INSERT INTO ambiance_jour(salle_id,grandeur,jour,moy,mini,maxi,n,minutes_hors_plage,bande_id) \
         SELECT salle_id, grandeur, date(heure,'unixepoch','localtime'), \
                CAST(ROUND(SUM(moy*n)*1.0/NULLIF(SUM(n),0)) AS INTEGER), \
                MIN(mini), MAX(maxi), SUM(n), 0, MAX(bande_id) \
         FROM ambiance_heure WHERE heure >= ? \
         GROUP BY salle_id, grandeur, date(heure,'unixepoch','localtime') \
         ON CONFLICT(salle_id,grandeur,jour) DO UPDATE SET \
           moy=excluded.moy, mini=excluded.mini, maxi=excluded.maxi, \
           n=excluded.n, bande_id=excluded.bande_id",
    )
    .bind(depuis_jour)
    .execute(pool)
    .await?;

    // Temps passé hors des seuils d'alerte de la salle, compté à l'heure
    // près : une heure dont le maximum dépasse le seuil compte pour soixante
    // minutes. C'est volontairement grossier — la valeur sert à comparer des
    // bandes entre elles, pas à facturer des minutes.
    let jour_depuis = chrono::Local::now()
        .date_naive()
        .checked_sub_days(chrono::Days::new(7))
        .map(|date| date.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    sqlx::query(
        "UPDATE ambiance_jour SET minutes_hors_plage = COALESCE(( \
             SELECT COUNT(*)*60 FROM ambiance_heure h \
             JOIN ambiance_salle a ON a.salle_id = h.salle_id \
             WHERE h.salle_id = ambiance_jour.salle_id AND h.grandeur='t' \
               AND date(h.heure,'unixepoch','localtime') = ambiance_jour.jour \
               AND ((a.t_alerte_max IS NOT NULL AND h.maxi > CAST(a.t_alerte_max*10 AS INTEGER)) \
                 OR (a.t_alerte_min IS NOT NULL AND h.mini < CAST(a.t_alerte_min*10 AS INTEGER))) \
         ),0) WHERE grandeur='t' AND jour >= ?",
    )
    .bind(&jour_depuis)
    .execute(pool)
    .await?;

    let retention_brut =
        parametre_i64(pool, "ambiance_retention_brut_jours", RETENTION_BRUT_DEFAUT)
            .await
            .max(2);
    let retention_heure = parametre_i64(
        pool,
        "ambiance_retention_heure_jours",
        RETENTION_HEURE_DEFAUT,
    )
    .await
    .max(retention_brut + 1);

    // La purge ne descend jamais en dessous de la dernière heure repliée :
    // impossible de supprimer du brut qui n'aurait pas encore été agrégé.
    let limite_brut = (heure_courante - retention_brut * 24 * 3600).min(heure_courante);
    sqlx::query("DELETE FROM ambiance_mesure WHERE ts < ?")
        .bind(limite_brut)
        .execute(pool)
        .await?;
    let limite_heure = heure_courante - retention_heure * 24 * 3600;
    sqlx::query("DELETE FROM ambiance_heure WHERE heure < ?")
        .bind(limite_heure)
        .execute(pool)
        .await?;

    // Alertes résolues et anciennes : on garde un an, de quoi expliquer une
    // contre-performance de bande après coup.
    sqlx::query(
        "DELETE FROM ambiance_alerte WHERE fin IS NOT NULL AND fin < datetime('now','-365 days')",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Au-delà de ce délai sans battement, la régulation est tenue pour arrêtée.
const REGULATION_MUETTE_S: i64 = 180;

/// Vérifie que la régulation vit tant qu'elle a des salles à piloter. Si elle
/// s'arrête, rien ne reprend la main : les sorties restent figées à leur
/// dernière valeur, d'où l'alerte critique.
pub async fn verifier_regulation(pool: &SqlitePool, maintenant: i64) -> anyhow::Result<()> {
    let pilotees: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ambiance_salle WHERE actif=1 AND pilote=1")
            .fetch_one(pool)
            .await?;
    let battement: Option<i64> = sqlx::query_scalar("SELECT ts FROM ambiance_heartbeat WHERE id=1")
        .fetch_optional(pool)
        .await?;
    let arretee = pilotees > 0
        && battement.is_none_or(|ts| maintenant - ts > REGULATION_MUETTE_S);
    if arretee {
        sqlx::query(
            "INSERT INTO ambiance_alerte(salle_id,type,gravite,debut,message) \
             SELECT NULL,'regulation_arretee','critique',CURRENT_TIMESTAMP,? \
             WHERE NOT EXISTS(SELECT 1 FROM ambiance_alerte \
                              WHERE type='regulation_arretee' AND fin IS NULL)",
        )
        .bind("Régulation de ventilation arrêtée : les sorties restent figées à leur dernière valeur")
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            "UPDATE ambiance_alerte SET fin=CURRENT_TIMESTAMP \
             WHERE type='regulation_arretee' AND fin IS NULL",
        )
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Surveillance de la régulation, lancée au démarrage du serveur. La première
/// passe attend que le démon ait redémarré lui aussi après une mise à jour.
pub async fn boucle_regulation(pool: SqlitePool) {
    tokio::time::sleep(std::time::Duration::from_secs(120)).await;
    loop {
        let maintenant = chrono::Utc::now().timestamp();
        if let Err(error) = verifier_regulation(&pool, maintenant).await {
            tracing::error!(error = %error, "surveillance de la régulation impossible");
        }
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}

/// Boucle d'entretien, lancée au démarrage du serveur. Une passe par heure
/// suffit largement, et un échec n'interrompt jamais la boucle.
pub async fn boucle_entretien(pool: SqlitePool) {
    loop {
        if let Err(error) = replier(&pool).await {
            tracing::error!(error = %error, "repliement des mesures d'ambiance impossible");
        }
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn base() -> anyhow::Result<SqlitePool> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::raw_sql(include_str!("../migrations/0001_schema.sql"))
            .execute(&pool)
            .await?;
        sqlx::raw_sql(include_str!("../migrations/0005_ambiance.sql"))
            .execute(&pool)
            .await?;
        Ok(pool)
    }

    async fn alertes_regulation(pool: &SqlitePool) -> anyhow::Result<(i64, i64)> {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM ambiance_alerte WHERE type='regulation_arretee'",
        )
        .fetch_one(pool)
        .await?;
        let ouvertes: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM ambiance_alerte WHERE type='regulation_arretee' AND fin IS NULL",
        )
        .fetch_one(pool)
        .await?;
        Ok((total, ouvertes))
    }

    #[tokio::test]
    async fn une_regulation_muette_alerte_une_seule_fois_puis_se_ferme() -> anyhow::Result<()> {
        let pool = base().await?;
        sqlx::query("INSERT INTO site(id,code) VALUES(1,'S1')").execute(&pool).await?;
        sqlx::query("INSERT INTO salle(id,site_id,nom) VALUES(1,1,'Salle')").execute(&pool).await?;
        sqlx::query("INSERT INTO ambiance_salle(salle_id,actif,pilote) VALUES(1,1,1)")
            .execute(&pool)
            .await?;
        sqlx::query("INSERT INTO ambiance_heartbeat(id,ts) VALUES(1,1000)").execute(&pool).await?;

        verifier_regulation(&pool, 1180).await?;
        assert_eq!(alertes_regulation(&pool).await?, (0, 0));
        verifier_regulation(&pool, 1181).await?;
        verifier_regulation(&pool, 1241).await?;
        assert_eq!(alertes_regulation(&pool).await?, (1, 1));

        sqlx::query("UPDATE ambiance_heartbeat SET ts=1300").execute(&pool).await?;
        verifier_regulation(&pool, 1310).await?;
        assert_eq!(alertes_regulation(&pool).await?, (1, 0));
        Ok(())
    }

    #[tokio::test]
    async fn sans_salle_pilotee_le_silence_de_la_regulation_est_normal() -> anyhow::Result<()> {
        let pool = base().await?;
        verifier_regulation(&pool, 100_000).await?;
        assert_eq!(alertes_regulation(&pool).await?, (0, 0));
        Ok(())
    }

    #[tokio::test]
    async fn le_brut_se_replie_en_heures_puis_en_jours() -> anyhow::Result<()> {
        let pool = base().await?;
        sqlx::query("INSERT INTO site(id,code) VALUES(1,'S1')")
            .execute(&pool)
            .await?;
        sqlx::query("INSERT INTO salle(id,site_id,nom) VALUES(1,1,'Mater 1')")
            .execute(&pool)
            .await?;
        sqlx::query("INSERT INTO ambiance_salle(salle_id,actif) VALUES(1,1)")
            .execute(&pool)
            .await?;

        // Une heure révolue, trois mesures : 24,0 / 25,0 / 26,0 °C.
        let heure = ((chrono::Utc::now().timestamp() / 3600) * 3600) - 3600;
        for (decalage, valeur) in [(10_i64, 240_i64), (20, 250), (30, 260)] {
            sqlx::query(
                "INSERT INTO ambiance_mesure(salle_id,grandeur,ts,valeur) VALUES(1,'t',?,?)",
            )
            .bind(heure + decalage)
            .bind(valeur)
            .execute(&pool)
            .await?;
        }

        replier(&pool).await?;

        let ligne: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT moy,mini,maxi,n FROM ambiance_heure WHERE salle_id=1 AND grandeur='t'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(ligne, (250, 240, 260, 3));

        let jours: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM ambiance_jour WHERE salle_id=1 AND grandeur='t'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(jours, 1);
        Ok(())
    }

    #[tokio::test]
    async fn le_repliement_est_idempotent() -> anyhow::Result<()> {
        let pool = base().await?;
        sqlx::query("INSERT INTO site(id,code) VALUES(1,'S1')")
            .execute(&pool)
            .await?;
        sqlx::query("INSERT INTO salle(id,site_id,nom) VALUES(1,1,'PS1')")
            .execute(&pool)
            .await?;
        let heure = ((chrono::Utc::now().timestamp() / 3600) * 3600) - 3600;
        sqlx::query("INSERT INTO ambiance_mesure(salle_id,grandeur,ts,valeur) VALUES(1,'t',?,280)")
            .bind(heure + 5)
            .execute(&pool)
            .await?;
        replier(&pool).await?;
        replier(&pool).await?;
        let lignes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ambiance_heure")
            .fetch_one(&pool)
            .await?;
        assert_eq!(lignes, 1);
        Ok(())
    }

    /// Le brut ancien disparaît, mais l'agrégat qui le résume reste : c'est
    /// tout l'intérêt de la manœuvre.
    #[tokio::test]
    async fn la_purge_garde_les_agregats() -> anyhow::Result<()> {
        let pool = base().await?;
        sqlx::query("INSERT INTO site(id,code) VALUES(1,'S1')")
            .execute(&pool)
            .await?;
        sqlx::query("INSERT INTO salle(id,site_id,nom) VALUES(1,1,'EN1')")
            .execute(&pool)
            .await?;
        let vieux = chrono::Utc::now().timestamp() - 90 * 24 * 3600;
        sqlx::query("INSERT INTO ambiance_mesure(salle_id,grandeur,ts,valeur) VALUES(1,'t',?,220)")
            .bind(vieux)
            .execute(&pool)
            .await?;
        sqlx::query(
            "INSERT INTO ambiance_heure(salle_id,grandeur,heure,moy,mini,maxi,n) \
             VALUES(1,'t',?,220,220,220,1)",
        )
        .bind((vieux / 3600) * 3600)
        .execute(&pool)
        .await?;

        replier(&pool).await?;

        let brut: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ambiance_mesure")
            .fetch_one(&pool)
            .await?;
        let heures: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ambiance_heure")
            .fetch_one(&pool)
            .await?;
        assert_eq!(brut, 0);
        assert_eq!(heures, 1);
        Ok(())
    }
}
