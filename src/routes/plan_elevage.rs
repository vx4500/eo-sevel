//! Plan de l'élevage en un coup d'œil (/ambiance/plan).
//!
//! Une image du plan (plan_reglage, 0015) sur laquelle on pose des repères
//! (plan_repere, 0029) : carte de salle, température, hygrométrie, % de
//! ventilation, mode de ventilation, éclairage. Un clic sur un repère ouvre
//! les réglages courants (consigne, ventilation mini/maxi, forçage de la
//! lumière) sans passer par la page complète de chaque module.
//!
//! Les valeurs viennent des mêmes sources que /ambiance et
//! /ambiance/eclairage : rien n'est lu directement sur l'automate ici.

use super::*;

const TYPES: [&str; 10] = [
    "case",
    "zigbee",
    "relais",
    "salle",
    "temperature",
    "humidite",
    "ventil",
    "mode",
    "eclairage",
    "texte",
];

async fn reperes(pool: &SqlitePool) -> AppResult<Vec<Value>> {
    generic_rows(
        pool,
        "SELECT id,type,ref_id,libelle,x,y FROM plan_repere ORDER BY id",
    )
    .await
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
) -> AppResult<Html<String>> {
    let (image, rotation): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(image_data), COALESCE(MAX(rotation),0) FROM plan_reglage WHERE id=1",
    )
    .fetch_one(&state.pool)
    .await?;
    let salles = generic_rows(
        &state.pool,
        "SELECT s.id, s.nom FROM salle s JOIN site si ON si.id=s.site_id \
         ORDER BY si.code, s.ordre, s.nom",
    )
    .await?;
    let programmes = generic_rows(
        &state.pool,
        "SELECT id, nom FROM eclairage_programme ORDER BY nom",
    )
    .await?;
    let relais = generic_rows(
        &state.pool,
        "SELECT id, nom FROM ipx_point \
         WHERE pilotable=1 AND chemin_ecriture IS NOT NULL ORDER BY nom",
    )
    .await?;
    let zigbee = generic_rows(
        &state.pool,
        "SELECT ieee, nom, categorie FROM zigbee_appareil \
         WHERE categorie IN ('sonde','variateur','interrupteur') ORDER BY nom",
    )
    .await?;
    let cases = generic_rows(
        &state.pool,
        "SELECT c.id, c.salle_id, s.nom AS salle, c.nom, c.num_vanne FROM casesalle c \
         JOIN salle s ON s.id=c.salle_id ORDER BY s.nom, c.id",
    )
    .await?;
    let mut contexte = context(&session);
    contexte.insert("cases".into(), Value::Array(cases));
    contexte.insert("zigbee".into(), Value::Array(zigbee));
    contexte.insert("relais".into(), Value::Array(relais));
    contexte.insert("image".into(), json!(image > 0));
    contexte.insert("rotation".into(), json!(rotation));
    contexte.insert("salles".into(), Value::Array(salles));
    contexte.insert("programmes".into(), Value::Array(programmes));
    render(&state, "plan_elevage.html", Value::Object(contexte))
}

/// Tout ce que la page affiche, relu toutes les 30 secondes.
pub(super) async fn etat_json(State(state): State<AppState>) -> AppResult<Response> {
    let programmes: Vec<Value> = super::eclairage::charger_programmes(&state.pool, None)
        .await?
        .into_iter()
        .map(|p| {
            let allume = p
                .get("sorties")
                .and_then(Value::as_array)
                .map(|s| s.iter().any(|s| s.get("allume") == Some(&json!(true))))
                .unwrap_or(false);
            json!({
                "id": p.get("id"),
                "nom": p.get("nom"),
                "actif": p.get("actif"),
                "allume": allume,
                "forcage": p.get("forcage"),
            })
        })
        .collect();
    // Une salle sans mesure n'affiche rien plutôt que des zéros.
    let mut salles = super::ambiance::salles(&state.pool).await?;
    for salle in &mut salles {
        if salle.get("a_mesure") != Some(&json!(true)) {
            if let Some(objet) = salle.as_object_mut() {
                for champ in ["t", "h", "lux", "ventil", "consigne", "mode", "mesure_age_texte"] {
                    objet.insert(champ.into(), Value::Null);
                }
            }
        }
    }
    // Relais posés sur le plan : état lu sur l'automate (ou Zigbee).
    let points: Vec<(i64, String, String, String)> = sqlx::query_as(
        "SELECT p.id, p.nom, p.hote, p.chemin FROM ipx_point p \
         WHERE p.id IN (SELECT ref_id FROM plan_repere WHERE type='relais')",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut relais = Vec::with_capacity(points.len());
    for (id, nom, hote, chemin) in points {
        let valeur = super::ambiance::valeur_directe(&state.pool, &hote, &chemin).await;
        relais.push(json!({ "id": id, "nom": nom, "valeur": valeur }));
    }
    // Appareils Zigbee posés sur le plan (ieee dans `libelle`) : dernières
    // valeurs reçues de Zigbee2MQTT.
    let lignes: Vec<(String, String, String, Option<f64>, Option<String>, i64)> = sqlx::query_as(
        "SELECT a.ieee, a.nom, v.cle, v.valeur, v.texte, v.ts FROM zigbee_appareil a \
         JOIN zigbee_valeur v ON v.ieee=a.ieee \
         WHERE a.ieee IN (SELECT libelle FROM plan_repere WHERE type='zigbee') \
           AND v.cle IN ('temperature','humidity','brightness','state')",
    )
    .fetch_all(&state.pool)
    .await?;
    let instant = chrono::Utc::now().timestamp();
    let mut zigbee = serde_json::Map::new();
    for (ieee, nom, cle, valeur, texte, ts) in lignes {
        let entree = zigbee
            .entry(ieee)
            .or_insert_with(|| json!({ "nom": nom, "age_s": Value::Null }));
        let Some(objet) = entree.as_object_mut() else { continue };
        let valeur = match cle.as_str() {
            "brightness" => valeur.map(|b| json!((b / 254.0 * 100.0).round())),
            "state" => Some(json!(texte.as_deref() == Some("ON"))),
            _ => valeur.map(|v| json!(v)),
        };
        objet.insert(cle, valeur.unwrap_or(Value::Null));
        cle_mesure_age(objet, instant - ts);
    }
    // Cases posées sur le plan : nombre de porcs (inventaire + mouvements),
    // truies présentes et, en maternité, porcelets de la portée en cours.
    let case_ids: Vec<(i64, String, Option<String>)> = sqlx::query_as(
        "SELECT id, nom, num_vanne FROM casesalle \
         WHERE id IN (SELECT ref_id FROM plan_repere WHERE type='case')",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut cases = serde_json::Map::new();
    for (case_id, nom, vanne) in case_ids {
        let truies: Vec<(String, Option<i64>)> = sqlx::query_as(
            "SELECT t.num_travail, \
                    (SELECT p.presents FROM portee_effectif p WHERE p.truie_id=t.id \
                       AND p.cloturee=0 AND p.date<=date('now','localtime') \
                     ORDER BY p.date DESC, p.id DESC LIMIT 1) \
             FROM truie t WHERE t.reformee=0 AND t.case_id=? ORDER BY t.num_travail",
        )
        .bind(case_id)
        .fetch_all(&state.pool)
        .await?;
        cases.insert(
            case_id.to_string(),
            json!({
                "nom": nom,
                "vanne": vanne,
                "porcs": case_pig_count(&state.pool, case_id).await?.max(0),
                "truies": truies.into_iter().map(|(num, porcelets)| json!({
                    "num": num,
                    "porcelets": porcelets,
                })).collect::<Vec<_>>(),
            }),
        );
    }
    Ok(axum::Json(json!({
        "cases": cases,
        "zigbee": zigbee,
        "relais": relais,
        "demon": super::ambiance::etat_demon(&state.pool).await,
        "salles": salles,
        "programmes": programmes,
        "reperes": reperes(&state.pool).await?,
    }))
    .into_response())
}

/// Garde l'âge de la valeur la plus récente d'un appareil Zigbee.
fn cle_mesure_age(objet: &mut serde_json::Map<String, Value>, age: i64) -> bool {
    let plus_recent = objet
        .get("age_s")
        .and_then(Value::as_i64)
        .map_or(true, |actuel| age < actuel);
    if plus_recent {
        objet.insert("age_s".into(), json!(age));
    }
    plus_recent
}

pub(super) async fn image(State(state): State<AppState>) -> AppResult<Response> {
    let image: Option<(Vec<u8>, String)> = sqlx::query_as(
        "SELECT image_data,image_mime FROM plan_reglage \
         WHERE id=1 AND image_data IS NOT NULL AND image_mime IS NOT NULL",
    )
    .fetch_optional(&state.pool)
    .await?;
    let Some((bytes, mime)) = image else {
        return Err(AppError::NotFound);
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&mime).map_err(|_| AppError::Invalid("Image invalide".into()))?,
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok((headers, bytes).into_response())
}

pub(super) async fn image_maj(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    multipart: Multipart,
) -> AppResult<Response> {
    require_writer(&session)?;
    let (form, file, _) = parity::multipart_fields(multipart, "image").await?;
    verify_csrf(&session, &form)?;
    sqlx::query("INSERT OR IGNORE INTO plan_reglage(id) VALUES(1)")
        .execute(&state.pool)
        .await?;
    if form.contains_key("supprimer") {
        sqlx::query("UPDATE plan_reglage SET image_data=NULL,image_mime=NULL WHERE id=1")
            .execute(&state.pool)
            .await?;
    } else {
        let bytes = file.ok_or_else(|| AppError::Invalid("Image du plan obligatoire".into()))?;
        let mime = detect_image_mime(&bytes)?;
        let mut tx = state.pool.begin().await?;
        sqlx::query("UPDATE plan_reglage SET image_data=?,image_mime=? WHERE id=1")
            .bind(bytes)
            .bind(mime)
            .execute(&mut *tx)
            .await?;
        // Image recadrée dans le navigateur : les repères gardent leur place
        // sur le dessin.
        let cadre = ["crop_x", "crop_y", "crop_w", "crop_h", "orig_w", "orig_h"]
            .map(|cle| form_f64(&form, cle));
        if let [Some(cx), Some(cy), Some(cw), Some(ch), Some(ow), Some(oh)] = cadre {
            if cw > 0.0 && ch > 0.0 && ow > 0.0 && oh > 0.0 && (cw < ow || ch < oh) {
                let rotation: i64 =
                    sqlx::query_scalar("SELECT rotation FROM plan_reglage WHERE id=1")
                        .fetch_one(&mut *tx)
                        .await?;
                let reperes: Vec<(i64, f64, f64)> =
                    sqlx::query_as("SELECT id,x,y FROM plan_repere")
                        .fetch_all(&mut *tx)
                        .await?;
                for (id, x, y) in reperes {
                    let (x, y) = recadrer(x, y, rotation, [cx, cy, cw, ch, ow, oh]);
                    sqlx::query("UPDATE plan_repere SET x=?,y=? WHERE id=?")
                        .bind(x)
                        .bind(y)
                        .bind(id)
                        .execute(&mut *tx)
                        .await?;
                }
            }
        }
        tx.commit().await?;
    }
    Ok(Redirect::to("/ambiance/plan?travaux=1").into_response())
}

/// Position d'un repère (en % du plan affiché, pivoté de `rotation`) après
/// recadrage de l'image d'origine sur [x, y, largeur, hauteur] en pixels.
fn recadrer(x: f64, y: f64, rotation: i64, [cx, cy, cw, ch, ow, oh]: [f64; 6]) -> (f64, f64) {
    let quarts = rotation.rem_euclid(360) / 90;
    // Retour dans le repère de l'image non pivotée : inverse de (x,y)→(100−y,x).
    let (mut u, mut v) = (x, y);
    for _ in 0..quarts {
        (u, v) = (v, 100.0 - u);
    }
    u = ((u / 100.0 * ow - cx) / cw * 100.0).clamp(0.0, 100.0);
    v = ((v / 100.0 * oh - cy) / ch * 100.0).clamp(0.0, 100.0);
    for _ in 0..quarts {
        (u, v) = (100.0 - v, u);
    }
    (u, v)
}

/// Fait pivoter le plan d'un quart de tour dans le sens des aiguilles d'une
/// montre. Les repères tournent avec : (x, y) devient (100 − y, x).
pub(super) async fn pivoter(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("INSERT OR IGNORE INTO plan_reglage(id) VALUES(1)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE plan_reglage SET rotation=(rotation+90)%360 WHERE id=1")
        .execute(&mut *tx)
        .await?;
    // SQLite évalue les deux membres avec les anciennes valeurs de la ligne.
    sqlx::query("UPDATE plan_repere SET x=100-y, y=x")
        .execute(&mut *tx)
        .await?;
    let rotation: i64 = sqlx::query_scalar("SELECT rotation FROM plan_reglage WHERE id=1")
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(axum::Json(json!({ "rotation": rotation })).into_response())
}

/// Crée un repère (sans `id`) ou le déplace (avec `id`). Répond le repère.
pub(super) async fn repere(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let x = form_f64(&form, "x").unwrap_or(50.0).clamp(0.0, 100.0);
    let y = form_f64(&form, "y").unwrap_or(50.0).clamp(0.0, 100.0);
    let id = if let Some(id) = form_i64(&form, "id") {
        sqlx::query("UPDATE plan_repere SET x=?,y=? WHERE id=?")
            .bind(x)
            .bind(y)
            .bind(id)
            .execute(&state.pool)
            .await?;
        id
    } else {
        let type_repere = form_text(&form, "type")
            .filter(|t| TYPES.contains(&t.as_str()))
            .ok_or_else(|| AppError::Invalid("Type de repère inconnu".into()))?;
        let ref_id = form_i64(&form, "ref_id");
        let libelle = form_text(&form, "libelle");
        if type_repere == "zigbee" && libelle.is_none() {
            return Err(AppError::Invalid("Choisissez l'appareil Zigbee".into()));
        }
        if type_repere == "texte" && libelle.is_none() {
            return Err(AppError::Invalid("Écrivez le texte du repère".into()));
        }
        if !matches!(type_repere.as_str(), "texte" | "zigbee") && ref_id.is_none() {
            return Err(AppError::Invalid(
                if type_repere == "case" {
                    "Choisissez la case"
                } else if type_repere == "eclairage" {
                    "Choisissez le programme d'éclairage"
                } else if type_repere == "relais" {
                    "Choisissez le relais"
                } else {
                    "Choisissez la salle"
                }
                .into(),
            ));
        }
        sqlx::query("INSERT INTO plan_repere(type,ref_id,libelle,x,y) VALUES(?,?,?,?,?)")
            .bind(&type_repere)
            .bind(ref_id)
            .bind(libelle)
            .bind(x)
            .bind(y)
            .execute(&state.pool)
            .await?
            .last_insert_rowid()
    };
    Ok(axum::Json(json!({ "id": id })).into_response())
}

/// Pose d'un coup toutes les cases d'une salle, en grille autour du point
/// donné ; les cases déjà sur le plan ne sont pas reposées.
pub(super) async fn cases_salle(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let salle_id = form_i64(&form, "salle_id")
        .ok_or_else(|| AppError::Invalid("Choisissez la salle".into()))?;
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM casesalle WHERE salle_id=? \
           AND id NOT IN (SELECT ref_id FROM plan_repere WHERE type='case' AND ref_id IS NOT NULL) \
         ORDER BY CAST(num_vanne AS INTEGER), num_vanne, id",
    )
    .bind(salle_id)
    .fetch_all(&state.pool)
    .await?;
    if ids.is_empty() {
        return Err(AppError::Invalid(
            "Aucune case à ajouter : la salle n'en a pas, ou elles sont déjà sur le plan".into(),
        ));
    }
    let colonnes = (ids.len() as f64).sqrt().ceil().max(1.0) as usize;
    for (rang, id) in ids.iter().enumerate() {
        let x = 25.0 + (rang % colonnes) as f64 * 11.0;
        let y = 15.0 + (rang / colonnes) as f64 * 9.0;
        sqlx::query("INSERT INTO plan_repere(type,ref_id,x,y) VALUES('case',?,?,?)")
            .bind(id)
            .bind(x.min(98.0))
            .bind(y.min(98.0))
            .execute(&state.pool)
            .await?;
    }
    Ok(axum::Json(json!({ "ajoutees": ids.len() })).into_response())
}

pub(super) async fn repere_supprimer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    sqlx::query("DELETE FROM plan_repere WHERE id=?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(axum::Json(json!({ "ok": true })).into_response())
}

/// Réglages courants d'une salle depuis le plan : seulement les champs
/// envoyés sont modifiés, le reste de la configuration (/ambiance) est gardé.
pub(super) async fn salle_reglage(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(salle_id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let actuel: Option<(f64, f64, f64, f64)> = sqlx::query_as(
        "SELECT t_cible_fixe,plage,ventil_min,ventil_max FROM ambiance_salle WHERE salle_id=?",
    )
    .bind(salle_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((cible, plage, vmin, vmax)) = actuel else {
        return Err(AppError::Invalid(
            "Cette salle n'est pas encore raccordée : réglez-la d'abord dans Températures et ventilation"
                .into(),
        ));
    };
    let cible = form_f64(&form, "t_cible_fixe").unwrap_or(cible);
    if !(0.0..=45.0).contains(&cible) {
        return Err(AppError::Invalid(
            "La température de consigne doit rester entre 0 et 45 °C".into(),
        ));
    }
    let plage = form_f64(&form, "plage").unwrap_or(plage);
    if plage <= 0.0 {
        return Err(AppError::Invalid(
            "La plage de régulation doit être supérieure à zéro".into(),
        ));
    }
    let vmin = form_f64(&form, "ventil_min").unwrap_or(vmin).clamp(0.0, 100.0);
    let vmax = form_f64(&form, "ventil_max").unwrap_or(vmax).clamp(vmin, 100.0);
    sqlx::query(
        "UPDATE ambiance_salle SET t_cible_fixe=?,plage=?,ventil_min=?,ventil_max=? WHERE salle_id=?",
    )
    .bind(cible)
    .bind(plage)
    .bind(vmin)
    .bind(vmax)
    .bind(salle_id)
    .execute(&state.pool)
    .await?;
    Ok(axum::Json(json!({ "ok": true })).into_response())
}

#[cfg(test)]
mod tests {
    #[test]
    fn le_recadrage_garde_les_reperes_sur_le_dessin() {
        // Image 1000×500, dessin de 250 à 750 en x et 100 à 400 en y.
        let cadre = [250.0, 100.0, 500.0, 300.0, 1000.0, 500.0];
        assert_eq!(super::recadrer(50.0, 50.0, 0, cadre), (50.0, 50.0));
        assert_eq!(super::recadrer(25.0, 20.0, 0, cadre), (0.0, 0.0));
        // Pivoté d'un quart de tour : le coin haut-gauche du dessin est en
        // haut à droite de l'affichage.
        assert_eq!(super::recadrer(80.0, 25.0, 90, cadre), (100.0, 0.0));
    }

    #[test]
    fn le_gabarit_se_compile() {
        let mut env = minijinja::Environment::new();
        env.add_template("base.html", "{% block content %}{% endblock %}").unwrap();
        env.add_template(
            "plan_elevage.html",
            include_str!("../../templates/plan_elevage.html"),
        )
        .unwrap();
        let rendu = env
            .get_template("plan_elevage.html")
            .unwrap()
            .render(minijinja::context! {
                session => minijinja::context! { peut_modifier => true, csrf => "x" },
                image => false,
                rotation => 90,
                salles => vec![minijinja::context! { id => 1, nom => "Mater 1" }],
                programmes => Vec::<i32>::new(),
                relais => Vec::<i32>::new(),
                zigbee => Vec::<i32>::new(),
                cases => Vec::<i32>::new(),
            })
            .unwrap();
        assert!(rendu.contains("Mater 1"));
    }
}
