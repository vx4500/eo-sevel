//! Départs prévus (/departs) : le camion est annoncé (date, heure, nombre de
//! porcs). Une fois l'heure passée, le logiciel demande combien de porcs ont
//! été prélevés dans chaque salle et enregistre les sorties (transferts sans
//! destination), ce qui soulage l'effectif des salles.

use super::*;
use chrono::Datelike;

/// Départs pas encore confirmés, les plus proches d'abord. `a_confirmer` :
/// l'heure est passée.
pub(super) async fn a_venir(pool: &SqlitePool) -> AppResult<Vec<Value>> {
    let maintenant = Local::now().format("%Y-%m-%dT%H:%M").to_string();
    let lignes: Vec<(i64, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT id,date_heure,nombre,note FROM depart_prevu WHERE fait=0 ORDER BY date_heure,id",
    )
    .fetch_all(pool)
    .await?;
    Ok(lignes
        .into_iter()
        .map(|(id, dh, nombre, note)| {
            json!({"id": id, "date_heure": dh, "quand": lisible(&dh), "nombre": nombre,
                   "note": note, "a_confirmer": dh <= maintenant})
        })
        .collect())
}

/// « lundi 05/10 à 01:00 ».
fn lisible(dh: &str) -> String {
    const JOURS: [&str; 7] = ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"];
    match chrono::NaiveDateTime::parse_from_str(dh, "%Y-%m-%dT%H:%M") {
        Ok(d) => format!("{} {}", JOURS[d.weekday().num_days_from_monday() as usize], d.format("%d/%m à %H:%M")),
        Err(_) => dh.to_string(),
    }
}

/// Salles où il y a des porcs, avec leurs cases et effectifs.
async fn salles_avec_porcs(pool: &SqlitePool) -> AppResult<Vec<(i64, String, Vec<(i64, i64)>)>> {
    let salles: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, nom FROM salle ORDER BY ordre, nom").fetch_all(pool).await?;
    let mut sortie = Vec::new();
    for (id, nom) in salles {
        let cases: Vec<i64> = sqlx::query_scalar("SELECT id FROM casesalle WHERE salle_id=? ORDER BY id")
            .bind(id)
            .fetch_all(pool)
            .await?;
        let mut avec = Vec::new();
        for c in cases {
            let n = case_pig_count(pool, c).await?;
            if n > 0 {
                avec.push((c, n));
            }
        }
        if !avec.is_empty() {
            sortie.push((id, nom, avec));
        }
    }
    Ok(sortie)
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
) -> AppResult<Html<String>> {
    let departs = a_venir(&state.pool).await?;
    let salles: Vec<Value> = salles_avec_porcs(&state.pool)
        .await?
        .into_iter()
        .map(|(id, nom, cases)| json!({"id": id, "nom": nom, "porcs": cases.iter().map(|c| c.1).sum::<i64>()}))
        .collect();
    let faits: Vec<(String, i64, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT date_heure,nombre,nombre_reel,note FROM depart_prevu WHERE fait=1 ORDER BY date_heure DESC LIMIT 20",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut ctx = context(&session);
    ctx.insert("quai".into(), Value::Array(quai_en_attente(&state.pool).await?));
    ctx.insert("departs".into(), Value::Array(departs));
    ctx.insert("salles".into(), Value::Array(salles));
    ctx.insert(
        "faits".into(),
        Value::Array(faits.into_iter().map(|(dh, n, r, note)| json!({"quand": lisible(&dh), "nombre": n, "reel": r, "note": note})).collect()),
    );
    render(&state, "departs.html", Value::Object(ctx))
}

pub(super) async fn creer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let dh = form_text(&form, "date_heure")
        .filter(|v| chrono::NaiveDateTime::parse_from_str(v, "%Y-%m-%dT%H:%M").is_ok())
        .ok_or_else(|| AppError::Invalid("Date et heure du départ obligatoires".into()))?;
    let nombre = form_i64(&form, "nombre")
        .filter(|n| *n > 0)
        .ok_or_else(|| AppError::Invalid("Nombre de porcs invalide".into()))?;
    sqlx::query("INSERT INTO depart_prevu(date_heure,nombre,note) VALUES(?,?,?)")
        .bind(&dh)
        .bind(nombre)
        .bind(form_text(&form, "note"))
        .execute(&state.pool)
        .await?;
    db::journal(&state.pool, &session.nom, "prévoir", "départ", &format!("{dh} · {nombre} porcs"), "/departs").await;
    Ok(Redirect::to("/departs").into_response())
}

pub(super) async fn supprimer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    sqlx::query("DELETE FROM depart_prevu WHERE id=? AND fait=0").bind(id).execute(&state.pool).await?;
    db::journal(&state.pool, &session.nom, "annuler", "départ", &id.to_string(), "/departs").await;
    Ok(Redirect::to("/departs").into_response())
}

/// Confirmation : `salle_<id>` = porcs prélevés dans la salle. Ils sont
/// retirés case par case (la plus pleine d'abord), chaque sortie gardant la
/// bande de la case.
pub(super) async fn confirmer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let (dh,): (String,) = sqlx::query_as("SELECT date_heure FROM depart_prevu WHERE id=? AND fait=0")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    let date = dh.get(..10).unwrap_or(&dh).to_string();
    let mut sorties = Vec::new();
    let mut total = 0;
    for (salle_id, nom, mut cases) in salles_avec_porcs(&state.pool).await? {
        let demande = form_i64(&form, &format!("salle_{salle_id}")).unwrap_or(0);
        if demande <= 0 {
            continue;
        }
        let present: i64 = cases.iter().map(|c| c.1).sum();
        if demande > present {
            return Err(AppError::Invalid(format!("{nom} : {demande} porcs demandés, {present} présents")));
        }
        cases.sort_by(|a, b| b.1.cmp(&a.1));
        let mut reste = demande;
        for (case_id, n) in cases {
            if reste == 0 {
                break;
            }
            let pris = reste.min(n);
            reste -= pris;
            sorties.push((salle_id, case_id, pris));
        }
        total += demande;
    }
    if total == 0 {
        return Err(AppError::Invalid("Indiquez combien de porcs sont partis de chaque salle".into()));
    }
    let note = format!("Départ du {}", lisible(&dh));
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    for (salle_id, case_id, nombre) in sorties {
        let bande: Option<i64> = sqlx::query_scalar(
            "SELECT bande_id FROM transfert WHERE case_dest_id=? AND bande_id IS NOT NULL ORDER BY date DESC,id DESC LIMIT 1",
        )
        .bind(case_id)
        .fetch_optional(&mut *tx)
        .await?
        .flatten();
        sqlx::query("INSERT INTO transfert(date,espece,bande_id,salle_source_id,case_source_id,nombre,note) VALUES(?,'porc',?,?,?,?,?)")
            .bind(&date).bind(bande).bind(salle_id).bind(case_id).bind(nombre).bind(&note)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE depart_prevu SET fait=1,nombre_reel=? WHERE id=?").bind(total).bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    db::journal(&state.pool, &session.nom, "confirmer", "départ", &format!("{dh} · {total} porcs"), "/departs").await;
    Ok(Redirect::to("/departs").into_response())
}

/// Saisie rapide « Quai » : cases où il reste des porcs, avec leur effectif
/// et leur bande, pour choisir d'où partent les porcs mis sur le quai.
pub(super) async fn api_cases_porcs(State(state): State<AppState>) -> AppResult<axum::Json<Value>> {
    let cases: Vec<(i64, String, String, String)> = sqlx::query_as(
        "SELECT c.id,COALESCE(si.nom,si.code,''),s.nom,c.nom FROM casesalle c JOIN salle s ON s.id=c.salle_id \
         LEFT JOIN site si ON si.id=s.site_id ORDER BY s.ordre,s.nom,c.id",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut sortie = Vec::new();
    for (id, site, salle, nom) in cases {
        let n = case_pig_count(&state.pool, id).await?;
        if n <= 0 {
            continue;
        }
        let bande: Option<String> = sqlx::query_scalar(
            "SELECT b.code FROM transfert t JOIN bande b ON b.id=t.bande_id WHERE t.case_dest_id=? ORDER BY t.date DESC,t.id DESC LIMIT 1",
        )
        .bind(id)
        .fetch_optional(&state.pool)
        .await?;
        sortie.push(json!({"id": id, "site": site, "salle": salle, "nom": nom, "porcs": n, "bande": bande}));
    }
    Ok(axum::Json(Value::Array(sortie)))
}

/// Porcs mis sur le quai pour l'abattoir : ils sortent de leur case
/// (transfert sans destination, bande de la case conservée).
pub(super) async fn sortie_quai(
    pool: &SqlitePool,
    case_id: i64,
    nombre: i64,
    date: &str,
    note: Option<String>,
) -> AppResult<()> {
    let (salle_id,): (i64,) = sqlx::query_as("SELECT salle_id FROM casesalle WHERE id=?")
        .bind(case_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::Invalid("Case introuvable".into()))?;
    let present = case_pig_count(pool, case_id).await?;
    if nombre > present {
        return Err(AppError::Invalid(format!("Effectif insuffisant dans la case : {present} porc(s) présent(s)")));
    }
    let bande: Option<i64> = sqlx::query_scalar(
        "SELECT bande_id FROM transfert WHERE case_dest_id=? AND bande_id IS NOT NULL ORDER BY date DESC,id DESC LIMIT 1",
    )
    .bind(case_id)
    .fetch_optional(pool)
    .await?
    .flatten();
    let note = match note {
        Some(n) => format!("Quai — départ abattoir · {n}"),
        None => "Quai — départ abattoir".to_string(),
    };
    let mut tx = pool.begin().await?;
    let transfert = sqlx::query("INSERT INTO transfert(date,espece,bande_id,salle_source_id,case_source_id,nombre,note) VALUES(?,'porc',?,?,?,?,?)")
        .bind(date).bind(bande).bind(salle_id).bind(case_id).bind(nombre).bind(note)
        .execute(&mut *tx)
        .await?
        .last_insert_rowid();
    sqlx::query("INSERT INTO quai_porc(transfert_id,bande_id,case_id,date,nombre) VALUES(?,?,?,?,?)")
        .bind(transfert).bind(bande).bind(case_id).bind(date).bind(nombre)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Au-delà, des porcs restés sur le quai sans bon d'apport sont signalés
/// dans les alertes : bon oublié, ou sortie à noter (vente directe…).
pub(super) const QUAI_JOURS_MAX: i64 = 7;

/// Nombre de lots du quai en attente depuis plus de `QUAI_JOURS_MAX` jours.
pub(super) async fn quai_en_retard(pool: &SqlitePool) -> AppResult<i64> {
    Ok(quai_en_attente(pool).await?.iter().filter(|q| q["en_retard"].as_bool() == Some(true)).count() as i64)
}

/// Porcs sortis du quai sans bon d'apport : partis à l'abattoir (bon pas
/// arrivé), en vente directe ou ailleurs. Ils quittent l'élevage.
pub(super) async fn quai_sortir(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let motif = form.get("motif").map(String::as_str).unwrap_or_default();
    if !["abattoir", "vente_directe", "autre"].contains(&motif) {
        return Err(AppError::Invalid("Choisissez où sont partis les porcs".into()));
    }
    let date = form
        .get("date")
        .filter(|d| !d.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| chrono::Local::now().date_naive().format("%Y-%m-%d").to_string());
    let note = form.get("note").map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    let reste = quai_en_attente(&state.pool)
        .await?
        .iter()
        .find(|q| q["id"].as_i64() == Some(id))
        .and_then(|q| q["reste"].as_i64())
        .ok_or_else(|| AppError::Invalid("Plus aucun porc en attente sur ce lot du quai".into()))?;
    let nombre = match form.get("nombre").map(|n| n.trim()).filter(|n| !n.is_empty()) {
        Some(n) => n.parse::<i64>().map_err(|_| AppError::Invalid("Nombre de porcs invalide".into()))?,
        None => reste,
    };
    if nombre <= 0 || nombre > reste {
        return Err(AppError::Invalid(format!("Nombre de porcs entre 1 et {reste}")));
    }
    sqlx::query("INSERT INTO quai_sortie(quai_id,date,nombre,motif,note) VALUES(?,?,?,?,?)")
        .bind(id).bind(&date).bind(nombre).bind(motif).bind(note)
        .execute(&state.pool)
        .await?;
    db::journal(&state.pool, &session.nom, "sortir", "quai", &format!("{id} · {nombre} porcs · {motif}"), "/departs").await;
    Ok(Redirect::to("/departs").into_response())
}

/// Porcs sur le quai pas encore repris par un bon d'apport : sortis des
/// salles mais toujours à l'élevage.
pub(super) async fn quai_en_attente(pool: &SqlitePool) -> AppResult<Vec<Value>> {
    let lignes: Vec<(i64, String, i64, i64, Option<String>, String, String, i64)> = sqlx::query_as(
        "SELECT q.id,q.date,q.nombre,q.nombre-COALESCE((SELECT SUM(a.nombre) FROM quai_affectation a WHERE a.quai_id=q.id),0)-COALESCE((SELECT SUM(x.nombre) FROM quai_sortie x WHERE x.quai_id=q.id),0) AS reste, \
                b.code,COALESCE(s.nom,''),COALESCE(c.nom,''),CAST(julianday(date('now','localtime'))-julianday(date(q.date)) AS INTEGER) \
         FROM quai_porc q LEFT JOIN bande b ON b.id=q.bande_id LEFT JOIN casesalle c ON c.id=q.case_id LEFT JOIN salle s ON s.id=c.salle_id \
         WHERE reste>0 ORDER BY q.date,q.id",
    )
    .fetch_all(pool)
    .await?;
    Ok(lignes
        .into_iter()
        .map(|(id, date, nombre, reste, bande, salle, case, jours)| {
            let origine = [salle, case].into_iter().filter(|v| !v.is_empty()).collect::<Vec<_>>().join(" · ");
            json!({"id": id, "date": date, "nombre": nombre, "reste": reste, "bande": bande,
                   "origine": origine, "jours": jours, "en_retard": jours > QUAI_JOURS_MAX,
                   "annulable": reste == nombre})
        })
        .collect())
}

/// Erreur de saisie : les porcs du quai retournent dans leur case.
pub(super) async fn quai_annuler(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let affecte: i64 = sqlx::query_scalar("SELECT (SELECT COUNT(*) FROM quai_affectation WHERE quai_id=?)+(SELECT COUNT(*) FROM quai_sortie WHERE quai_id=?)")
        .bind(id)
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    if affecte > 0 {
        return Err(AppError::Invalid("Ces porcs sont déjà sortis du quai (bon d'apport ou sortie notée)".into()));
    }
    let mut tx = state.pool.begin().await?;
    let transfert: Option<i64> = sqlx::query_scalar("SELECT transfert_id FROM quai_porc WHERE id=?")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    let transfert = transfert.ok_or(AppError::NotFound)?;
    sqlx::query("DELETE FROM transfert WHERE id=?").bind(transfert).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM quai_porc WHERE id=?").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    db::journal(&state.pool, &session.nom, "annuler", "quai", &id.to_string(), "/departs").await;
    Ok(Redirect::to("/departs").into_response())
}
