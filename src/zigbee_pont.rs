//! Pont Zigbee du démon `eo-regul` : Zigbee2MQTT ⇄ base EO_Sevel.
//!
//! Zigbee2MQTT publie l'état de chaque appareil sur `zigbee2mqtt/<nom>` et la
//! liste des appareils sur `zigbee2mqtt/bridge/devices`. Le pont les range en
//! base (`zigbee_appareil`, `zigbee_valeur`, `zigbee_mesure`) et publie les
//! commandes déposées dans `zigbee_commande` par l'application ou par le démon
//! lui-même (éclairage). Personne d'autre ne parle au MQTT.
//!
//! Écritures bornées : un compteur électrique publie toutes les deux
//! secondes. Une valeur n'est réécrite en base que si elle change, ou au plus
//! toutes les cinq minutes ; l'historique ne garde que les grandeurs lentes.

// Compilé uniquement dans le démon ; certaines aides ne servent qu'aux tests.
#![allow(dead_code)]

use anyhow::Result;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde_json::Value;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const BASE: &str = "zigbee2mqtt";
/// Une valeur inchangée est tout de même réécrite à cet intervalle, pour que
/// « vu il y a » reste juste.
const REECRITURE_S: i64 = 300;
/// Grandeurs conservées dans l'historique.
const HISTORISEES: &[&str] = &[
    "temperature",
    "humidity",
    "battery",
    "illuminance",
    "illuminance_lux",
    // Courbe de puissance des compteurs (surveillance électrique).
    "power",
    "total_power",
];
/// Intervalle minimal entre deux points d'historique d'une même grandeur.
const HISTORIQUE_PAS_S: i64 = 120;
const HISTORIQUE_JOURS: i64 = 90;

fn maintenant() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Catégorie d'un appareil d'après ce qu'il expose.
pub fn categoriser(grandeurs: &[String]) -> &'static str {
    let a = |nom: &str| grandeurs.iter().any(|g| g == nom);
    if a("temperature") || a("humidity") {
        "sonde"
    } else if a("energy") || (a("power") && !a("switch") && !a("light")) {
        "compteur"
    } else if a("light") || a("brightness") {
        "variateur"
    } else if a("switch") || a("state") {
        "interrupteur"
    } else if a("vibration") {
        "vibration"
    } else {
        "autre"
    }
}

/// Noms des grandeurs exposées par une définition Zigbee2MQTT (y compris
/// celles imbriquées dans un « light » ou un « switch »).
fn grandeurs_exposees(exposes: &Value) -> Vec<String> {
    let mut sortie = Vec::new();
    fn parcourir(v: &Value, sortie: &mut Vec<String>) {
        match v {
            Value::Array(liste) => liste.iter().for_each(|e| parcourir(e, sortie)),
            Value::Object(objet) => {
                if let Some(nom) = objet.get("name").and_then(Value::as_str) {
                    sortie.push(nom.to_string());
                } else if let Some(genre) = objet.get("type").and_then(Value::as_str) {
                    // « light », « switch » : le type porte le sens.
                    if matches!(genre, "light" | "switch" | "lock" | "cover" | "fan" | "climate") {
                        sortie.push(genre.to_string());
                    }
                }
                if let Some(fonctions) = objet.get("features") {
                    parcourir(fonctions, sortie);
                }
            }
            _ => {}
        }
    }
    parcourir(exposes, &mut sortie);
    sortie.sort();
    sortie.dedup();
    sortie
}

/// Valeur numérique d'un champ d'état : nombres, booléens, ON/OFF.
pub fn en_nombre(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::String(t) => match t.to_ascii_uppercase().as_str() {
            "ON" | "OPEN" | "TRUE" | "ONLINE" => Some(1.0),
            "OFF" | "CLOSE" | "CLOSED" | "FALSE" | "OFFLINE" => Some(0.0),
            _ => t.trim().replace(',', ".").parse().ok(),
        },
        _ => None,
    }
}

/// Charge JSON à publier pour mettre `cle` à `valeur`.
pub fn charge_commande(cle: &str, valeur: f64) -> String {
    let charge = match cle {
        "state" => serde_json::json!({ "state": if valeur >= 0.5 { "ON" } else { "OFF" } }),
        "brightness" => serde_json::json!({ "brightness": valeur.round().clamp(0.0, 255.0) as i64 }),
        autre => serde_json::json!({ autre: valeur }),
    };
    charge.to_string()
}

/// `<ieee>/<cle>` → (ieee, cle).
pub fn decouper_chemin(chemin: &str) -> Option<(&str, &str)> {
    let (ieee, cle) = chemin.trim().split_once('/')?;
    (ieee.starts_with("0x") && !cle.is_empty()).then_some((ieee, cle))
}

#[derive(Default)]
struct Cache {
    /// nom (friendly_name) → ieee
    noms: HashMap<String, String>,
    /// (ieee, cle) → (valeur écrite, instant d'écriture)
    ecrit: HashMap<(String, String), (Option<f64>, Option<String>, i64)>,
    historique: HashMap<(String, String), i64>,
    vu: HashMap<String, i64>,
}

/// Boucle du pont : ne rend jamais la main. Une coupure du MQTT est
/// journalisée et retentée ; elle n'affecte pas la régulation.
pub async fn lancer(pool: SqlitePool, hote: String, port: u16) {
    let cache = Arc::new(Mutex::new(Cache::default()));
    // Noms connus dès le départ : l'état d'un appareil peut arriver avant la
    // liste des appareils.
    if let Ok(lignes) = sqlx::query_as::<_, (String, String)>("SELECT nom, ieee FROM zigbee_appareil")
        .fetch_all(&pool)
        .await
    {
        let mut c = cache.lock().unwrap();
        for (nom, ieee) in lignes {
            c.noms.insert(nom, ieee);
        }
    }

    let mut options = MqttOptions::new(format!("eo-regul-{}", std::process::id()), hote, port);
    options.set_keep_alive(Duration::from_secs(30));
    options.set_max_packet_size(4 * 1024 * 1024, 64 * 1024);
    let (client, mut boucle) = AsyncClient::new(options, 64);

    tokio::spawn(envoyer_commandes(pool.clone(), client.clone()));

    let mut messages: i64 = 0;
    let mut dernier_battement = 0;
    loop {
        match boucle.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                tracing::info!("pont Zigbee : connecté au MQTT");
                if let Err(error) = client.subscribe(format!("{BASE}/#"), QoS::AtMostOnce).await {
                    tracing::error!(error = %error, "pont Zigbee : abonnement impossible");
                }
                let _ = battement(&pool, true, messages, None).await;
            }
            Ok(Event::Incoming(Packet::Publish(message))) => {
                messages += 1;
                if let Err(error) = traiter(&pool, &cache, &message.topic, &message.payload).await {
                    tracing::warn!(sujet = %message.topic, error = %error, "pont Zigbee : message ignoré");
                }
                let ts = maintenant();
                if ts - dernier_battement >= 30 {
                    dernier_battement = ts;
                    let _ = battement(&pool, true, messages, None).await;
                }
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(error = %error, "pont Zigbee : MQTT injoignable, nouvel essai dans 10 s");
                let _ = battement(&pool, false, messages, Some(error.to_string())).await;
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }
    }
}

async fn battement(pool: &SqlitePool, connecte: bool, messages: i64, erreur: Option<String>) -> Result<()> {
    sqlx::query(
        "INSERT INTO zigbee_pont(id,connecte,ts,messages,erreur) VALUES(1,?,?,?,?) \
         ON CONFLICT(id) DO UPDATE SET connecte=excluded.connecte, ts=excluded.ts, \
         messages=excluded.messages, erreur=excluded.erreur",
    )
    .bind(i64::from(connecte))
    .bind(maintenant())
    .bind(messages)
    .bind(erreur)
    .execute(pool)
    .await?;
    Ok(())
}

async fn traiter(pool: &SqlitePool, cache: &Arc<Mutex<Cache>>, sujet: &str, charge: &[u8]) -> Result<()> {
    let Some(reste) = sujet.strip_prefix(&format!("{BASE}/")) else {
        return Ok(());
    };
    if reste == "bridge/devices" {
        return appareils(pool, cache, charge).await;
    }
    if reste.starts_with("bridge/") {
        return Ok(());
    }
    if let Some(nom) = reste.strip_suffix("/availability") {
        let texte = String::from_utf8_lossy(charge);
        let en_ligne = serde_json::from_str::<Value>(&texte)
            .ok()
            .and_then(|v| v.get("state").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| texte.trim().to_string())
            == "online";
        let ieee = cache.lock().unwrap().noms.get(nom).cloned();
        if let Some(ieee) = ieee {
            sqlx::query("UPDATE zigbee_appareil SET disponible=? WHERE ieee=?")
                .bind(i64::from(en_ligne))
                .bind(ieee)
                .execute(pool)
                .await?;
        }
        return Ok(());
    }
    // Sous-sujets (…/set, …/get) : ce sont des commandes, pas des états.
    if reste.contains('/') {
        return Ok(());
    }
    let ieee = {
        let c = cache.lock().unwrap();
        c.noms.get(reste).cloned()
    }
    .or_else(|| reste.starts_with("0x").then(|| reste.to_string()));
    let Some(ieee) = ieee else {
        return Ok(());
    };
    let Ok(Value::Object(etat)) = serde_json::from_slice::<Value>(charge) else {
        return Ok(());
    };
    etat_appareil(pool, cache, &ieee, &etat).await
}

async fn etat_appareil(
    pool: &SqlitePool,
    cache: &Arc<Mutex<Cache>>,
    ieee: &str,
    etat: &serde_json::Map<String, Value>,
) -> Result<()> {
    // Heure du dernier contact réel avec l'appareil (option `last_seen` de
    // Zigbee2MQTT, en millisecondes) : au redémarrage, Zigbee2MQTT republie
    // l'état en cache d'un capteur muet depuis des heures ; sans cela, cette
    // vieille valeur paraissait toute fraîche.
    let ts = etat
        .get("last_seen")
        .and_then(Value::as_i64)
        .map(|ms| ms / 1000)
        .filter(|t| *t > 1_600_000_000 && *t <= maintenant() + 60)
        .unwrap_or_else(maintenant);
    // Ce qu'il faut écrire, décidé sous le verrou, écrit ensuite.
    let mut a_ecrire: Vec<(String, Option<f64>, Option<String>)> = Vec::new();
    let mut a_historiser: Vec<(String, f64)> = Vec::new();
    let mut demarrage = false;
    let vu_a_ecrire;
    {
        let mut c = cache.lock().unwrap();
        for (cle, valeur) in etat {
            if matches!(valeur, Value::Object(_) | Value::Array(_) | Value::Null) || cle == "last_seen" {
                continue;
            }
            let nombre = en_nombre(valeur);
            let texte = valeur.as_str().map(str::to_string);
            let clef = (ieee.to_string(), cle.clone());
            // Démarrage d'une machine : la vibration passe de 0 à 1. Sans
            // valeur connue (redémarrage du service), seul un message récent
            // compte, pas l'état republié par Zigbee2MQTT.
            if cle == "vibration" && nombre == Some(1.0) {
                demarrage = match c.ecrit.get(&clef) {
                    Some((avant, ..)) => *avant != Some(1.0),
                    None => maintenant() - ts < 120,
                };
            }
            let changer = match c.ecrit.get(&clef) {
                Some((v, t, quand)) => *v != nombre || *t != texte || ts - quand >= REECRITURE_S,
                None => true,
            };
            if changer {
                c.ecrit.insert(clef.clone(), (nombre, texte.clone(), ts));
                a_ecrire.push((cle.clone(), nombre, texte));
            }
            if let Some(n) = nombre.filter(|_| HISTORISEES.contains(&cle.as_str())) {
                let dernier = c.historique.get(&clef).copied().unwrap_or(0);
                if ts - dernier >= HISTORIQUE_PAS_S {
                    c.historique.insert(clef, ts);
                    a_historiser.push((cle.clone(), n));
                }
            }
        }
        let dernier_vu = c.vu.get(ieee).copied().unwrap_or(0);
        vu_a_ecrire = ts - dernier_vu >= 60;
        if vu_a_ecrire {
            c.vu.insert(ieee.to_string(), ts);
        }
    }
    if a_ecrire.is_empty() && a_historiser.is_empty() && !vu_a_ecrire {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    if demarrage {
        sqlx::query("INSERT INTO zigbee_demarrage(ieee,ts) VALUES(?,?)")
            .bind(ieee)
            .bind(ts)
            .execute(&mut *tx)
            .await?;
    }
    for (cle, nombre, texte) in &a_ecrire {
        sqlx::query(
            "INSERT INTO zigbee_valeur(ieee,cle,valeur,texte,ts) VALUES(?,?,?,?,?) \
             ON CONFLICT(ieee,cle) DO UPDATE SET valeur=excluded.valeur, texte=excluded.texte, ts=excluded.ts",
        )
        .bind(ieee)
        .bind(cle)
        .bind(nombre)
        .bind(texte)
        .bind(ts)
        .execute(&mut *tx)
        .await?;
    }
    for (cle, valeur) in &a_historiser {
        sqlx::query("INSERT OR IGNORE INTO zigbee_mesure(ieee,cle,ts,valeur) VALUES(?,?,?,?)")
            .bind(ieee)
            .bind(cle)
            .bind(ts)
            .bind(valeur)
            .execute(&mut *tx)
            .await?;
    }
    if vu_a_ecrire || !a_ecrire.is_empty() {
        sqlx::query(
            "UPDATE zigbee_appareil SET vu_le=?, \
             batterie=COALESCE(?, batterie), signal=COALESCE(?, signal) WHERE ieee=?",
        )
        .bind(ts)
        .bind(etat.get("battery").and_then(Value::as_f64))
        .bind(etat.get("linkquality").and_then(Value::as_i64))
        .bind(ieee)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn appareils(pool: &SqlitePool, cache: &Arc<Mutex<Cache>>, charge: &[u8]) -> Result<()> {
    let Value::Array(liste) = serde_json::from_slice::<Value>(charge)? else {
        return Ok(());
    };
    let ts = maintenant();
    let mut noms = HashMap::new();
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE zigbee_appareil SET present=0").execute(&mut *tx).await?;
    for appareil in &liste {
        let type_reseau = appareil.get("type").and_then(Value::as_str).unwrap_or("");
        if type_reseau == "Coordinator" {
            continue;
        }
        let Some(ieee) = appareil.get("ieee_address").and_then(Value::as_str) else {
            continue;
        };
        let nom = appareil.get("friendly_name").and_then(Value::as_str).unwrap_or(ieee);
        let definition = appareil.get("definition").cloned().unwrap_or(Value::Null);
        let grandeurs = grandeurs_exposees(definition.get("exposes").unwrap_or(&Value::Null));
        noms.insert(nom.to_string(), ieee.to_string());
        sqlx::query(
            "INSERT INTO zigbee_appareil(ieee,nom,modele,fabricant,description,type_reseau,alimentation,\
             categorie,grandeurs,present,maj) VALUES(?,?,?,?,?,?,?,?,?,1,?) \
             ON CONFLICT(ieee) DO UPDATE SET nom=excluded.nom, modele=excluded.modele, \
             fabricant=excluded.fabricant, description=excluded.description, \
             type_reseau=excluded.type_reseau, alimentation=excluded.alimentation, \
             categorie=excluded.categorie, grandeurs=excluded.grandeurs, present=1, maj=excluded.maj",
        )
        .bind(ieee)
        .bind(nom)
        .bind(definition.get("model").and_then(Value::as_str))
        .bind(definition.get("vendor").and_then(Value::as_str))
        .bind(definition.get("description").and_then(Value::as_str))
        .bind(type_reseau)
        .bind(appareil.get("power_source").and_then(Value::as_str))
        .bind(categoriser(&grandeurs))
        .bind(grandeurs.join(","))
        .bind(ts)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    tracing::info!(appareils = noms.len(), "pont Zigbee : liste des appareils à jour");
    cache.lock().unwrap().noms = noms;
    Ok(())
}

/// Publie les commandes en attente, et purge l'historique ancien.
async fn envoyer_commandes(pool: SqlitePool, client: AsyncClient) {
    let mut derniere_purge = 0;
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let attente: Vec<(i64, String, String, i64)> = sqlx::query_as(
            "SELECT id, ieee, charge, demande FROM zigbee_commande WHERE envoye IS NULL ORDER BY id LIMIT 20",
        )
        .fetch_all(&pool)
        .await
        .unwrap_or_default();
        for (id, ieee, charge, demande) in attente {
            // Une commande restée en file plus de deux minutes (MQTT coupé)
            // n'est plus envoyée : allumer une lampe une heure trop tard
            // serait pire que de ne rien faire.
            let resultat = if maintenant() - demande > 120 {
                "abandonnée (trop ancienne)".to_string()
            } else {
                match client
                    .publish(format!("{BASE}/{ieee}/set"), QoS::AtLeastOnce, false, charge.clone())
                    .await
                {
                    Ok(()) => "publiée".to_string(),
                    Err(error) => format!("échec : {error}"),
                }
            };
            let _ = sqlx::query("UPDATE zigbee_commande SET envoye=?, resultat=? WHERE id=?")
                .bind(maintenant())
                .bind(&resultat)
                .bind(id)
                .execute(&pool)
                .await;
            tracing::info!(ieee, charge, resultat, "pont Zigbee : commande");
        }
        let ts = maintenant();
        if ts - derniere_purge > 3600 {
            derniere_purge = ts;
            let _ = sqlx::query("DELETE FROM zigbee_mesure WHERE ts < ?")
                .bind(ts - HISTORIQUE_JOURS * 86400)
                .execute(&pool)
                .await;
            let _ = sqlx::query("DELETE FROM zigbee_commande WHERE envoye IS NOT NULL AND envoye < ?")
                .bind(ts - 7 * 86400)
                .execute(&pool)
                .await;
        }
    }
}

/// Lecture d'une grandeur depuis la base : valeur et âge en secondes.
pub async fn lire(pool: &SqlitePool, chemin: &str) -> Option<(f64, i64)> {
    let (ieee, cle) = decouper_chemin(chemin)?;
    let (valeur, ts): (Option<f64>, i64) =
        sqlx::query_as("SELECT valeur, ts FROM zigbee_valeur WHERE ieee=? AND cle=?")
            .bind(ieee)
            .bind(cle)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()?;
    Some((valeur?, (maintenant() - ts).max(0)))
}

/// Dépose une commande dans la file (publiée dans la seconde par le pont).
pub async fn commander(pool: &SqlitePool, chemin: &str, valeur: f64) -> Result<()> {
    let (ieee, cle) = decouper_chemin(chemin).ok_or_else(|| anyhow::anyhow!("chemin Zigbee invalide"))?;
    sqlx::query("INSERT INTO zigbee_commande(ieee,charge,demande) VALUES(?,?,?)")
        .bind(ieee)
        .bind(charge_commande(cle, valeur))
        .bind(maintenant())
        .execute(pool)
        .await?;
    Ok(())
}

/// Alertes Zigbee : pile faible, appareil muet, température hors des seuils
/// propres à l'appareil (frigo). Rangées dans `ambiance_alerte` avec la salle
/// de rattachement quand il y en a une.
pub async fn alertes(pool: &SqlitePool) -> Result<()> {
    let ts = maintenant();
    let appareils: Vec<(String, String, String, Option<String>, Option<f64>, Option<i64>, Option<i64>, Option<f64>, Option<f64>)> =
        sqlx::query_as(
            "SELECT z.ieee, z.nom, z.categorie, z.alimentation, z.batterie, z.vu_le, z.disponible, \
                    COALESCE(z.t_min, a.t_alerte_min), COALESCE(z.t_max, a.t_alerte_max) \
             FROM zigbee_appareil z \
             LEFT JOIN ipx_point p ON p.hote='zigbee' AND p.chemin = z.ieee || '/temperature' \
             LEFT JOIN ambiance_salle a ON a.salle_id = p.salle_id \
             WHERE z.present=1",
        )
        .fetch_all(pool)
        .await?;
    for (ieee, nom, categorie, alimentation, batterie, vu_le, disponible, t_min, t_max) in appareils {
        let salle: Option<i64> = sqlx::query_scalar(
            "SELECT salle_id FROM ipx_point WHERE hote='zigbee' AND chemin LIKE ? AND salle_id IS NOT NULL LIMIT 1",
        )
        .bind(format!("{ieee}/%"))
        .fetch_optional(pool)
        .await?
        .flatten();

        let pile_faible = alimentation.as_deref() == Some("Battery") && batterie.is_some_and(|b| b <= 15.0);
        signaler(
            pool,
            salle,
            &format!("zigbee_pile:{ieee}"),
            pile_faible,
            "alerte",
            &format!("{nom} : pile à {} %, à changer", batterie.unwrap_or(0.0).round()),
        )
        .await?;

        // Une sonde sur pile envoie au moins toutes les heures ; au-delà de
        // six heures sans nouvelles, elle est tenue pour muette.
        let muet = match vu_le {
            Some(vu) => {
                disponible == Some(0) || (categorie == "sonde" && ts - vu > 6 * 3600)
            }
            None => false,
        };
        signaler(pool, salle, &format!("zigbee_muet:{ieee}"), muet, "alerte", &format!("{nom} ne répond plus"))
            .await?;

        if t_min.is_some() || t_max.is_some() {
            let temperature = lire(pool, &format!("{ieee}/temperature")).await;
            let hors = temperature.is_some_and(|(t, age)| {
                age < 3 * 3600 && (t_min.is_some_and(|m| t < m) || t_max.is_some_and(|m| t > m))
            });
            let t = temperature.map(|(t, _)| t).unwrap_or(0.0);
            signaler(
                pool,
                salle,
                &format!("zigbee_temp:{ieee}"),
                hors,
                "critique",
                &format!(
                    "{nom} : {t:.1} °C, hors de la plage {} – {} °C",
                    t_min.map_or("—".into(), |v| format!("{v}")),
                    t_max.map_or("—".into(), |v| format!("{v}"))
                ),
            )
            .await?;
        }
    }
    surveillance_electrique(pool).await?;
    surveillance_demarrages(pool).await?;
    Ok(())
}

/// Trop de démarrages d'une machine (compresseur qui fait du « court
/// cycle ») : alerte quand le nombre de démarrages sur la période glissante
/// dépasse le maximum réglé pour le détecteur.
pub async fn surveillance_demarrages(pool: &SqlitePool) -> Result<()> {
    let ts = maintenant();
    sqlx::query("DELETE FROM zigbee_demarrage WHERE ts < ?")
        .bind(ts - HISTORIQUE_JOURS * 86400)
        .execute(pool)
        .await?;
    let reglages: Vec<(String, String, Option<i64>, i64)> = sqlx::query_as(
        "SELECT r.ieee, a.nom, r.demarrages_max, r.periode_min FROM zigbee_demarrage_reglage r \
         JOIN zigbee_appareil a ON a.ieee=r.ieee WHERE a.present=1",
    )
    .fetch_all(pool)
    .await?;
    for (ieee, nom, max, periode) in reglages {
        let periode = periode.max(1);
        let nombre: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM zigbee_demarrage WHERE ieee=? AND ts>=?")
            .bind(&ieee)
            .bind(ts - periode * 60)
            .fetch_one(pool)
            .await?;
        let salle: Option<i64> = sqlx::query_scalar(
            "SELECT salle_id FROM ipx_point WHERE hote='zigbee' AND chemin LIKE ? AND salle_id IS NOT NULL LIMIT 1",
        )
        .bind(format!("{ieee}/%"))
        .fetch_optional(pool)
        .await?
        .flatten();
        signaler(
            pool,
            salle,
            &format!("zigbee_demarrages:{ieee}"),
            max.is_some_and(|m| nombre > m),
            "alerte",
            &format!("{nom} : {nombre} démarrages en {} (maximum {})", duree_texte(periode), max.unwrap_or(0)),
        )
        .await?;
    }
    Ok(())
}

pub fn duree_texte(minutes: i64) -> String {
    if minutes % 60 == 0 {
        format!("{} h", minutes / 60)
    } else {
        format!("{minutes} min")
    }
}

// ---------------------------------------------------------------------------
// Surveillance électrique (compteurs Zigbee : machine à soupe, général…)
// ---------------------------------------------------------------------------

/// Une mesure électrique plus vieille que ça n'est pas utilisée.
const ELEC_FRAICHEUR_S: i64 = 600;

/// Phase d'une grandeur de tension : `voltage` / `voltage_a` → A,
/// `voltage_phase_b` / `voltage_b` → B…
pub fn phase_de(cle: &str) -> Option<char> {
    let reste = cle.strip_prefix("voltage")?;
    match reste.trim_start_matches("_phase").trim_start_matches('_') {
        "" | "a" => Some('A'),
        "b" => Some('B'),
        "c" => Some('C'),
        _ => None,
    }
}

/// Phases dont la tension est sous le seuil.
pub fn phases_coupees(tensions: &[(String, f64)], seuil: f64) -> Vec<(char, f64)> {
    let mut coupees: Vec<(char, f64)> = tensions
        .iter()
        .filter_map(|(cle, v)| phase_de(cle).map(|p| (p, *v)))
        .filter(|(_, v)| *v < seuil)
        .collect();
    coupees.sort_by_key(|(p, _)| *p);
    coupees.dedup_by_key(|(p, _)| *p);
    coupees
}

pub async fn surveillance_electrique(pool: &SqlitePool) -> Result<()> {
    let ts = maintenant();
    // Tout compteur a des réglages par défaut, sans rien saisir.
    sqlx::query(
        "INSERT OR IGNORE INTO zigbee_surveillance(ieee) \
         SELECT ieee FROM zigbee_appareil WHERE categorie='compteur' AND present=1",
    )
    .execute(pool)
    .await?;
    let lignes: Vec<(String, String, f64, Option<f64>, Option<i64>, Option<f64>, i64, f64, Option<i64>, Option<i64>)> =
        sqlx::query_as(
            "SELECT s.ieee, a.nom, s.seuil_marche_w, s.puissance_max_w, s.marche_max_min, s.arret_max_h, \
                    s.phases, s.tension_min_v, s.en_marche, s.depuis \
             FROM zigbee_surveillance s JOIN zigbee_appareil a ON a.ieee=s.ieee WHERE a.present=1",
        )
        .fetch_all(pool)
        .await?;
    for (ieee, nom, seuil, p_max, marche_max, arret_max, phases, tension_min, en_marche, depuis) in lignes {
        let salle: Option<i64> = sqlx::query_scalar(
            "SELECT salle_id FROM ipx_point WHERE hote='zigbee' AND chemin LIKE ? AND salle_id IS NOT NULL LIMIT 1",
        )
        .bind(format!("{ieee}/%"))
        .fetch_optional(pool)
        .await?
        .flatten();
        let valeurs: Vec<(String, Option<f64>, i64)> =
            sqlx::query_as("SELECT cle, valeur, ts FROM zigbee_valeur WHERE ieee=?")
                .bind(&ieee)
                .fetch_all(pool)
                .await?;
        let frais = |cle: &str| {
            valeurs
                .iter()
                .find(|(c, v, t)| c == cle && v.is_some() && ts - t < ELEC_FRAICHEUR_S)
                .and_then(|(_, v, _)| *v)
        };
        let puissance = frais("total_power").or_else(|| frais("power"));
        let energie = frais("energy");

        // Marche / arrêt, sur les fronts.
        let mut en_marche = en_marche.map(|m| m != 0);
        let mut depuis = depuis.unwrap_or(ts);
        if let Some(p) = puissance {
            let maintenant_marche = p > seuil;
            if en_marche != Some(maintenant_marche) {
                if en_marche.is_some() {
                    if maintenant_marche {
                        sqlx::query("INSERT INTO zigbee_marche(ieee,debut,energie_debut) VALUES(?,?,?)")
                            .bind(&ieee)
                            .bind(ts)
                            .bind(energie)
                            .execute(pool)
                            .await?;
                    } else {
                        sqlx::query(
                            "UPDATE zigbee_marche SET fin=?, energie_fin=? WHERE ieee=? AND fin IS NULL",
                        )
                        .bind(ts)
                        .bind(energie)
                        .bind(&ieee)
                        .execute(pool)
                        .await?;
                    }
                } else if maintenant_marche {
                    // Premier passage : déjà en marche, période ouverte.
                    sqlx::query("INSERT INTO zigbee_marche(ieee,debut,energie_debut) VALUES(?,?,?)")
                        .bind(&ieee)
                        .bind(ts)
                        .bind(energie)
                        .execute(pool)
                        .await?;
                }
                en_marche = Some(maintenant_marche);
                depuis = ts;
                sqlx::query("UPDATE zigbee_surveillance SET en_marche=?, depuis=? WHERE ieee=?")
                    .bind(i64::from(maintenant_marche))
                    .bind(ts)
                    .bind(&ieee)
                    .execute(pool)
                    .await?;
            }
        }

        // Coupure de phase.
        let tensions: Vec<(String, f64)> = valeurs
            .iter()
            .filter(|(c, v, t)| c.starts_with("voltage") && v.is_some() && ts - t < ELEC_FRAICHEUR_S)
            .map(|(c, v, _)| (c.clone(), v.unwrap_or(0.0)))
            .collect();
        let coupees = if phases != 0 { phases_coupees(&tensions, tension_min) } else { Vec::new() };
        signaler(
            pool,
            salle,
            &format!("zigbee_phase:{ieee}"),
            !coupees.is_empty(),
            "critique",
            &format!(
                "{nom} : coupure de phase {}",
                coupees.iter().map(|(p, v)| format!("{p} ({v:.0} V)")).collect::<Vec<_>>().join(", ")
            ),
        )
        .await?;

        let trop = matches!((puissance, p_max), (Some(p), Some(m)) if p > m);
        signaler(
            pool,
            salle,
            &format!("zigbee_surpuissance:{ieee}"),
            trop,
            "alerte",
            &format!("{nom} : {:.0} W, au-dessus de {:.0} W", puissance.unwrap_or(0.0), p_max.unwrap_or(0.0)),
        )
        .await?;

        let duree = ts - depuis;
        let longue = en_marche == Some(true) && marche_max.is_some_and(|m| duree > m * 60);
        signaler(
            pool,
            salle,
            &format!("zigbee_marche_longue:{ieee}"),
            longue,
            "alerte",
            &format!("{nom} tourne depuis {} min sans s'arrêter", duree / 60),
        )
        .await?;
        let arret = en_marche == Some(false) && arret_max.is_some_and(|h| duree as f64 > h * 3600.0);
        signaler(
            pool,
            salle,
            &format!("zigbee_arret_long:{ieee}"),
            arret,
            "alerte",
            &format!("{nom} n'a pas tourné depuis {} h", duree / 3600),
        )
        .await?;
    }
    Ok(())
}

async fn signaler(
    pool: &SqlitePool,
    salle: Option<i64>,
    type_alerte: &str,
    active: bool,
    gravite: &str,
    message: &str,
) -> Result<()> {
    if active {
        let ouverte: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ambiance_alerte WHERE type=? AND fin IS NULL")
            .bind(type_alerte)
            .fetch_one(pool)
            .await?;
        if ouverte == 0 {
            sqlx::query(
                "INSERT INTO ambiance_alerte(salle_id,type,gravite,debut,message) VALUES(?,?,?,CURRENT_TIMESTAMP,?)",
            )
            .bind(salle)
            .bind(type_alerte)
            .bind(gravite)
            .bind(message)
            .execute(pool)
            .await?;
            tracing::warn!(message, "alerte Zigbee");
        } else {
            sqlx::query("UPDATE ambiance_alerte SET message=? WHERE type=? AND fin IS NULL")
                .bind(message)
                .bind(type_alerte)
                .execute(pool)
                .await?;
        }
    } else {
        sqlx::query("UPDATE ambiance_alerte SET fin=CURRENT_TIMESTAMP WHERE type=? AND fin IS NULL")
            .bind(type_alerte)
            .execute(pool)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(liste: &[&str]) -> Vec<String> {
        liste.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn les_phases_se_lisent_quel_que_soit_le_nommage() {
        assert_eq!(phase_de("voltage"), Some('A'));
        assert_eq!(phase_de("voltage_a"), Some('A'));
        assert_eq!(phase_de("voltage_phase_b"), Some('B'));
        assert_eq!(phase_de("voltage_c"), Some('C'));
        assert_eq!(phase_de("voltage_x"), None);
        let t = |c: &str, v: f64| (c.to_string(), v);
        // Compteur général : phase B coupée.
        assert_eq!(
            phases_coupees(&[t("voltage", 229.0), t("voltage_phase_b", 0.0), t("voltage_phase_c", 237.0)], 190.0),
            vec![('B', 0.0)]
        );
        assert!(phases_coupees(&[t("voltage_a", 237.0), t("voltage_b", 233.0), t("voltage_c", 228.0)], 190.0).is_empty());
    }

    #[test]
    fn les_appareils_de_l_elevage_sont_bien_classes() {
        assert_eq!(categoriser(&g(&["battery", "humidity", "temperature", "voltage"])), "sonde");
        assert_eq!(categoriser(&g(&["countdown", "state", "switch"])), "interrupteur");
        assert_eq!(categoriser(&g(&["brightness", "light", "state"])), "variateur");
        assert_eq!(categoriser(&g(&["current", "energy", "power", "voltage"])), "compteur");
        assert_eq!(categoriser(&g(&["battery", "sensitivity", "vibration"])), "vibration");
    }

    #[test]
    fn les_grandeurs_imbriquees_sont_trouvees() {
        let exposes = serde_json::json!([
            {"type": "switch", "features": [{"name": "state", "property": "state"}]},
            {"name": "linkquality"}
        ]);
        assert_eq!(grandeurs_exposees(&exposes), g(&["linkquality", "state", "switch"]));
    }

    #[test]
    fn un_etat_texte_devient_un_nombre() {
        assert_eq!(en_nombre(&serde_json::json!("ON")), Some(1.0));
        assert_eq!(en_nombre(&serde_json::json!("OFF")), Some(0.0));
        assert_eq!(en_nombre(&serde_json::json!(false)), Some(0.0));
        assert_eq!(en_nombre(&serde_json::json!(21.5)), Some(21.5));
        assert_eq!(en_nombre(&serde_json::json!("toggle")), None);
    }

    #[test]
    fn les_commandes_ont_le_format_zigbee2mqtt() {
        assert_eq!(charge_commande("state", 1.0), r#"{"state":"ON"}"#);
        assert_eq!(charge_commande("state", 0.0), r#"{"state":"OFF"}"#);
        assert_eq!(charge_commande("brightness", 300.0), r#"{"brightness":255}"#);
        assert_eq!(decouper_chemin("0xa4c13843e9d59a69/state"), Some(("0xa4c13843e9d59a69", "state")));
        assert_eq!(decouper_chemin("api/core/io/1"), None);
    }
}
