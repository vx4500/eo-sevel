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
