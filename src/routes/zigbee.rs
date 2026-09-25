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
        let appareil = json!({
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn base() -> anyhow::Result<AppState> {
        let state = super::super::documents::tests::state().await;
        for sql in [
            include_str!("../../migrations/0007_ipx_points.sql"),
            include_str!("../../migrations/0016_eclairage.sql"),
            include_str!("../../migrations/0017_zigbee.sql"),
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
