//! Mes salles (/salles) : tout ce que le logiciel sait d'une salle, réuni.
//!
//! Rien n'est saisi ici. Chaque bloc est calculé à partir de ce qui est relié
//! à la salle dans la base — cases, truies, transferts, inventaires, sondes,
//! lampes, ventilateurs, alertes, mortalités, observations, lavage. Ajouter
//! une lampe ou une sonde à une salle, y transférer une bande, la fait
//! apparaître ici sans autre réglage ; un bloc sans rien de relié est masqué.

use super::*;

fn maintenant() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Valeur des tables d'agrégats d'ambiance : stockée en dixièmes.
fn dixiemes(valeur: Option<i64>) -> Option<f64> {
    valeur.map(|v| v as f64 / 10.0)
}

/// Âge au-delà duquel l'état d'ambiance n'est plus présenté comme actuel.
const ETAT_PERIME_S: i64 = 300;

/// Moment d'une mesure, lisible : « à 14:32 » dans les dernières 24 h,
/// « le 26/09 à 01:14 » au-delà.
pub(super) fn quand(ts: i64, maintenant: i64) -> String {
    use chrono::TimeZone;
    let Some(d) = Local.timestamp_opt(ts, 0).single() else {
        return String::new();
    };
    if maintenant - ts < 86_400 {
        d.format("à %H:%M").to_string()
    } else {
        d.format("le %d/%m à %H:%M").to_string()
    }
}

/// Valeur et heure d'un point Zigbee (`<ieee>/<grandeur>`).
async fn zigbee_valeur_ts(pool: &SqlitePool, chemin: &str) -> Option<(Option<f64>, i64)> {
    let (ieee, cle) = chemin.split_once('/')?;
    sqlx::query_as::<_, (Option<f64>, i64)>("SELECT valeur, ts FROM zigbee_valeur WHERE ieee=? AND cle=?")
        .bind(ieee)
        .bind(cle)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

/// Bande présente et jour de présence : dernier transfert de bande vers la
/// salle, sinon mise-bas des truies qui s'y trouvent (maternité). Même règle
/// que la régulation, pour que la fiche et la consigne parlent du même lot.
async fn bande_en_place(pool: &SqlitePool, salle_id: i64) -> AppResult<Option<Value>> {
    // Dernier transfert de porcs arrivé dans la salle (ou l'une de ses
    // cases). Sans bande notée, on remonte à la case d'où ils venaient,
    // quatre étapes au plus.
    let arrivee: Option<(Option<i64>, Option<i64>, String)> = sqlx::query_as(
        "SELECT t.bande_id, t.case_source_id, t.date FROM transfert t \
         WHERE t.espece='porc' AND t.date IS NOT NULL \
           AND (t.salle_dest_id=?1 OR t.case_dest_id IN (SELECT id FROM casesalle WHERE salle_id=?1)) \
         ORDER BY t.date DESC, t.id DESC LIMIT 1",
    )
    .bind(salle_id)
    .fetch_optional(pool)
    .await?;
    let mut bande_trouvee: Option<(i64, String)> = None;
    if let Some((mut bande, mut source, date_arrivee)) = arrivee {
        let mut date = date_arrivee.clone();
        for _ in 0..4 {
            if bande.is_some() {
                break;
            }
            let Some(case) = source else { break };
            let precedent: Option<(Option<i64>, Option<i64>, String)> = sqlx::query_as(
                "SELECT bande_id, case_source_id, date FROM transfert WHERE case_dest_id=? AND espece='porc' \
                 AND date<=? ORDER BY date DESC, id DESC LIMIT 1",
            )
            .bind(case)
            .bind(&date)
            .fetch_optional(pool)
            .await?;
            let Some((b, s, d)) = precedent else { break };
            bande = b;
            source = s;
            date = d;
        }
        if let Some(id) = bande {
            bande_trouvee = sqlx::query_as("SELECT id, code FROM bande WHERE id=? AND active=1")
                .bind(id)
                .fetch_optional(pool)
                .await?;
        }
        if let Some((id, code)) = bande_trouvee {
            let jours: i64 = sqlx::query_scalar("SELECT CAST(julianday('now') - julianday(?) AS INTEGER)")
                .bind(&date_arrivee)
                .fetch_one(pool)
                .await?;
            return Ok(Some(json!({"id": id, "code": code, "jour": jours.max(0)})));
        }
    }
    let ligne: Option<(i64, String, i64)> = sqlx::query_as(
        "SELECT b.id, b.code, CAST(julianday('now') - julianday(t.date) AS INTEGER) \
         FROM transfert t JOIN bande b ON b.id=t.bande_id \
         WHERE t.salle_dest_id=? AND t.date IS NOT NULL AND b.active=1 \
         ORDER BY t.date DESC, t.id DESC LIMIT 1",
    )
    .bind(salle_id)
    .fetch_optional(pool)
    .await?;
    let ligne = match ligne {
        Some(ligne) => Some(ligne),
        None => {
            sqlx::query_as(
                "SELECT b.id, b.code, CAST(julianday('now') - julianday(e.date) AS INTEGER) \
                 FROM evenement e JOIN truie t ON t.id=e.truie_id JOIN bande b ON b.id=e.bande_id \
                 WHERE e.type='mise_bas' AND t.salle_id=? AND t.reformee=0 AND b.active=1 \
                 ORDER BY e.date DESC, e.id DESC LIMIT 1",
            )
            .bind(salle_id)
            .fetch_optional(pool)
            .await?
        }
    };
    Ok(ligne.map(|(id, code, jours)| json!({"id": id, "code": code, "jour": jours.max(0)})))
}

/// Résumé d'une salle pour la liste : ce qui se lit d'un coup d'œil.
async fn resume(pool: &SqlitePool, ligne: &sqlx::sqlite::SqliteRow) -> AppResult<Value> {
    let id: i64 = ligne.try_get("id")?;
    let ts = maintenant();
    let cases: Vec<(i64, Option<i64>)> =
        sqlx::query_as("SELECT id, nb_max_porcs FROM casesalle WHERE salle_id=?")
            .bind(id)
            .fetch_all(pool)
            .await?;
    let mut porcs = 0;
    for (case_id, _) in &cases {
        porcs += case_pig_count(pool, *case_id).await?.max(0);
    }
    let places: i64 = cases.iter().filter_map(|(_, p)| *p).sum();
    let truies: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM truie t LEFT JOIN casesalle c ON c.id=t.case_id \
         WHERE t.reformee=0 AND COALESCE(c.salle_id, t.salle_id)=?",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    let etat: Option<(i64, Option<f64>, Option<f64>, Option<f64>, Option<f64>, i64)> =
        sqlx::query_as("SELECT ts,t,h,lux,consigne,sonde_ok FROM ambiance_etat WHERE salle_id=?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    // Sans état de régulation (salle sans sonde IPX), les sondes Zigbee
    // rattachées à la salle donnent la température et l'humidité (moyenne
    // des sondes de la salle).
    let etat = match etat {
        Some(etat) => Some(etat),
        None => sqlx::query_as::<_, (Option<i64>, Option<f64>, Option<f64>, Option<f64>)>(
            "SELECT MAX(v.ts), \
                    AVG(CASE WHEN p.role='temperature' THEN v.valeur * p.coefficient + p.decalage END), \
                    AVG(CASE WHEN p.role='humidite' THEN v.valeur * p.coefficient + p.decalage END), \
                    AVG(CASE WHEN p.role='luminosite' THEN v.valeur * p.coefficient + p.decalage END) \
             FROM ipx_point p JOIN zigbee_valeur v ON p.chemin = v.ieee || '/' || v.cle \
             WHERE p.hote='zigbee' AND p.actif=1 AND p.salle_id=? \
               AND p.role IN ('temperature','humidite','luminosite')",
        )
        .bind(id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .and_then(|(t_ts, t, h, lux)| {
            t_ts.filter(|_| t.is_some() || h.is_some())
                .map(|t_ts| (t_ts, t, h, lux, None, 1))
        }),
    };
    let alertes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ambiance_alerte WHERE salle_id=? AND fin IS NULL",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    let lampes: Vec<(String, Option<i64>)> = sqlx::query_as(
        "SELECT p.nom, e.allume FROM ipx_point p LEFT JOIN eclairage_etat e ON e.point_id=p.id \
         WHERE p.salle_id=? AND p.role='eclairage' AND p.actif=1 ORDER BY p.nom",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let lavage: Option<String> = ligne.try_get("dernier_lavage").ok().flatten();
    let jours_lavage = lavage.as_deref().and_then(|d| {
        NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .ok()
            .map(|d| (Local::now().date_naive() - d).num_days())
    });
    let type_salle: String = ligne.try_get("type").unwrap_or_default();
    Ok(json!({
        "id": id,
        "nom": ligne.try_get::<String, _>("nom").unwrap_or_default(),
        "type": type_salle,
        "site": ligne.try_get::<Option<String>, _>("site").ok().flatten(),
        "bande": bande_en_place(pool, id).await?,
        "truies": truies,
        "porcs": porcs,
        "places": (places > 0).then_some(places),
        "cases": cases.len(),
        "ambiance": etat.map(|(t_ts, t, h, lux, consigne, sonde_ok)| json!({
            "t": t, "h": h, "lux": lux, "consigne": consigne,
            "sonde_ok": sonde_ok != 0, "perime": ts - t_ts > ETAT_PERIME_S,
            "mesure_le": quand(t_ts, ts), "vieille": ts - t_ts > 3600,
        })),
        "alertes": alertes,
        "lampes": lampes.iter().map(|(nom, allume)| json!({"nom": nom, "allume": allume.map(|a| a != 0)})).collect::<Vec<_>>(),
        "lavage": lavage,
        "jours_lavage": jours_lavage,
    }))
}

/// Stade d'une salle, dans l'ordre du parcours des animaux : rang et titre.
/// Le type de la salle décide ; le nom départage (une « Post-Engraissement »
/// notée de type engraissement reste à part).
pub(super) fn stade(type_salle: &str, nom: &str) -> (u8, &'static str) {
    let t = format!("{type_salle} {nom}")
        .to_lowercase()
        .replace(['_', '-'], " ")
        .replace('é', "e")
        .replace('è', "e");
    if t.contains("verrat") {
        (1, "Verraterie")
    } else if t.contains("gestant") || t.contains("gestation") {
        (2, "Gestante")
    } else if t.contains("mater") {
        (3, "Maternité")
    } else if t.contains("post engr") {
        (5, "Post-engraissement")
    } else if t.contains("post sevr") || t.contains("sevrage") {
        (4, "Post-sevrage")
    } else if t.contains("engr") {
        (6, "Engraissement")
    } else {
        (7, "Autres")
    }
}

/// Tri « naturel » : Engraissement 2 avant Engraissement 10.
fn cle_naturelle(nom: &str) -> Vec<(String, u64)> {
    let mut sortie = Vec::new();
    let mut texte = String::new();
    let mut nombre = String::new();
    for c in nom.to_lowercase().chars() {
        if c.is_ascii_digit() {
            nombre.push(c);
        } else {
            if !nombre.is_empty() {
                sortie.push((std::mem::take(&mut texte), nombre.parse().unwrap_or(0)));
                nombre.clear();
            }
            texte.push(c);
        }
    }
    sortie.push((texte, nombre.parse().unwrap_or(0)));
    sortie
}

const SALLES_SQL: &str = "SELECT s.id, s.nom, COALESCE(s.type,'') AS type, s.dernier_lavage, s.ordre, \
     s.longueur, s.largeur, s.hauteur, s.mode_chauffage, si.nom AS site \
     FROM salle s LEFT JOIN site si ON si.id=s.site_id";

pub(super) async fn liste(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
) -> AppResult<Html<String>> {
    let lignes = sqlx::query(&format!("{SALLES_SQL} ORDER BY si.nom, s.ordre, s.nom"))
        .fetch_all(&state.pool)
        .await?;
    let incoherentes = super::cases_incoherentes(&state.pool).await?;
    let mut salles = Vec::with_capacity(lignes.len());
    for ligne in &lignes {
        let mut salle = resume(&state.pool, ligne).await?;
        let id_salle = salle.get("id").and_then(Value::as_i64).unwrap_or(0);
        let nb = incoherentes.iter().filter(|c| c.1 == id_salle).count();
        if let Some(objet) = salle.as_object_mut() {
            objet.insert("cases_incoherentes".into(), json!(nb));
        }
        let type_salle = salle.get("type").and_then(Value::as_str).unwrap_or("").to_string();
        let nom = salle.get("nom").and_then(Value::as_str).unwrap_or("").to_string();
        let (rang, titre) = stade(&type_salle, &nom);
        let ordre: i64 = ligne.try_get("ordre").unwrap_or(0);
        if let Some(objet) = salle.as_object_mut() {
            objet.insert("stade".into(), json!(titre));
        }
        // Dans un stade : site par site, puis l'ordre de Structure et le nom.
        let site = salle.get("site").and_then(Value::as_str).unwrap_or("").to_lowercase();
        salles.push(((rang, site, ordre, cle_naturelle(&nom)), salle));
    }
    // Toujours le même ordre : le parcours des animaux, de la verraterie à
    // l'engraissement ; dans un stade, par site, ordre de Structure puis nom.
    salles.sort_by(|a, b| a.0.cmp(&b.0));
    let salles: Vec<Value> = salles.into_iter().map(|(_, s)| s).collect();
    let mut contexte = context(&session);
    contexte.insert("salles".into(), Value::Array(salles));
    contexte.insert("departs".into(), Value::Array(super::departs::a_venir(&state.pool).await?));
    contexte.insert("quai".into(), Value::Array(super::departs::quai_en_attente(&state.pool).await?));
    render(&state, "salles.html", Value::Object(contexte))
}

pub(super) async fn fiche(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
) -> AppResult<Html<String>> {
    let pool = &state.pool;
    let ligne = sqlx::query(&format!("{SALLES_SQL} WHERE s.id=?"))
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(AppError::NotFound)?;
    let mut salle = resume(pool, &ligne).await?;
    let ts = maintenant();

    // Dimensions : surface et volume quand elles sont renseignées.
    let longueur: Option<f64> = ligne.try_get("longueur").ok().flatten();
    let largeur: Option<f64> = ligne.try_get("largeur").ok().flatten();
    let hauteur: Option<f64> = ligne.try_get("hauteur").ok().flatten();
    let surface = longueur.zip(largeur).map(|(l, w)| l * w);
    let volume = surface.zip(hauteur).map(|(s, h)| s * h);

    // Cases : effectif (dernier inventaire + mouvements) et truies logées.
    let cases_lignes: Vec<(i64, String, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT id, nom, nb_max_porcs, nb_max_truies FROM casesalle WHERE salle_id=? ORDER BY nom",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let incoherentes = super::cases_incoherentes(pool).await?;
    let mut cases = Vec::new();
    for (case_id, nom, max_porcs, max_truies) in cases_lignes {
        let truies = generic_rows_bind(
            pool,
            "SELECT id, num_travail, statut, bande_code FROM truie WHERE reformee=0 AND case_id=? ORDER BY num_travail",
            case_id,
        )
        .await?;
        cases.push(json!({
            "incoherente": incoherentes.iter().any(|c| c.0 == case_id),
            "id": case_id,
            "nom": nom,
            "porcs": case_pig_count(pool, case_id).await?.max(0),
            "max_porcs": max_porcs,
            "max_truies": max_truies,
            "truies": truies,
        }));
    }
    // Truies de la salle sans case précise.
    let truies_sans_case = generic_rows_bind(
        pool,
        "SELECT id, num_travail, statut, bande_code FROM truie \
         WHERE reformee=0 AND case_id IS NULL AND salle_id=? ORDER BY num_travail",
        id,
    )
    .await?;

    // Ambiance : réglages, 14 derniers jours, alertes.
    let reglage = sqlx::query(
        "SELECT a.actif, a.pilote, a.t_alerte_min, a.t_alerte_max, a.h_alerte_max, c.nom AS courbe \
         FROM ambiance_salle a LEFT JOIN ambiance_courbe c ON c.id=a.courbe_id WHERE a.salle_id=?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .map(|r| {
        json!({
            "actif": r.try_get::<i64, _>("actif").unwrap_or(0) != 0,
            "pilote": r.try_get::<i64, _>("pilote").unwrap_or(0) != 0,
            "t_alerte_min": r.try_get::<Option<f64>, _>("t_alerte_min").ok().flatten(),
            "t_alerte_max": r.try_get::<Option<f64>, _>("t_alerte_max").ok().flatten(),
            "h_alerte_max": r.try_get::<Option<f64>, _>("h_alerte_max").ok().flatten(),
            "courbe": r.try_get::<Option<String>, _>("courbe").ok().flatten(),
        })
    });
    let jours: Vec<(String, String, Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT jour, grandeur, moy, mini, maxi FROM ambiance_jour \
         WHERE salle_id=? AND grandeur IN ('t','h','lux') AND jour >= date('now','-13 day') ORDER BY jour",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    // Salle suivie seulement par des sondes Zigbee : même frise, calculée
    // sur leur historique (en dixièmes, comme les agrégats d'ambiance).
    let jours = if jours.is_empty() {
        sqlx::query_as::<_, (String, String, Option<i64>, Option<i64>, Option<i64>)>(
            "SELECT date(m.ts,'unixepoch','localtime') AS jour, \
                    CASE p.role WHEN 'temperature' THEN 't' WHEN 'humidite' THEN 'h' ELSE 'lux' END, \
                    CAST(ROUND(AVG(m.valeur) * 10) AS INTEGER), CAST(ROUND(MIN(m.valeur) * 10) AS INTEGER), \
                    CAST(ROUND(MAX(m.valeur) * 10) AS INTEGER) \
             FROM ipx_point p JOIN zigbee_mesure m ON p.chemin = m.ieee || '/' || m.cle \
             WHERE p.hote='zigbee' AND p.salle_id=? AND p.role IN ('temperature','humidite','luminosite') \
               AND m.ts >= strftime('%s','now','-13 day','start of day') \
             GROUP BY 1, 2 ORDER BY 1",
        )
        .bind(id)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
    } else {
        jours
    };
    let mut historique: Vec<Value> = Vec::new();
    for (jour, grandeur, moy, mini, maxi) in jours {
        let position = match historique.iter().position(|j| j.get("jour").and_then(Value::as_str) == Some(jour.as_str())) {
            Some(p) => p,
            None => {
                historique.push(json!({"jour": jour}));
                historique.len() - 1
            }
        };
        if let Some(objet) = historique[position].as_object_mut() {
            objet.insert(grandeur.clone(), json!({"moy": dixiemes(moy), "mini": dixiemes(mini), "maxi": dixiemes(maxi)}));
        }
    }
    // Bornes de la frise de température, pour dessiner les barres.
    let (t_bas, t_haut) = historique
        .iter()
        .filter_map(|j| j.get("t"))
        .fold((f64::MAX, f64::MIN), |(bas, haut), t| {
            (
                t.get("mini").and_then(Value::as_f64).map_or(bas, |v| bas.min(v)),
                t.get("maxi").and_then(Value::as_f64).map_or(haut, |v| haut.max(v)),
            )
        });
    let echelle = if t_haut > t_bas { Some((t_bas.floor() - 1.0, t_haut.ceil() + 1.0)) } else { None };
    if let Some((bas, haut)) = echelle {
        for jour in &mut historique {
            let Some(t) = jour.get("t").cloned() else { continue };
            let pct = |v: Option<f64>| v.map(|v| ((v - bas) / (haut - bas) * 100.0).clamp(0.0, 100.0));
            let mini = pct(t.get("mini").and_then(Value::as_f64));
            let maxi = pct(t.get("maxi").and_then(Value::as_f64));
            let moy = pct(t.get("moy").and_then(Value::as_f64));
            if let (Some(mini), Some(maxi), Some(objet)) = (mini, maxi, jour.as_object_mut()) {
                objet.insert("barre".into(), json!({"bas": mini, "haut": (maxi - mini).max(1.0), "moy": moy}));
            }
        }
    }
    let alertes = generic_rows_bind(
        pool,
        "SELECT type, gravite, debut, fin, message FROM ambiance_alerte WHERE salle_id=? \
         ORDER BY fin IS NULL DESC, debut DESC LIMIT 8",
        id,
    )
    .await?;

    // Équipements reliés : sondes et sorties de l'automate, lampes et
    // programmes, ventilateurs.
    let mut points = generic_rows_bind(
        pool,
        "SELECT p.id, p.nom, p.role, p.unite, p.pilotable, p.hote, p.chemin, p.coefficient, p.decalage FROM ipx_point p \
         WHERE p.salle_id=? AND p.actif=1 ORDER BY p.role, p.nom",
        id,
    )
    .await?;
    let maintenant_ts = chrono::Utc::now().timestamp();
    for p in points.iter_mut() {
        if p["hote"].as_str() != Some(super::zigbee::HOTE) {
            continue;
        }
        let Some(chemin) = p["chemin"].as_str().map(str::to_string) else { continue };
        if let Some((valeur, t)) = zigbee_valeur_ts(pool, &chemin).await {
            let coef = p["coefficient"].as_f64().unwrap_or(1.0);
            let dec = p["decalage"].as_f64().unwrap_or(0.0);
            p["valeur"] = json!(valeur.map(|v| ((v * coef + dec) * 10.0).round() / 10.0));
            p["mesure_le"] = json!(quand(t, maintenant_ts));
            p["vieille"] = json!(maintenant_ts - t > 3600);
        } else {
            p["mesure_le"] = json!("jamais reçue");
            p["vieille"] = json!(true);
        }
    }
    let lampes = sqlx::query(
        "SELECT p.id, p.nom, e.allume, e.raison, ep.nom AS programme, ep.id AS programme_id, ep.essai \
         FROM ipx_point p LEFT JOIN eclairage_etat e ON e.point_id=p.id \
         LEFT JOIN eclairage_sortie es ON es.point_id=p.id \
         LEFT JOIN eclairage_programme ep ON ep.id=es.programme_id \
         WHERE p.salle_id=? AND p.role='eclairage' AND p.actif=1 ORDER BY p.nom",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|l| {
        json!({
            "id": l.try_get::<i64, _>("id").unwrap_or(0),
            "nom": l.try_get::<String, _>("nom").unwrap_or_default(),
            "allume": l.try_get::<Option<i64>, _>("allume").ok().flatten().map(|a| a != 0),
            "raison": l.try_get::<Option<String>, _>("raison").ok().flatten(),
            "programme": l.try_get::<Option<String>, _>("programme").ok().flatten(),
            "programme_id": l.try_get::<Option<i64>, _>("programme_id").ok().flatten(),
            "essai": l.try_get::<Option<i64>, _>("essai").ok().flatten().map(|e| e != 0),
        })
    })
    .collect::<Vec<_>>();
    let ventilateurs = generic_rows_bind(
        pool,
        "SELECT nom, position, fonction, voie, diametre_mm, debit_m3h FROM ventilateur_salle \
         WHERE salle_id=? AND actif=1 ORDER BY nom",
        id,
    )
    .await?;
    let debit: f64 = ventilateurs
        .iter()
        .filter_map(|v| v.get("debit_m3h").and_then(Value::as_f64))
        .sum();
    let renouvellement = volume.filter(|v| *v > 0.0 && debit > 0.0).map(|v| debit / v);

    // Santé et mouvements des 30 derniers jours.
    let morts = generic_rows_bind(
        pool,
        "SELECT d.date, d.nombre, d.stade, d.cause, c.nom AS case_nom, d.bande_code \
         FROM declarationmort d JOIN casesalle c ON c.id=d.case_id \
         WHERE c.salle_id=? AND COALESCE(d.date, date(d.horodatage)) >= date('now','-30 day') \
         ORDER BY COALESCE(d.date, d.horodatage) DESC LIMIT 20",
        id,
    )
    .await?;
    let total_morts: i64 = morts.iter().filter_map(|m| m.get("nombre").and_then(Value::as_i64)).sum();
    let observations = generic_rows_bind(
        pool,
        "SELECT o.date, o.categorie, o.gravite, o.nombre, o.note, b.code AS bande \
         FROM observation_comportement o LEFT JOIN bande b ON b.id=o.bande_id \
         WHERE (o.salle_id=?1 OR o.case_id IN (SELECT id FROM casesalle WHERE salle_id=?1)) \
           AND o.date >= date('now','-30 day') ORDER BY o.date DESC LIMIT 10",
        id,
    )
    .await
    .unwrap_or_default();
    let mouvements = sqlx::query(
        "SELECT t.date, t.espece, t.nombre, t.salle_source_id, t.salle_dest_id, b.code AS bande, \
                tr.num_travail, ss.nom AS source, sd.nom AS dest \
         FROM transfert t LEFT JOIN bande b ON b.id=t.bande_id LEFT JOIN truie tr ON tr.id=t.truie_id \
         LEFT JOIN salle ss ON ss.id=t.salle_source_id LEFT JOIN salle sd ON sd.id=t.salle_dest_id \
         WHERE t.salle_source_id=?1 OR t.salle_dest_id=?1 \
            OR t.case_source_id IN (SELECT id FROM casesalle WHERE salle_id=?1) \
            OR t.case_dest_id IN (SELECT id FROM casesalle WHERE salle_id=?1) \
         ORDER BY t.date DESC, t.id DESC LIMIT 12",
    )
    .bind(id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|m| {
        let entree = m.try_get::<Option<i64>, _>("salle_dest_id").ok().flatten() == Some(id)
            || m.try_get::<Option<i64>, _>("salle_source_id").ok().flatten() != Some(id);
        json!({
            "date": m.try_get::<Option<String>, _>("date").ok().flatten(),
            "entree": entree,
            "espece": m.try_get::<String, _>("espece").unwrap_or_default(),
            "nombre": m.try_get::<Option<i64>, _>("nombre").ok().flatten(),
            "bande": m.try_get::<Option<String>, _>("bande").ok().flatten(),
            "truie": m.try_get::<Option<String>, _>("num_travail").ok().flatten(),
            "autre": if entree {
                m.try_get::<Option<String>, _>("source").ok().flatten()
            } else {
                m.try_get::<Option<String>, _>("dest").ok().flatten()
            },
        })
    })
    .collect::<Vec<_>>();
    // Ventilation : réglage réel de chaque sortie de ventilation de la salle,
    // et consigne calculée par la régulation.
    let ventil_etat: Option<(Option<f64>, String)> =
        sqlx::query_as("SELECT ventil, mode FROM ambiance_etat WHERE salle_id=?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    let sorties_ventil: Vec<(String, String, String, Option<String>, f64, f64, Option<String>)> = sqlx::query_as(
        "SELECT nom, hote, chemin, formule, coefficient, decalage, unite FROM ipx_point \
         WHERE salle_id=? AND role='ventilation' AND actif=1 ORDER BY nom",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut ventilation = Vec::new();
    for (nom, hote, chemin, formule, coefficient, decalage, unite) in sorties_ventil {
        let valeur = super::ambiance::lire_point(pool, &hote, &chemin, formule.as_deref(), coefficient, decalage).await;
        let zigbee = hote == super::zigbee::HOTE;
        // Heure de la dernière vitesse connue (Zigbee) ; un variateur Tuya
        // n'envoie sa vitesse qu'à un changement, pas sur demande.
        let (mesure_le, vieille) = if zigbee {
            match zigbee_valeur_ts(pool, &chemin).await {
                Some((_, t)) => (Some(quand(t, maintenant_ts)), maintenant_ts - t > 3600),
                None => (None, true),
            }
        } else {
            (valeur.map(|_| "lue à l'instant".to_string()), false)
        };
        // Etat marche/arrêt du variateur quand il l'a envoyé.
        let marche = if zigbee {
            match chemin.split_once('/') {
                Some((ieee, _)) => sqlx::query_scalar::<_, Option<String>>("SELECT texte FROM zigbee_valeur WHERE ieee=? AND cle='state'")
                    .bind(ieee)
                    .fetch_optional(pool)
                    .await
                    .ok()
                    .flatten()
                    .flatten(),
                None => None,
            }
        } else {
            None
        };
        ventilation.push(json!({
            "nom": nom,
            "valeur": valeur.map(|v| (v * 10.0).round() / 10.0),
            "unite": unite.unwrap_or_default(),
            "zigbee": zigbee,
            "mesure_le": mesure_le,
            "vieille": vieille,
            "arret": marche.as_deref() == Some("OFF"),
        }));
    }
    let reglages = sqlx::query(
        "SELECT actif, pilote, courbe_id, t_cible_fixe, plage, ventil_min, ventil_max, \
                t_alerte_min, t_alerte_max, h_alerte_max, ventil_vide FROM ambiance_salle WHERE salle_id=?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .map(|r| {
        json!({
            "actif": r.try_get::<i64, _>("actif").unwrap_or(0) != 0,
            "pilote": r.try_get::<i64, _>("pilote").unwrap_or(0) != 0,
            "courbe_id": r.try_get::<Option<i64>, _>("courbe_id").ok().flatten(),
            "t_cible_fixe": r.try_get::<f64, _>("t_cible_fixe").ok(),
            "plage": r.try_get::<f64, _>("plage").ok(),
            "ventil_min": r.try_get::<f64, _>("ventil_min").ok(),
            "ventil_max": r.try_get::<f64, _>("ventil_max").ok(),
            "t_alerte_min": r.try_get::<Option<f64>, _>("t_alerte_min").ok().flatten(),
            "t_alerte_max": r.try_get::<Option<f64>, _>("t_alerte_max").ok().flatten(),
            "h_alerte_max": r.try_get::<Option<f64>, _>("h_alerte_max").ok().flatten(),
            "ventil_vide": r.try_get::<Option<f64>, _>("ventil_vide").ok().flatten(),
        })
    });
    let courbes = generic_rows(pool, "SELECT id, nom FROM ambiance_courbe ORDER BY nom")
        .await
        .unwrap_or_default();

    let preconditionnement = generic_rows_bind(
        pool,
        "SELECT p.arrivee_prevue, p.heures_avant, b.code AS bande FROM ambiance_preconditionnement p \
         LEFT JOIN bande b ON b.id=p.bande_id WHERE p.salle_id=? ORDER BY p.arrivee_prevue LIMIT 3",
        id,
    )
    .await
    .unwrap_or_default();

    if let Some(objet) = salle.as_object_mut() {
        objet.insert("longueur".into(), json!(longueur));
        objet.insert("largeur".into(), json!(largeur));
        objet.insert("hauteur".into(), json!(hauteur));
        objet.insert("surface".into(), json!(surface.map(nombre_court)));
        objet.insert("volume".into(), json!(volume.map(|v| v.round() as i64)));
        objet.insert(
            "chauffage".into(),
            json!(ligne.try_get::<Option<String>, _>("mode_chauffage").ok().flatten()),
        );
    }

    let mut contexte = context(&session);
    contexte.insert("salle".into(), salle);
    contexte.insert("cases_detail".into(), Value::Array(cases));
    contexte.insert("truies_sans_case".into(), Value::Array(truies_sans_case));
    contexte.insert("reglage".into(), json!(reglage));
    contexte.insert("historique".into(), Value::Array(historique));
    contexte.insert("echelle".into(), json!(echelle.map(|(b, h)| json!({"bas": b, "haut": h}))));
    contexte.insert("alertes".into(), Value::Array(alertes));
    contexte.insert("points".into(), Value::Array(points));
    contexte.insert("lampes_detail".into(), Value::Array(lampes));
    contexte.insert("ventilateurs".into(), Value::Array(ventilateurs));
    contexte.insert("debit".into(), json!((debit > 0.0).then_some(debit)));
    contexte.insert("renouvellement".into(), json!(renouvellement.map(nombre_court)));
    contexte.insert("morts".into(), Value::Array(morts));
    contexte.insert("total_morts".into(), json!(total_morts));
    contexte.insert("observations".into(), Value::Array(observations));
    contexte.insert("mouvements".into(), Value::Array(mouvements));
    contexte.insert("preconditionnement".into(), Value::Array(preconditionnement));
    contexte.insert(
        "ventil_etat".into(),
        json!(ventil_etat.map(|(v, mode)| json!({"ventil": v, "mode": mode}))),
    );
    contexte.insert("ventilation".into(), Value::Array(ventilation));
    contexte.insert("reglages".into(), json!(reglages));
    contexte.insert("courbes".into(), Value::Array(courbes));
    // Caméras : table absente sur une base qui n'a pas encore migré.
    let cameras = generic_rows_bind(pool, "SELECT id, nom FROM camera WHERE actif=1 AND salle_id=? ORDER BY nom", id)
        .await
        .unwrap_or_default();
    contexte.insert("cameras".into(), Value::Array(cameras));
    contexte.insert("maj".into(), json!(ts));
    render(&state, "salle_fiche.html", Value::Object(contexte))
}

/// Un nombre à une décimale, sans « ,0 » quand il est entier.
fn nombre_court(valeur: f64) -> Value {
    let arrondi = (valeur * 10.0).round() / 10.0;
    if arrondi.fract() == 0.0 {
        json!(arrondi as i64)
    } else {
        json!(arrondi)
    }
}

/// Réglages d'ambiance de la salle, depuis sa fiche : consigne, plage,
/// ventilation mini/maxi et seuils d'alerte. Les autres réglages (sondes,
/// sortie pilotée, mise en régulation) restent sur la page EO_Awel ; une
/// salle sans réglage d'ambiance en reçoit un, inactif.
pub(super) async fn reglages(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let existe: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM salle WHERE id=?")
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    if existe == 0 {
        return Err(AppError::NotFound);
    }
    let ventil_min = form_f64(&form, "ventil_min").unwrap_or(20.0).clamp(0.0, 100.0);
    let ventil_max = form_f64(&form, "ventil_max").unwrap_or(100.0).clamp(ventil_min, 100.0);
    let t_alerte_min = form_f64(&form, "t_alerte_min");
    let t_alerte_max = form_f64(&form, "t_alerte_max");
    if let (Some(bas), Some(haut)) = (t_alerte_min, t_alerte_max) {
        if bas >= haut {
            return Err(AppError::Invalid(
                "L'alerte basse doit être inférieure à l'alerte haute".into(),
            ));
        }
    }
    sqlx::query(
        "INSERT INTO ambiance_salle(salle_id,courbe_id,t_cible_fixe,plage,ventil_min,ventil_max,\
         t_alerte_min,t_alerte_max,h_alerte_max,ventil_vide) VALUES(?,?,?,?,?,?,?,?,?,?) \
         ON CONFLICT(salle_id) DO UPDATE SET courbe_id=excluded.courbe_id, t_cible_fixe=excluded.t_cible_fixe, \
         plage=excluded.plage, ventil_min=excluded.ventil_min, ventil_max=excluded.ventil_max, \
         t_alerte_min=excluded.t_alerte_min, t_alerte_max=excluded.t_alerte_max, h_alerte_max=excluded.h_alerte_max, \
         ventil_vide=excluded.ventil_vide",
    )
    .bind(id)
    .bind(form_i64(&form, "courbe_id"))
    .bind(form_f64(&form, "t_cible_fixe").unwrap_or(22.0).clamp(5.0, 40.0))
    .bind(form_f64(&form, "plage").unwrap_or(3.0).clamp(0.5, 10.0))
    .bind(ventil_min)
    .bind(ventil_max)
    .bind(t_alerte_min)
    .bind(t_alerte_max)
    .bind(form_f64(&form, "h_alerte_max"))
    .bind(form_f64(&form, "ventil_vide").map(|v| v.clamp(0.0, 100.0)))
    .execute(&state.pool)
    .await?;
    Ok(Redirect::to(&format!("/salles/{id}#reglages")).into_response())
}

/// `generic_rows` avec un seul paramètre lié.
async fn generic_rows_bind(pool: &SqlitePool, sql: &str, valeur: i64) -> AppResult<Vec<Value>> {
    let lignes = sqlx::query(sql).bind(valeur).fetch_all(pool).await?;
    Ok(lignes.iter().map(ligne_en_json).collect())
}

fn ligne_en_json(ligne: &sqlx::sqlite::SqliteRow) -> Value {
    use sqlx::{Column, TypeInfo, ValueRef};
    let mut objet = Map::new();
    for (index, colonne) in ligne.columns().iter().enumerate() {
        let brut = ligne.try_get_raw(index);
        let valeur = match brut {
            Ok(brut) if brut.is_null() => Value::Null,
            Ok(brut) => match brut.type_info().name() {
                "INTEGER" | "BOOLEAN" => ligne.try_get::<i64, _>(index).map(Value::from).unwrap_or(Value::Null),
                "REAL" | "FLOAT" => ligne.try_get::<f64, _>(index).map(Value::from).unwrap_or(Value::Null),
                _ => ligne.try_get::<String, _>(index).map(Value::from).unwrap_or(Value::Null),
            },
            Err(_) => Value::Null,
        };
        objet.insert(colonne.name().to_string(), valeur);
    }
    Value::Object(objet)
}

#[cfg(test)]
pub(super) async fn fiche_pour_test(state: &AppState, id: i64) -> AppResult<String> {
    Ok(fiche(State(state.clone()), Extension(super::documents::tests::session()), Path(id)).await?.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn la_fiche_reunit_ce_qui_est_relie_a_la_salle() -> anyhow::Result<()> {
        use super::super::documents::tests::{session, state};
        let state = state().await;
        for sql in [
            include_str!("../../migrations/0007_ipx_points.sql"),
            include_str!("../../migrations/0011_ventilation_salle.sql"),
            include_str!("../../migrations/0012_observation_comportement.sql"),
            include_str!("../../migrations/0016_eclairage.sql"),
            include_str!("../../migrations/0025_depart_prevu.sql"),
        ] {
            sqlx::raw_sql(sql).execute(&state.pool).await?;
        }
        // Colonnes ajoutées au démarrage en production (ensure_column).
        sqlx::raw_sql(
            "ALTER TABLE salle ADD COLUMN longueur REAL; ALTER TABLE salle ADD COLUMN largeur REAL; \
             ALTER TABLE salle ADD COLUMN hauteur REAL; ALTER TABLE salle ADD COLUMN mode_chauffage TEXT; \
             ALTER TABLE ipx_point ADD COLUMN formule TEXT; ALTER TABLE ambiance_salle ADD COLUMN ventil_vide REAL;",
        )
        .execute(&state.pool)
        .await?;
        sqlx::raw_sql(
            "INSERT INTO site(id,code,nom) VALUES(1,'S','Site'); \
             INSERT INTO salle(id,site_id,nom,type,nb_cases,longueur,largeur,hauteur) VALUES(1,1,'Verraterie','verraterie',1,10,5,3); \
             INSERT INTO casesalle(id,salle_id,nom,nb_max_porcs) VALUES(1,1,'C1',20); \
             INSERT INTO truie(id,num_travail,statut,rang,reformee,mere_cochette,salle_id,case_id) VALUES(1,'T42','vide',1,0,0,1,1); \
             INSERT INTO ipx_point(hote,chemin,chemin_ecriture,nom,role,salle_id,pilotable) VALUES('h','api/core/io/65536','api/core/io/65536','Néon verraterie','eclairage',1,1); \
             INSERT INTO ventilateur_salle(salle_id,nom,debit_m3h) VALUES(1,'Extracteur',3000);",
        )
        .execute(&state.pool)
        .await?;

        let liste = liste(State(state.clone()), Extension(session())).await?.0;
        assert!(liste.contains("Verraterie") && liste.contains("Néon verraterie"), "{liste}");

        let fiche = fiche(State(state.clone()), Extension(session()), Path(1)).await?.0;
        assert!(fiche.contains("T42"), "truie logée");
        assert!(fiche.contains("Néon verraterie"), "lampe reliée");
        assert!(fiche.contains("Extracteur") && fiche.contains("20"), "ventilateur et renouvellement (3000/150)");
        assert!(fiche.contains("150 m³"), "volume");

        // Réglages depuis la fiche : créés pour une salle qui n'en avait pas.
        let mut f: HashMap<String, String> = [("csrf_token", "test"), ("t_cible_fixe", "24"), ("ventil_min", "30"),
            ("ventil_max", "90"), ("t_alerte_min", "18"), ("t_alerte_max", "32")]
            .iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        reglages(State(state.clone()), Extension(session()), Path(1), Form(f.clone())).await?;
        let (t, vmin, vmax): (f64, f64, f64) = sqlx::query_as("SELECT t_cible_fixe, ventil_min, ventil_max FROM ambiance_salle WHERE salle_id=1")
            .fetch_one(&state.pool).await?;
        assert_eq!((t, vmin, vmax), (24.0, 30.0, 90.0));
        assert!(fiche_texte(&state).await?.contains("Enregistrer les réglages"));
        f.insert("t_alerte_min".into(), "35".into());
        assert!(reglages(State(state.clone()), Extension(session()), Path(1), Form(f)).await.is_err(), "seuils inversés refusés");

        // Retirer la lampe la fait disparaître de la fiche, sans autre réglage.
        sqlx::query("DELETE FROM ipx_point").execute(&state.pool).await?;
        let fiche = fiche_texte(&state).await?;
        assert!(!fiche.contains("Néon verraterie"));
        Ok(())
    }

    /// Toutes les fiches sur une copie de la base réelle :
    /// `EO_TEST_BASE=/var/tmp/copie/elevage.db cargo test -- --ignored toutes_les_fiches`.
    #[tokio::test]
    #[ignore]
    async fn toutes_les_fiches_sur_copie_de_la_base_reelle() -> anyhow::Result<()> {
        let chemin = std::env::var("EO_TEST_BASE")?;
        let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&chemin).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await?;
        crate::db::init(&pool).await?;
        let state = AppState::new(
            Config { bind: "127.0.0.1:8080".parse()?, db_path: chemin.into(), secure_cookies: false },
            pool,
            crate::templates::build()?,
        );
        let session = super::super::documents::tests::session;
        let liste_html = liste(State(state.clone()), Extension(session())).await?.0;
        let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM salle").fetch_all(&state.pool).await?;
        for id in ids {
            let html = fiche(State(state.clone()), Extension(session()), Path(id)).await?.0;
            println!("salle {id} : {} octets", html.len());
        }
        println!("liste : {} octets", liste_html.len());
        Ok(())
    }

    #[test]
    fn les_salles_se_rangent_dans_l_ordre_du_parcours() {
        assert_eq!(stade("verraterie", "Verraterie").0, 1);
        assert_eq!(stade("gestante", "Gestante").0, 2);
        assert_eq!(stade("maternite", "Mater 3").0, 3);
        assert_eq!(stade("Post-sevrage", "Post-sevrage 2").0, 4);
        assert_eq!(stade("post_sevrage", "Post-sevrage 1").0, 4);
        assert_eq!(stade("engraissement", "Post-Engraissement").0, 5);
        assert_eq!(stade("Engraissement", "Engraissement 4").0, 6);
        assert_eq!(stade("", "Quai").0, 7);
        assert!(cle_naturelle("Engraissement 2") < cle_naturelle("Engraissement 10"));
    }

    #[tokio::test]
    async fn la_bande_suit_les_porcs_d_une_case_a_l_autre() -> anyhow::Result<()> {
        let state = super::super::documents::tests::state().await;
        sqlx::raw_sql(
            "INSERT INTO site(id,code) VALUES(1,'S'); \
             INSERT INTO salle(id,site_id,nom,nb_cases) VALUES(1,1,'PS 1',1),(2,1,'Post-Engraissement',1); \
             INSERT INTO casesalle(id,salle_id,nom) VALUES(1,1,'Case 1'),(2,2,'PE Paille'); \
             INSERT INTO bande(id,code,active) VALUES(1,'B2.12',1); \
             INSERT INTO transfert(date,espece,bande_id,case_dest_id,salle_dest_id,nombre) VALUES('2026-08-05','porc',1,1,1,113); \
             INSERT INTO transfert(date,espece,case_source_id,salle_source_id,case_dest_id,salle_dest_id,nombre) VALUES('2026-09-01','porc',1,1,2,2,113);",
        )
        .execute(&state.pool)
        .await?;
        let bande = bande_en_place(&state.pool, 2).await?.expect("bande retrouvée");
        assert_eq!(bande.get("code").and_then(Value::as_str), Some("B2.12"));
        assert_eq!(super::super::bande_de_la_case(&state.pool, 1, "2026-09-01").await?, Some(1));
        Ok(())
    }

    async fn fiche_texte(state: &AppState) -> AppResult<String> {
        Ok(fiche(State(state.clone()), Extension(super::super::documents::tests::session()), Path(1)).await?.0)
    }
}
