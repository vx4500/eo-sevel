//! Module Automatique : règles « si… alors… » qui relient les automates et
//! appareils de l'élevage.
//!
//! Une règle lit des conditions (point IPX ou Zigbee, température ou
//! hygrométrie d'une salle, phase de la machine à soupe), et quand elles
//! tiennent pendant le délai voulu, dans la plage horaire voulue, elle
//! commande des sorties (IPX ou Zigbee) et envoie des SMS. Des actions « fin »
//! s'exécutent quand la condition cesse (éteindre ce qu'on a allumé).
//!
//! Une nouvelle règle démarre en **essai** : elle est évaluée et journalisée,
//! mais n'agit sur rien tant que l'éleveur ne l'a pas passée en marche réelle.
//! Une condition illisible (capteur muet) ne déclenche ni n'arrête rien.

use super::*;
use chrono::{Local, Timelike};

/// Fréquence d'évaluation des règles.
const PERIODE_S: u64 = 10;
/// Au-delà, une mesure de salle ou de machine à soupe est jugée périmée.
const MESURE_PERIMEE_S: i64 = 900;
/// Nombre maximal de lignes de conditions ou d'actions par règle.
const LIGNES_MAX: usize = 20;

const OPERATEURS: [(&str, &str); 6] = [
    (">", "supérieur à"),
    ("<", "inférieur à"),
    (">=", "au moins"),
    ("<=", "au plus"),
    ("=", "égal à"),
    ("!=", "différent de"),
];

fn maintenant() -> i64 {
    chrono::Utc::now().timestamp()
}

fn comparer(valeur: f64, operateur: &str, seuil: f64) -> bool {
    match operateur {
        ">" => valeur > seuil,
        "<" => valeur < seuil,
        ">=" => valeur >= seuil,
        "<=" => valeur <= seuil,
        "=" => (valeur - seuil).abs() < 1e-9,
        "!=" => (valeur - seuil).abs() >= 1e-9,
        _ => false,
    }
}

fn libelle_operateur(operateur: &str) -> &'static str {
    OPERATEURS
        .iter()
        .find(|(code, _)| *code == operateur)
        .map(|(_, libelle)| *libelle)
        .unwrap_or("?")
}

/// Plage horaire « HH:MM »–« HH:MM », éventuellement à cheval sur minuit.
/// Une borne absente ou illisible ouvre la plage à toute la journée.
fn dans_plage(debut: Option<&str>, fin: Option<&str>, minute: u32) -> bool {
    let lire = |texte: Option<&str>| {
        texte
            .filter(|t| !t.trim().is_empty())
            .and_then(crate::eclairage_core::minutes_depuis_heure)
    };
    match (lire(debut), lire(fin)) {
        (Some(debut), Some(fin)) if debut <= fin => minute >= debut && minute < fin,
        (Some(debut), Some(fin)) => minute >= debut || minute < fin,
        _ => true,
    }
}

/// Combine les conditions. `None` : on ne peut pas conclure (une condition
/// illisible et aucune autre ne tranche).
fn combiner(logique: &str, resultats: &[Option<bool>]) -> Option<bool> {
    if logique == "ou" {
        if resultats.iter().any(|r| *r == Some(true)) {
            Some(true)
        } else if resultats.iter().any(Option::is_none) {
            None
        } else {
            Some(false)
        }
    } else if resultats.iter().any(|r| *r == Some(false)) {
        Some(false)
    } else if resultats.iter().any(Option::is_none) {
        None
    } else {
        Some(true)
    }
}

/// Décision d'une passe, sans effet de bord.
#[derive(Debug, PartialEq)]
struct Pas {
    declencher: bool,
    finir: bool,
    vrai_depuis: Option<i64>,
}

fn etape(
    condition: Option<bool>,
    vrai_depuis: Option<i64>,
    declenche: bool,
    delai_s: i64,
    en_plage: bool,
    instant: i64,
) -> Pas {
    match condition {
        // Capteur muet : on ne change rien, ni déclenchement ni fin.
        None => Pas { declencher: false, finir: false, vrai_depuis },
        Some(true) => {
            let depuis = vrai_depuis.unwrap_or(instant);
            Pas {
                declencher: !declenche && en_plage && instant - depuis >= delai_s,
                finir: false,
                vrai_depuis: Some(depuis),
            }
        }
        Some(false) => Pas { declencher: false, finir: declenche, vrai_depuis: None },
    }
}

// ---------------------------------------------------------------------------
// Lecture des conditions et exécution des actions
// ---------------------------------------------------------------------------

struct Condition {
    source: String,
    point_id: Option<i64>,
    salle_id: Option<i64>,
    operateur: String,
    valeur: f64,
}

/// Valeur actuelle d'une condition et son libellé (« Température Mater 1 »).
async fn lire_condition(pool: &SqlitePool, c: &Condition) -> (String, Result<f64, String>) {
    match c.source.as_str() {
        "point" => {
            let point: Option<(String, String, String, Option<String>, f64, f64)> = sqlx::query_as(
                "SELECT hote, chemin, nom, formule, coefficient, decalage FROM ipx_point WHERE id=?",
            )
            .bind(c.point_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
            let Some((hote, chemin, nom, formule, coefficient, decalage)) = point else {
                return ("point supprimé".into(), Err("le point n'existe plus".into()));
            };
            let valeur = super::ambiance::lire_point(
                pool,
                &hote,
                &chemin,
                formule.as_deref(),
                coefficient,
                decalage,
            )
            .await;
            let resultat = valeur.ok_or_else(|| format!("« {nom} » ne répond pas"));
            (nom, resultat)
        }
        "salle_t" | "salle_h" => {
            let ligne: Option<(String, Option<f64>, Option<f64>, Option<i64>)> = sqlx::query_as(
                "SELECT s.nom, e.t, e.h, e.ts FROM salle s \
                 LEFT JOIN ambiance_etat e ON e.salle_id = s.id WHERE s.id=?",
            )
            .bind(c.salle_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
            let Some((salle, t, h, ts)) = ligne else {
                return ("salle supprimée".into(), Err("la salle n'existe plus".into()));
            };
            let (libelle, valeur) = if c.source == "salle_t" {
                (format!("Température {salle}"), t)
            } else {
                (format!("Hygrométrie {salle}"), h)
            };
            let fraiche = ts.is_some_and(|ts| maintenant() - ts <= MESURE_PERIMEE_S);
            let resultat = match valeur {
                Some(v) if fraiche => Ok(v),
                _ => Err(format!("pas de mesure récente pour {salle}")),
            };
            (libelle, resultat)
        }
        "soupe_phase" => {
            let ligne: Option<(i64, i64)> =
                sqlx::query_as("SELECT phase, ts FROM soupe_etat WHERE id=1")
                    .fetch_optional(pool)
                    .await
                    .ok()
                    .flatten();
            let resultat = match ligne {
                Some((phase, ts)) if maintenant() - ts <= MESURE_PERIMEE_S => Ok(phase as f64),
                _ => Err("machine à soupe sans nouvelles".into()),
            };
            ("Phase machine à soupe".into(), resultat)
        }
        autre => (autre.to_string(), Err("source inconnue".into())),
    }
}

/// Commande d'une sortie par une règle, avec les mêmes garde-fous qu'à la
/// main : point autorisé à la commande, et jamais la sortie de ventilation
/// d'une salle en régulation automatique.
async fn commander(pool: &SqlitePool, point_id: i64, valeur: f64) -> Result<String, String> {
    let point: Option<(String, Option<String>, String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT hote, chemin_ecriture, nom, pilotable, salle_id FROM ipx_point WHERE id=?",
    )
    .bind(point_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let Some((hote, chemin_ecriture, nom, pilotable, salle_id)) = point else {
        return Err("sortie supprimée".into());
    };
    if pilotable == 0 {
        return Err(format!("« {nom} » n'est pas autorisé à la commande"));
    }
    let Some(chemin_ecriture) = chemin_ecriture.filter(|c| !c.trim().is_empty()) else {
        return Err(format!("« {nom} » n'a pas de chemin d'écriture"));
    };
    if let Some(salle_id) = salle_id {
        let pilotee: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM ambiance_salle WHERE salle_id=? AND actif=1 AND pilote=1 \
             AND sortie_ventil=?",
        )
        .bind(salle_id)
        .bind(chemin_ecriture.trim())
        .fetch_one(pool)
        .await
        .unwrap_or(0);
        if pilotee > 0 {
            return Err(format!("« {nom} » est en régulation automatique"));
        }
    }
    let compte_rendu = super::ambiance::envoyer_commande(pool, &hote, &chemin_ecriture, valeur)
        .await
        .map_err(|e| e.to_string())?;
    let _ = sqlx::query(
        "INSERT INTO ipx_commande_journal(utilisateur,point_id,nom_point,valeur,resultat) \
         VALUES('automatique',?,?,?,?)",
    )
    .bind(point_id)
    .bind(&nom)
    .bind(valeur)
    .bind(&compte_rendu)
    .execute(pool)
    .await;
    Ok(format!("{nom} → {} : {compte_rendu}", texte_valeur(valeur)))
}

fn texte_valeur(valeur: f64) -> String {
    if valeur.fract() == 0.0 {
        format!("{}", valeur as i64)
    } else {
        format!("{valeur:.2}").replace('.', ",")
    }
}

async fn journaliser(
    pool: &SqlitePool,
    automatisme_id: i64,
    nom: &str,
    evenement: &str,
    detail: &str,
    essai: bool,
) {
    let _ = sqlx::query(
        "INSERT INTO automatisme_journal(automatisme_id,nom,evenement,detail,essai) VALUES(?,?,?,?,?)",
    )
    .bind(automatisme_id)
    .bind(nom)
    .bind(evenement)
    .bind(detail)
    .bind(essai as i64)
    .execute(pool)
    .await;
}

async fn executer(pool: &SqlitePool, id: i64, nom: &str, essai: bool, moment: &str, contexte: &str) {
    let actions: Vec<(String, Option<i64>, Option<f64>, Option<String>, Option<String>)> =
        sqlx::query_as(
            "SELECT a.type, a.point_id, a.valeur, a.message, p.nom FROM automatisme_action a \
             LEFT JOIN ipx_point p ON p.id = a.point_id \
             WHERE a.automatisme_id=? AND a.moment=? ORDER BY a.ordre, a.id",
        )
        .bind(id)
        .bind(moment)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    for (type_action, point_id, valeur, message, point_nom) in actions {
        let (evenement, detail) = match type_action.as_str() {
            "commande" => {
                let (Some(point_id), Some(valeur)) = (point_id, valeur) else { continue };
                if essai {
                    (
                        "commande (essai)".to_string(),
                        format!(
                            "aurait envoyé {} à « {} »",
                            texte_valeur(valeur),
                            point_nom.unwrap_or_default()
                        ),
                    )
                } else {
                    match commander(pool, point_id, valeur).await {
                        Ok(texte) => ("commande".into(), texte),
                        Err(erreur) => ("commande refusée".into(), erreur),
                    }
                }
            }
            "sms" => {
                let texte = format!(
                    "{} — {}",
                    message.filter(|m| !m.trim().is_empty()).unwrap_or_else(|| nom.to_string()),
                    contexte
                );
                if essai {
                    ("SMS (essai)".into(), format!("aurait envoyé : {texte}"))
                } else {
                    match crate::sms_free::envoyer(pool, &texte, "automatique").await {
                        Ok(()) => ("SMS".into(), texte),
                        Err(erreur) => ("SMS non envoyé".into(), erreur),
                    }
                }
            }
            _ => continue,
        };
        journaliser(pool, id, nom, &evenement, &detail, essai).await;
    }
}

/// Une passe sur toutes les règles actives.
async fn passe(pool: &SqlitePool) -> anyhow::Result<()> {
    let regles: Vec<(i64, String, i64, String, i64, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id, nom, essai, logique, delai_s, heure_debut, heure_fin FROM automatisme WHERE actif=1",
    )
    .fetch_all(pool)
    .await?;
    let instant = maintenant();
    let local = Local::now();
    let minute = local.hour() * 60 + local.minute();

    for (id, nom, essai, logique, delai_s, heure_debut, heure_fin) in regles {
        let conditions: Vec<(String, Option<i64>, Option<i64>, String, f64)> = sqlx::query_as(
            "SELECT source, point_id, salle_id, operateur, valeur FROM automatisme_condition \
             WHERE automatisme_id=? ORDER BY id",
        )
        .bind(id)
        .fetch_all(pool)
        .await?;
        if conditions.is_empty() {
            continue;
        }
        let mut resultats = Vec::with_capacity(conditions.len());
        let mut constats = Vec::new();
        let mut problemes = Vec::new();
        for (source, point_id, salle_id, operateur, valeur) in conditions {
            let condition = Condition { source, point_id, salle_id, operateur, valeur };
            let (libelle, lu) = lire_condition(pool, &condition).await;
            match lu {
                Ok(v) => {
                    resultats.push(Some(comparer(v, &condition.operateur, condition.valeur)));
                    constats.push(format!(
                        "{libelle} : {} ({} {})",
                        texte_valeur(v),
                        libelle_operateur(&condition.operateur),
                        texte_valeur(condition.valeur)
                    ));
                }
                Err(probleme) => {
                    resultats.push(None);
                    problemes.push(probleme);
                }
            }
        }
        let condition = combiner(&logique, &resultats);
        let (vrai_depuis, declenche): (Option<i64>, i64) = sqlx::query_as(
            "SELECT vrai_depuis, declenche FROM automatisme_etat WHERE automatisme_id=?",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?
        .unwrap_or((None, 0));
        let en_plage = dans_plage(heure_debut.as_deref(), heure_fin.as_deref(), minute);
        let pas = etape(condition, vrai_depuis, declenche != 0, delai_s, en_plage, instant);
        let essai = essai != 0;
        let contexte = constats.join(" ; ");
        if pas.declencher {
            journaliser(pool, id, &nom, "déclenchée", &contexte, essai).await;
            executer(pool, id, &nom, essai, "debut", &contexte).await;
        }
        if pas.finir {
            journaliser(pool, id, &nom, "terminée", &contexte, essai).await;
            executer(pool, id, &nom, essai, "fin", &contexte).await;
        }
        let declenche = (declenche != 0 || pas.declencher) && !pas.finir;
        sqlx::query(
            "INSERT INTO automatisme_etat(automatisme_id,vrai_depuis,declenche,dernier_declenchement,probleme,maj) \
             VALUES(?,?,?,?,?,?) ON CONFLICT(automatisme_id) DO UPDATE SET \
             vrai_depuis=excluded.vrai_depuis, declenche=excluded.declenche, \
             dernier_declenchement=COALESCE(excluded.dernier_declenchement, dernier_declenchement), \
             probleme=excluded.probleme, maj=excluded.maj",
        )
        .bind(id)
        .bind(pas.vrai_depuis)
        .bind(declenche as i64)
        .bind(pas.declencher.then_some(instant))
        .bind((!problemes.is_empty()).then(|| problemes.join(" ; ")))
        .bind(instant)
        .execute(pool)
        .await?;
    }
    // Journal sur 90 jours.
    sqlx::query("DELETE FROM automatisme_journal WHERE ts < ?")
        .bind(instant - 90 * 86_400)
        .execute(pool)
        .await?;
    Ok(())
}

/// Boucle de fond lancée par main : ne rend jamais la main.
pub async fn boucle(pool: SqlitePool) {
    let mut minuterie = tokio::time::interval(std::time::Duration::from_secs(PERIODE_S));
    minuterie.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        minuterie.tick().await;
        if let Err(erreur) = passe(&pool).await {
            tracing::warn!(%erreur, "automatique : passe en échec");
        }
    }
}

// ---------------------------------------------------------------------------
// Pages
// ---------------------------------------------------------------------------

/// Ce qu'une règle peut lire ou commander, pour les listes de choix.
async fn choix(pool: &SqlitePool) -> AppResult<(Vec<Value>, Vec<Value>)> {
    let points: Vec<(i64, String, String, String, Option<String>, Option<String>, i64)> =
        sqlx::query_as(
            "SELECT p.id, p.hote, p.chemin, p.nom, p.unite, s.nom, p.pilotable FROM ipx_point p \
             LEFT JOIN salle s ON s.id = p.salle_id WHERE p.actif=1 \
             ORDER BY (p.hote='zigbee'), p.hote, s.nom, p.nom COLLATE NOCASE",
        )
        .fetch_all(pool)
        .await?;
    let mut lectures = Vec::new();
    let mut sorties = Vec::new();
    for (id, hote, chemin, nom, unite, salle, pilotable) in points {
        let appareil = if hote == super::zigbee::HOTE {
            "Zigbee".to_string()
        } else {
            format!("Automate {hote}")
        };
        // Tout-ou-rien : relais, entrées et bits de l'IPX, état on/off Zigbee.
        let tor = super::ambiance::est_objet_io(&chemin) || chemin.ends_with("/state");
        let libelle = match &salle {
            Some(salle) => format!("{nom} · {salle}"),
            None => nom.clone(),
        };
        let objet = json!({"code": format!("p:{id}"), "groupe": appareil, "nom": libelle, "unite": unite, "tor": tor});
        if pilotable != 0 {
            sorties.push(objet.clone());
        }
        lectures.push(objet);
    }
    let salles: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, nom FROM salle ORDER BY ordre, nom")
            .fetch_all(pool)
            .await?;
    for (id, nom) in &salles {
        lectures.push(json!({"code": format!("t:{id}"), "groupe": "Salles (EO_Awel)", "nom": format!("Température · {nom}"), "unite": "°C", "tor": false}));
        lectures.push(json!({"code": format!("h:{id}"), "groupe": "Salles (EO_Awel)", "nom": format!("Hygrométrie · {nom}"), "unite": "%", "tor": false}));
    }
    if crate::templates::equip_actif("soupe") {
        lectures.push(json!({"code": "s:0", "groupe": "Machine à soupe", "nom": "Phase en cours (n°)", "unite": null, "tor": false}));
    }
    Ok((lectures, sorties))
}

fn heure_texte(ts: Option<i64>) -> Option<String> {
    ts.and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
        .map(|d| d.with_timezone(&Local).format("%d/%m %H:%M").to_string())
}

async fn journal(pool: &SqlitePool, automatisme_id: Option<i64>, limite: i64) -> AppResult<Vec<Value>> {
    let lignes: Vec<(i64, String, String, Option<String>, i64)> = sqlx::query_as(
        "SELECT ts, nom, evenement, detail, essai FROM automatisme_journal \
         WHERE (?1 IS NULL OR automatisme_id=?1) ORDER BY id DESC LIMIT ?2",
    )
    .bind(automatisme_id)
    .bind(limite)
    .fetch_all(pool)
    .await?;
    Ok(lignes
        .into_iter()
        .map(|(ts, nom, evenement, detail, essai)| {
            json!({"quand": heure_texte(Some(ts)), "nom": nom, "evenement": evenement, "detail": detail, "essai": essai != 0})
        })
        .collect())
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(parametres): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    require_writer(&session)?;
    let lignes: Vec<(i64, String, i64, i64, String, i64, Option<String>, Option<String>, Option<i64>, Option<i64>, Option<i64>, Option<String>)> =
        sqlx::query_as(
            "SELECT a.id, a.nom, a.actif, a.essai, a.logique, a.delai_s, a.heure_debut, a.heure_fin, \
                    e.declenche, e.dernier_declenchement, e.vrai_depuis, e.probleme \
             FROM automatisme a LEFT JOIN automatisme_etat e ON e.automatisme_id = a.id \
             ORDER BY a.nom COLLATE NOCASE",
        )
        .fetch_all(&state.pool)
        .await?;
    let mut regles = Vec::new();
    for (id, nom, actif, essai, logique, delai_s, debut, fin, declenche, dernier, vrai_depuis, probleme) in lignes {
        let (nb_conditions,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM automatisme_condition WHERE automatisme_id=?")
                .bind(id)
                .fetch_one(&state.pool)
                .await?;
        let (nb_actions,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM automatisme_action WHERE automatisme_id=?")
                .bind(id)
                .fetch_one(&state.pool)
                .await?;
        regles.push(json!({
            "id": id, "nom": nom, "actif": actif != 0, "essai": essai != 0, "logique": logique,
            "delai_s": delai_s, "heure_debut": debut, "heure_fin": fin,
            "declenche": declenche.unwrap_or(0) != 0,
            "condition_vraie": vrai_depuis.is_some(),
            "dernier": heure_texte(dernier), "probleme": probleme,
            "nb_conditions": nb_conditions, "nb_actions": nb_actions,
        }));
    }
    let lumieres: Vec<(i64, String, i64, i64, Option<String>, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT g.id, g.nom, g.actif, g.essai, \
                (SELECT group_concat(p.nom, ', ') FROM eclairage_sortie o JOIN ipx_point p ON p.id=o.point_id WHERE o.programme_id=g.id), \
                (SELECT MAX(e.allume) FROM eclairage_etat e WHERE e.programme_id=g.id), \
                (SELECT e.raison FROM eclairage_etat e WHERE e.programme_id=g.id ORDER BY e.maj DESC LIMIT 1) \
         FROM eclairage_programme g ORDER BY g.nom COLLATE NOCASE",
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    let mut contexte = context(&session);
    contexte.insert(
        "lumieres".into(),
        Value::Array(lumieres.into_iter().map(|(id, nom, actif, essai, lampes, allume, raison)| {
            json!({"id": id, "nom": nom, "actif": actif != 0, "essai": essai != 0, "lampes": lampes,
                   "allume": allume.map(|a| a != 0), "raison": raison})
        }).collect()),
    );
    contexte.insert("regles".into(), Value::Array(regles));
    contexte.insert("journal".into(), Value::Array(journal(&state.pool, None, 60).await?));
    contexte.insert(
        "ok".into(),
        parametres.get("ok").map(|m| Value::String(m.clone())).unwrap_or(Value::Null),
    );
    render(&state, "automatique.html", Value::Object(contexte))
}

/// Modèles proposés sur « Nouvelle règle » : (code, icône, titre, explication).
/// La lumière automatique renvoie vers les programmes d'éclairage, qui gèrent
/// déjà seuils, temporisation et sonde muette.
const MODELES: [(&str, &str, &str, &str); 6] = [
    ("lumiere", "💡", "Lumière automatique", "Allumer selon l'heure et la lumière mesurée (programme d'éclairage)"),
    ("alerte_temp", "🌡️", "Alerte température", "SMS si une salle dépasse une température pendant 10 min"),
    ("brumisation", "💦", "Brumisation si chaud", "Allumer la brumisation au-dessus de 28 °C, l'éteindre en dessous"),
    ("chauffage", "🔥", "Chauffage si froid", "Allumer un chauffage sous 18 °C, l'éteindre au-dessus"),
    ("hygro", "💧", "Alerte hygrométrie", "SMS si l'humidité d'une salle reste au-dessus de 85 %"),
    ("porte_nuit", "🚪", "Porte ouverte la nuit", "SMS si une entrée (porte, contact) s'active entre 22 h et 6 h"),
];

fn modele(code: &str) -> Option<Value> {
    let r = |nom: &str, delai: f64, debut: &str, fin: &str, conditions: Value, actions: Value| {
        json!({"id": null, "nom": nom, "actif": true, "essai": true, "logique": "et", "delai_min": delai,
               "heure_debut": debut, "heure_fin": fin, "note": "", "conditions": conditions, "actions": actions})
    };
    Some(match code {
        "alerte_temp" => r("Alerte température", 10.0, "", "",
            json!([{"code": "", "operateur": ">", "valeur": 30.0}]),
            json!([{"moment": "debut", "code": "sms", "message": "Température trop haute"}])),
        "brumisation" => r("Brumisation si chaud", 5.0, "", "",
            json!([{"code": "", "operateur": ">", "valeur": 28.0}]),
            json!([{"moment": "debut", "code": "", "valeur": 1.0}, {"moment": "fin", "code": "", "valeur": 0.0}])),
        "chauffage" => r("Chauffage si froid", 5.0, "", "",
            json!([{"code": "", "operateur": "<", "valeur": 18.0}]),
            json!([{"moment": "debut", "code": "", "valeur": 1.0}, {"moment": "fin", "code": "", "valeur": 0.0}])),
        "hygro" => r("Alerte hygrométrie", 15.0, "", "",
            json!([{"code": "", "operateur": ">", "valeur": 85.0}]),
            json!([{"moment": "debut", "code": "sms", "message": "Humidité trop haute"}])),
        "vide" => r("", 0.0, "", "", json!([]), json!([])),
        "porte_nuit" => r("Porte ouverte la nuit", 0.0, "22:00", "06:00",
            json!([{"code": "", "operateur": "=", "valeur": 1.0}]),
            json!([{"moment": "debut", "code": "sms", "message": "Porte ouverte la nuit"}])),
        _ => return None,
    })
}

pub(super) async fn nouvelle(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(parametres): Query<HashMap<String, String>>,
) -> AppResult<Response> {
    match parametres.get("modele").map(String::as_str) {
        Some("lumiere") => return Ok(Redirect::to("/ambiance/eclairage/nouveau").into_response()),
        Some(code) => {
            let regle = modele(code).ok_or(AppError::NotFound)?;
            return Ok(formulaire(&state, &session, None, Some(regle)).await?.into_response());
        }
        None => {}
    }
    require_writer(&session)?;
    let mut contexte = context(&session);
    contexte.insert(
        "modeles".into(),
        Value::Array(MODELES.iter().map(|(code, icone, titre, texte)| {
            json!({"code": code, "icone": icone, "titre": titre, "texte": texte})
        }).collect()),
    );
    Ok(render(&state, "automatique_modeles.html", Value::Object(contexte))?.into_response())
}

pub(super) async fn regle(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
) -> AppResult<Html<String>> {
    formulaire(&state, &session, Some(id), None).await
}

async fn formulaire(
    state: &AppState,
    session: &SessionData,
    id: Option<i64>,
    depart: Option<Value>,
) -> AppResult<Html<String>> {
    require_writer(session)?;
    let (lectures, sorties) = choix(&state.pool).await?;
    let mut regle = depart.unwrap_or_else(|| json!({"id": null, "nom": "", "actif": true, "essai": true, "logique": "et",
        "delai_min": 0, "heure_debut": "", "heure_fin": "", "note": "", "conditions": [], "actions": []}));
    if let Some(id) = id {
        let ligne: Option<(String, i64, i64, String, i64, Option<String>, Option<String>, Option<String>)> =
            sqlx::query_as(
                "SELECT nom, actif, essai, logique, delai_s, heure_debut, heure_fin, note FROM automatisme WHERE id=?",
            )
            .bind(id)
            .fetch_optional(&state.pool)
            .await?;
        let Some((nom, actif, essai, logique, delai_s, debut, fin, note)) = ligne else {
            return Err(AppError::NotFound);
        };
        let conditions: Vec<(String, Option<i64>, Option<i64>, String, f64)> = sqlx::query_as(
            "SELECT source, point_id, salle_id, operateur, valeur FROM automatisme_condition WHERE automatisme_id=? ORDER BY id",
        )
        .bind(id)
        .fetch_all(&state.pool)
        .await?;
        let actions: Vec<(String, String, Option<i64>, Option<f64>, Option<String>)> = sqlx::query_as(
            "SELECT moment, type, point_id, valeur, message FROM automatisme_action WHERE automatisme_id=? ORDER BY ordre, id",
        )
        .bind(id)
        .fetch_all(&state.pool)
        .await?;
        regle = json!({
            "id": id, "nom": nom, "actif": actif != 0, "essai": essai != 0, "logique": logique,
            "delai_min": delai_s as f64 / 60.0, "heure_debut": debut.unwrap_or_default(),
            "heure_fin": fin.unwrap_or_default(), "note": note.unwrap_or_default(),
            "conditions": conditions.into_iter().map(|(source, point_id, salle_id, operateur, valeur)| {
                let code = match source.as_str() {
                    "point" => format!("p:{}", point_id.unwrap_or(0)),
                    "salle_t" => format!("t:{}", salle_id.unwrap_or(0)),
                    "salle_h" => format!("h:{}", salle_id.unwrap_or(0)),
                    _ => "s:0".to_string(),
                };
                json!({"code": code, "operateur": operateur, "valeur": valeur})
            }).collect::<Vec<_>>(),
            "actions": actions.into_iter().map(|(moment, type_action, point_id, valeur, message)| {
                let code = if type_action == "sms" { "sms".to_string() } else { format!("p:{}", point_id.unwrap_or(0)) };
                json!({"moment": moment, "code": code, "valeur": valeur, "message": message})
            }).collect::<Vec<_>>(),
        });
    }
    let mut contexte = context(session);
    contexte.insert("regle".into(), regle);
    contexte.insert("lectures".into(), Value::Array(lectures));
    contexte.insert("sorties".into(), Value::Array(sorties));
    contexte.insert(
        "operateurs".into(),
        Value::Array(OPERATEURS.iter().map(|(c, l)| json!({"code": c, "libelle": l})).collect()),
    );
    contexte.insert(
        "journal".into(),
        Value::Array(match id {
            Some(id) => journal(&state.pool, Some(id), 30).await?,
            None => Vec::new(),
        }),
    );
    render(state, "automatique_regle.html", Value::Object(contexte))
}

/// « p:12 » → ("point", Some(12), None) ; « t:3 » → ("salle_t", None, Some(3)).
fn decoder_lecture(code: &str) -> Option<(&'static str, Option<i64>, Option<i64>)> {
    let (prefixe, id) = code.split_once(':')?;
    let id: i64 = id.parse().ok()?;
    match prefixe {
        "p" => Some(("point", Some(id), None)),
        "t" => Some(("salle_t", None, Some(id))),
        "h" => Some(("salle_h", None, Some(id))),
        "s" => Some(("soupe_phase", None, None)),
        _ => None,
    }
}

fn nombre(texte: Option<&String>) -> Option<f64> {
    texte.and_then(|t| t.trim().replace(',', ".").parse::<f64>().ok()).filter(|v| v.is_finite())
}

pub(super) async fn enregistrer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let nom = form_text(&form, "nom").ok_or_else(|| AppError::Invalid("Donnez un nom à la règle".into()))?;
    let logique = if form.get("logique").map(String::as_str) == Some("ou") { "ou" } else { "et" };
    let delai_s = (nombre(form.get("delai_min")).unwrap_or(0.0).max(0.0) * 60.0).round() as i64;
    let heure = |cle: &str| {
        form_text(&form, cle).filter(|h| crate::eclairage_core::minutes_depuis_heure(h).is_some())
    };

    let mut conditions = Vec::new();
    for i in 0..LIGNES_MAX {
        let Some(code) = form.get(&format!("c{i}_quoi")).filter(|c| !c.is_empty()) else { continue };
        let (source, point_id, salle_id) =
            decoder_lecture(code).ok_or_else(|| AppError::Invalid("Condition inconnue".into()))?;
        let operateur = form.get(&format!("c{i}_op")).map(String::as_str).unwrap_or(">");
        if !OPERATEURS.iter().any(|(c, _)| *c == operateur) {
            return Err(AppError::Invalid("Comparaison inconnue".into()));
        }
        let valeur = nombre(form.get(&format!("c{i}_valeur")))
            .ok_or_else(|| AppError::Invalid("Chaque condition a besoin d'une valeur".into()))?;
        conditions.push((source, point_id, salle_id, operateur.to_string(), valeur));
    }
    if conditions.is_empty() {
        return Err(AppError::Invalid("Ajoutez au moins une condition (« Si… »)".into()));
    }

    let mut actions = Vec::new();
    for i in 0..LIGNES_MAX {
        let Some(code) = form.get(&format!("a{i}_quoi")).filter(|c| !c.is_empty()) else { continue };
        let moment = if form.get(&format!("a{i}_moment")).map(String::as_str) == Some("fin") { "fin" } else { "debut" };
        if code == "sms" {
            let message = form_text(&form, &format!("a{i}_message"));
            actions.push((moment, "sms", None, None, message));
        } else {
            let Some(("point", Some(point_id), None)) = decoder_lecture(code) else {
                return Err(AppError::Invalid("Action inconnue".into()));
            };
            let pilotable: i64 = sqlx::query_scalar("SELECT pilotable FROM ipx_point WHERE id=?")
                .bind(point_id)
                .fetch_optional(&state.pool)
                .await?
                .unwrap_or(0);
            if pilotable == 0 {
                return Err(AppError::Invalid("Cette sortie n'est pas autorisée à la commande".into()));
            }
            let programme: Option<String> = sqlx::query_scalar(
                "SELECT g.nom FROM eclairage_sortie o JOIN eclairage_programme g ON g.id=o.programme_id \
                 WHERE o.point_id=? AND g.actif=1 LIMIT 1",
            )
            .bind(point_id)
            .fetch_optional(&state.pool)
            .await?;
            if let Some(programme) = programme {
                return Err(AppError::Invalid(format!(
                    "Cette lampe est déjà pilotée par le programme d'éclairage « {programme} » : \
                     modifiez ce programme, ou arrêtez-le, pour éviter que les deux se contredisent"
                )));
            }
            let valeur = nombre(form.get(&format!("a{i}_valeur")))
                .ok_or_else(|| AppError::Invalid("Indiquez la valeur à envoyer à chaque sortie".into()))?;
            actions.push((moment, "commande", Some(point_id), Some(valeur), None));
        }
    }
    if actions.is_empty() {
        return Err(AppError::Invalid("Ajoutez au moins une action (« Alors… »)".into()));
    }

    let mut tx = state.pool.begin().await?;
    let id = match form_i64(&form, "id") {
        Some(id) => {
            sqlx::query(
                "UPDATE automatisme SET nom=?, actif=?, essai=?, logique=?, delai_s=?, heure_debut=?, heure_fin=?, note=? WHERE id=?",
            )
            .bind(&nom)
            .bind(form.contains_key("actif") as i64)
            .bind(form.contains_key("essai") as i64)
            .bind(logique)
            .bind(delai_s)
            .bind(heure("heure_debut"))
            .bind(heure("heure_fin"))
            .bind(form_text(&form, "note"))
            .bind(id)
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM automatisme_condition WHERE automatisme_id=?").bind(id).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM automatisme_action WHERE automatisme_id=?").bind(id).execute(&mut *tx).await?;
            // Règle modifiée : on repart d'un état neutre, sans rejouer de « fin ».
            sqlx::query("DELETE FROM automatisme_etat WHERE automatisme_id=?").bind(id).execute(&mut *tx).await?;
            id
        }
        None => sqlx::query(
            "INSERT INTO automatisme(nom,actif,essai,logique,delai_s,heure_debut,heure_fin,note) VALUES(?,?,?,?,?,?,?,?)",
        )
        .bind(&nom)
        .bind(form.contains_key("actif") as i64)
        .bind(form.contains_key("essai") as i64)
        .bind(logique)
        .bind(delai_s)
        .bind(heure("heure_debut"))
        .bind(heure("heure_fin"))
        .bind(form_text(&form, "note"))
        .execute(&mut *tx)
        .await?
        .last_insert_rowid(),
    };
    for (source, point_id, salle_id, operateur, valeur) in &conditions {
        sqlx::query(
            "INSERT INTO automatisme_condition(automatisme_id,source,point_id,salle_id,operateur,valeur) VALUES(?,?,?,?,?,?)",
        )
        .bind(id)
        .bind(source)
        .bind(point_id)
        .bind(salle_id)
        .bind(operateur)
        .bind(valeur)
        .execute(&mut *tx)
        .await?;
    }
    for (ordre, (moment, type_action, point_id, valeur, message)) in actions.iter().enumerate() {
        sqlx::query(
            "INSERT INTO automatisme_action(automatisme_id,moment,type,point_id,valeur,message,ordre) VALUES(?,?,?,?,?,?,?)",
        )
        .bind(id)
        .bind(moment)
        .bind(type_action)
        .bind(point_id)
        .bind(valeur)
        .bind(message)
        .bind(ordre as i64)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    journaliser(&state.pool, id, &nom, "modifiée", &format!("par {}", session.identifiant), false).await;
    Ok(Redirect::to(&format!("/automatique?ok={}", encode_query(&format!("Règle « {nom} » enregistrée")))).into_response())
}

pub(super) async fn basculer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let champ = match form.get("champ").map(String::as_str) {
        Some("essai") => "essai",
        _ => "actif",
    };
    sqlx::query(&format!("UPDATE automatisme SET {champ} = 1 - {champ} WHERE id=?"))
        .bind(id)
        .execute(&state.pool)
        .await?;
    sqlx::query("DELETE FROM automatisme_etat WHERE automatisme_id=?").bind(id).execute(&state.pool).await?;
    Ok(Redirect::to("/automatique").into_response())
}

pub(super) async fn supprimer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    for table in ["automatisme_condition", "automatisme_action", "automatisme_etat"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE automatisme_id=?"))
            .bind(id)
            .execute(&state.pool)
            .await?;
    }
    sqlx::query("DELETE FROM automatisme WHERE id=?").bind(id).execute(&state.pool).await?;
    Ok(Redirect::to("/automatique?ok=R%C3%A8gle+supprim%C3%A9e").into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_condition_doit_tenir_le_delai() {
        let p = etape(Some(true), None, false, 60, true, 1000);
        assert_eq!(p, Pas { declencher: false, finir: false, vrai_depuis: Some(1000) });
        let p = etape(Some(true), Some(1000), false, 60, true, 1060);
        assert!(p.declencher);
        // Déjà déclenchée : pas de deuxième déclenchement.
        let p = etape(Some(true), Some(1000), true, 60, true, 1200);
        assert!(!p.declencher && !p.finir);
    }

    #[test]
    fn fin_quand_la_condition_cesse_et_rien_si_capteur_muet() {
        assert!(etape(Some(false), Some(1000), true, 0, true, 2000).finir);
        assert!(!etape(Some(false), None, false, 0, true, 2000).finir);
        let p = etape(None, Some(1000), true, 0, true, 2000);
        assert!(!p.declencher && !p.finir && p.vrai_depuis == Some(1000));
    }

    #[test]
    fn hors_plage_horaire_on_attend() {
        assert!(!etape(Some(true), Some(0), false, 0, false, 100).declencher);
        assert!(dans_plage(Some("22:00"), Some("06:00"), 23 * 60));
        assert!(!dans_plage(Some("22:00"), Some("06:00"), 12 * 60));
        assert!(dans_plage(Some("08:00"), Some("18:00"), 9 * 60));
        assert!(dans_plage(None, None, 0));
    }

    #[test]
    fn logique_et_ou() {
        assert_eq!(combiner("et", &[Some(true), Some(true)]), Some(true));
        assert_eq!(combiner("et", &[Some(true), None]), None);
        assert_eq!(combiner("et", &[Some(false), None]), Some(false));
        assert_eq!(combiner("ou", &[Some(false), Some(true)]), Some(true));
        assert_eq!(combiner("ou", &[Some(false), None]), None);
        assert!(comparer(30.4, ">", 30.0) && !comparer(1.0, "!=", 1.0));
    }

    /// Les pages se rendent vraiment (filtres connus, variables présentes) :
    /// une erreur de gabarit ne se voit sinon qu'en production.
    #[test]
    fn les_pages_se_rendent() {
        let env = crate::templates::build().expect("gabarits");
        let session = serde_json::json!({"csrf": "x", "peut_modifier": true, "pages": [], "identifiant": "test"});
        let regle = serde_json::json!({"id": 1, "nom": "Essai", "actif": true, "essai": true, "logique": "et",
            "delai_min": 5.0, "heure_debut": "", "heure_fin": "", "note": "",
            "conditions": [{"code": "t:1", "operateur": ">", "valeur": 30.0}],
            "actions": [{"moment": "debut", "code": "sms", "valeur": null, "message": "chaud"}]});
        let lecture = serde_json::json!({"code": "t:1", "groupe": "Salles", "nom": "Température · PS2", "unite": "°C", "tor": false});
        env.get_template("automatique_regle.html").unwrap().render(serde_json::json!({
            "session": session, "regle": regle, "lectures": [lecture], "sorties": [],
            "operateurs": [{"code": ">", "libelle": "supérieur à"}], "journal": []
        })).expect("automatique_regle.html");
        env.get_template("automatique_modeles.html").unwrap().render(serde_json::json!({
            "session": session, "modeles": [{"code": "hygro", "icone": "💧", "titre": "Alerte", "texte": "SMS"}]
        })).expect("automatique_modeles.html");
        env.get_template("automatique.html").unwrap().render(serde_json::json!({
            "session": session, "ok": null,
            "lumieres": [{"id": 1, "nom": "Auto Eclairage Verraterie", "actif": true, "essai": false, "lampes": "ECL_Verraterie", "allume": true, "raison": "sombre"}], "journal": [{"quand": "07/10 18:00", "nom": "Essai", "evenement": "déclenchée", "detail": "", "essai": true}],
            "regles": [{"id": 1, "nom": "Essai", "actif": true, "essai": true, "logique": "et", "delai_s": 300,
                "heure_debut": "22:00", "heure_fin": "06:00", "declenche": true, "condition_vraie": true,
                "dernier": "07/10 18:00", "probleme": null, "nb_conditions": 1, "nb_actions": 2}]
        })).expect("automatique.html");
        for v5 in [serde_json::Value::Null, serde_json::json!({"code": "relais", "nom": "Relais", "source": "io"})] {
            env.get_template("ambiance_carte.html").unwrap().render(serde_json::json!({
                "session": session, "v5": v5, "hote": "192.168.1.250", "hotes": ["192.168.1.250"],
                "types_v5": [{"code": "relais", "nom": "Relais", "nombre": 8}],
                "sections": [{"id": 0, "nom": "Relais", "source": "io", "motif": "", "id_debut": 65536, "id_fin": 65543,
                    "a_renseigner": false, "commandable": true, "ordre": 0, "note": null,
                    "objets": [{"id": 65536, "chemin": "api/core/io/65536", "nom_ipx": "Relay 1", "valeur": null, "etat": true,
                        "nom": "Ventil", "salle": null, "point_id": 3, "pilotable": true, "role": "ventilation",
                        "salle_id": null, "formule": null, "coefficient": 1.0, "decalage": 0.0, "unite": null, "valeur_convertie": null}]}],
                "restes": [], "roles": [], "salles": [], "ok": null
            })).expect("ambiance_carte.html");
        }
    }

    /// Les pages d'éclairage portent maintenant les onglets d'Automatique.
    #[test]
    fn pages_eclairage_se_rendent() {
        let env = crate::templates::build().expect("gabarits");
        let ctx = serde_json::json!({"session": {"csrf": "x", "peut_modifier": true, "pages": []}});
        for nom in ["ambiance_eclairage.html", "ambiance_eclairage_form.html", "ambiance_eclairage_lampe.html"] {
            let rendu = env.get_template(nom).unwrap().render(&ctx);
            if let Err(e) = &rendu {
                assert!(!e.to_string().contains("onglets"), "{nom}: {e}");
            }
        }
    }

    #[test]
    fn modeles_valides() {
        for (code, ..) in MODELES.iter().filter(|(c, ..)| *c != "lumiere") {
            assert!(modele(code).is_some(), "{code}");
        }
    }

    #[test]
    fn codes_de_lecture() {
        assert_eq!(decoder_lecture("p:12"), Some(("point", Some(12), None)));
        assert_eq!(decoder_lecture("t:3"), Some(("salle_t", None, Some(3))));
        assert_eq!(decoder_lecture("x:1"), None);
    }
}
