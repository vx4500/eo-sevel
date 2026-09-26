//! Portail prestataire (/engraissement) : bandes confiées, inventaire, sorties
//! mortes (n° d'équarrissage obligatoire selon l'option du prestataire),
//! aliment consommé (livraisons et relevés) et pesées, avec le calendrier des
//! stades de la bande (transitions et début des pesées).
//!
//! Une bande n'a qu'un prestataire à la fois (`bande.engraisseur_id`) ;
//! `bande_prestataire` garde l'historique des périodes confiées.

use super::*;

/// Étapes du calendrier d'une bande vues par le prestataire, en jours après
/// la mise-bas (mêmes réglages que le calendrier de conduite).
const ETAPES: &[(&str, &str, i64)] = &[
    ("sevrage", "Sevrage — entrée en post-sevrage", 28),
    ("aliment_2e_age", "Passage à l'aliment 2e âge", 42),
    ("transfert_engr", "Transfert post-sevrage → engraissement", 71),
    ("aliment_finition", "Passage à l'aliment finition", 140),
    ("debut_pesees", "Début des pesées", 180),
    ("depart", "Départ abattoir", 215),
];

/// Une étape datée : `jours_restants` négatif quand elle est passée.
#[derive(Debug, Clone, PartialEq)]
struct Etape {
    cle: &'static str,
    libelle: &'static str,
    jour: i64,
    date: NaiveDate,
    jours_restants: i64,
}

fn calendrier(date_mb: NaiveDate, jours: &[(&'static str, &'static str, i64)], today: NaiveDate) -> Vec<Etape> {
    let mut etapes: Vec<Etape> = jours
        .iter()
        .map(|(cle, libelle, jour)| {
            let date = date_mb + chrono::Duration::days(*jour);
            Etape { cle, libelle, jour: *jour, date, jours_restants: (date - today).num_days() }
        })
        .collect();
    etapes.sort_by_key(|e| e.jour);
    etapes
}

/// Stade du moment d'après les étapes passées.
fn stade_actuel(etapes: &[Etape]) -> &'static str {
    let passee = |cle: &str| etapes.iter().any(|e| e.cle == cle && e.jours_restants <= 0);
    if passee("depart") {
        "Départ prévu dépassé"
    } else if passee("debut_pesees") {
        "Engraissement — période de pesées"
    } else if passee("transfert_engr") {
        "Engraissement"
    } else if passee("sevrage") {
        "Post-sevrage"
    } else {
        "Maternité (avant sevrage)"
    }
}

/// Gain moyen quotidien entre la première et la dernière pesée (g/j).
fn gmq(pesees: &[(NaiveDate, f64)]) -> Option<f64> {
    let premiere = pesees.iter().min_by_key(|p| p.0)?;
    let derniere = pesees.iter().max_by_key(|p| p.0)?;
    let jours = (derniere.0 - premiere.0).num_days();
    (jours > 0).then(|| (derniere.1 - premiere.1) * 1000.0 / jours as f64)
}

async fn reglages_etapes(pool: &SqlitePool) -> AppResult<Vec<(&'static str, &'static str, i64)>> {
    let mut jours = Vec::new();
    for (cle, libelle, defaut) in ETAPES {
        jours.push((*cle, *libelle, reglage_i64(pool, cle, *defaut).await?));
    }
    Ok(jours)
}

/// Le prestataire ne saisit que sur ses bandes actives ; l'éleveur partout.
async fn autoriser_bande(pool: &SqlitePool, session: &SessionData, code: &str) -> AppResult<()> {
    if session.role == "engraisseur" {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM bande WHERE code=? AND engraisseur_id=? AND active=1",
        )
        .bind(code)
        .bind(session.uid)
        .fetch_one(pool)
        .await?;
        if n == 0 {
            return Err(AppError::Forbidden);
        }
        Ok(())
    } else {
        require_writer(session)
    }
}

/// Vrai si le prestataire de la bande doit fournir un n° d'équarrissage.
pub(super) async fn equarrissage_obligatoire(pool: &SqlitePool, code: &str) -> AppResult<bool> {
    let v: Option<i64> = sqlx::query_scalar(
        "SELECT o.equarrissage_obligatoire FROM bande b JOIN prestataire_option o ON o.utilisateur_id=b.engraisseur_id WHERE b.code=?",
    )
    .bind(code)
    .fetch_optional(pool)
    .await?;
    Ok(v.unwrap_or(0) == 1)
}

/// Confie une bande à un prestataire (ou la reprend avec `None`) en tenant
/// l'historique. Sans effet si le prestataire est déjà le bon.
pub(super) async fn confier_bande(pool: &SqlitePool, band_id: i64, presta: Option<i64>, date: &str) -> AppResult<()> {
    let actuel: Option<Option<i64>> = sqlx::query_scalar("SELECT engraisseur_id FROM bande WHERE id=?")
        .bind(band_id)
        .fetch_optional(pool)
        .await?;
    let Some(actuel) = actuel else {
        return Err(AppError::NotFound);
    };
    if actuel == presta {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE bande_prestataire SET date_fin=? WHERE bande_id=? AND date_fin IS NULL")
        .bind(date)
        .bind(band_id)
        .execute(&mut *tx)
        .await?;
    if let Some(uid) = presta {
        sqlx::query("INSERT INTO bande_prestataire(bande_id,utilisateur_id,date_debut) VALUES(?,?,?)")
            .bind(band_id)
            .bind(uid)
            .bind(date)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE bande SET engraisseur_id=?,updated_at=CURRENT_TIMESTAMP WHERE id=?")
        .bind(presta)
        .bind(band_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
) -> AppResult<Html<String>> {
    if !session.module_prestataires {
        return Err(AppError::Forbidden);
    }
    let pool = &state.pool;
    let filtre = if session.role == "engraisseur" {
        format!("AND b.engraisseur_id={}", session.uid)
    } else {
        String::new()
    };
    let bandes = generic_rows(
        pool,
        &format!(
            "SELECT b.id,b.code,b.date_mb,b.instructions,b.poids_cible,b.engraisseur_id,\
             COALESCE(NULLIF(TRIM(COALESCE(u.prenom,'')||' '||COALESCE(u.nom,'')),''),u.identifiant) AS prestataire,\
             COALESCE(o.equarrissage_obligatoire,0) AS equarrissage_obligatoire \
             FROM bande b LEFT JOIN utilisateur u ON u.id=b.engraisseur_id \
             LEFT JOIN prestataire_option o ON o.utilisateur_id=b.engraisseur_id \
             WHERE b.active=1 {filtre} ORDER BY b.date_mb DESC,b.code"
        ),
    )
    .await?;
    let jours = reglages_etapes(pool).await?;
    let today = Local::now().date_naive();
    let mut fiches = Vec::new();
    let mut codes = Vec::new();
    for b in &bandes {
        let (Some(id), Some(code)) = (b["id"].as_i64(), b["code"].as_str()) else {
            continue;
        };
        codes.push(format!("'{}'", code.replace('\'', "''")));
        let effectif = total_band_pigs(pool, id, code).await?;
        let inventaire: Option<(String, f64)> = sqlx::query_as(
            "SELECT date,CAST(nombre AS REAL) FROM mouvementstock WHERE est_stock=1 AND bande_code=? ORDER BY date DESC,id DESC LIMIT 1",
        )
        .bind(code)
        .fetch_optional(pool)
        .await?;
        let aliment: (f64, i64) = sqlx::query_as(
            "SELECT CAST(COALESCE(SUM(quantite_kg),0) AS REAL),COUNT(*) FROM consoaliment_bande WHERE bande_code=?",
        )
        .bind(code)
        .fetch_one(pool)
        .await?;
        let pesees: Vec<(String, f64)> =
            sqlx::query_as("SELECT date,poids_moyen FROM pesee_bande WHERE bande_code=? ORDER BY date")
                .bind(code)
                .fetch_all(pool)
                .await?;
        let pesees: Vec<(NaiveDate, f64)> = pesees
            .into_iter()
            .filter_map(|(d, p)| parse_stored_date(&d).map(|d| (d, p)))
            .collect();
        let gain = gmq(&pesees);
        let derniere = pesees.iter().max_by_key(|p| p.0).copied();
        // Date estimée d'atteinte du poids cible au GMQ mesuré.
        let cible = b["poids_cible"].as_f64();
        let date_cible = match (derniere, gain, cible) {
            (Some((d, p)), Some(g), Some(c)) if g > 0.0 && c > p => {
                Some((d + chrono::Duration::days(((c - p) * 1000.0 / g).ceil() as i64)).to_string())
            }
            _ => None,
        };
        let date_mb = b["date_mb"].as_str().and_then(parse_stored_date);
        let (etapes, stade) = match date_mb {
            Some(mb) => {
                let e = calendrier(mb, &jours, today);
                let stade = stade_actuel(&e);
                let prochaine = e.iter().position(|x| x.jours_restants > 0);
                let lignes: Vec<Value> = e
                    .iter()
                    .enumerate()
                    .map(|(i, x)| {
                        json!({"libelle": x.libelle, "jour": x.jour, "date": x.date.to_string(),
                               "jours_restants": x.jours_restants, "passee": x.jours_restants <= 0,
                               "prochaine": Some(i) == prochaine})
                    })
                    .collect();
                (lignes, stade)
            }
            None => (Vec::new(), "Date de mise-bas inconnue"),
        };
        let pesees_ouvertes = etapes.iter().any(|e| e["passee"].as_bool() == Some(true) && e["libelle"] == "Début des pesées");
        let mut fiche = b.clone();
        fiche["effectif"] = json!(effectif);
        fiche["inventaire"] = json!(inventaire.map(|(d, n)| json!({"date": d, "nombre": n as i64})));
        fiche["aliment_kg"] = json!((aliment.0 * 10.0).round() / 10.0);
        fiche["aliment_saisies"] = json!(aliment.1);
        fiche["aliment_par_porc"] = json!((effectif > 0).then(|| (aliment.0 / effectif as f64 * 10.0).round() / 10.0));
        fiche["derniere_pesee"] = json!(derniere.map(|(d, p)| json!({"date": d.to_string(), "poids": p})));
        fiche["gmq"] = json!(gain.map(f64::round));
        fiche["date_cible"] = json!(date_cible);
        fiche["etapes"] = json!(etapes);
        fiche["stade"] = json!(stade);
        fiche["pesees_ouvertes"] = json!(pesees_ouvertes);
        fiches.push(fiche);
    }
    let dans = if codes.is_empty() { "''".to_string() } else { codes.join(",") };
    let tout = session.role != "engraisseur";
    let clause = |col: &str| if tout { String::new() } else { format!("WHERE {col} IN ({dans})") };
    let declarations = generic_rows(
        pool,
        &format!("SELECT id,horodatage,bande_code,date,stade,cause,poids,nombre,num_equarrissage,declare_par,note FROM declarationmort {} ORDER BY date DESC,horodatage DESC LIMIT 250", clause("bande_code")),
    )
    .await?;
    let aliments = generic_rows(
        pool,
        &format!("SELECT id,bande_code,type,date_debut,date,aliment,quantite_kg,declare_par,note FROM consoaliment_bande {} ORDER BY date DESC,id DESC LIMIT 250", clause("bande_code")),
    )
    .await?;
    let pesees = generic_rows(
        pool,
        &format!("SELECT id,bande_code,date,nombre,poids_moyen,declare_par,note FROM pesee_bande {} ORDER BY date DESC,id DESC LIMIT 250", clause("bande_code")),
    )
    .await?;
    let inventaires = generic_rows(
        pool,
        &format!("SELECT id,bande_code,date,CAST(nombre AS INTEGER) AS nombre,poids,libelle FROM mouvementstock WHERE est_stock=1 AND type_saisie='inventaire' {} ORDER BY date DESC,id DESC LIMIT 100",
            if tout { String::new() } else { format!("AND bande_code IN ({dans})") }),
    )
    .await?;
    let cases = generic_rows(
        pool,
        "SELECT c.id,COALESCE(si.nom,si.code)||' · '||s.nom||' · '||c.nom AS nom FROM casesalle c JOIN salle s ON s.id=c.salle_id JOIN site si ON si.id=s.site_id ORDER BY si.nom,s.ordre,c.nom",
    )
    .await?;
    let causes = generic_rows(pool, "SELECT libelle FROM causeperte ORDER BY libelle").await?;
    let aliments_connus = generic_rows(
        pool,
        "SELECT DISTINCT aliment FROM (SELECT aliment FROM planaliment UNION SELECT aliment FROM consoaliment_bande) WHERE aliment IS NOT NULL AND aliment<>'' ORDER BY aliment",
    )
    .await?;
    let recurrences = mortalites_suspectes_recurrentes(pool).await?;
    let mut ctx = context(&session);
    ctx.insert("bandes".into(), Value::Array(fiches));
    ctx.insert("declarations".into(), Value::Array(declarations));
    ctx.insert("aliments".into(), Value::Array(aliments));
    ctx.insert("pesees".into(), Value::Array(pesees));
    ctx.insert("inventaires".into(), Value::Array(inventaires));
    ctx.insert("cases".into(), Value::Array(cases));
    ctx.insert("causes".into(), Value::Array(causes));
    ctx.insert("aliments_connus".into(), Value::Array(aliments_connus));
    ctx.insert("recurrences".into(), Value::Array(recurrences));
    ctx.insert("today".into(), json!(today_iso()));
    render(&state, "prestataire.html", Value::Object(ctx))
}

fn bande_du_formulaire(form: &HashMap<String, String>) -> AppResult<String> {
    form_text(form, "bande_code").ok_or_else(|| AppError::Invalid("Bande obligatoire".into()))
}

pub(super) async fn inventaire(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    verify_csrf(&session, &form)?;
    let code = bande_du_formulaire(&form)?;
    autoriser_bande(&state.pool, &session, &code).await?;
    let nombre = form_i64(&form, "nombre")
        .filter(|v| (0..=100_000).contains(v))
        .ok_or_else(|| AppError::Invalid("Effectif compté invalide".into()))?;
    let date = form_date_or_today(&form, "date")?;
    let libelle = match form_text(&form, "note") {
        Some(note) => format!("Inventaire ({}) — {note}", session.identifiant),
        None => format!("Inventaire ({})", session.identifiant),
    };
    sqlx::query("INSERT INTO mouvementstock(date,bande_code,nombre,poids,libelle,type_saisie,est_stock) VALUES(?,?,?,?,?,'inventaire',1)")
        .bind(date)
        .bind(&code)
        .bind(nombre)
        .bind(form_f64(&form, "poids"))
        .bind(libelle)
        .execute(&state.pool)
        .await?;
    Ok(Redirect::to("/engraissement#inventaire").into_response())
}

pub(super) async fn aliment(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    verify_csrf(&session, &form)?;
    let code = bande_du_formulaire(&form)?;
    autoriser_bande(&state.pool, &session, &code).await?;
    let kg = form_f64(&form, "quantite_kg")
        .filter(|v| *v > 0.0 && *v <= 1_000_000.0)
        .ok_or_else(|| AppError::Invalid("Quantité (kg) invalide".into()))?;
    let type_saisie = if form.get("type").map(String::as_str) == Some("releve") { "releve" } else { "livraison" };
    let date = form_date_or_today(&form, "date")?;
    let date_debut = if type_saisie == "releve" {
        let debut = form_date(&form, "date_debut")?
            .ok_or_else(|| AppError::Invalid("Relevé : date de début de période obligatoire".into()))?;
        if debut > date {
            return Err(AppError::Invalid("La période commence après sa fin".into()));
        }
        Some(debut)
    } else {
        None
    };
    sqlx::query("INSERT INTO consoaliment_bande(horodatage,bande_code,type,date_debut,date,aliment,quantite_kg,declare_par,note) VALUES(CURRENT_TIMESTAMP,?,?,?,?,?,?,?,?)")
        .bind(&code)
        .bind(type_saisie)
        .bind(date_debut)
        .bind(date)
        .bind(form_text(&form, "aliment"))
        .bind(kg)
        .bind(&session.identifiant)
        .bind(form_text(&form, "note"))
        .execute(&state.pool)
        .await?;
    Ok(Redirect::to("/engraissement#aliment").into_response())
}

pub(super) async fn pesee(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    verify_csrf(&session, &form)?;
    let code = bande_du_formulaire(&form)?;
    autoriser_bande(&state.pool, &session, &code).await?;
    let nombre = form_i64(&form, "nombre")
        .filter(|v| (1..=100_000).contains(v))
        .ok_or_else(|| AppError::Invalid("Nombre de porcs pesés invalide".into()))?;
    // Poids moyen direct, ou poids total ÷ nombre.
    let moyen = match (form_f64(&form, "poids_moyen"), form_f64(&form, "poids_total")) {
        (Some(m), _) => Some(m),
        (None, Some(t)) => Some(t / nombre as f64),
        _ => None,
    }
    .filter(|p| *p > 0.0 && *p < 400.0)
    .ok_or_else(|| AppError::Invalid("Poids moyen ou poids total obligatoire".into()))?;
    sqlx::query("INSERT INTO pesee_bande(horodatage,bande_code,date,nombre,poids_moyen,declare_par,note) VALUES(CURRENT_TIMESTAMP,?,?,?,?,?,?)")
        .bind(&code)
        .bind(form_date_or_today(&form, "date")?)
        .bind(nombre)
        .bind((moyen * 10.0).round() / 10.0)
        .bind(&session.identifiant)
        .bind(form_text(&form, "note"))
        .execute(&state.pool)
        .await?;
    Ok(Redirect::to("/engraissement#pesees").into_response())
}

async fn supprimer_ligne(state: &AppState, session: &SessionData, table: &str, id: i64) -> AppResult<()> {
    let code: Option<String> = sqlx::query_scalar(&format!("SELECT bande_code FROM {table} WHERE id=?"))
        .bind(id)
        .fetch_optional(&state.pool)
        .await?;
    let Some(code) = code else {
        return Ok(());
    };
    autoriser_bande(&state.pool, session, &code).await?;
    sqlx::query(&format!("DELETE FROM {table} WHERE id=?")).bind(id).execute(&state.pool).await?;
    Ok(())
}

pub(super) async fn aliment_supprimer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    verify_csrf(&session, &form)?;
    supprimer_ligne(&state, &session, "consoaliment_bande", id).await?;
    Ok(Redirect::to("/engraissement#aliment").into_response())
}

pub(super) async fn pesee_supprimer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    verify_csrf(&session, &form)?;
    supprimer_ligne(&state, &session, "pesee_bande", id).await?;
    Ok(Redirect::to("/engraissement#pesees").into_response())
}

/// Réglages d'un prestataire depuis Utilisateurs : option équarrissage et
/// bandes confiées (cases cochées). Une bande cochée est retirée à son
/// prestataire précédent ; une bande décochée est reprise par l'élevage.
pub(super) async fn utilisateur_reglages(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(uid): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    if !session.est_admin() {
        return Err(AppError::Forbidden);
    }
    verify_csrf(&session, &form)?;
    let pool = &state.pool;
    let role: Option<String> = sqlx::query_scalar("SELECT role FROM utilisateur WHERE id=?")
        .bind(uid)
        .fetch_optional(pool)
        .await?;
    if role.as_deref() != Some("engraisseur") {
        return Err(AppError::Invalid("Cet utilisateur n'est pas un prestataire".into()));
    }
    sqlx::query("INSERT INTO prestataire_option(utilisateur_id,equarrissage_obligatoire) VALUES(?,?) ON CONFLICT(utilisateur_id) DO UPDATE SET equarrissage_obligatoire=excluded.equarrissage_obligatoire")
        .bind(uid)
        .bind(i64::from(form.contains_key("equarrissage_obligatoire")))
        .execute(pool)
        .await?;
    let date = form_date_or_today(&form, "date")?;
    let actives: Vec<(i64, Option<i64>)> =
        sqlx::query_as("SELECT id,engraisseur_id FROM bande WHERE active=1").fetch_all(pool).await?;
    for (band_id, actuel) in actives {
        let cochee = form.contains_key(&format!("bande_{band_id}"));
        if cochee && actuel != Some(uid) {
            confier_bande(pool, band_id, Some(uid), &date).await?;
        } else if !cochee && actuel == Some(uid) {
            confier_bande(pool, band_id, None, &date).await?;
        }
    }
    Ok(Redirect::to("/utilisateurs").into_response())
}

/// Bandes actives et options, pour la fiche de chaque prestataire dans
/// Utilisateurs.
pub(super) async fn contexte_utilisateurs(pool: &SqlitePool) -> AppResult<(Vec<Value>, Vec<Value>)> {
    let bandes = generic_rows(
        pool,
        "SELECT b.id,b.code,b.date_mb,b.engraisseur_id,u.identifiant AS prestataire FROM bande b LEFT JOIN utilisateur u ON u.id=b.engraisseur_id WHERE b.active=1 ORDER BY b.date_mb DESC,b.code",
    )
    .await?;
    let options = generic_rows(pool, "SELECT utilisateur_id,equarrissage_obligatoire FROM prestataire_option").await?;
    Ok((bandes, options))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn calendrier_et_stade() {
        let e = calendrier(d("2026-06-01"), ETAPES, d("2026-09-01"));
        // J92 : transfert (J71) passé, finition (J140) à venir.
        assert_eq!(stade_actuel(&e), "Engraissement");
        let finition = e.iter().find(|x| x.cle == "aliment_finition").unwrap();
        assert_eq!(finition.date, d("2026-10-19"));
        assert_eq!(finition.jours_restants, 48);
        let e = calendrier(d("2026-06-01"), ETAPES, d("2026-06-10"));
        assert_eq!(stade_actuel(&e), "Maternité (avant sevrage)");
        let e = calendrier(d("2026-01-01"), ETAPES, d("2026-07-10"));
        assert_eq!(stade_actuel(&e), "Engraissement — période de pesées");
    }

    #[test]
    fn gmq_entre_premiere_et_derniere_pesee() {
        assert_eq!(gmq(&[(d("2026-09-01"), 90.0)]), None);
        let g = gmq(&[(d("2026-09-11"), 99.0), (d("2026-09-01"), 90.0)]).unwrap();
        assert!((g - 900.0).abs() < 1e-9);
    }

    async fn base() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(include_str!("../../migrations/0001_schema.sql")).execute(&pool).await.unwrap();
        sqlx::query("ALTER TABLE declarationmort ADD COLUMN num_equarrissage TEXT").execute(&pool).await.unwrap();
        sqlx::raw_sql(include_str!("../../migrations/0019_prestataires.sql")).execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn confier_bande_tient_l_historique() {
        let pool = base().await;
        let p1 = sqlx::query("INSERT INTO utilisateur(identifiant,hash_mdp,role,actif) VALUES('p1','x','engraisseur',1)").execute(&pool).await.unwrap().last_insert_rowid();
        let p2 = sqlx::query("INSERT INTO utilisateur(identifiant,hash_mdp,role,actif) VALUES('p2','x','engraisseur',1)").execute(&pool).await.unwrap().last_insert_rowid();
        let b = sqlx::query("INSERT INTO bande(code,date_mb,active) VALUES('B1','2026-06-01',1)").execute(&pool).await.unwrap().last_insert_rowid();
        confier_bande(&pool, b, Some(p1), "2026-07-01").await.unwrap();
        confier_bande(&pool, b, Some(p1), "2026-07-05").await.unwrap();
        confier_bande(&pool, b, Some(p2), "2026-08-01").await.unwrap();
        let hist: Vec<(i64, String, Option<String>)> = sqlx::query_as("SELECT utilisateur_id,date_debut,date_fin FROM bande_prestataire ORDER BY id").fetch_all(&pool).await.unwrap();
        assert_eq!(hist, vec![(p1, "2026-07-01".into(), Some("2026-08-01".into())), (p2, "2026-08-01".into(), None)]);
        sqlx::query("INSERT INTO prestataire_option(utilisateur_id,equarrissage_obligatoire) VALUES(?,1)").bind(p2).execute(&pool).await.unwrap();
        assert!(equarrissage_obligatoire(&pool, "B1").await.unwrap());
        confier_bande(&pool, b, None, "2026-09-01").await.unwrap();
        assert!(!equarrissage_obligatoire(&pool, "B1").await.unwrap());
        let en_cours: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bande_prestataire WHERE date_fin IS NULL").fetch_one(&pool).await.unwrap();
        assert_eq!(en_cours, 0);
    }
}
