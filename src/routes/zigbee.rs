//! Onglet Zigbee de l'automate (/ambiance/zigbee).
//!
//! Les appareils, leurs valeurs et l'état du pont sont tenus par le démon
//! `eo-regul` (voir src/zigbee_pont.rs). Ici, on les montre et on les
//! rattache : un appareil rattaché devient un ou plusieurs points
//! `ipx_point` (hote « zigbee », chemin « <ieee>/<grandeur> »), ce qui le rend
//! aussitôt visible dans Mes salles, utilisable comme lampe, relevé comme
//! compteur, surveillé par les alertes.

use super::*;

pub(super) const HOTE: &str = "zigbee";

fn maintenant() -> i64 {
    chrono::Utc::now().timestamp()
}

fn il_y_a(secondes: i64) -> String {
    match secondes {
        s if s < 90 => "à l'instant".into(),
        s if s < 5400 => format!("il y a {} min", s / 60),
        s if s < 172_800 => format!("il y a {} h", s / 3600),
        s => format!("il y a {} j", s / 86400),
    }
}

const CATEGORIES: &[(&str, &str, &str)] = &[
    ("sonde", "🌡️ Sondes", "Température et humidité"),
    ("interrupteur", "💡 Interrupteurs", "Éclairage et sorties marche/arrêt"),
    ("variateur", "🌀 Variateurs", "Ventilation"),
    ("compteur", "⚡ Compteurs électriques", "Consommation"),
    ("vibration", "📳 Détecteurs", "Marche des machines"),
    ("autre", "🔧 Autres", ""),
];

/// Usages proposés pour un appareil selon sa catégorie : (code, libellé).
fn usages(categorie: &str) -> &'static [(&'static str, &'static str)] {
    match categorie {
        "sonde" => &[("ambiance", "Température et humidité de la salle")],
        "interrupteur" => &[("eclairage", "Lampe (programmes d'éclairage)"), ("sortie", "Autre sortie marche/arrêt")],
        "variateur" => &[("ventilation", "Ventilation (affichage du réglage)")],
        "compteur" => &[("compteur", "Compteur électrique (relevé automatique)")],
        "vibration" => &[("marche", "Marche d'une machine")],
        _ => &[("autre", "Autre")],
    }
}

/// Points à créer pour un usage : (grandeur, rôle, suffixe du nom, unité,
/// pilotable, coefficient).
fn points_pour(usage: &str, grandeurs: &[&str]) -> Vec<(&'static str, &'static str, &'static str, &'static str, bool, f64)> {
    let a = |g: &str| grandeurs.contains(&g);
    let mut sortie = Vec::new();
    match usage {
        "ambiance" => {
            if a("temperature") {
                sortie.push(("temperature", "temperature", "température", "°C", false, 1.0));
            }
            if a("humidity") {
                sortie.push(("humidity", "humidite", "humidité", "%", false, 1.0));
            }
            if a("illuminance") || a("illuminance_lux") {
                sortie.push((if a("illuminance_lux") { "illuminance_lux" } else { "illuminance" }, "luminosite", "luminosité", "lux", false, 1.0));
            }
        }
        "eclairage" => sortie.push(("state", "eclairage", "", "", true, 1.0)),
        "sortie" => sortie.push(("state", "autre", "", "", true, 1.0)),
        // Réglage du variateur affiché en pourcentage (0–254 → 0–100 %).
        "ventilation" => sortie.push(("brightness", "ventilation", "réglage", "%", false, 100.0 / 254.0)),
        "compteur" => {
            if a("energy") {
                sortie.push(("energy", "compteur_elec", "index", "kWh", false, 1.0));
            }
            if a("power") {
                sortie.push(("power", "autre", "puissance", "W", false, 1.0));
            }
        }
        "marche" => sortie.push(("vibration", "autre", "vibration", "", false, 1.0)),
        _ => {}
    }
    sortie
}

async fn etat_pont(pool: &SqlitePool) -> Value {
    let ligne: Option<(i64, i64, i64, Option<String>)> =
        sqlx::query_as("SELECT connecte, ts, messages, erreur FROM zigbee_pont WHERE id=1")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    match ligne {
        None => json!({"actif": false, "texte": "Le pont Zigbee n'a jamais démarré (service eo-regul)."}),
        Some((connecte, ts, messages, erreur)) => {
            let age = maintenant() - ts;
            // Le pont écrit au plus toutes les 30 s quand des messages
            // arrivent ; les compteurs publient en continu, donc un silence de
            // plusieurs minutes signale une coupure.
            let actif = connecte != 0 && age < 600;
            json!({
                "actif": actif,
                "messages": messages,
                "texte": if actif {
                    format!("Pont Zigbee connecté — dernier message {}", il_y_a(age))
                } else {
                    format!(
                        "Pont Zigbee coupé ({}){}",
                        il_y_a(age),
                        erreur.map(|e| format!(" : {e}")).unwrap_or_default()
                    )
                },
            })
        }
    }
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(parametres): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let pool = &state.pool;
    let ts = maintenant();
    let lignes = sqlx::query(
        "SELECT ieee, nom, modele, fabricant, description, categorie, grandeurs, alimentation, \
                disponible, vu_le, batterie, signal, t_min, t_max \
         FROM zigbee_appareil WHERE present=1 ORDER BY nom COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await?;
    let valeurs: Vec<(String, String, Option<f64>, Option<String>)> =
        sqlx::query_as("SELECT ieee, cle, valeur, texte FROM zigbee_valeur")
            .fetch_all(pool)
            .await?;
    let rattaches: Vec<(String, Option<i64>, Option<String>, String, Option<i64>)> = sqlx::query_as(
        "SELECT p.chemin, p.salle_id, s.nom, p.role, p.compteur_id FROM ipx_point p \
         LEFT JOIN salle s ON s.id=p.salle_id WHERE p.hote='zigbee'",
    )
    .fetch_all(pool)
    .await?;

    let demarrages: Vec<(String, Option<i64>, i64, i64, i64, Option<i64>)> = sqlx::query_as(
        "SELECT a.ieee, r.demarrages_max, COALESCE(r.periode_min,60), \
                (SELECT COUNT(*) FROM zigbee_demarrage d WHERE d.ieee=a.ieee AND d.ts>=?1-COALESCE(r.periode_min,60)*60), \
                (SELECT COUNT(*) FROM zigbee_demarrage d WHERE d.ieee=a.ieee AND d.ts>=?1-86400), \
                (SELECT MAX(ts) FROM zigbee_demarrage d WHERE d.ieee=a.ieee) \
         FROM zigbee_appareil a LEFT JOIN zigbee_demarrage_reglage r ON r.ieee=a.ieee WHERE a.categorie='vibration'",
    )
    .bind(ts)
    .fetch_all(pool)
    .await?;
    let mut par_categorie: HashMap<String, Vec<Value>> = HashMap::new();
    let mut piles_faibles = Vec::new();
    for l in &lignes {
        let ieee: String = l.try_get("ieee")?;
        let nom: String = l.try_get("nom").unwrap_or_default();
        let categorie: String = l.try_get("categorie").unwrap_or_else(|_| "autre".into());
        let grandeurs: String = l.try_get::<Option<String>, _>("grandeurs").ok().flatten().unwrap_or_default();
        let val = |cle: &str| {
            valeurs
                .iter()
                .find(|(i, c, _, _)| *i == ieee && c == cle)
                .map(|(_, _, v, t)| (*v, t.clone()))
        };
        let nombre = |cle: &str| val(cle).and_then(|(v, _)| v);
        let batterie: Option<f64> = l.try_get::<Option<f64>, _>("batterie").ok().flatten();
        let sur_pile = l.try_get::<Option<String>, _>("alimentation").ok().flatten().as_deref() == Some("Battery");
        if sur_pile && batterie.is_some_and(|b| b <= 25.0) {
            piles_faibles.push(json!({"nom": nom, "batterie": batterie}));
        }
        let lies: Vec<&(String, Option<i64>, Option<String>, String, Option<i64>)> =
            rattaches.iter().filter(|(c, ..)| c.starts_with(&format!("{ieee}/"))).collect();
        let salle_id = lies.iter().find_map(|(_, s, ..)| *s);
        let salle = lies.iter().find_map(|(_, _, n, ..)| n.clone());
        let usage = lies.iter().find_map(|(chemin, _, _, role, _)| {
            let cle = chemin.rsplit('/').next().unwrap_or("");
            Some(match (cle, role.as_str()) {
                ("state", "eclairage") => "eclairage",
                ("state", _) => "sortie",
                ("temperature" | "humidity", _) => "ambiance",
                ("brightness", _) => "ventilation",
                ("energy" | "power", _) => "compteur",
                ("vibration", _) => "marche",
                _ => "autre",
            })
        });
        let compteur_id = lies.iter().find_map(|(.., c)| *c);
        let vu_le: Option<i64> = l.try_get::<Option<i64>, _>("vu_le").ok().flatten();
        let brightness = nombre("brightness");
        let demarrage = demarrages.iter().find(|d| d.0 == ieee).map(|(_, max, periode, n, jour, dernier)| {
            json!({"max": max, "periode_min": periode, "sur_periode": n, "sur_24h": jour,
                   "periode": if periode % 60 == 0 { format!("{} h", periode / 60) } else { format!("{periode} min") },
                   "dernier": dernier.map(|d| il_y_a(ts - d)), "trop": max.is_some_and(|m| *n > m)})
        });
        let appareil = json!({
            "demarrages": demarrage,
            "ieee": ieee,
            "nom": nom,
            "modele": l.try_get::<Option<String>, _>("modele").ok().flatten(),
            "fabricant": l.try_get::<Option<String>, _>("fabricant").ok().flatten(),
            "description": l.try_get::<Option<String>, _>("description").ok().flatten(),
            "categorie": categorie,
            "disponible": l.try_get::<Option<i64>, _>("disponible").ok().flatten().map(|d| d != 0),
            "vu": vu_le.map(|v| il_y_a(ts - v)),
            "vieux": vu_le.is_none_or(|v| ts - v > 6 * 3600),
            "batterie": batterie,
            "sur_pile": sur_pile,
            "signal": l.try_get::<Option<i64>, _>("signal").ok().flatten(),
            "t_min": l.try_get::<Option<f64>, _>("t_min").ok().flatten(),
            "t_max": l.try_get::<Option<f64>, _>("t_max").ok().flatten(),
            "temperature": nombre("temperature"),
            "humidite": nombre("humidity"),
            "etat": val("state").and_then(|(_, t)| t),
            "reglage_pct": brightness.map(|b| (b * 100.0 / 254.0).round() as i64),
            "puissance": nombre("power"),
            "energie": nombre("energy"),
            "vibration": nombre("vibration").map(|v| v >= 0.5),
            "salle_id": salle_id,
            "salle": salle,
            "usage": usage,
            "compteur_id": compteur_id,
            "usages": usages(&categorie).iter().map(|(c, l)| json!({"code": c, "libelle": l})).collect::<Vec<_>>(),
            "grandeurs": grandeurs,
        });
        par_categorie.entry(categorie).or_default().push(appareil);
    }
    let groupes: Vec<Value> = CATEGORIES
        .iter()
        .filter_map(|(code, titre, aide)| {
            par_categorie.remove(*code).map(|liste| json!({"code": code, "titre": titre, "aide": aide, "appareils": liste}))
        })
        .collect();

    let mut contexte = context(&session);
    contexte.insert("groupes".into(), Value::Array(groupes));
    contexte.insert("nb".into(), json!(lignes.len()));
    contexte.insert("pont".into(), etat_pont(pool).await);
    contexte.insert("piles_faibles".into(), Value::Array(piles_faibles));
    contexte.insert(
        "salles".into(),
        Value::Array(generic_rows(pool, "SELECT id, nom FROM salle ORDER BY ordre, nom").await?),
    );
    contexte.insert(
        "compteurs".into(),
        Value::Array(
            generic_rows(pool, "SELECT id, nom FROM compteur_energie WHERE actif=1 AND type IN ('electricite','elec','électricité') ORDER BY nom")
                .await
                .unwrap_or_default(),
        ),
    );
    contexte.insert("ok".into(), json!(parametres.get("ok")));
    render(&state, "ambiance_zigbee.html", Value::Object(contexte))
}

fn retour(message: &str, ieee: Option<&str>) -> Response {
    let ancre = ieee.map(|i| format!("#zb-{i}")).unwrap_or_default();
    Redirect::to(&format!("/ambiance/zigbee?ok={}{ancre}", encode_query(message))).into_response()
}

/// Rattache un appareil à une salle pour un usage, ou le détache.
pub(super) async fn rattacher(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let pool = &state.pool;
    let ieee = form_text(&form, "ieee").unwrap_or_default();
    let appareil: Option<(String, String, Option<String>)> =
        sqlx::query_as("SELECT nom, categorie, grandeurs FROM zigbee_appareil WHERE ieee=?")
            .bind(&ieee)
            .fetch_optional(pool)
            .await?;
    let (nom, categorie, grandeurs) = appareil.ok_or(AppError::NotFound)?;
    let grandeurs = grandeurs.unwrap_or_default();
    let grandeurs: Vec<&str> = grandeurs.split(',').collect();
    let usage = form_text(&form, "usage").unwrap_or_default();
    let salle_id = form_i64(&form, "salle_id");

    // Démarrages d'une machine (compresseur) : alerte au-delà du maximum.
    if categorie == "vibration" {
        sqlx::query(
            "INSERT INTO zigbee_demarrage_reglage(ieee,demarrages_max,periode_min) VALUES(?,?,?) \
             ON CONFLICT(ieee) DO UPDATE SET demarrages_max=excluded.demarrages_max, periode_min=excluded.periode_min",
        )
        .bind(&ieee)
        .bind(form_i64(&form, "demarrages_max").filter(|n| *n > 0))
        .bind(form_i64(&form, "periode_min").filter(|n| *n > 0).unwrap_or(60))
        .execute(pool)
        .await?;
    }

    // Seuils de température (frigo, salle) : propres à l'appareil.
    if categorie == "sonde" {
        sqlx::query("UPDATE zigbee_appareil SET t_min=?, t_max=? WHERE ieee=?")
            .bind(form_f64(&form, "t_min"))
            .bind(form_f64(&form, "t_max"))
            .bind(&ieee)
            .execute(pool)
            .await?;
    }

    let voulus = if usage == "aucun" || usage.is_empty() {
        Vec::new()
    } else {
        if !usages(&categorie).iter().any(|(c, _)| *c == usage) {
            return Err(AppError::Invalid("Usage inconnu pour cet appareil".into()));
        }
        points_pour(&usage, &grandeurs)
    };

    // Compteur d'énergie : existant, ou créé à la volée.
    let mut compteur_id = form_i64(&form, "compteur_id");
    if usage == "compteur" && compteur_id.is_none() {
        compteur_id = Some(
            sqlx::query("INSERT INTO compteur_energie(nom,type,unite,actif,note) VALUES(?,'electricite','kWh',1,?)")
                .bind(format!("Électricité — {nom}"))
                .bind("Créé depuis l'onglet Zigbee")
                .execute(pool)
                .await?
                .last_insert_rowid(),
        );
    }

    let mut tx = pool.begin().await?;
    let chemins_voulus: Vec<String> = voulus.iter().map(|(g, ..)| format!("{ieee}/{g}")).collect();
    // Points de cet appareil devenus inutiles : retirés (et de leurs programmes).
    let existants: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, chemin FROM ipx_point WHERE hote='zigbee' AND chemin LIKE ?")
            .bind(format!("{ieee}/%"))
            .fetch_all(&mut *tx)
            .await?;
    for (id, chemin) in existants {
        if !chemins_voulus.contains(&chemin) {
            for sql in [
                "UPDATE ipx_commande_journal SET point_id=NULL WHERE point_id=?",
                "DELETE FROM eclairage_etat WHERE point_id=?",
                "DELETE FROM eclairage_sortie WHERE point_id=?",
                "DELETE FROM ipx_point WHERE id=?",
            ] {
                sqlx::query(sql).bind(id).execute(&mut *tx).await?;
            }
        }
    }
    for (grandeur, role, suffixe, unite, pilotable, coefficient) in &voulus {
        let chemin = format!("{ieee}/{grandeur}");
        let nom_point = if suffixe.is_empty() { nom.clone() } else { format!("{nom} — {suffixe}") };
        let compteur = (*role == "compteur_elec").then_some(compteur_id).flatten();
        sqlx::query(
            "INSERT INTO ipx_point(hote,chemin,chemin_ecriture,nom,role,salle_id,unite,coefficient,decalage,\
             compteur_id,pilotable,actif,note) VALUES('zigbee',?,?,?,?,?,?,?,0,?,?,1,'Zigbee') \
             ON CONFLICT(hote,chemin) DO UPDATE SET chemin_ecriture=excluded.chemin_ecriture, nom=excluded.nom, \
             role=excluded.role, salle_id=excluded.salle_id, unite=excluded.unite, \
             coefficient=excluded.coefficient, compteur_id=excluded.compteur_id, \
             pilotable=excluded.pilotable, actif=1",
        )
        .bind(&chemin)
        .bind(pilotable.then_some(chemin.clone()))
        .bind(&nom_point)
        .bind(role)
        .bind(salle_id)
        .bind(unite)
        .bind(coefficient)
        .bind(compteur)
        .bind(i64::from(*pilotable))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    let message = if voulus.is_empty() {
        format!("« {nom} » détaché")
    } else {
        let libelle = usages(&categorie).iter().find(|(c, _)| *c == usage).map(|(_, l)| *l).unwrap_or("");
        format!("« {nom} » rattaché : {libelle}")
    };
    Ok(retour(&message, Some(&ieee)))
}

/// Allume ou éteint un interrupteur pour vérifier ce qu'il commande.
pub(super) async fn commander(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let ieee = form_text(&form, "ieee").unwrap_or_default();
    let (nom, categorie): (String, String) =
        sqlx::query_as("SELECT nom, categorie FROM zigbee_appareil WHERE ieee=?")
            .bind(&ieee)
            .fetch_optional(&state.pool)
            .await?
            .ok_or(AppError::NotFound)?;
    if categorie != "interrupteur" {
        return Err(AppError::Invalid(
            "Seuls les interrupteurs se commandent d'ici. Les variateurs de ventilation se règlent sur place ou dans Zigbee2MQTT.".into(),
        ));
    }
    let allumer = form.get("etat").map(String::as_str) == Some("1");
    let compte_rendu = super::ambiance::envoyer_commande(
        &state.pool,
        HOTE,
        &format!("{ieee}/state"),
        if allumer { 1.0 } else { 0.0 },
    )
    .await?;
    sqlx::query("INSERT INTO ipx_commande_journal(utilisateur,point_id,nom_point,valeur,resultat) VALUES(?,NULL,?,?,?)")
        .bind(&session.identifiant)
        .bind(&nom)
        .bind(if allumer { 1.0 } else { 0.0 })
        .bind(format!("Zigbee : {compte_rendu}"))
        .execute(&state.pool)
        .await?;
    Ok(retour(
        &format!("« {nom} » : {} {compte_rendu}", if allumer { "allumer" } else { "éteindre" }),
        Some(&ieee),
    ))
}

/// Valeur la plus récente d'un point Zigbee (pour la page des points).
pub(super) async fn valeur_point(pool: &SqlitePool, chemin: &str) -> Option<f64> {
    let (ieee, cle) = chemin.split_once('/')?;
    sqlx::query_scalar::<_, Option<f64>>("SELECT valeur FROM zigbee_valeur WHERE ieee=? AND cle=?")
        .bind(ieee)
        .bind(cle)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .flatten()
}

/// Dépose une commande pour le pont : `<ieee>/<grandeur>` à `valeur`.
pub(super) async fn deposer_commande(pool: &SqlitePool, chemin: &str, valeur: f64) -> AppResult<String> {
    let Some((ieee, cle)) = chemin.split_once('/') else {
        return Ok("refusée (chemin Zigbee invalide)".into());
    };
    let charge = match cle {
        "state" => json!({"state": if valeur >= 0.5 { "ON" } else { "OFF" }}),
        "brightness" => json!({"brightness": valeur.round().clamp(0.0, 255.0) as i64}),
        autre => json!({ autre: valeur }),
    };
    sqlx::query("INSERT INTO zigbee_commande(ieee,charge,demande) VALUES(?,?,?)")
        .bind(ieee)
        .bind(charge.to_string())
        .bind(maintenant())
        .execute(pool)
        .await?;
    Ok("transmise au Zigbee".into())
}

// ---------------------------------------------------------------------------
// Surveillance électrique (/ambiance/zigbee/electricite)
// ---------------------------------------------------------------------------

/// Phase d'une grandeur `voltage…` ou `current…` : A, B ou C.
fn phase(prefixe: &str, cle: &str) -> Option<char> {
    let reste = cle.strip_prefix(prefixe)?;
    match reste.trim_start_matches("_phase").trim_start_matches('_') {
        "" | "a" => Some('A'),
        "b" => Some('B'),
        "c" => Some('C'),
        _ => None,
    }
}

/// Courbe de puissance des dernières 24 h, en moyennes par quart d'heure,
/// prête pour un tracé SVG de 600 × 120.
fn courbe(mesures: &[(i64, f64)], debut: i64, fin: i64) -> (String, f64) {
    const PAS: i64 = 900;
    let mut seaux: Vec<(i64, f64, u32)> = Vec::new();
    for (ts, v) in mesures {
        let seau = (ts - debut) / PAS;
        match seaux.last_mut() {
            Some((s, somme, n)) if *s == seau => {
                *somme += v;
                *n += 1;
            }
            _ => seaux.push((seau, *v, 1)),
        }
    }
    let max = seaux.iter().map(|(_, s, n)| s / f64::from(*n)).fold(0.0_f64, f64::max).max(1.0);
    let largeur = (fin - debut) as f64;
    let points = seaux
        .iter()
        .map(|(s, somme, n)| {
            let x = ((*s * PAS + PAS / 2) as f64 / largeur * 600.0).clamp(0.0, 600.0);
            let y = 118.0 - (somme / f64::from(*n)) / max * 110.0;
            format!("{x:.1},{y:.1}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    (points, max)
}

fn duree_lisible(secondes: i64) -> String {
    let minutes = secondes / 60;
    if minutes < 60 {
        format!("{minutes} min")
    } else {
        format!("{} h {:02}", minutes / 60, minutes % 60)
    }
}

pub(super) async fn electricite(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(parametres): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let pool = &state.pool;
    let ts = maintenant();
    let minuit = Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|d| d.and_local_timezone(Local).single())
        .map(|d| d.timestamp())
        .unwrap_or(ts - 86400);
    let appareils: Vec<(String, String)> = sqlx::query_as(
        "SELECT ieee, nom FROM zigbee_appareil WHERE categorie='compteur' AND present=1 ORDER BY nom COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await?;
    let mut cartes = Vec::new();
    for (ieee, nom) in appareils {
        let valeurs: Vec<(String, Option<f64>, i64)> =
            sqlx::query_as("SELECT cle, valeur, ts FROM zigbee_valeur WHERE ieee=?")
                .bind(&ieee)
                .fetch_all(pool)
                .await?;
        let v = |cle: &str| valeurs.iter().find(|(c, ..)| c == cle).and_then(|(_, v, _)| *v);
        let vu = valeurs.iter().map(|(.., t)| *t).max();
        let puissance = v("total_power").or_else(|| v("power"));
        let energie = v("energy");
        let mut phases: Vec<Value> = Vec::new();
        for p in ['A', 'B', 'C'] {
            let tension = valeurs.iter().find(|(c, ..)| phase("voltage", c) == Some(p)).and_then(|(_, v, _)| *v);
            let courant = valeurs.iter().find(|(c, ..)| phase("current", c) == Some(p)).and_then(|(_, v, _)| *v);
            if tension.is_some() || courant.is_some() {
                phases.push(json!({"phase": p.to_string(), "tension": tension, "courant": courant}));
            }
        }
        let reglage: Option<(f64, Option<f64>, Option<i64>, Option<f64>, i64, f64, Option<i64>, Option<i64>)> = sqlx::query_as(
            "SELECT seuil_marche_w, puissance_max_w, marche_max_min, arret_max_h, phases, tension_min_v, en_marche, depuis \
             FROM zigbee_surveillance WHERE ieee=?",
        )
        .bind(&ieee)
        .fetch_optional(pool)
        .await?;
        let (seuil, p_max, marche_max, arret_max, surveiller_phases, tension_min, en_marche, depuis) =
            reglage.unwrap_or((20.0, None, None, None, 1, 190.0, None, None));

        // Aujourd'hui : temps de marche et démarrages.
        let periodes: Vec<(i64, Option<i64>)> = sqlx::query_as(
            "SELECT debut, fin FROM zigbee_marche WHERE ieee=? AND COALESCE(fin, ?) >= ? ORDER BY debut",
        )
        .bind(&ieee)
        .bind(ts)
        .bind(minuit)
        .fetch_all(pool)
        .await?;
        let marche_s: i64 = periodes
            .iter()
            .map(|(d, f)| (f.unwrap_or(ts) - (*d).max(minuit)).max(0))
            .sum();
        let demarrages = periodes.iter().filter(|(d, _)| *d >= minuit).count();

        // kWh depuis le relevé du jour (6 h).
        let releve: Option<f64> = sqlx::query_scalar(
            "SELECT r.valeur_index FROM releve_compteur r JOIN ipx_point p ON p.compteur_id=r.compteur_id \
             WHERE p.hote='zigbee' AND p.chemin=? AND r.date_releve=date('now','localtime')",
        )
        .bind(format!("{ieee}/energy"))
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        let depuis_releve = energie.zip(releve).map(|(e, r)| ((e - r) * 100.0).round() / 100.0);

        let cle_courbe = if v("total_power").is_some() { "total_power" } else { "power" };
        let mesures: Vec<(i64, f64)> = sqlx::query_as(
            "SELECT ts, valeur FROM zigbee_mesure WHERE ieee=? AND cle=? AND ts>=? ORDER BY ts",
        )
        .bind(&ieee)
        .bind(cle_courbe)
        .bind(ts - 86400)
        .fetch_all(pool)
        .await?;
        let (trace, max) = courbe(&mesures, ts - 86400, ts);
        let y_seuil = |w: f64| 118.0 - (w / max * 110.0).min(118.0);

        let alertes = generic_rows(
            pool,
            &format!(
                "SELECT debut, message, gravite FROM ambiance_alerte WHERE fin IS NULL AND type LIKE 'zigbee_%:{}'",
                ieee.replace('\'', "")
            ),
        )
        .await?;
        cartes.push(json!({
            "ieee": ieee,
            "nom": nom,
            "puissance": puissance,
            "energie": energie,
            "vu": vu.map(|t| il_y_a(ts - t)),
            "phases": phases,
            "en_marche": en_marche.map(|m| m != 0),
            "depuis": depuis.map(|d| duree_lisible(ts - d)),
            "marche_aujourdhui": duree_lisible(marche_s),
            "demarrages": demarrages,
            "depuis_releve": depuis_releve,
            "trace": trace,
            "echelle_max": max.round(),
            "y_seuil": y_seuil(seuil),
            "y_max": p_max.map(y_seuil),
            "seuil": seuil,
            "p_max": p_max,
            "marche_max": marche_max,
            "arret_max": arret_max,
            "surveiller_phases": surveiller_phases != 0,
            "tension_min": tension_min,
            "alertes": alertes,
        }));
    }
    let mut contexte = context(&session);
    contexte.insert("appareils".into(), Value::Array(cartes));
    contexte.insert("ok".into(), json!(parametres.get("ok")));
    render(&state, "zigbee_electricite.html", Value::Object(contexte))
}

pub(super) async fn electricite_reglages(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let ieee = form_text(&form, "ieee").unwrap_or_default();
    let nom: String = sqlx::query_scalar("SELECT nom FROM zigbee_appareil WHERE ieee=? AND categorie='compteur'")
        .bind(&ieee)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    let positif = |cle: &str| form_f64(&form, cle).filter(|v| *v > 0.0);
    sqlx::query(
        "INSERT INTO zigbee_surveillance(ieee,seuil_marche_w,puissance_max_w,marche_max_min,arret_max_h,phases,tension_min_v) \
         VALUES(?,?,?,?,?,?,?) ON CONFLICT(ieee) DO UPDATE SET seuil_marche_w=excluded.seuil_marche_w, \
         puissance_max_w=excluded.puissance_max_w, marche_max_min=excluded.marche_max_min, \
         arret_max_h=excluded.arret_max_h, phases=excluded.phases, tension_min_v=excluded.tension_min_v",
    )
    .bind(&ieee)
    .bind(form_f64(&form, "seuil_marche_w").filter(|v| *v >= 0.0).unwrap_or(20.0))
    .bind(positif("puissance_max_w"))
    .bind(positif("marche_max_min").map(|v| v.round() as i64))
    .bind(positif("arret_max_h"))
    .bind(i64::from(form.contains_key("phases")))
    .bind(form_f64(&form, "tension_min_v").filter(|v| *v > 0.0).unwrap_or(190.0))
    .execute(&state.pool)
    .await?;
    Ok(Redirect::to(&format!(
        "/ambiance/zigbee/electricite?ok={}#el-{ieee}",
        encode_query(&format!("Surveillance de « {nom} » enregistrée"))
    ))
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn base() -> anyhow::Result<AppState> {
        let state = super::super::documents::tests::state().await;
        for sql in [
            include_str!("../../migrations/0007_ipx_points.sql"),
            include_str!("../../migrations/0016_eclairage.sql"),
            include_str!("../../migrations/0017_zigbee.sql"),
            include_str!("../../migrations/0022_demarrages.sql"),
        ] {
            sqlx::raw_sql(sql).execute(&state.pool).await?;
        }
        sqlx::raw_sql(
            "ALTER TABLE ipx_point ADD COLUMN formule TEXT; \
             CREATE TABLE IF NOT EXISTS compteur_energie(id INTEGER PRIMARY KEY AUTOINCREMENT, nom TEXT NOT NULL, type TEXT NOT NULL, site_id INTEGER, unite TEXT NOT NULL DEFAULT 'm3', rappel_jours INTEGER, actif INTEGER NOT NULL DEFAULT 1, note TEXT); \
             INSERT INTO site(id,code) VALUES(1,'S'); INSERT INTO salle(id,site_id,nom,nb_cases) VALUES(1,1,'Post-sevrage',3); \
             INSERT INTO zigbee_appareil(ieee,nom,categorie,grandeurs,alimentation,batterie,vu_le) VALUES \
               ('0xa1','PS-1','sonde','battery,humidity,linkquality,temperature','Battery',12,strftime('%s','now')), \
               ('0xb2','Ecl PS','interrupteur','state,switch','Mains (single phase)',NULL,strftime('%s','now')), \
               ('0xc3','tri','compteur','energy,power,voltage','Mains (three phase)',NULL,strftime('%s','now')); \
             INSERT INTO zigbee_valeur(ieee,cle,valeur,texte,ts) VALUES('0xa1','temperature',24.5,NULL,1),('0xb2','state',0,'OFF',1);",
        )
        .execute(&state.pool)
        .await?;
        Ok(state)
    }

    fn f(paires: &[(&str, &str)]) -> HashMap<String, String> {
        let mut m: HashMap<String, String> = paires.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        m.insert("csrf_token".into(), "test".into());
        m
    }

    /// Sur une copie de la base réelle déjà remplie par le pont :
    /// `EO_TEST_BASE=… cargo test -- --ignored zigbee_sur_copie --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn zigbee_sur_copie_de_la_base_reelle() -> anyhow::Result<()> {
        use super::super::documents::tests::session;
        let chemin = std::env::var("EO_TEST_BASE")?;
        let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&chemin).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await?;
        crate::db::init(&pool).await?;
        let state = AppState::new(
            Config { bind: "127.0.0.1:8080".parse()?, db_path: chemin.into(), secure_cookies: false },
            pool,
            crate::templates::build()?,
        );
        let html = page(State(state.clone()), Extension(session()), Query(HashMap::new())).await?.0;
        println!("page zigbee : {} octets, PS-1 présent : {}", html.len(), html.contains("PS-1"));
        let ieee: String = sqlx::query_scalar("SELECT ieee FROM zigbee_appareil WHERE nom='PS-1'").fetch_one(&state.pool).await?;
        let salle: i64 = sqlx::query_scalar("SELECT id FROM salle WHERE nom='Post-sevrage'").fetch_one(&state.pool).await?;
        rattacher(State(state.clone()), Extension(session()),
            Form(f(&[("ieee", &ieee), ("usage", "ambiance"), ("salle_id", &salle.to_string())]))).await?;
        let fiche = super::super::salles::fiche_pour_test(&state, salle).await?;
        let debut = fiche.find("Ambiance").unwrap_or(0);
        println!("fiche post-sevrage : bloc ambiance présent : {}", fiche.contains("🌡️ Ambiance"));
        println!("{}", fiche[debut..(debut + 600).min(fiche.len())].split_whitespace().collect::<Vec<_>>().join(" "));
        Ok(())
    }

    #[tokio::test]
    async fn la_surveillance_electrique_s_affiche_et_se_regle() -> anyhow::Result<()> {
        use super::super::documents::tests::session;
        let state = base().await?;
        sqlx::raw_sql(include_str!("../../migrations/0018_surveillance_electrique.sql")).execute(&state.pool).await?;
        sqlx::raw_sql(
            "INSERT INTO zigbee_valeur(ieee,cle,valeur,texte,ts) VALUES \
               ('0xc3','power',4775,NULL,strftime('%s','now')),('0xc3','voltage',229,NULL,strftime('%s','now')), \
               ('0xc3','voltage_phase_b',0,NULL,strftime('%s','now')),('0xc3','current_phase_b',12.4,NULL,strftime('%s','now')); \
             INSERT INTO zigbee_mesure(ieee,cle,ts,valeur) VALUES('0xc3','power',strftime('%s','now')-3600,4000),('0xc3','power',strftime('%s','now')-60,4775);",
        )
        .execute(&state.pool)
        .await?;
        let html = electricite(State(state.clone()), Extension(session()), Query(HashMap::new())).await?.0;
        assert!(html.contains("tri") && html.contains("4775") && html.contains("polyline"), "{html}");
        assert!(html.contains("class=\"coupee\">0 V"), "phase B coupée signalée");
        electricite_reglages(State(state.clone()), Extension(session()),
            Form(f(&[("ieee", "0xc3"), ("seuil_marche_w", "50"), ("puissance_max_w", "9000"), ("phases", "1"), ("tension_min_v", "200")]))).await?;
        let (seuil, max): (f64, Option<f64>) = sqlx::query_as("SELECT seuil_marche_w, puissance_max_w FROM zigbee_surveillance WHERE ieee='0xc3'")
            .fetch_one(&state.pool).await?;
        assert_eq!((seuil, max), (50.0, Some(9000.0)));
        Ok(())
    }

    #[tokio::test]
    async fn ipx800_et_zigbee_ont_leur_carte_dans_mes_equipements() -> anyhow::Result<()> {
        use super::super::documents::tests::session;
        let state = base().await?;
        sqlx::raw_sql("INSERT INTO zigbee_pont(id,connecte,ts,messages) VALUES(1,1,strftime('%s','now'),10)")
            .execute(&state.pool).await?;
        let html = super::super::equipements(State(state.clone()), Extension(session())).await?.0;
        assert!(html.contains("Ipx800v5") && html.contains("Adresse à renseigner"), "{html}");
        assert!(html.contains("Réseau Zigbee") && html.contains("Connecté · 3 appareils"), "{html}");
        Ok(())
    }

    #[tokio::test]
    async fn rattacher_une_sonde_une_lampe_et_un_compteur() -> anyhow::Result<()> {
        use super::super::documents::tests::session;
        let state = base().await?;
        let html = page(State(state.clone()), Extension(session()), Query(HashMap::new())).await?.0;
        assert!(html.contains("PS-1") && html.contains("24.5"), "{html}");
        assert!(html.contains("pile"), "pile faible signalée");

        rattacher(State(state.clone()), Extension(session()),
            Form(f(&[("ieee", "0xa1"), ("usage", "ambiance"), ("salle_id", "1"), ("t_min", "18"), ("t_max", "30")]))).await?;
        let roles: Vec<String> = sqlx::query_scalar("SELECT role FROM ipx_point WHERE chemin LIKE '0xa1/%' ORDER BY role")
            .fetch_all(&state.pool).await?;
        assert_eq!(roles, vec!["humidite", "temperature"]);

        rattacher(State(state.clone()), Extension(session()),
            Form(f(&[("ieee", "0xb2"), ("usage", "eclairage"), ("salle_id", "1")]))).await?;
        let (role, pilotable, ecriture): (String, i64, String) =
            sqlx::query_as("SELECT role, pilotable, chemin_ecriture FROM ipx_point WHERE chemin='0xb2/state'")
                .fetch_one(&state.pool).await?;
        assert_eq!((role.as_str(), pilotable, ecriture.as_str()), ("eclairage", 1, "0xb2/state"));

        rattacher(State(state.clone()), Extension(session()),
            Form(f(&[("ieee", "0xc3"), ("usage", "compteur")]))).await?;
        let compteur: Option<i64> = sqlx::query_scalar("SELECT compteur_id FROM ipx_point WHERE chemin='0xc3/energy'")
            .fetch_one(&state.pool).await?;
        assert!(compteur.is_some(), "compteur d'énergie créé et relié");

        // Commande d'essai : déposée dans la file du pont.
        commander(State(state.clone()), Extension(session()), Form(f(&[("ieee", "0xb2"), ("etat", "1")]))).await?;
        let charge: String = sqlx::query_scalar("SELECT charge FROM zigbee_commande").fetch_one(&state.pool).await?;
        assert_eq!(charge, r#"{"state":"ON"}"#);

        // Détacher retire les points.
        rattacher(State(state.clone()), Extension(session()), Form(f(&[("ieee", "0xa1"), ("usage", "aucun")]))).await?;
        let reste: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ipx_point WHERE chemin LIKE '0xa1/%'")
            .fetch_one(&state.pool).await?;
        assert_eq!(reste, 0);
        Ok(())
    }
}
