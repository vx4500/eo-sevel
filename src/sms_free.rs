//! Alertes par SMS via l'option « Notifications par SMS » de Free Mobile :
//! un SMS vers le numéro de l'abonné, par un simple appel HTTP.
//!
//! Réglages dans `parametre` : `sms_free_user`, `sms_free_pass`,
//! `sms_free_actif` ("1"), `sms_free_niveau` ("critique" ou "alerte").
//! La boucle d'envoi tient aussi un battement (`sms_battement`, heure Unix) :
//! au démarrage, un battement trop ancien signifie que le serveur était
//! arrêté (coupure de courant le plus souvent) et un SMS le signale.

use sqlx::SqlitePool;

const URL: &str = "https://smsapi.free-mobile.fr/sendmsg";
/// Au-delà, un arrêt du serveur est signalé au redémarrage.
const ARRET_SIGNALE_S: i64 = 300;
/// Garde-fou : au plus ce nombre d'alertes détaillées par minute.
const ALERTES_PAR_SMS: usize = 5;

async fn parametre(pool: &SqlitePool, cle: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>("SELECT valeur FROM parametre WHERE cle=?")
        .bind(cle)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .flatten()
        .filter(|v| !v.trim().is_empty())
}

pub async fn ecrire_parametre(pool: &SqlitePool, cle: &str, valeur: &str) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO parametre(cle,valeur) VALUES(?,?) ON CONFLICT(cle) DO UPDATE SET valeur=excluded.valeur")
        .bind(cle)
        .bind(valeur)
        .execute(pool)
        .await?;
    Ok(())
}

/// Sens des codes de retour documentés par Free.
pub fn sens_code(code: u16) -> &'static str {
    match code {
        200 => "envoyé",
        400 => "paramètre manquant",
        402 => "trop de SMS envoyés en peu de temps",
        403 => "option non activée ou identifiant/clé incorrects",
        500 => "erreur côté Free, réessayer plus tard",
        _ => "réponse inattendue",
    }
}

/// Envoie un SMS avec les identifiants enregistrés et le trace dans
/// `sms_envoi`. Renvoie le texte du résultat pour l'affichage.
pub async fn envoyer(pool: &SqlitePool, message: &str, origine: &str) -> Result<(), String> {
    let (Some(user), Some(pass)) = (
        parametre(pool, "sms_free_user").await,
        parametre(pool, "sms_free_pass").await,
    ) else {
        return Err("identifiant et clé Free à renseigner".into());
    };
    let message: String = message.chars().take(900).collect();
    let resultat = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(client) => match client
            .get(URL)
            .query(&[("user", user.trim()), ("pass", pass.trim()), ("msg", message.as_str())])
            .send()
            .await
        {
            Ok(r) if r.status().as_u16() == 200 => Ok(()),
            Ok(r) => Err(format!("Free : {} ({})", sens_code(r.status().as_u16()), r.status().as_u16())),
            Err(e) => Err(format!("Free injoignable : {}", e.without_url())),
        },
        Err(e) => Err(format!("client HTTP : {e}")),
    };
    let statut = match &resultat {
        Ok(()) => "envoyé".to_string(),
        Err(e) => e.clone(),
    };
    let _ = sqlx::query("INSERT INTO sms_envoi(horodatage,origine,message,statut) VALUES(datetime('now','localtime'),?,?,?)")
        .bind(origine)
        .bind(&message)
        .bind(&statut)
        .execute(pool)
        .await;
    if let Err(e) = &resultat {
        tracing::warn!(erreur=%e, "SMS Free non envoyé");
    }
    resultat
}

async fn actif(pool: &SqlitePool) -> bool {
    parametre(pool, "sms_free_actif").await.as_deref() == Some("1")
}

fn heure(ts: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%d/%m %H:%M").to_string())
        .unwrap_or_else(|| "?".into())
}

/// Durée lisible : « 10 h 30 », « 12 min ».
pub fn duree(secondes: i64) -> String {
    let minutes = secondes / 60;
    if minutes >= 60 {
        format!("{} h {:02}", minutes / 60, minutes % 60)
    } else {
        format!("{minutes} min")
    }
}

/// Texte du SMS pour un lot de nouvelles alertes.
pub fn texte_alertes(alertes: &[(String, Option<String>, String)]) -> String {
    let mut texte = String::from("EO_Sevel ALERTE");
    for (gravite, salle, message) in alertes.iter().take(ALERTES_PAR_SMS) {
        texte.push_str(&format!(
            "\n- {}{}{}",
            if gravite == "critique" { "CRITIQUE " } else { "" },
            salle.as_deref().map(|s| format!("{s} : ")).unwrap_or_default(),
            message
        ));
    }
    if alertes.len() > ALERTES_PAR_SMS {
        texte.push_str(&format!("\n+ {} autre(s) alerte(s)", alertes.len() - ALERTES_PAR_SMS));
    }
    texte
}

async fn tour(pool: &SqlitePool) -> anyhow::Result<()> {
    let maintenant = chrono::Utc::now().timestamp();
    ecrire_parametre(pool, "sms_battement", &maintenant.to_string()).await?;
    let dernier: i64 = parametre(pool, "sms_dernier_id_alerte").await.and_then(|v| v.parse().ok()).unwrap_or(0);
    let niveau = parametre(pool, "sms_free_niveau").await.unwrap_or_else(|| "critique".into());
    let filtre = if niveau == "alerte" { "" } else { "AND a.gravite='critique'" };
    let alertes: Vec<(i64, String, Option<String>, String)> = sqlx::query_as(&format!(
        "SELECT a.id,a.gravite,s.nom,a.message FROM ambiance_alerte a LEFT JOIN salle s ON s.id=a.salle_id \
         WHERE a.id>? AND a.fin IS NULL AND a.acquittee=0 {filtre} ORDER BY a.id"
    ))
    .bind(dernier)
    .fetch_all(pool)
    .await?;
    let max_id: Option<i64> = sqlx::query_scalar("SELECT MAX(id) FROM ambiance_alerte").fetch_one(pool).await?;
    if !alertes.is_empty() && actif(pool).await {
        let lignes: Vec<_> = alertes.iter().map(|(_, g, s, m)| (g.clone(), s.clone(), m.clone())).collect();
        // En cas d'échec (Free injoignable), on réessaie au tour suivant.
        if envoyer(pool, &texte_alertes(&lignes), "alerte").await.is_err() {
            return Ok(());
        }
    }
    if let Some(max_id) = max_id.filter(|m| *m > dernier) {
        ecrire_parametre(pool, "sms_dernier_id_alerte", &max_id.to_string()).await?;
    }
    Ok(())
}

/// Boucle lancée au démarrage d'eo-sevel.
pub async fn boucle(pool: SqlitePool) {
    let maintenant = chrono::Utc::now().timestamp();
    if let Some(avant) = parametre(&pool, "sms_battement").await.and_then(|v| v.parse::<i64>().ok()) {
        let arret = maintenant - avant;
        if arret > ARRET_SIGNALE_S && actif(&pool).await {
            let texte = format!(
                "EO_Sevel : serveur revenu le {}. Arrêté depuis le {} ({}). Coupure de courant probable : vérifier ventilation et salles.",
                heure(maintenant),
                heure(avant),
                duree(arret)
            );
            // La box peut mettre quelques minutes à retrouver internet.
            for _ in 0..10 {
                if envoyer(&pool, &texte, "redemarrage").await.is_ok() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        }
    }
    // Premier lancement : ne pas envoyer tout l'historique des alertes.
    if parametre(&pool, "sms_dernier_id_alerte").await.is_none() {
        if let Ok(Some(max_id)) = sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(id) FROM ambiance_alerte").fetch_one(&pool).await {
            let _ = ecrire_parametre(&pool, "sms_dernier_id_alerte", &max_id.to_string()).await;
        }
    }
    loop {
        if let Err(e) = tour(&pool).await {
            tracing::error!(error=%e, "Boucle SMS Free");
        }
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn textes() {
        assert_eq!(duree(37_800), "10 h 30");
        assert_eq!(duree(720), "12 min");
        let a = vec![("critique".to_string(), Some("PS1".to_string()), "Température 35 °C".to_string())];
        assert_eq!(texte_alertes(&a), "EO_Sevel ALERTE\n- CRITIQUE PS1 : Température 35 °C");
        let many: Vec<_> = (0..7).map(|i| ("alerte".to_string(), None, format!("a{i}"))).collect();
        assert!(texte_alertes(&many).ends_with("+ 2 autre(s) alerte(s)"));
        assert_eq!(sens_code(403), "option non activée ou identifiant/clé incorrects");
    }
}
