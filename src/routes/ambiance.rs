//! Page Ambiance : ce que mesurent les salles, ce que commande la régulation.
//!
//! L'application ne parle jamais à l'automate. Elle lit ce que le démon
//! `eo-regul` a écrit dans la base et écrit les consignes qu'il ira relire.
//! Conséquence directe : une page qui s'affiche ne prouve pas que la
//! régulation tourne — d'où le contrôle du battement de cœur en tête de page,
//! qui dit franchement « la régulation ne tourne plus » plutôt que de laisser
//! croire à des valeurs figées.

use super::*;
use crate::regul_core::{consigne_courbe, PointCourbe};

/// Au-delà de ce délai sans battement, la régulation est considérée à l'arrêt.
const SEUIL_BATTEMENT_S: i64 = 180;

/// Âge maximal d'un état avant de le présenter comme périmé.
const SEUIL_ETAT_S: i64 = 300;

fn maintenant() -> i64 {
    chrono::Utc::now().timestamp()
}

fn duree_lisible(secondes: i64) -> String {
    if secondes < 0 {
        return "à l'instant".into();
    }
    if secondes < 90 {
        return format!("il y a {secondes} s");
    }
    let minutes = secondes / 60;
    if minutes < 90 {
        return format!("il y a {minutes} min");
    }
    let heures = minutes / 60;
    if heures < 48 {
        return format!("il y a {heures} h");
    }
    format!("il y a {} j", heures / 24)
}

/// État du démon de régulation, tel qu'on peut l'affirmer honnêtement.
pub(super) async fn etat_demon(pool: &SqlitePool) -> Value {
    let ligne: Option<(i64, Option<String>, i64, i64)> =
        sqlx::query_as("SELECT ts,version,cycles,erreurs FROM ambiance_heartbeat WHERE id=1")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    match ligne {
        None => json!({
            "actif": false,
            "jamais_demarre": true,
            "texte": "La régulation n'a jamais tourné sur cette base",
        }),
        Some((ts, version, cycles, erreurs)) => {
            let age = maintenant() - ts;
            json!({
                "actif": age <= SEUIL_BATTEMENT_S,
                "jamais_demarre": false,
                "age_s": age,
                "version": version,
                "cycles": cycles,
                "erreurs": erreurs,
                "texte": if age <= SEUIL_BATTEMENT_S {
                    format!("Régulation active, dernier cycle {}", duree_lisible(age))
                } else {
                    format!(
                        "Régulation à l'arrêt : aucun signe de vie {}. \
                         Les salles pilotées sont retombées sur le repli de l'automate.",
                        duree_lisible(age)
                    )
                },
            })
        }
    }
}

/// Consigne théorique du jour pour une salle, recalculée côté application avec
/// exactement le même noyau que le démon. Sert à afficher ce qui *devrait*
/// être appliqué, y compris quand la régulation ne tourne pas.
async fn consigne_theorique(
    pool: &SqlitePool,
    courbe_id: Option<i64>,
    jour: Option<i64>,
    fixe: f64,
) -> f64 {
    let (Some(courbe_id), Some(jour)) = (courbe_id, jour) else {
        return fixe;
    };
    let lignes = sqlx::query_as::<_, (i64, f64, Option<f64>, Option<f64>)>(
        "SELECT jour,t_cible,ventil_min,ventil_max FROM ambiance_courbe_point \
         WHERE courbe_id=? ORDER BY jour",
    )
    .bind(courbe_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let points: Vec<PointCourbe> = lignes
        .into_iter()
        .map(|(jour, t_cible, ventil_min, ventil_max)| PointCourbe {
            jour,
            t_cible,
            ventil_min,
            ventil_max,
        })
        .collect();
    consigne_courbe(&points, jour)
        .map(|consigne| consigne.t_cible)
        .unwrap_or(fixe)
}

/// Lignes de salles affichées, état compris.
async fn salles(pool: &SqlitePool) -> AppResult<Vec<Value>> {
    let lignes = sqlx::query(
        "SELECT s.id, s.nom, s.type AS type_salle, si.nom AS site_nom, si.code AS site_code, \
                a.actif, a.pilote, a.ipx_hote, a.entree_t, a.entree_h, a.entree_lux, \
                a.sortie_ventil, a.courbe_id, a.t_cible_fixe, a.plage, a.ventil_min, \
                a.ventil_max, a.hygro_cible, a.hygro_influence, a.anticipation, \
                a.ventil_securite, a.t_alerte_min, a.t_alerte_max, a.h_alerte_max, \
                c.nom AS courbe_nom, \
                e.ts, e.t, e.h, e.lux, e.consigne, e.ventil, e.jour_bande, e.mode, \
                e.sonde_ok, e.message, b.code AS bande_code \
         FROM salle s \
         JOIN site si ON si.id = s.site_id \
         LEFT JOIN ambiance_salle a ON a.salle_id = s.id \
         LEFT JOIN ambiance_courbe c ON c.id = a.courbe_id \
         LEFT JOIN ambiance_etat e ON e.salle_id = s.id \
         LEFT JOIN bande b ON b.id = e.bande_id \
         ORDER BY si.code, s.ordre, s.nom",
    )
    .fetch_all(pool)
    .await?;

    let instant = maintenant();
    let mut resultat = Vec::with_capacity(lignes.len());
    for ligne in lignes {
        let salle_id: i64 = ligne.try_get("id")?;
        let courbe_id: Option<i64> = ligne.try_get("courbe_id").ok();
        let jour_bande: Option<i64> = ligne.try_get("jour_bande").ok();
        let t_cible_fixe: f64 = ligne.try_get("t_cible_fixe").unwrap_or(22.0);
        let ts: Option<i64> = ligne.try_get("ts").ok();
        let age = ts.map(|ts| instant - ts);
        let alertes = sqlx::query_as::<_, (String, String, String, String)>(
            "SELECT type,gravite,debut,message FROM ambiance_alerte \
             WHERE salle_id=? AND fin IS NULL ORDER BY debut",
        )
        .bind(salle_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        let preconditionnement: Option<(Option<i64>, String, f64, Option<String>)> =
            sqlx::query_as(
                "SELECT p.bande_id,p.arrivee_prevue,p.heures_avant,b.code \
             FROM ambiance_preconditionnement p LEFT JOIN bande b ON b.id=p.bande_id \
             WHERE p.salle_id=?",
            )
            .bind(salle_id)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);

        resultat.push(json!({
            "id": salle_id,
            "nom": ligne.try_get::<String, _>("nom").unwrap_or_default(),
            "type": ligne.try_get::<String, _>("type_salle").ok(),
            "site": ligne.try_get::<String, _>("site_nom").ok()
                .or_else(|| ligne.try_get::<String, _>("site_code").ok()),
            "configuree": ligne.try_get::<i64, _>("actif").is_ok(),
            "actif": ligne.try_get::<i64, _>("actif").unwrap_or(0) != 0,
            "pilote": ligne.try_get::<i64, _>("pilote").unwrap_or(0) != 0,
            "ipx_hote": ligne.try_get::<String, _>("ipx_hote").ok(),
            "entree_t": ligne.try_get::<String, _>("entree_t").ok(),
            "entree_h": ligne.try_get::<String, _>("entree_h").ok(),
            "entree_lux": ligne.try_get::<String, _>("entree_lux").ok(),
            "sortie_ventil": ligne.try_get::<String, _>("sortie_ventil").ok(),
            "courbe_id": courbe_id,
            "courbe_nom": ligne.try_get::<String, _>("courbe_nom").ok(),
            "t_cible_fixe": t_cible_fixe,
            "plage": ligne.try_get::<f64, _>("plage").unwrap_or(3.0),
            "ventil_min": ligne.try_get::<f64, _>("ventil_min").unwrap_or(20.0),
            "ventil_max": ligne.try_get::<f64, _>("ventil_max").unwrap_or(100.0),
            "hygro_cible": ligne.try_get::<f64, _>("hygro_cible").ok(),
            "hygro_influence": ligne.try_get::<f64, _>("hygro_influence").unwrap_or(0.0),
            "anticipation": ligne.try_get::<f64, _>("anticipation").unwrap_or(0.0),
            "ventil_securite": ligne.try_get::<f64, _>("ventil_securite").unwrap_or(60.0),
            "t_alerte_min": ligne.try_get::<f64, _>("t_alerte_min").ok(),
            "t_alerte_max": ligne.try_get::<f64, _>("t_alerte_max").ok(),
            "h_alerte_max": ligne.try_get::<f64, _>("h_alerte_max").ok(),
            "bande": ligne.try_get::<String, _>("bande_code").ok(),
            "jour_bande": jour_bande,
            "t": ligne.try_get::<f64, _>("t").ok(),
            "h": ligne.try_get::<f64, _>("h").ok(),
            "lux": ligne.try_get::<f64, _>("lux").ok(),
            "consigne": ligne.try_get::<f64, _>("consigne").ok(),
            "consigne_theorique":
                consigne_theorique(pool, courbe_id, jour_bande, t_cible_fixe).await,
            "ventil": ligne.try_get::<f64, _>("ventil").ok(),
            "mode": ligne.try_get::<String, _>("mode").ok(),
            "sonde_ok": ligne.try_get::<i64, _>("sonde_ok").unwrap_or(1) != 0,
            "message": ligne.try_get::<String, _>("message").ok(),
            "mesure_agee": age.map(|age| age > SEUIL_ETAT_S).unwrap_or(true),
            "mesure_age_texte": age.map(duree_lisible),
            "alertes": alertes.into_iter().map(|(type_alerte, gravite, debut, message)| json!({
                "type": type_alerte,
                "gravite": gravite,
                "debut": debut,
                "message": message,
            })).collect::<Vec<_>>(),
            "preconditionnement": preconditionnement.map(|(bande_id, arrivee_prevue, heures_avant, bande_code)| json!({
                "bande_id": bande_id,
                "bande": bande_code,
                "arrivee_prevue": arrivee_prevue,
                "heures_avant": heures_avant,
            })),
        }));
    }
    Ok(resultat)
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
) -> AppResult<Html<String>> {
    let mut salles = salles(&state.pool).await?;
    let courbes = generic_rows(
        &state.pool,
        "SELECT c.id, c.nom, c.stade, c.note, \
                (SELECT COUNT(*) FROM ambiance_courbe_point p WHERE p.courbe_id=c.id) AS points \
         FROM ambiance_courbe c ORDER BY c.nom",
    )
    .await?;
    let alertes = generic_rows(
        &state.pool,
        "SELECT a.id, a.type, a.gravite, a.debut, a.message, s.nom AS salle \
         FROM ambiance_alerte a LEFT JOIN salle s ON s.id=a.salle_id \
         WHERE a.fin IS NULL ORDER BY a.gravite='critique' DESC, a.debut DESC LIMIT 50",
    )
    .await?;

    let tous_points = points(&state.pool).await?;

    // Éclairages pilotables déclarés dans l'inventaire IPX (rôle
    // « eclairage », voir /ambiance/ipx) : un panneau on/off à portée de main
    // plutôt qu'enfoui dans l'explorateur de l'automate.
    let eclairages: Vec<Value> = tous_points
        .iter()
        .filter(|p| {
            p.get("role").and_then(Value::as_str) == Some("eclairage")
                && p.get("pilotable").and_then(Value::as_bool).unwrap_or(false)
        })
        .cloned()
        .collect();

    // Points déjà rangés dans l'inventaire IPX (voir « Carte de l'automate »
    // et l'Explorateur), proposés en menu déroulant pour le raccordement de
    // chaque salle plutôt que de resaisir un chemin à la main. Ceux déjà
    // affectés à cette salle passent en tête de liste.
    for salle in &mut salles {
        let salle_id = salle.get("id").and_then(Value::as_i64);
        let object = json_object_mut(salle, "les points disponibles de la salle")?;
        for (champ, role) in [
            ("points_t", "temperature"),
            ("points_h", "humidite"),
            ("points_lux", "luminosite"),
            ("points_ventil", "ventilation"),
        ] {
            let mut candidats: Vec<Value> = tous_points
                .iter()
                .filter(|p| p.get("role").and_then(Value::as_str) == Some(role))
                .cloned()
                .collect();
            candidats.sort_by_key(|p| p.get("salle_id").and_then(Value::as_i64) != salle_id);
            object.insert(champ.into(), Value::Array(candidats));
        }
    }
    let hotes = hotes_autorises(&state.pool).await;

    let bandes_actives = generic_rows(
        &state.pool,
        "SELECT id,code FROM bande WHERE active=1 ORDER BY code",
    )
    .await?;

    let mut contexte = context(&session);
    contexte.insert("salles".into(), Value::Array(salles));
    contexte.insert("courbes".into(), Value::Array(courbes));
    contexte.insert("alertes".into(), Value::Array(alertes));
    contexte.insert("eclairages".into(), Value::Array(eclairages));
    contexte.insert("bandes_actives".into(), Value::Array(bandes_actives));
    contexte.insert(
        "hotes".into(),
        Value::Array(hotes.into_iter().map(Value::String).collect()),
    );
    contexte.insert("demon".into(), etat_demon(&state.pool).await);
    render(&state, "ambiance.html", Value::Object(contexte))
}

/// Enregistre l'hypothèse d'arrivée d'une bande dans une salle encore vide,
/// pour démarrer le préchauffage avant le transfert réel (voir 0013).
pub(super) async fn preconditionnement_enregistrer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(salle_id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let bande_id = form_i64(&form, "bande_id")
        .ok_or_else(|| AppError::Invalid("Bande attendue obligatoire".into()))?;
    let arrivee_prevue = form_text(&form, "arrivee_prevue")
        .ok_or_else(|| AppError::Invalid("Date et heure d'arrivée prévues obligatoires".into()))?;
    let heures_avant = form_f64(&form, "heures_avant")
        .filter(|h| *h >= 0.0 && *h <= 72.0)
        .ok_or_else(|| AppError::Invalid("Heures d'avance entre 0 et 72".into()))?;
    sqlx::query(
        "INSERT INTO ambiance_preconditionnement(salle_id,bande_id,arrivee_prevue,heures_avant,note) \
         VALUES(?,?,?,?,?) \
         ON CONFLICT(salle_id) DO UPDATE SET bande_id=excluded.bande_id,\
         arrivee_prevue=excluded.arrivee_prevue,heures_avant=excluded.heures_avant,note=excluded.note",
    )
    .bind(salle_id)
    .bind(bande_id)
    .bind(&arrivee_prevue)
    .bind(heures_avant)
    .bind(form_text(&form, "note"))
    .execute(&state.pool)
    .await?;
    Ok(Redirect::to("/ambiance").into_response())
}

pub(super) async fn preconditionnement_supprimer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(salle_id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    sqlx::query("DELETE FROM ambiance_preconditionnement WHERE salle_id=?")
        .bind(salle_id)
        .execute(&state.pool)
        .await?;
    Ok(Redirect::to("/ambiance").into_response())
}

/// Flux JSON du même contenu, pour le rafraîchissement de la page sans
/// rechargement complet.
pub(super) async fn etat_json(State(state): State<AppState>) -> AppResult<Response> {
    let corps = json!({
        "demon": etat_demon(&state.pool).await,
        "salles": salles(&state.pool).await?,
    });
    Ok((
        [(header::CONTENT_TYPE, "application/json")],
        corps.to_string(),
    )
        .into_response())
}

/// Enregistre les réglages d'une salle.
///
/// Deux interrupteurs distincts, et la distinction est importante :
/// `actif` fait mesurer et enregistrer, `pilote` autorise en plus l'écriture
/// sur l'automate. On peut donc observer une salle pendant des semaines, et
/// comparer avec la régulation en place, avant de lui confier quoi que ce soit.
pub(super) async fn enregistrer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(salle_id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;

    let existe: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM salle WHERE id=?")
        .bind(salle_id)
        .fetch_one(&state.pool)
        .await?;
    if existe == 0 {
        return Err(AppError::Invalid("Salle inconnue".into()));
    }

    let coche = |cle: &str| form.contains_key(cle);
    let ventil_min = form_f64(&form, "ventil_min")
        .unwrap_or(20.0)
        .clamp(0.0, 100.0);
    let ventil_max = form_f64(&form, "ventil_max")
        .unwrap_or(100.0)
        .clamp(ventil_min, 100.0);
    let securite = form_f64(&form, "ventil_securite")
        .unwrap_or(60.0)
        .clamp(0.0, 100.0);
    let plage = form_f64(&form, "plage").unwrap_or(3.0);
    if plage <= 0.0 {
        return Err(AppError::Invalid(
            "La plage de régulation doit être supérieure à zéro".into(),
        ));
    }
    let cible = form_f64(&form, "t_cible_fixe").unwrap_or(22.0);
    if !(0.0..=45.0).contains(&cible) {
        return Err(AppError::Invalid(
            "La température de consigne doit rester entre 0 et 45 °C".into(),
        ));
    }
    // Refuser un pilotage sans sortie configurée évite le cas silencieux d'une
    // salle déclarée pilotée qui ne commande rien.
    let sortie = form_text(&form, "sortie_ventil");
    if coche("pilote") && sortie.is_none() {
        return Err(AppError::Invalid(
            "Indiquez la sortie de ventilation avant d'activer le pilotage".into(),
        ));
    }
    // Une salle mesurée ou pilotée sans adresse d'automate ne lit ni n'écrit
    // jamais rien : mieux vaut le refuser tout de suite qu'un « repli de
    // sécurité » silencieux qu'on met du temps à relier à sa vraie cause.
    // (Le champ affiche une adresse d'exemple en gris tant qu'il est vide —
    // facile à prendre pour une valeur déjà saisie.)
    let hote = form_text(&form, "ipx_hote");
    if (coche("actif") || coche("pilote")) && hote.is_none() {
        return Err(AppError::Invalid(
            "Indiquez l'adresse de l'automate avant d'activer la mesure ou le pilotage".into(),
        ));
    }

    sqlx::query(
        "INSERT INTO ambiance_salle(salle_id,actif,pilote,ipx_hote,entree_t,entree_h,entree_lux,\
         sortie_ventil,courbe_id,t_cible_fixe,plage,ventil_min,ventil_max,hygro_cible,\
         hygro_influence,anticipation,ventil_securite,t_alerte_min,t_alerte_max,h_alerte_max) \
         VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) \
         ON CONFLICT(salle_id) DO UPDATE SET \
           actif=excluded.actif, pilote=excluded.pilote, ipx_hote=excluded.ipx_hote, \
           entree_t=excluded.entree_t, entree_h=excluded.entree_h, \
           entree_lux=excluded.entree_lux, sortie_ventil=excluded.sortie_ventil, \
           courbe_id=excluded.courbe_id, t_cible_fixe=excluded.t_cible_fixe, \
           plage=excluded.plage, ventil_min=excluded.ventil_min, \
           ventil_max=excluded.ventil_max, hygro_cible=excluded.hygro_cible, \
           hygro_influence=excluded.hygro_influence, anticipation=excluded.anticipation, \
           ventil_securite=excluded.ventil_securite, t_alerte_min=excluded.t_alerte_min, \
           t_alerte_max=excluded.t_alerte_max, h_alerte_max=excluded.h_alerte_max",
    )
    .bind(salle_id)
    .bind(i64::from(coche("actif")))
    .bind(i64::from(coche("pilote")))
    .bind(hote)
    .bind(form_text(&form, "entree_t"))
    .bind(form_text(&form, "entree_h"))
    .bind(form_text(&form, "entree_lux"))
    .bind(sortie)
    .bind(form_i64(&form, "courbe_id"))
    .bind(cible)
    .bind(plage)
    .bind(ventil_min)
    .bind(ventil_max)
    .bind(form_f64(&form, "hygro_cible"))
    .bind(form_f64(&form, "hygro_influence").unwrap_or(0.0))
    .bind(form_f64(&form, "anticipation").unwrap_or(0.0))
    .bind(securite)
    .bind(form_f64(&form, "t_alerte_min"))
    .bind(form_f64(&form, "t_alerte_max"))
    .bind(form_f64(&form, "h_alerte_max"))
    .execute(&state.pool)
    .await?;

    Ok(Redirect::to("/ambiance?ok=1").into_response())
}

/// Acquitte une alerte sans la fermer : la cause peut durer, mais l'éleveur a
/// vu le message. Le démon rouvrira le compteur si la situation se reproduit
/// après retour à la normale.
pub(super) async fn acquitter(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(alerte_id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    sqlx::query("UPDATE ambiance_alerte SET acquittee=1 WHERE id=?")
        .bind(alerte_id)
        .execute(&state.pool)
        .await?;
    Ok(Redirect::to("/ambiance").into_response())
}

/// Historique d'une salle, en JSON, pour tracer une courbe.
///
/// On sert le détail brut sur les périodes courtes et les moyennes horaires
/// au-delà : inutile d'envoyer au navigateur des dizaines de milliers de
/// points qu'il affichera sur trois cents pixels.
pub(super) async fn historique(
    State(state): State<AppState>,
    Path(salle_id): Path<i64>,
    Query(parametres): Query<HashMap<String, String>>,
) -> AppResult<Response> {
    // Deux façons de choisir la période : un nombre d'heures avant
    // maintenant (par défaut, et ce qu'utilisent les petits graphiques de
    // /ambiance), ou une plage de dates explicite (`depuis`/`jusqu_a`, format
    // AAAA-MM-JJ) pour consulter une période passée précise — ce que sert la
    // page d'historique complète.
    // En UTC, pas en heure locale : c'est ce que `julianday('now')` utilise
    // déjà partout ailleurs dans l'application (SQLite raisonne en UTC par
    // défaut), et ça évite toute dépendance à une base de fuseaux horaires
    // du système — absente sur certains environnements minimalistes.
    let bornes_explicites = parametres
        .get("depuis")
        .and_then(|v| chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").ok())
        .map(|debut| {
            let fin = parametres
                .get("jusqu_a")
                .and_then(|v| chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").ok())
                .unwrap_or_else(|| chrono::Utc::now().date_naive());
            let debut_ts = debut
                .and_hms_opt(0, 0, 0)
                .map(|dt| dt.and_utc().timestamp())
                .unwrap_or(0);
            let fin_ts = fin
                .succ_opt()
                .unwrap_or(fin)
                .and_hms_opt(0, 0, 0)
                .map(|dt| dt.and_utc().timestamp())
                .unwrap_or_else(maintenant);
            (debut_ts, fin_ts.min(maintenant()))
        });
    let (depuis, jusqu_a, heures) = match bornes_explicites {
        Some((debut_ts, fin_ts)) => (debut_ts, fin_ts, (fin_ts - debut_ts) / 3600),
        None => {
            let heures: i64 = parametres
                .get("heures")
                .and_then(|valeur| valeur.parse().ok())
                .unwrap_or(48)
                .clamp(1, 24 * 365);
            (maintenant() - heures * 3600, maintenant(), heures)
        }
    };
    let detaille = jusqu_a - depuis <= 72 * 3600;

    let mut series: Map<String, Value> = Map::new();
    for grandeur in ["t", "h", "lux", "ventil", "consigne"] {
        let points: Vec<(i64, i64)> = if detaille {
            sqlx::query_as(
                "SELECT ts,valeur FROM ambiance_mesure \
                 WHERE salle_id=? AND grandeur=? AND ts>=? AND ts<=? ORDER BY ts",
            )
            .bind(salle_id)
            .bind(grandeur)
            .bind(depuis)
            .bind(jusqu_a)
            .fetch_all(&state.pool)
            .await?
        } else {
            sqlx::query_as(
                "SELECT heure,moy FROM ambiance_heure \
                 WHERE salle_id=? AND grandeur=? AND heure>=? AND heure<=? ORDER BY heure",
            )
            .bind(salle_id)
            .bind(grandeur)
            .bind(depuis)
            .bind(jusqu_a)
            .fetch_all(&state.pool)
            .await?
        };
        series.insert(
            grandeur.to_string(),
            Value::Array(
                points
                    .into_iter()
                    .map(|(ts, valeur)| json!([ts, valeur as f64 / 10.0]))
                    .collect(),
            ),
        );
    }

    let corps = json!({
        "salle_id": salle_id,
        "heures": heures,
        "depuis": depuis,
        "jusqu_a": jusqu_a,
        "resolution": if detaille { "brut" } else { "horaire" },
        "series": Value::Object(series),
    });
    Ok((
        [(header::CONTENT_TYPE, "application/json")],
        corps.to_string(),
    )
        .into_response())
}

/// Page d'historique complète d'une salle : courbes température, hygrométrie
/// et ventilation sur une période choisie (par défaut les 7 derniers jours),
/// avec sélection libre de la date de départ et de fin. Le tracé lui-même
/// est fait côté navigateur, à partir du même JSON que les petits graphiques
/// de /ambiance — cette page ne fait qu'en donner une vue large avec une
/// vraie sélection de dates.
pub(super) async fn historique_page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(salle_id): Path<i64>,
    Query(parametres): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let salle: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT nom,type FROM salle WHERE id=?")
            .bind(salle_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((nom, type_salle)) = salle else {
        return Err(AppError::NotFound);
    };
    let aujourd_hui = chrono::Utc::now().date_naive();
    let depuis = parametres
        .get("depuis")
        .filter(|v| chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").is_ok())
        .cloned()
        .unwrap_or_else(|| {
            (aujourd_hui - chrono::Duration::days(7))
                .format("%Y-%m-%d")
                .to_string()
        });
    let jusqu_a = parametres
        .get("jusqu_a")
        .filter(|v| chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").is_ok())
        .cloned()
        .unwrap_or_else(|| aujourd_hui.format("%Y-%m-%d").to_string());

    let mut contexte = context(&session);
    contexte.insert("salle_id".into(), json!(salle_id));
    contexte.insert("salle_nom".into(), json!(nom));
    contexte.insert("salle_type".into(), json!(type_salle));
    contexte.insert("depuis".into(), json!(depuis));
    contexte.insert("jusqu_a".into(), json!(jusqu_a));
    render(&state, "ambiance_historique.html", Value::Object(contexte))
}

// ---------------------------------------------------------------------------
// Explorateur IPX800
//
// Une page de diagnostic qui interroge l'automate en direct et montre ce qu'il
// répond, objet par objet. Elle sert à deux choses : vérifier que la liaison
// fonctionne, et surtout relever les identifiants à recopier dans le
// raccordement des salles — ils ne se devinent pas, et les chercher dans
// l'interface de l'automate est fastidieux.
//
// Le serveur sert d'intermédiaire plutôt que de laisser le navigateur
// interroger l'automate : la clé d'API ne quitte jamais le serveur, et il n'y a
// pas de question de politique d'origine à régler.
// ---------------------------------------------------------------------------

/// Chemins d'API sondés lors de la découverte. L'API de l'IPX800 V5 évolue avec
/// son firmware : plutôt que de parier sur une forme, on essaie et on affiche
/// honnêtement ce qui a répondu.
/// Chemins sondés lors de la découverte.
///
/// La documentation GCE annonce cinq catégories — ipx, core, object, extension,
/// plugin — sans en détailler les chemins. On sonde donc les entrées probables
/// de chacune : ce qui répond est vrai, ce qui ne répond pas est signalé comme
/// tel. Aucune plage d'identifiants n'est déduite d'ici.
/// Chemins proposés dans l'explorateur. Ils ne sont plus devinés : ils viennent
/// des spécifications OpenAPI officielles de GCE (core, extension, object,
/// plugin). Le préfixe des extensions est `api/ebx/`, pas `api/extension`.
/// Ajouter `&option=filter_name` ou `&option=filter_state` à une collection
/// `api/core/*` pour n'obtenir que les noms ou que les états.
const CHEMINS_CANDIDATS: &[&str] = &[
    // L'IPX lui-même : 8 entrées digitales, 8 opto-isolées, 8 entrées
    // analogiques, 8 relais, 8 sorties collecteur ouvert, détection secteur
    // XPSU. Chemin non couvert par les specs publiées : les trois premiers
    // sont les candidats à tester.
    "api/system/ipx",
    "api/core/ipx",
    "api/system/info",
    // Cœur de l'automate
    "api/core/io",
    "api/core/ana",
    "api/core/ana/ana8",
    "api/core/ana/ana16",
    "api/core/ana/ana32",
    "api/core/ana/unit",
    "api/core/io/info",
    "api/core/ana/info",
    // Extensions sur le bus EBX
    "api/ebx/xthl",
    "api/ebx/x8r",
    "api/ebx/x8d",
    "api/ebx/x24d",
    "api/ebx/xpwm",
    "api/ebx/x010v",
    "api/ebx/xdimmer",
    "api/ebx/x4vr",
    "api/ebx/x4fp",
    "api/ebx/x200",
    "api/ebx/x400",
    "api/ebx/xdisplay",
    "api/ext/xdmx",
    // Objets logiciels internes
    "api/object/counter",
    "api/object/thermostat",
    "api/object/timer",
    "api/object/comparator",
    "api/object/switch",
    "api/object/planning",
    // Greffons
    "api/plugin/ipx800v4",
    "api/plugin/weather",
    "api/xdevice/xpool",
];

/// Hôtes que le serveur accepte d'interroger : ceux déjà enregistrés, et rien
/// d'autre. Sans cette restriction, la page deviendrait un relais permettant de
/// faire émettre au serveur des requêtes vers n'importe quelle adresse.
async fn hotes_autorises(pool: &SqlitePool) -> Vec<String> {
    let mut hotes: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT ipx_hote FROM ambiance_salle \
         WHERE ipx_hote IS NOT NULL AND trim(ipx_hote)<>''",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    if let Some(defaut) = sqlx::query_scalar::<_, Option<String>>(
        "SELECT valeur FROM parametre WHERE cle='ambiance_ipx_hote'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .flatten()
    {
        let defaut = defaut.trim().to_string();
        if !defaut.is_empty() {
            hotes.push(defaut);
        }
    }
    hotes.sort();
    hotes.dedup();
    hotes
}

pub(super) async fn parametre_texte(pool: &SqlitePool, cle: &str, defaut: &str) -> String {
    sqlx::query_scalar::<_, Option<String>>("SELECT valeur FROM parametre WHERE cle=?")
        .bind(cle)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .flatten()
        .map(|texte| texte.trim().to_string())
        .filter(|texte| !texte.is_empty())
        .unwrap_or_else(|| defaut.to_string())
}

/// Aplatit une réponse JSON en lignes « chemin → valeur ». C'est sous cette
/// forme qu'un identifiant se repère et se recopie, alors qu'un JSON imbriqué
/// de plusieurs centaines de lignes ne se lit pas à l'écran.
/// Nombre maximal de lignes extraites d'une réponse. Assez haut pour tenir un
/// automate complet avec toutes ses extensions ; la troncature éventuelle est
/// signalée dans la page, car une liste coupée en silence ferait chercher pour
/// rien un objet qui existe.
const LIGNES_MAX: usize = 20_000;

fn aplatir(prefixe: &str, valeur: &Value, sortie: &mut Vec<Value>) {
    if sortie.len() >= LIGNES_MAX {
        return;
    }
    match valeur {
        Value::Object(table) => {
            for (cle, imbrique) in table {
                let chemin = if prefixe.is_empty() {
                    cle.clone()
                } else {
                    format!("{prefixe}.{cle}")
                };
                aplatir(&chemin, imbrique, sortie);
            }
        }
        Value::Array(elements) => {
            for (index, element) in elements.iter().enumerate() {
                aplatir(&format!("{prefixe}[{index}]"), element, sortie);
            }
        }
        autre => {
            let (texte, numerique) = match autre {
                Value::Number(nombre) => (nombre.to_string(), nombre.as_f64()),
                Value::Bool(vrai) => (vrai.to_string(), Some(if *vrai { 1.0 } else { 0.0 })),
                Value::String(texte) => {
                    let nombre = texte.trim().replace(',', ".").parse::<f64>().ok();
                    (texte.clone(), nombre)
                }
                _ => ("null".to_string(), None),
            };
            sortie.push(json!({
                "chemin": prefixe,
                "valeur": texte,
                "nombre": numerique,
            }));
        }
    }
}

/// Interroge un chemin de l'automate et renvoie la réponse aplatie.
pub(super) async fn ipx_lire(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(parametres): Query<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    let hote = parametres
        .get("hote")
        .map(|valeur| valeur.trim().to_string())
        .unwrap_or_default();
    let autorises = hotes_autorises(&state.pool).await;
    if !autorises.iter().any(|connu| connu == &hote) {
        return Err(AppError::Invalid(
            "Adresse d'automate inconnue. Enregistrez-la d'abord dans les réglages d'une salle \
             ou dans l'accès à l'automate."
                .into(),
        ));
    }
    let chemin = parametres
        .get("chemin")
        .map(|valeur| valeur.trim().trim_start_matches('/').to_string())
        .unwrap_or_else(|| "api/core/io".to_string());
    // Un chemin ne contient ni schéma ni hôte : sinon la restriction d'hôte
    // ci-dessus se contournerait en glissant une adresse complète ici.
    if chemin.contains("://") || chemin.contains('@') || chemin.starts_with("//") {
        return Err(AppError::Invalid("Chemin d'API invalide".into()));
    }

    let cle = parametre_texte(&state.pool, "ambiance_ipx_cle", "").await;
    let gabarit = parametre_texte(
        &state.pool,
        "ambiance_ipx_url_lecture",
        "http://{hote}/{id}?ApiKey={cle}",
    )
    .await;
    let adresse = gabarit
        .replace("{hote}", &hote)
        .replace("{id}", &chemin)
        .replace("{cle}", &cle);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .build()
        .map_err(|error| AppError::Internal(error.into()))?;

    let debut = std::time::Instant::now();
    let reponse = client.get(&adresse).send().await;
    let duree = debut.elapsed().as_millis() as i64;

    // Une erreur de liaison est une information utile, pas un incident : on la
    // renvoie telle quelle pour qu'elle s'affiche dans la page.
    let corps = match reponse {
        Err(error) => json!({
            "ok": false,
            "chemin": chemin,
            "duree_ms": duree,
            "erreur": error.to_string(),
        }),
        Ok(reponse) => {
            let statut = reponse.status().as_u16();
            if !reponse.status().is_success() {
                json!({
                    "ok": false,
                    "chemin": chemin,
                    "statut": statut,
                    "duree_ms": duree,
                    "erreur": format!("l'automate a répondu {statut}"),
                })
            } else {
                let texte = reponse.text().await.unwrap_or_default();
                match serde_json::from_str::<Value>(&texte) {
                    Ok(json) => {
                        let mut lignes = Vec::new();
                        aplatir("", &json, &mut lignes);
                        let tronque = lignes.len() >= LIGNES_MAX;
                        json!({
                            "ok": true,
                            "chemin": chemin,
                            "statut": statut,
                            "duree_ms": duree,
                            "tronque": tronque,
                            "lignes": lignes,
                        })
                    }
                    Err(_) => json!({
                        "ok": false,
                        "chemin": chemin,
                        "statut": statut,
                        "duree_ms": duree,
                        "erreur": "réponse illisible (ce n'est pas du JSON)",
                        "extrait": texte.chars().take(300).collect::<String>(),
                    }),
                }
            }
        }
    };
    Ok((
        [(header::CONTENT_TYPE, "application/json")],
        corps.to_string(),
    )
        .into_response())
}

/// Explorateur en direct : même page, onglet « Explorateur ».
pub(super) async fn ipx_explorateur(
    state: State<AppState>,
    session: Extension<SessionData>,
    Query(mut parametres): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    parametres.insert("vue".into(), "explorateur".into());
    ipx_page(state, session, Query(parametres)).await
}

/// Réglages d'accès (adresse, clé, gabarits) : onglet « Accès ».
pub(super) async fn ipx_acces(
    state: State<AppState>,
    session: Extension<SessionData>,
    Query(mut parametres): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    parametres.insert("vue".into(), "acces".into());
    ipx_page(state, session, Query(parametres)).await
}

/// Entrées et sorties enregistrées (onglet par défaut), explorateur ou accès,
/// selon `vue`. Les trois vues partagent le même gabarit et le même contexte.
pub(super) async fn ipx_page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(parametres): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let hotes = hotes_autorises(&state.pool).await;
    let mut contexte = context(&session);
    let vue = match parametres.get("vue").map(String::as_str) {
        Some("explorateur") => "explorateur",
        Some("acces") => "acces",
        _ => "points",
    };
    contexte.insert("vue".into(), json!(vue));
    contexte.insert("points".into(), Value::Array(points(&state.pool).await?));
    contexte.insert(
        "roles".into(),
        Value::Array(
            ROLES
                .iter()
                .map(|(code, libelle, unite)| {
                    json!({"code": code, "libelle": libelle, "unite": unite})
                })
                .collect(),
        ),
    );
    contexte.insert(
        "salles".into(),
        Value::Array(
            generic_rows(
                &state.pool,
                "SELECT s.id, s.nom FROM salle s ORDER BY s.ordre, s.nom",
            )
            .await?,
        ),
    );
    contexte.insert(
        "compteurs".into(),
        Value::Array(
            generic_rows(
                &state.pool,
                "SELECT id, nom, type, unite FROM compteur_energie WHERE actif=1 ORDER BY type,nom",
            )
            .await?,
        ),
    );
    contexte.insert(
        "commandes".into(),
        Value::Array(
            generic_rows(
                &state.pool,
                "SELECT horodatage, utilisateur, nom_point, valeur, resultat \
                 FROM ipx_commande_journal ORDER BY id DESC LIMIT 10",
            )
            .await?,
        ),
    );
    contexte.insert(
        "ok".into(),
        match parametres.get("ok") {
            Some(message) => Value::String(message.replace('+', " ")),
            None => Value::Null,
        },
    );
    contexte.insert(
        "hotes".into(),
        Value::Array(hotes.into_iter().map(Value::String).collect()),
    );
    contexte.insert(
        "chemins".into(),
        Value::Array(
            CHEMINS_CANDIDATS
                .iter()
                .map(|chemin| Value::String((*chemin).to_string()))
                .collect(),
        ),
    );
    contexte.insert(
        "acces".into(),
        json!({
            "hote": parametre_texte(&state.pool, "ambiance_ipx_hote", "").await,
            "cle_definie": !parametre_texte(&state.pool, "ambiance_ipx_cle", "").await.is_empty(),
            "url_lecture": parametre_texte(&state.pool, "ambiance_ipx_url_lecture",
                "http://{hote}/{id}?ApiKey={cle}").await,
            "url_ecriture": parametre_texte(&state.pool, "ambiance_ipx_url_ecriture",
                "http://{hote}/{id}?ApiKey={cle}").await,
            "champ_valeur": parametre_texte(&state.pool, "ambiance_ipx_champ_valeur", "value").await,
            "champ_ecriture": parametre_texte(&state.pool, "ambiance_ipx_champ_ecriture", "value").await,
        }),
    );
    render(&state, "ambiance_ipx.html", Value::Object(contexte))
}

/// Enregistre l'accès à l'automate. Ces réglages étaient jusqu'ici à saisir
/// directement en base ; ils ont leur écran.
pub(super) async fn ipx_reglages(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;

    let mut valeurs: Vec<(&str, String)> = vec![
        (
            "ambiance_ipx_hote",
            form_text(&form, "hote").unwrap_or_default(),
        ),
        (
            "ambiance_ipx_url_lecture",
            form_text(&form, "url_lecture").unwrap_or_default(),
        ),
        (
            "ambiance_ipx_url_ecriture",
            form_text(&form, "url_ecriture").unwrap_or_default(),
        ),
        (
            "ambiance_ipx_champ_valeur",
            form_text(&form, "champ_valeur").unwrap_or_else(|| "value".into()),
        ),
        (
            "ambiance_ipx_champ_ecriture",
            form_text(&form, "champ_ecriture").unwrap_or_else(|| "value".into()),
        ),
    ];
    // La clé n'est pas réaffichée : un champ laissé vide veut dire « ne change
    // rien », pas « efface la clé ».
    if let Some(cle) = form_text(&form, "cle") {
        valeurs.push(("ambiance_ipx_cle", cle));
    }
    for (cle, valeur) in valeurs {
        if valeur.is_empty() && cle != "ambiance_ipx_hote" {
            continue;
        }
        sqlx::query(
            "INSERT INTO parametre(cle,valeur) VALUES(?,?) \
             ON CONFLICT(cle) DO UPDATE SET valeur=excluded.valeur",
        )
        .bind(cle)
        .bind(&valeur)
        .execute(&state.pool)
        .await?;
    }
    Ok(Redirect::to("/ambiance/ipx/acces?ok=Acc%C3%A8s+enregistr%C3%A9").into_response())
}

// ---------------------------------------------------------------------------
// Inventaire des points de l'automate
//
// Nommer les entrées et sorties, leur donner un rôle, et pour les compteurs les
// raccorder aux compteurs d'eau et d'électricité déjà suivis par le logiciel.
// ---------------------------------------------------------------------------

/// Rôles proposés. Le rôle ne change pas le comportement de la lecture : il
/// sert à regrouper, à choisir une unité par défaut, et à savoir quoi faire de
/// la valeur (un compteur s'accumule, une température se relève).
pub(super) const ROLES: &[(&str, &str, &str)] = &[
    ("temperature", "Température", "°C"),
    ("humidite", "Hygrométrie", "%"),
    ("luminosite", "Luminosité", "lux"),
    ("ventilation", "Commande de ventilation", "%"),
    ("eclairage", "Éclairage", ""),
    ("disjoncteur", "Disjoncteur / défaut", ""),
    ("compteur_eau", "Compteur d'eau", "L"),
    ("compteur_elec", "Compteur électrique", "kWh"),
    ("autre", "Autre", ""),
];

fn role_valide(role: &str) -> bool {
    ROLES.iter().any(|(code, _, _)| *code == role)
}

async fn points(pool: &SqlitePool) -> AppResult<Vec<Value>> {
    let lignes = sqlx::query(
        "SELECT p.id, p.hote, p.chemin, p.chemin_ecriture, p.nom, p.role, p.salle_id, \
                p.unite, p.coefficient, p.decalage, p.formule, p.compteur_id, p.releve_heure, \
                p.dernier_releve, p.pilotable, p.actif, p.ordre, p.note, \
                s.nom AS salle_nom, c.nom AS compteur_nom, c.unite AS compteur_unite \
         FROM ipx_point p \
         LEFT JOIN salle s ON s.id = p.salle_id \
         LEFT JOIN compteur_energie c ON c.id = p.compteur_id \
         ORDER BY p.role, p.ordre, p.nom",
    )
    .fetch_all(pool)
    .await?;
    Ok(lignes
        .into_iter()
        .map(|ligne| {
            json!({
                "id": ligne.try_get::<i64, _>("id").unwrap_or(0),
                "hote": ligne.try_get::<String, _>("hote").unwrap_or_default(),
                "chemin": ligne.try_get::<String, _>("chemin").unwrap_or_default(),
                "chemin_ecriture": ligne.try_get::<String, _>("chemin_ecriture").ok(),
                "nom": ligne.try_get::<String, _>("nom").unwrap_or_default(),
                "role": ligne.try_get::<String, _>("role").unwrap_or_default(),
                "salle_id": ligne.try_get::<i64, _>("salle_id").ok(),
                "salle": ligne.try_get::<String, _>("salle_nom").ok(),
                "unite": ligne.try_get::<String, _>("unite").ok(),
                "coefficient": ligne.try_get::<f64, _>("coefficient").unwrap_or(1.0),
                "decalage": ligne.try_get::<f64, _>("decalage").unwrap_or(0.0),
                "formule": ligne.try_get::<String, _>("formule").ok(),
                "compteur_id": ligne.try_get::<i64, _>("compteur_id").ok(),
                "compteur": ligne.try_get::<String, _>("compteur_nom").ok(),
                "compteur_unite": ligne.try_get::<String, _>("compteur_unite").ok(),
                "releve_heure": ligne.try_get::<i64, _>("releve_heure").unwrap_or(6),
                "dernier_releve": ligne.try_get::<String, _>("dernier_releve").ok(),
                "pilotable": ligne.try_get::<i64, _>("pilotable").unwrap_or(0) != 0,
                "actif": ligne.try_get::<i64, _>("actif").unwrap_or(1) != 0,
                "ordre": ligne.try_get::<i64, _>("ordre").unwrap_or(0),
                "note": ligne.try_get::<String, _>("note").ok(),
            })
        })
        .collect())
}

/// Valeurs en direct de tous les points enregistrés d'un automate.
pub(super) async fn ipx_points_json(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
) -> AppResult<Response> {
    require_writer(&session)?;
    let liste = points(&state.pool).await?;
    let acces = acces_lecture(&state.pool).await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|error| AppError::Internal(error.into()))?;

    let mut sortie = Vec::with_capacity(liste.len());
    for point in liste {
        let actif = point.get("actif").and_then(Value::as_bool).unwrap_or(false);
        let hote = point
            .get("hote")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let chemin = point
            .get("chemin")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let coefficient = point
            .get("coefficient")
            .and_then(Value::as_f64)
            .unwrap_or(1.0);
        let decalage = point.get("decalage").and_then(Value::as_f64).unwrap_or(0.0);
        let formule = point
            .get("formule")
            .and_then(Value::as_str)
            .map(str::to_string);

        let mut enrichi = point.clone();
        if actif {
            let brut = if hote == super::zigbee::HOTE {
                super::zigbee::valeur_point(&state.pool, &chemin).await
            } else {
                lire_valeur(&client, &acces, &hote, &chemin).await
            };
            if let Some(objet) = enrichi.as_object_mut() {
                objet.insert("brut".into(), json!(brut));
                objet.insert(
                    "valeur".into(),
                    json!(brut.and_then(|valeur| convertir(
                        valeur,
                        formule.as_deref(),
                        coefficient,
                        decalage
                    ))),
                );
            }
        }
        sortie.push(enrichi);
    }
    Ok((
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "points": sortie }).to_string(),
    )
        .into_response())
}

/// Convertit une valeur brute vers son unité réelle : la formule (ex. sonde
/// TC100 câblée sur une entrée analogique) l'emporte sur le coefficient et le
/// décalage quand elle est renseignée. Une formule mal formée ne se rabat
/// jamais silencieusement sur une autre conversion : elle renvoie l'absence
/// de valeur, comme partout ailleurs dans l'application.
fn convertir(brut: f64, formule: Option<&str>, coefficient: f64, decalage: f64) -> Option<f64> {
    match formule.filter(|f| !f.trim().is_empty()) {
        Some(formule) => crate::formule::evaluer(formule, brut),
        None => Some(brut * coefficient + decalage),
    }
}

/// Accès de lecture à l'automate, résolu une fois pour toute une passe.
struct AccesLecture {
    gabarit: String,
    gabarit_ecriture: String,
    champ: String,
    champ_ecriture: String,
    cle: String,
}

async fn acces_lecture(pool: &SqlitePool) -> AccesLecture {
    AccesLecture {
        gabarit: parametre_texte(
            pool,
            "ambiance_ipx_url_lecture",
            "http://{hote}/{id}?ApiKey={cle}",
        )
        .await,
        gabarit_ecriture: parametre_texte(
            pool,
            "ambiance_ipx_url_ecriture",
            "http://{hote}/{id}?ApiKey={cle}",
        )
        .await,
        champ: parametre_texte(pool, "ambiance_ipx_champ_valeur", "value").await,
        champ_ecriture: parametre_texte(pool, "ambiance_ipx_champ_ecriture", "value").await,
        cle: parametre_texte(pool, "ambiance_ipx_cle", "").await,
    }
}

async fn lire_valeur(
    client: &reqwest::Client,
    acces: &AccesLecture,
    hote: &str,
    chemin: &str,
) -> Option<f64> {
    if hote.trim().is_empty() || chemin.trim().is_empty() {
        return None;
    }
    let adresse = acces
        .gabarit
        .replace("{hote}", hote)
        .replace("{id}", chemin.trim())
        .replace("{cle}", &acces.cle);
    let reponse = client.get(&adresse).send().await.ok()?;
    if !reponse.status().is_success() {
        return None;
    }
    let corps: Value = reponse.json().await.ok()?;
    let champ = if est_objet_io(chemin) { "on" } else { acces.champ.as_str() };
    extraire_premier_nombre(&corps, champ)
}

/// Objet tout-ou-rien de l'IPX800 V5 (relais, sortie, bit virtuel) : il se
/// lit et se commande par son champ `on` (vrai/faux), pas par `value`. Sans
/// cette distinction, la lecture d'un relais renvoyait son identifiant (le
/// premier nombre de la réponse) et la commande envoyait `{"value": 1}`, que
/// l'automate ignore.
pub(super) fn est_objet_io(chemin: &str) -> bool {
    chemin.trim().trim_start_matches('/').starts_with("api/core/io/")
}

/// Envoie une valeur à une sortie et rend un compte rendu lisible
/// (« envoyée », « refusée (404) », « échec : … »). Ni contrôle d'accès ni
/// journal ici : c'est à l'appelant de les faire.
pub(super) async fn envoyer_commande(
    pool: &SqlitePool,
    hote: &str,
    chemin_ecriture: &str,
    valeur: f64,
) -> AppResult<String> {
    if hote.trim() == super::zigbee::HOTE {
        return super::zigbee::deposer_commande(pool, chemin_ecriture, valeur).await;
    }
    let acces = acces_lecture(pool).await;
    let adresse = acces
        .gabarit_ecriture
        .replace("{hote}", hote)
        .replace("{id}", chemin_ecriture.trim())
        .replace("{cle}", &acces.cle);
    let mut objet = serde_json::Map::new();
    if est_objet_io(chemin_ecriture) {
        objet.insert("on".into(), Value::Bool(valeur >= 0.5));
    } else {
        objet.insert(acces.champ_ecriture.clone(), Value::from(valeur));
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|error| AppError::Internal(error.into()))?;
    let resultat = client
        .put(&adresse)
        .json(&Value::Object(objet))
        .send()
        .await;
    Ok(match &resultat {
        Ok(reponse) if reponse.status().is_success() => "envoyée".to_string(),
        Ok(reponse) => format!("refusée ({})", reponse.status()),
        Err(error) => format!("échec : {error}"),
    })
}

/// Hôtes d'automate que le serveur accepte d'interroger (voir plus haut).
pub(super) async fn hotes_connus(pool: &SqlitePool) -> Vec<String> {
    hotes_autorises(pool).await
}

/// Sorties commandables de l'automate, lues en direct : les objets `io` des
/// sections de la carte marquées « commandable » (relais, collecteurs
/// ouverts, bits virtuels, marche/arrêt des variateurs). Renvoie `None` si
/// l'automate n'a pas répondu.
pub(super) async fn sorties_commandables(
    pool: &SqlitePool,
    hote: &str,
) -> AppResult<Option<Vec<Value>>> {
    let acces = acces_lecture(pool).await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(6))
        .build()
        .map_err(|error| AppError::Internal(error.into()))?;
    let elements = lire_source(&client, &acces, hote, "io").await;
    if elements.is_empty() {
        return Ok(None);
    }
    let sections = sqlx::query(
        "SELECT nom,source,motif,id_debut,id_fin FROM ipx_section \
         WHERE source='io' AND commandable=1 ORDER BY ordre, nom",
    )
    .fetch_all(pool)
    .await?;
    let criteres: Vec<(String, Critere)> = sections
        .iter()
        .map(|ligne| {
            (
                ligne.try_get("nom").unwrap_or_default(),
                Critere {
                    source: "io".into(),
                    motif: ligne
                        .try_get::<Option<String>, _>("motif")
                        .ok()
                        .flatten()
                        .unwrap_or_default(),
                    debut: ligne.try_get("id_debut").ok(),
                    fin: ligne.try_get("id_fin").ok(),
                },
            )
        })
        .collect();
    let mut sorties = Vec::new();
    for element in &elements {
        let Some(id) = element.get("_id").and_then(Value::as_i64) else {
            continue;
        };
        let nom = element.get("name").and_then(Value::as_str).unwrap_or("");
        let Some((section, _)) = criteres.iter().find(|(_, c)| c.accepte("io", id, nom)) else {
            continue;
        };
        sorties.push(json!({
            "id": id,
            "chemin": format!("api/core/io/{id}"),
            "nom_ipx": nom,
            "etat": element.get("on").and_then(Value::as_bool),
            "section": section,
        }));
    }
    Ok(Some(sorties))
}

fn extraire_premier_nombre(valeur: &Value, champ: &str) -> Option<f64> {
    fn en_nombre(valeur: &Value) -> Option<f64> {
        match valeur {
            Value::Number(nombre) => nombre.as_f64(),
            Value::String(texte) => texte.trim().replace(',', ".").parse().ok(),
            Value::Bool(vrai) => Some(if *vrai { 1.0 } else { 0.0 }),
            _ => None,
        }
    }
    match valeur {
        Value::Object(table) => {
            if let Some(trouve) = table.get(champ).and_then(en_nombre) {
                return Some(trouve);
            }
            table
                .values()
                .find_map(|imbrique| extraire_premier_nombre(imbrique, champ))
        }
        Value::Array(elements) => elements
            .iter()
            .find_map(|element| extraire_premier_nombre(element, champ)),
        autre => en_nombre(autre),
    }
}

/// Crée ou met à jour un point. L'identité d'un point est le couple
/// automate + chemin : rescanner le même objet ne crée pas de doublon.
pub(super) async fn ipx_point_enregistrer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;

    let hote =
        form_text(&form, "hote").ok_or_else(|| AppError::Invalid("Indiquez l'automate".into()))?;
    let chemin = form_text(&form, "chemin")
        .ok_or_else(|| AppError::Invalid("Indiquez le chemin du point".into()))?;
    let nom = form_text(&form, "nom")
        .ok_or_else(|| AppError::Invalid("Donnez un nom au point".into()))?;
    let role = form_text(&form, "role").unwrap_or_else(|| "autre".into());
    if !role_valide(&role) {
        return Err(AppError::Invalid("Rôle inconnu".into()));
    }
    let coefficient = form_f64(&form, "coefficient").unwrap_or(1.0);
    if coefficient == 0.0 {
        return Err(AppError::Invalid(
            "Un coefficient nul rendrait toutes les valeurs nulles".into(),
        ));
    }
    let formule = form_text(&form, "formule").filter(|f| !f.trim().is_empty());
    if let Some(formule) = &formule {
        if crate::formule::evaluer(formule, 1.0).is_none() {
            return Err(AppError::Invalid(
                "Formule invalide : vérifiez la syntaxe (X, + - * / ^, parenthèses)".into(),
            ));
        }
    }
    let pilotable = form.contains_key("pilotable");
    let chemin_ecriture = form_text(&form, "chemin_ecriture");
    if pilotable && chemin_ecriture.is_none() {
        return Err(AppError::Invalid(
            "Indiquez le chemin d'écriture avant de rendre ce point pilotable".into(),
        ));
    }
    // Un compteur sans compteur rattaché ne produirait aucun relevé : autant le
    // dire tout de suite plutôt que de laisser croire que le suivi tourne.
    let compteur_id = form_i64(&form, "compteur_id");
    if role.starts_with("compteur_") && compteur_id.is_none() {
        return Err(AppError::Invalid(
            "Choisissez le compteur d'eau ou d'électricité que ce point alimente, \
             ou créez-le d'abord dans « Eau et électricité »"
                .into(),
        ));
    }

    sqlx::query(
        "INSERT INTO ipx_point(hote,chemin,chemin_ecriture,nom,role,salle_id,unite,coefficient,\
         decalage,formule,compteur_id,releve_heure,pilotable,actif,ordre,note) \
         VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) \
         ON CONFLICT(hote,chemin) DO UPDATE SET \
           chemin_ecriture=excluded.chemin_ecriture, nom=excluded.nom, role=excluded.role, \
           salle_id=excluded.salle_id, unite=excluded.unite, coefficient=excluded.coefficient, \
           decalage=excluded.decalage, formule=excluded.formule, compteur_id=excluded.compteur_id, \
           releve_heure=excluded.releve_heure, pilotable=excluded.pilotable, \
           actif=excluded.actif, ordre=excluded.ordre, note=excluded.note",
    )
    .bind(&hote)
    .bind(&chemin)
    .bind(chemin_ecriture)
    .bind(&nom)
    .bind(&role)
    .bind(form_i64(&form, "salle_id"))
    .bind(form_text(&form, "unite"))
    .bind(coefficient)
    .bind(form_f64(&form, "decalage").unwrap_or(0.0))
    .bind(&formule)
    .bind(compteur_id)
    .bind(form_i64(&form, "releve_heure").unwrap_or(6).clamp(0, 23))
    .bind(i64::from(pilotable))
    .bind(i64::from(form.contains_key("actif")))
    .bind(form_i64(&form, "ordre").unwrap_or(0))
    .bind(form_text(&form, "note"))
    .execute(&state.pool)
    .await?;

    Ok(Redirect::to("/ambiance/ipx#points").into_response())
}

pub(super) async fn ipx_point_supprimer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(point_id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    sqlx::query("DELETE FROM ipx_point WHERE id=?")
        .bind(point_id)
        .execute(&state.pool)
        .await?;
    Ok(Redirect::to("/ambiance/ipx#points").into_response())
}

/// Ce qu'il faut savoir d'un point avant de lui envoyer une commande :
/// automate, chemin d'écriture, nom, autorisation, salle rattachée.
type PointCommandable = (String, Option<String>, String, i64, Option<i64>);

/// Envoie une valeur sur une sortie de l'automate.
///
/// Réservé aux points explicitement déclarés pilotables, et refusé sur un point
/// rattaché à une salle dont la ventilation est en régulation automatique : le
/// démon réécrirait la consigne au cycle suivant, et l'éleveur croirait avoir
/// agi alors que rien n'a tenu.
pub(super) async fn ipx_point_commander(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(point_id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let valeur = form_f64(&form, "valeur")
        .ok_or_else(|| AppError::Invalid("Indiquez la valeur à envoyer".into()))?;

    let point: Option<PointCommandable> = sqlx::query_as(
        "SELECT hote,chemin_ecriture,nom,pilotable,salle_id FROM ipx_point WHERE id=?",
    )
    .bind(point_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((hote, chemin_ecriture, nom, pilotable, salle_id)) = point else {
        return Err(AppError::NotFound);
    };
    if pilotable == 0 {
        return Err(AppError::Invalid(
            "Ce point n'est pas déclaré pilotable".into(),
        ));
    }
    let Some(chemin_ecriture) = chemin_ecriture else {
        return Err(AppError::Invalid(
            "Aucun chemin d'écriture pour ce point".into(),
        ));
    };
    if let Some(salle_id) = salle_id {
        // eo-regul ne réécrit jamais que la sortie ventilation qu'il pilote
        // (`ambiance_salle.sortie_ventil`) : bloquer toute commande manuelle
        // dans la salle, éclairage compris, empêcherait par exemple d'allumer
        // la lumière d'une salle par ailleurs correctement régulée.
        let pilotee: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM ambiance_salle WHERE salle_id=? AND actif=1 AND pilote=1 \
             AND sortie_ventil=?",
        )
        .bind(salle_id)
        .bind(chemin_ecriture.trim())
        .fetch_one(&state.pool)
        .await?;
        if pilotee > 0 {
            return Err(AppError::Invalid(
                "Cette sortie est en régulation automatique : la consigne serait réécrite au \
                 cycle suivant. Décochez « Piloter » sur la salle avant d'agir à la main."
                    .into(),
            ));
        }
    }

    let compte_rendu = envoyer_commande(&state.pool, &hote, &chemin_ecriture, valeur).await?;

    sqlx::query(
        "INSERT INTO ipx_commande_journal(utilisateur,point_id,nom_point,valeur,resultat) \
         VALUES(?,?,?,?,?)",
    )
    .bind(&session.identifiant)
    .bind(point_id)
    .bind(&nom)
    .bind(valeur)
    .bind(&compte_rendu)
    .execute(&state.pool)
    .await?;

    let message = format!("{nom} : commande {compte_rendu}");
    Ok(Redirect::to(&format!(
        "/ambiance/ipx?ok={}#points",
        encode_query(&message)
    ))
    .into_response())
}

// ---------------------------------------------------------------------------
// Carte de l'automate
//
// La même information que l'explorateur, mais rangée comme le matériel : les
// entrées analogiques d'un côté, les relais de l'autre. Chaque ligne peut être
// nommée, affectée à une salle, et commandée si c'est une sortie.
// ---------------------------------------------------------------------------

/// Un objet lu sur l'automate, tel qu'il apparaît dans une section.
fn objet_carte(element: &Value, points: &HashMap<String, Value>, source: &str) -> Option<Value> {
    let objet = element.as_object()?;
    let id = objet.get("_id").and_then(Value::as_i64)?;
    let chemin = format!("api/core/{source}/{id}");
    let connu = points.get(&chemin);
    let nom_ipx = objet.get("name").and_then(Value::as_str).unwrap_or("");
    let brut = objet.get("value").and_then(Value::as_f64);
    let formule = connu.and_then(|p| p.get("formule").and_then(Value::as_str));
    let coefficient = connu
        .and_then(|p| p.get("coefficient").and_then(Value::as_f64))
        .unwrap_or(1.0);
    let decalage = connu
        .and_then(|p| p.get("decalage").and_then(Value::as_f64))
        .unwrap_or(0.0);
    Some(json!({
        "id": id,
        "chemin": chemin,
        "nom_ipx": nom_ipx,
        "valeur": brut,
        // Valeur convertie via la formule ou le coefficient/décalage
        // enregistrés pour ce point — absente tant que rien n'est réglé, ou
        // si la formule échoue à s'évaluer (jamais un résultat inventé).
        "valeur_convertie": brut.and_then(|v| convertir(v, formule, coefficient, decalage)),
        "formule": formule,
        "coefficient": coefficient,
        "decalage": decalage,
        "etat": objet.get("on").and_then(Value::as_bool),
        "unite": objet.get("unit").and_then(Value::as_str),
        "point_id": connu.and_then(|p| p.get("id").cloned()),
        "nom": connu.and_then(|p| p.get("nom").and_then(Value::as_str)).unwrap_or(""),
        "salle_id": connu.and_then(|p| p.get("salle_id").cloned()),
        "salle": connu.and_then(|p| p.get("salle").and_then(Value::as_str)),
        // Un objet déjà rangé garde le rôle choisi par l'éleveur ; un objet
        // encore inconnu se voit proposer un rôle déduit de son nom dans
        // l'automate. La proposition n'engage rien : elle préremplit le
        // formulaire, qui reste modifiable avant enregistrement.
        "role": connu
            .and_then(|p| p.get("role").and_then(Value::as_str))
            .unwrap_or_else(|| role_devine(nom_ipx)),
        "pilotable": connu
            .and_then(|p| p.get("pilotable").and_then(Value::as_bool))
            .unwrap_or(false),
    }))
}

/// Devine le rôle d'un objet d'après le nom que l'automate lui donne.
///
/// Les noms suivent une convention stable — `[X-THL-MATER-1]Temperature` — ce
/// qui suffit à proposer un rôle. La proposition reste modifiable : on devine
/// pour épargner de la saisie, pas pour décider à la place de l'éleveur.
fn role_devine(nom: &str) -> &'static str {
    let bas = nom.to_lowercase();
    if bas.contains("temperature") || bas.contains("température") {
        "temperature"
    } else if bas.contains("humidity") || bas.contains("humid") || bas.contains("hygro") {
        "humidite"
    } else if bas.contains("luminosity") || bas.contains("lumin") {
        "luminosite"
    } else if bas.contains("ventil") {
        "ventilation"
    } else if bas.contains("eclairage") || bas.contains("éclairage") || bas.contains("light") {
        "eclairage"
    } else if bas.contains("dijoncteur") || bas.contains("disjoncteur") || bas.contains("fault") {
        "disjoncteur"
    } else {
        "autre"
    }
}

/// Lit une source entière de l'automate et renvoie ses éléments bruts.
async fn lire_source(
    client: &reqwest::Client,
    acces: &AccesLecture,
    hote: &str,
    source: &str,
) -> Vec<Value> {
    let adresse = acces
        .gabarit
        .replace("{hote}", hote)
        .replace("{id}", &format!("api/core/{source}"))
        .replace("{cle}", &acces.cle);
    let Ok(reponse) = client.get(&adresse).send().await else {
        return Vec::new();
    };
    if !reponse.status().is_success() {
        return Vec::new();
    }
    match reponse.json::<Value>().await {
        Ok(Value::Array(elements)) => elements,
        _ => Vec::new(),
    }
}

pub(super) async fn ipx_carte(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(parametres): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    require_writer(&session)?;
    let hotes = hotes_autorises(&state.pool).await;
    let hote = parametres
        .get("hote")
        .cloned()
        .or_else(|| hotes.first().cloned())
        .unwrap_or_default();

    // Points déjà nommés, indexés par chemin : c'est ce qui donne son nom et sa
    // salle à une ligne de la carte.
    let mut connus: HashMap<String, Value> = HashMap::new();
    for point in points(&state.pool).await? {
        if let Some(chemin) = point.get("chemin").and_then(Value::as_str) {
            connus.insert(chemin.to_string(), point.clone());
        }
    }

    let acces = acces_lecture(&state.pool).await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(6))
        .build()
        .map_err(|error| AppError::Internal(error.into()))?;

    let mut sources: HashMap<String, Vec<Value>> = HashMap::new();
    if !hote.is_empty() {
        for source in ["ana", "io"] {
            sources.insert(
                source.to_string(),
                lire_source(&client, &acces, &hote, source).await,
            );
        }
    }

    let lignes_sections = sqlx::query(
        "SELECT id,nom,source,motif,id_debut,id_fin,commandable,ordre,note \
         FROM ipx_section ORDER BY ordre, nom",
    )
    .fetch_all(&state.pool)
    .await?;

    let criteres: Vec<Critere> = lignes_sections
        .iter()
        .map(|ligne| Critere {
            source: ligne.try_get("source").unwrap_or_else(|_| "io".into()),
            motif: ligne
                .try_get::<Option<String>, _>("motif")
                .ok()
                .flatten()
                .unwrap_or_default(),
            debut: ligne.try_get("id_debut").ok(),
            fin: ligne.try_get("id_fin").ok(),
        })
        .collect();

    // Un objet ne rejoint que la PREMIÈRE section qui l'accepte. Sans cette
    // règle, un objet portant à la fois un nom reconnu et un identifiant dans
    // une plage large apparaîtrait deux fois, et on ne saurait plus lequel des
    // deux affichages fait foi.
    let mut retenus: Vec<Vec<Value>> = vec![Vec::new(); criteres.len()];
    let mut restes = Vec::new();
    for (source, elements) in &sources {
        for element in elements {
            let Some(id) = element.get("_id").and_then(Value::as_i64) else {
                continue;
            };
            let nom = element.get("name").and_then(Value::as_str).unwrap_or("");
            let place = criteres
                .iter()
                .position(|critere| critere.accepte(source, id, nom));
            let Some(objet) = objet_carte(element, &connus, source) else {
                continue;
            };
            match place {
                Some(rang) => retenus[rang].push(objet),
                None => restes.push(objet),
            }
        }
    }
    let cle_id = |objet: &Value| objet.get("id").and_then(Value::as_i64).unwrap_or(0);
    restes.sort_by_key(cle_id);

    let mut sections = Vec::with_capacity(lignes_sections.len());
    for (rang, ligne) in lignes_sections.iter().enumerate() {
        let critere = &criteres[rang];
        let mut objets = std::mem::take(&mut retenus[rang]);
        objets.sort_by_key(cle_id);
        // Une section vide n'est « à renseigner » que si elle n'a ni motif ni
        // plage : avec un motif, il n'y a rien à saisir, l'automate n'a
        // simplement aucun objet de ce genre.
        let a_renseigner =
            critere.motif.trim().is_empty() && (critere.debut.is_none() || critere.fin.is_none());
        sections.push(json!({
            "id": ligne.try_get::<i64, _>("id").unwrap_or(0),
            "nom": ligne.try_get::<String, _>("nom").unwrap_or_default(),
            "source": critere.source,
            "motif": critere.motif,
            "id_debut": critere.debut,
            "id_fin": critere.fin,
            "a_renseigner": a_renseigner,
            "commandable": ligne.try_get::<i64, _>("commandable").unwrap_or(0) != 0,
            "ordre": ligne.try_get::<i64, _>("ordre").unwrap_or(0),
            "note": ligne.try_get::<String, _>("note").ok(),
            "objets": objets,
        }));
    }

    let mut contexte = context(&session);
    contexte.insert("hote".into(), Value::String(hote));
    contexte.insert(
        "hotes".into(),
        Value::Array(hotes.into_iter().map(Value::String).collect()),
    );
    contexte.insert("sections".into(), Value::Array(sections));
    contexte.insert("restes".into(), Value::Array(restes));
    contexte.insert(
        "roles".into(),
        Value::Array(
            ROLES
                .iter()
                .map(|(code, libelle, unite)| {
                    json!({"code": code, "libelle": libelle, "unite": unite})
                })
                .collect(),
        ),
    );
    contexte.insert(
        "salles".into(),
        Value::Array(
            generic_rows(
                &state.pool,
                "SELECT s.id, s.nom FROM salle s ORDER BY s.ordre, s.nom",
            )
            .await?,
        ),
    );
    contexte.insert(
        "ok".into(),
        match parametres.get("ok") {
            Some(message) => Value::String(message.replace('+', " ")),
            None => Value::Null,
        },
    );
    render(&state, "ambiance_carte.html", Value::Object(contexte))
}

/// Règle de rangement d'une section : par nom si un motif est donné, par plage
/// d'identifiants sinon.
///
/// Le nom est le seul critère fiable pour séparer ce que l'automate mélange :
/// les identifiants sont attribués dans l'ordre de création des objets, donc
/// une sonde, un variateur et une fonction de régulation créés le même jour se
/// suivent, sans qu'aucune plage ne puisse les distinguer.
struct Critere {
    source: String,
    /// Fragments de nom séparés par des points-virgules. Un seul suffit.
    motif: String,
    debut: Option<i64>,
    fin: Option<i64>,
}

impl Critere {
    fn accepte(&self, source: &str, id: i64, nom: &str) -> bool {
        if self.source != source {
            return false;
        }
        let motif = self.motif.trim();
        if !motif.is_empty() {
            let nom = nom.to_lowercase();
            return motif
                .split(';')
                .map(str::trim)
                .filter(|fragment| !fragment.is_empty())
                .any(|fragment| nom.contains(&fragment.to_lowercase()));
        }
        match (self.debut, self.fin) {
            (Some(debut), Some(fin)) => id >= debut && id <= fin,
            _ => false,
        }
    }
}

/// Affecte un objet de la carte à une salle, en une seule action.
///
/// Crée le point s'il n'existe pas, avec un nom et un rôle proposés d'après le
/// nom que porte l'objet dans l'automate. C'est le geste courant : on parcourt
/// la carte et on range chaque entrée dans sa salle.
pub(super) async fn ipx_carte_affecter(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;

    let hote =
        form_text(&form, "hote").ok_or_else(|| AppError::Invalid("Automate manquant".into()))?;
    let chemin =
        form_text(&form, "chemin").ok_or_else(|| AppError::Invalid("Chemin manquant".into()))?;
    let salle_id = form_i64(&form, "salle_id");
    let role = form_text(&form, "role").unwrap_or_else(|| "autre".into());
    if !role_valide(&role) {
        return Err(AppError::Invalid("Rôle inconnu".into()));
    }
    let nom = form_text(&form, "nom")
        .or_else(|| form_text(&form, "nom_ipx"))
        .unwrap_or_else(|| chemin.clone());
    let unite = ROLES
        .iter()
        .find(|(code, _, _)| *code == role)
        .map(|(_, _, unite)| (*unite).to_string())
        .filter(|unite| !unite.is_empty());
    let commandable = form.contains_key("commandable");
    // Formule de conversion (ex. sonde TC100 sur une entrée analogique) : du
    // texte libre, validé sur une valeur de contrôle avant enregistrement
    // pour ne jamais garder une formule qui ne s'évalue jamais.
    let formule = form_text(&form, "formule").filter(|f| !f.trim().is_empty());
    if let Some(formule) = &formule {
        if crate::formule::evaluer(formule, 1.0).is_none() {
            return Err(AppError::Invalid(
                "Formule invalide : vérifiez la syntaxe (X, + - * / ^, parenthèses)".into(),
            ));
        }
    }
    let coefficient = form_f64(&form, "coefficient").unwrap_or(1.0);
    if coefficient == 0.0 {
        return Err(AppError::Invalid(
            "Un coefficient nul rendrait toutes les valeurs nulles".into(),
        ));
    }
    let decalage = form_f64(&form, "decalage").unwrap_or(0.0);

    sqlx::query(
        "INSERT INTO ipx_point(hote,chemin,chemin_ecriture,nom,role,salle_id,unite,coefficient,decalage,formule,actif,pilotable) \
         VALUES(?,?,?,?,?,?,?,?,?,?,1,?) \
         ON CONFLICT(hote,chemin) DO UPDATE SET nom=excluded.nom, role=excluded.role, \
           salle_id=excluded.salle_id, coefficient=excluded.coefficient, \
           decalage=excluded.decalage, formule=excluded.formule, actif=1",
    )
    .bind(&hote)
    .bind(&chemin)
    .bind(if commandable {
        Some(chemin.clone())
    } else {
        None
    })
    .bind(&nom)
    .bind(&role)
    .bind(salle_id)
    .bind(unite)
    .bind(coefficient)
    .bind(decalage)
    .bind(formule)
    .bind(i64::from(commandable))
    .execute(&state.pool)
    .await?;

    Ok(Redirect::to(&format!("/ambiance/ipx/carte?hote={}", encode_query(&hote))).into_response())
}

/// Crée ou ajuste une section de la carte.
pub(super) async fn ipx_section_enregistrer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let nom =
        form_text(&form, "nom").ok_or_else(|| AppError::Invalid("Nommez la section".into()))?;
    let source = form_text(&form, "source").unwrap_or_else(|| "io".into());
    if source != "io" && source != "ana" {
        return Err(AppError::Invalid("Source inconnue".into()));
    }
    let debut = form_i64(&form, "id_debut");
    let fin = form_i64(&form, "id_fin");
    if let (Some(debut), Some(fin)) = (debut, fin) {
        if fin < debut {
            return Err(AppError::Invalid(
                "La fin de plage précède son début".into(),
            ));
        }
    }

    match form_i64(&form, "id") {
        Some(id) => {
            sqlx::query(
                "UPDATE ipx_section SET nom=?,source=?,motif=?,id_debut=?,id_fin=?,\
                 commandable=?,ordre=?,note=? WHERE id=?",
            )
            .bind(&nom)
            .bind(&source)
            .bind(form_text(&form, "motif"))
            .bind(debut)
            .bind(fin)
            .bind(i64::from(form.contains_key("commandable")))
            .bind(form_i64(&form, "ordre").unwrap_or(0))
            .bind(form_text(&form, "note"))
            .bind(id)
            .execute(&state.pool)
            .await?;
        }
        None => {
            sqlx::query(
                "INSERT INTO ipx_section(nom,source,motif,id_debut,id_fin,commandable,ordre,\
                 note) VALUES(?,?,?,?,?,?,?,?)",
            )
            .bind(&nom)
            .bind(&source)
            .bind(form_text(&form, "motif"))
            .bind(debut)
            .bind(fin)
            .bind(i64::from(form.contains_key("commandable")))
            .bind(form_i64(&form, "ordre").unwrap_or(0))
            .bind(form_text(&form, "note"))
            .execute(&state.pool)
            .await?;
        }
    }
    Ok(Redirect::to("/ambiance/ipx/carte").into_response())
}

/// Climat du jour d'une salle, en unités lisibles (les tables `ambiance_jour`
/// gardent des dixièmes entiers pour l'historisation, pas pour l'affichage).
async fn climat_du_jour(pool: &SqlitePool, salle_id: Option<i64>, jour: &str) -> AppResult<Value> {
    let Some(salle_id) = salle_id else {
        return Ok(Value::Null);
    };
    let ligne: Option<(i64, i64, i64)> = sqlx::query_as(
        "SELECT moy,mini,maxi FROM ambiance_jour WHERE salle_id=? AND grandeur='t' AND jour=?",
    )
    .bind(salle_id)
    .bind(jour)
    .fetch_optional(pool)
    .await?;
    let Some((moy, mini, maxi)) = ligne else {
        return Ok(Value::Null);
    };
    let hygro: Option<i64> = sqlx::query_scalar(
        "SELECT moy FROM ambiance_jour WHERE salle_id=? AND grandeur='h' AND jour=?",
    )
    .bind(salle_id)
    .bind(jour)
    .fetch_optional(pool)
    .await?
    .flatten();
    Ok(json!({
        "t_moy": moy as f64 / 10.0,
        "t_mini": mini as f64 / 10.0,
        "t_maxi": maxi as f64 / 10.0,
        "h_moy": hygro.map(|v| v as f64 / 10.0),
    }))
}

/// Rapport croisant les observations comportementales (cannibalisme,
/// queue/oreille mordue...) et le climat mesuré le même jour dans la même
/// salle. Filtrable par bande pour l'inspection d'un cas précis, ou global
/// pour repérer un motif entre plusieurs bandes/salles.
pub(super) async fn comportement_rapport(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    axum::extract::Query(query): axum::extract::Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let band_filter = query
        .get("bande_id")
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v > 0);
    let base_sql = "SELECT o.id,o.date,o.categorie,o.gravite,o.nombre,o.note,o.salle_id,\
             b.code AS bande,s.nom AS salle,c.nom AS case_nom \
             FROM observation_comportement o JOIN bande b ON b.id=o.bande_id \
             LEFT JOIN salle s ON s.id=o.salle_id LEFT JOIN casesalle c ON c.id=o.case_id";
    let mut observations = match band_filter {
        Some(band_id) => {
            generic_rows(
                &state.pool,
                &format!(
                    "{base_sql} WHERE o.bande_id={band_id} ORDER BY o.date DESC,o.id DESC LIMIT 200"
                ),
            )
            .await?
        }
        None => {
            generic_rows(
                &state.pool,
                &format!("{base_sql} ORDER BY o.date DESC,o.id DESC LIMIT 200"),
            )
            .await?
        }
    };
    for ligne in &mut observations {
        let object = json_object_mut(ligne, "le rapport comportemental")?;
        let salle_id = object.get("salle_id").and_then(Value::as_i64);
        let date = object
            .get("date")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let climat = climat_du_jour(&state.pool, salle_id, &date).await?;
        object.insert("climat".into(), climat);
    }
    let lignes = observations;
    let bandes = generic_rows(
        &state.pool,
        "SELECT id,code FROM bande ORDER BY active DESC,date_mb DESC LIMIT 100",
    )
    .await?;
    let mut contexte = context(&session);
    contexte.insert("observations".into(), Value::Array(lignes));
    contexte.insert("bandes".into(), Value::Array(bandes));
    contexte.insert("bande_filtre".into(), json!(band_filter));
    render(&state, "comportement.html", Value::Object(contexte))
}

#[cfg(test)]
mod historique_plage_dates_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool_avec_salle_et_mesures() -> anyhow::Result<(sqlx::SqlitePool, i64)> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::raw_sql(include_str!("../../migrations/0001_schema.sql"))
            .execute(&pool)
            .await?;
        sqlx::raw_sql(include_str!("../../migrations/0005_ambiance.sql"))
            .execute(&pool)
            .await?;
        let site = sqlx::query("INSERT INTO site(code,nom) VALUES('S1','Site test')")
            .execute(&pool)
            .await?
            .last_insert_rowid();
        let salle = sqlx::query(
            "INSERT INTO salle(site_id,nom,type,nb_cases,ordre) VALUES(?,'Verraterie','Verraterie',1,1)",
        )
        .bind(site)
        .execute(&pool)
        .await?
        .last_insert_rowid();
        // Trois mesures : une avant-hier, une hier, une aujourd'hui — toutes
        // à la même heure locale que maintenant, pour rester loin des bords
        // de journée quel que soit le fuseau de la machine qui exécute le
        // test (le découpage des dates se fait en heure locale, comme la
        // page elle-même).
        let maintenant = chrono::Utc::now().timestamp();
        for decalage_jours in [2, 1, 0] {
            sqlx::query("INSERT INTO ambiance_mesure(salle_id,grandeur,ts,valeur) VALUES(?,?,?,?)")
                .bind(salle)
                .bind("t")
                .bind(maintenant - decalage_jours * 86400)
                .bind(220) // 22.0 °C en dixièmes
                .execute(&pool)
                .await?;
        }
        Ok((pool, salle))
    }

    /// Une plage `depuis`/`jusqu_a` ne doit renvoyer que les mesures qui
    /// tombent réellement dedans — ni celles d'avant, ni celles d'après.
    #[tokio::test]
    async fn la_plage_de_dates_filtre_bien_les_mesures() -> anyhow::Result<()> {
        let (pool, salle) = pool_avec_salle_et_mesures().await?;
        let state = AppState::new(
            crate::config::Config {
                bind: "127.0.0.1:8080".parse().unwrap(),
                db_path: std::path::PathBuf::from("data/test.db"),
                secure_cookies: false,
            },
            pool,
            minijinja::Environment::new(),
        );
        let hier = (chrono::Utc::now() - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let aujourd_hui = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let mut params = HashMap::new();
        params.insert("depuis".to_string(), hier.clone());
        params.insert("jusqu_a".to_string(), aujourd_hui);
        let reponse = historique(State(state), Path(salle), Query(params))
            .await?
            .into_response();
        let corps = axum::body::to_bytes(reponse.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&corps).unwrap();
        let points = json["series"]["t"].as_array().unwrap();
        // Seules les mesures d'hier et d'aujourd'hui doivent apparaître,
        // jamais celle d'avant-hier (hors plage).
        assert_eq!(points.len(), 2, "points trouvés : {points:?}");
        Ok(())
    }
}

#[cfg(test)]
mod enregistrer_validation_tests {
    use super::*;
    use crate::config::Config;
    use sqlx::sqlite::SqlitePoolOptions;

    fn session() -> SessionData {
        SessionData {
            uid: 1,
            identifiant: "test".into(),
            nom: "Test".into(),
            role: "admin".into(),
            sections: vec![],
            csrf: "csrf-test".into(),
            doit_changer_mdp: false,
            type_elevage: "naisseur_engraisseur".into(),
            module_genetique: false,
            module_prestataires: true,
            module_charcutiers_rfid: false,
            module_vente_directe: true,
        }
    }

    async fn etat() -> anyhow::Result<(AppState, i64)> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::raw_sql(include_str!("../../migrations/0001_schema.sql"))
            .execute(&pool)
            .await?;
        sqlx::raw_sql(include_str!("../../migrations/0005_ambiance.sql"))
            .execute(&pool)
            .await?;
        let site = sqlx::query("INSERT INTO site(code,nom) VALUES('S1','Site test')")
            .execute(&pool)
            .await?
            .last_insert_rowid();
        let room = sqlx::query(
            "INSERT INTO salle(site_id,nom,type,nb_cases,ordre) VALUES(?,'Verraterie','Verraterie',1,1)",
        )
        .bind(site)
        .execute(&pool)
        .await?
        .last_insert_rowid();
        let state = AppState::new(
            Config {
                bind: "127.0.0.1:8080".parse().unwrap(),
                db_path: std::path::PathBuf::from("data/test.db"),
                secure_cookies: false,
            },
            pool,
            crate::templates::build().expect("modèles valides"),
        );
        Ok((state, room))
    }

    fn champs(actif: bool, ipx_hote: &str) -> HashMap<String, String> {
        let mut form: HashMap<String, String> = [("csrf_token", "csrf-test")]
            .into_iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        if actif {
            form.insert("actif".into(), "1".into());
        }
        if !ipx_hote.is_empty() {
            form.insert("ipx_hote".into(), ipx_hote.into());
        }
        form
    }

    /// Le placeholder « à saisir, ex. 192.168.1.250 » n'est jamais soumis
    /// par un navigateur : sans adresse réellement tapée, activer la mesure
    /// ou le pilotage doit être refusé plutôt que d'enregistrer une salle qui
    /// ne lira ni n'écrira jamais rien.
    #[tokio::test]
    async fn refuse_actif_sans_adresse_dautomate() -> anyhow::Result<()> {
        let (state, room) = etat().await?;
        let err = enregistrer(
            State(state),
            Extension(session()),
            Path(room),
            Form(champs(true, "")),
        )
        .await
        .expect_err("doit être refusé sans adresse");
        assert!(matches!(err, AppError::Invalid(ref m) if m.contains("automate")));
        Ok(())
    }

    #[tokio::test]
    async fn accepte_actif_avec_adresse_dautomate() -> anyhow::Result<()> {
        let (state, room) = etat().await?;
        enregistrer(
            State(state.clone()),
            Extension(session()),
            Path(room),
            Form(champs(true, "192.168.1.250")),
        )
        .await?;
        let hote: Option<String> =
            sqlx::query_scalar("SELECT ipx_hote FROM ambiance_salle WHERE salle_id=?")
                .bind(room)
                .fetch_one(&state.pool)
                .await?;
        assert_eq!(hote.as_deref(), Some("192.168.1.250"));
        Ok(())
    }

    /// Une salle laissée inactive n'a pas besoin d'adresse : ce garde-fou ne
    /// gêne jamais la simple création d'une salle non encore raccordée.
    #[tokio::test]
    async fn accepte_inactif_sans_adresse() -> anyhow::Result<()> {
        let (state, room) = etat().await?;
        enregistrer(
            State(state),
            Extension(session()),
            Path(room),
            Form(champs(false, "")),
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod page_points_disponibles_tests {
    use super::*;
    use crate::config::Config;
    use sqlx::sqlite::SqlitePoolOptions;

    fn session() -> SessionData {
        SessionData {
            uid: 1,
            identifiant: "test".into(),
            nom: "Test".into(),
            role: "admin".into(),
            sections: vec![],
            csrf: "csrf-test".into(),
            doit_changer_mdp: false,
            type_elevage: "naisseur_engraisseur".into(),
            module_genetique: false,
            module_prestataires: true,
            module_charcutiers_rfid: false,
            module_vente_directe: true,
        }
    }

    /// Un capteur déjà rangé dans l'inventaire IPX (Carte de l'automate ou
    /// Explorateur) avec le bon rôle doit apparaître en option du menu
    /// déroulant de raccordement de la salle à laquelle il est affecté,
    /// plutôt que de laisser un champ texte vide sans piste.
    #[tokio::test]
    async fn un_point_range_apparait_dans_le_menu_de_sa_salle() -> anyhow::Result<()> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::raw_sql(include_str!("../../migrations/0001_schema.sql"))
            .execute(&pool)
            .await?;
        sqlx::raw_sql(include_str!("../../migrations/0005_ambiance.sql"))
            .execute(&pool)
            .await?;
        sqlx::raw_sql(include_str!("../../migrations/0007_ipx_points.sql"))
            .execute(&pool)
            .await?;
        sqlx::query("ALTER TABLE ipx_point ADD COLUMN formule TEXT")
            .execute(&pool)
            .await?;
        let site = sqlx::query("INSERT INTO site(code,nom) VALUES('S1','Site test')")
            .execute(&pool)
            .await?
            .last_insert_rowid();
        let room = sqlx::query(
            "INSERT INTO salle(site_id,nom,type,nb_cases,ordre) VALUES(?,'Verraterie','Verraterie',1,1)",
        )
        .bind(site)
        .execute(&pool)
        .await?
        .last_insert_rowid();
        sqlx::query("INSERT INTO ambiance_salle(salle_id,actif) VALUES(?,1)")
            .bind(room)
            .execute(&pool)
            .await?;
        sqlx::query("INSERT INTO ipx_point(hote,chemin,nom,role,salle_id) VALUES('192.168.1.250','api/core/thl/12','Sonde Verraterie','temperature',?)")
            .bind(room)
            .execute(&pool)
            .await?;

        let state = AppState::new(
            Config {
                bind: "127.0.0.1:8080".parse().unwrap(),
                db_path: std::path::PathBuf::from("data/test.db"),
                secure_cookies: false,
            },
            pool,
            crate::templates::build().expect("modèles valides"),
        );
        let reponse = page(State(state), Extension(session())).await?;
        let corps = reponse.0;
        assert!(
            corps.contains("Sonde Verraterie"),
            "le point rangé doit apparaître dans le menu déroulant"
        );
        assert!(
            corps.contains("(cette salle)"),
            "il doit être signalé comme déjà affecté à cette salle"
        );
        Ok(())
    }
}

#[cfg(test)]
mod ipx_point_commander_tests {
    use super::*;
    use crate::config::Config;
    use minijinja::Environment;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn etat_avec_salle_pilotee(sortie_ventil: &str) -> anyhow::Result<(AppState, i64)> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::raw_sql(include_str!("../../migrations/0001_schema.sql"))
            .execute(&pool)
            .await?;
        sqlx::raw_sql(include_str!("../../migrations/0005_ambiance.sql"))
            .execute(&pool)
            .await?;
        sqlx::raw_sql(include_str!("../../migrations/0007_ipx_points.sql"))
            .execute(&pool)
            .await?;
        let site = sqlx::query("INSERT INTO site(code,nom) VALUES('S1','Site test')")
            .execute(&pool)
            .await?
            .last_insert_rowid();
        let room = sqlx::query(
            "INSERT INTO salle(site_id,nom,type,nb_cases,ordre) VALUES(?,'PS 1','Post-sevrage',1,1)",
        )
        .bind(site)
        .execute(&pool)
        .await?
        .last_insert_rowid();
        sqlx::query(
            "INSERT INTO ambiance_salle(salle_id,actif,pilote,sortie_ventil) VALUES(?,1,1,?)",
        )
        .bind(room)
        .bind(sortie_ventil)
        .execute(&pool)
        .await?;
        let config = Config {
            bind: "127.0.0.1:8080".parse().unwrap(),
            db_path: std::path::PathBuf::from("data/test.db"),
            secure_cookies: false,
        };
        let env = Environment::new();
        let state = AppState::new(config, pool, env);
        Ok((state, room))
    }

    fn session() -> SessionData {
        SessionData {
            uid: 1,
            identifiant: "test".into(),
            nom: "Test".into(),
            role: "admin".into(),
            sections: vec![],
            csrf: "csrf-test".into(),
            doit_changer_mdp: false,
            type_elevage: "naisseur_engraisseur".into(),
            module_genetique: false,
            module_prestataires: true,
            module_charcutiers_rfid: false,
            module_vente_directe: true,
        }
    }

    async fn point(pool: &SqlitePool, salle_id: i64, chemin_ecriture: &str) -> anyhow::Result<i64> {
        Ok(sqlx::query("INSERT INTO ipx_point(hote,chemin,chemin_ecriture,nom,role,salle_id,pilotable) VALUES('ipx-test.invalid',?,?,'Éclairage PS 1','eclairage',?,1)")
            .bind(chemin_ecriture)
            .bind(chemin_ecriture)
            .bind(salle_id)
            .execute(pool)
            .await?
            .last_insert_rowid())
    }

    fn commande_form(valeur: &str) -> HashMap<String, String> {
        [("csrf_token", "csrf-test"), ("valeur", valeur)]
            .into_iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    /// La sortie ventilation elle-même reste protégée : eo-regul la
    /// réécrirait au cycle suivant.
    #[tokio::test]
    async fn la_sortie_ventilation_pilotee_reste_protegee() -> anyhow::Result<()> {
        let (state, room) = etat_avec_salle_pilotee("api/core/io/1").await?;
        let point_id = point(&state.pool, room, "api/core/io/1").await?;
        let err = ipx_point_commander(
            State(state),
            Extension(session()),
            Path(point_id),
            Form(commande_form("50")),
        )
        .await
        .expect_err("la sortie ventilation doit rester protégée");
        assert!(matches!(err, AppError::Invalid(_)));
        Ok(())
    }

    /// Un autre point de la même salle (ici l'éclairage) n'est jamais
    /// réécrit par eo-regul : le commander ne doit pas être bloqué par la
    /// régulation automatique de la ventilation.
    #[tokio::test]
    async fn un_point_hors_ventilation_de_la_salle_pilotee_reste_commandable() -> anyhow::Result<()>
    {
        let (state, room) = etat_avec_salle_pilotee("api/core/io/1").await?;
        // Chemin d'écriture différent de la sortie ventilation régulée.
        let point_id = point(&state.pool, room, "api/core/io/2").await?;
        let resultat = ipx_point_commander(
            State(state),
            Extension(session()),
            Path(point_id),
            Form(commande_form("1")),
        )
        .await;
        assert!(
            !matches!(resultat, Err(AppError::Invalid(_))),
            "l'éclairage ne doit pas être bloqué par la régulation de ventilation : {resultat:?}"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests_critere {
    use super::Critere;

    fn motif(source: &str, motif: &str) -> Critere {
        Critere {
            source: source.into(),
            motif: motif.into(),
            debut: None,
            fin: None,
        }
    }

    fn plage(source: &str, debut: i64, fin: i64) -> Critere {
        Critere {
            source: source.into(),
            motif: String::new(),
            debut: Some(debut),
            fin: Some(fin),
        }
    }

    #[test]
    fn un_motif_reconnait_le_prefixe_de_lautomate() {
        let c = motif("io", "x-010v");
        assert!(c.accepte("io", 65587, "[x-010v-1]On/Off 1"));
        assert!(c.accepte("io", 65628, "[x-010v-4]On/Off 1"));
        assert!(!c.accepte("io", 65536, "[IPX]Relay cmd 1"));
    }

    #[test]
    fn la_casse_du_nom_est_indifferente() {
        // L'automate écrit tantôt [X-THL-MATER-1], tantôt [x-010v-1].
        let c = motif("ana", "X-THL;XTHL");
        assert!(c.accepte("ana", 262211, "[X-THL-MATER-1]Temperature"));
        assert!(c.accepte("ana", 262300, "[XTHL Gestente]Humidity"));
        assert!(c.accepte("ana", 262400, "[x-thl verraterie]Luminosity"));
    }

    #[test]
    fn un_motif_ignore_la_plage_didentifiants() {
        // C'est tout l'intérêt : les identifiants d'un même bloc ne se suivent
        // pas, l'automate les attribue dans l'ordre de création des objets.
        let c = motif("ana", "x-010v");
        assert!(c.accepte("ana", 196618, "[x-010v-1]Consign 1"));
        assert!(c.accepte("ana", 999999, "[x-010v-9]Consign 1"));
    }

    #[test]
    fn une_section_ne_prend_pas_les_objets_de_lautre_source() {
        let c = motif("ana", "[WEATHER]");
        assert!(!c.accepte("io", 65575, "[WEATHER]Day/Night"));
        assert!(c.accepte("ana", 196609, "[WEATHER]Levels day"));
    }

    #[test]
    fn une_plage_reste_bornee_aux_deux_extremites() {
        let c = plage("io", 65536, 65543);
        assert!(c.accepte("io", 65536, "[IPX]Relay cmd 1"));
        assert!(c.accepte("io", 65543, "[IPX]Relay cmd 8"));
        assert!(!c.accepte("io", 65535, "peu importe"));
        assert!(!c.accepte("io", 65544, "[IPX]Relay state 1"));
    }

    #[test]
    fn une_section_sans_motif_ni_plage_naccepte_rien() {
        let c = Critere {
            source: "io".into(),
            motif: "   ".into(),
            debut: None,
            fin: None,
        };
        assert!(!c.accepte("io", 65536, "[IPX]Relay cmd 1"));
    }

    #[test]
    fn lordre_des_sections_protege_les_variateurs_de_la_logique_interne() {
        // « Variateurs 0-10V » est en ordre 80, « Logique interne » en 200 :
        // [x-010v-1]On/Off 1 correspond aux deux, il doit rester au variateur.
        let variateurs = motif("io", "x-010v");
        let logique = motif("io", "]Enable;]Result;]Output;]On/Off");
        let nom = "[x-010v-1]On/Off 1";
        assert!(variateurs.accepte("io", 65587, nom));
        assert!(logique.accepte("io", 65587, nom));
        let sections = [&variateurs, &logique];
        assert_eq!(
            sections.iter().position(|c| c.accepte("io", 65587, nom)),
            Some(0)
        );
    }
}
