//! Sevrage d'une bande : une page, une ligne par truie, le nombre compté.
//!
//! L'effectif « présent » d'une portée (nés vifs + adoptés − retirés − pertes)
//! est calculé par la vue `portee_effectif`, et la base refuse un sevrage qui
//! ne lui est pas égal. Plutôt que d'obliger l'éleveur à déclarer chaque perte
//! avant de sevrer, la page enregistre l'écart constaté au sevrage comme une
//! perte (cause choisie), puis le sevrage, dans une seule transaction. Un
//! nombre supérieur aux présents est refusé : c'est une adoption à saisir.

use super::*;

/// Portées non sevrées de la bande, avec leurs effectifs.
async fn portees_ouvertes(pool: &SqlitePool, band_id: i64) -> AppResult<Vec<Value>> {
    rows_to_json(
        sqlx::query(
            "SELECT pe.id AS mise_bas_id,t.id AS truie_id,t.num_travail,t.rang,pe.date AS date_mb,\
             CAST(julianday('now')-julianday(pe.date) AS INTEGER) AS age,\
             e.nes_vifs AS nv,pe.adoptes,pe.retires,pe.pertes,pe.presents,\
             s.nom AS salle,c.nom AS case_nom \
             FROM portee_effectif pe JOIN truie t ON t.id=pe.truie_id JOIN evenement e ON e.id=pe.id \
             LEFT JOIN casesalle c ON c.id=COALESCE(e.case_id,t.case_id) LEFT JOIN salle s ON s.id=c.salle_id \
             WHERE pe.bande_id=? AND pe.cloturee=0 AND pe.date<=date('now') \
             ORDER BY s.nom,c.nom,CAST(t.num_travail AS INTEGER)",
        )
        .bind(band_id)
        .fetch_all(pool)
        .await?,
    )
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(band_id): Path<i64>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let pool = &state.pool;
    let bande =
        sqlx::query_as::<_, (String, Option<String>)>("SELECT code,date_mb FROM bande WHERE id=?")
            .bind(band_id)
            .fetch_optional(pool)
            .await?
            .ok_or(AppError::NotFound)?;
    let calendrier = load_band_schedule(pool).await?;
    let aujourd_hui = Local::now().date_naive();
    let prevu = bande
        .1
        .as_deref()
        .and_then(parse_stored_date)
        .map(|d| d + Duration::days(calendrier.weaning));
    // Date proposée : le sevrage prévu s'il est passé, sinon aujourd'hui.
    let date_proposee = prevu.filter(|d| *d <= aujourd_hui).unwrap_or(aujourd_hui);

    let portees = portees_ouvertes(pool, band_id).await?;
    let deja = rows_to_json(
        sqlx::query(
            "SELECT e.date,t.id AS truie_id,t.num_travail,e.nb_sevres,e.poids_moyen \
             FROM evenement e JOIN truie t ON t.id=e.truie_id \
             WHERE e.bande_id=? AND e.type='sevrage' ORDER BY e.date DESC,CAST(t.num_travail AS INTEGER)",
        )
        .bind(band_id)
        .fetch_all(pool)
        .await?,
    )?;
    let cases_ps = sqlx::query(
        "SELECT c.id,s.nom AS salle,c.nom,c.nb_max_porcs FROM casesalle c JOIN salle s ON s.id=c.salle_id \
         WHERE lower(COALESCE(s.type,'')) LIKE '%sevr%' OR lower(s.nom) LIKE '%sevr%' ORDER BY s.ordre,s.nom,c.nom",
    )
    .fetch_all(pool)
    .await?;
    let mut cases = Vec::new();
    for case in cases_ps {
        let id: i64 = case.try_get("id")?;
        let presents = case_pig_count(pool, id).await?.max(0);
        let max: Option<i64> = case
            .try_get::<Option<i64>, _>("nb_max_porcs")
            .ok()
            .flatten();
        cases.push(json!({
            "id": id,
            "libelle": format!("{} · {}", case.try_get::<String, _>("salle")?, case.try_get::<String, _>("nom")?),
            "presents": presents,
            "libres": max.filter(|m| *m > 0).map(|m| (m - presents).max(0)),
        }));
    }
    let salles_truies = rows_to_json(
        sqlx::query(
            "SELECT s.id,s.nom,CASE WHEN lower(COALESCE(s.type,'')) LIKE '%verrat%' OR lower(s.nom) LIKE '%verrat%' THEN 1 ELSE 0 END AS verraterie \
             FROM salle s WHERE lower(COALESCE(s.type,'')) NOT LIKE '%mater%' AND lower(COALESCE(s.type,'')) NOT LIKE '%sevr%' \
             AND lower(COALESCE(s.type,'')) NOT LIKE '%engrais%' ORDER BY verraterie DESC,s.ordre,s.nom",
        )
        .fetch_all(pool)
        .await?,
    )?;
    let causes: Vec<String> = sqlx::query_scalar("SELECT libelle FROM causeperte ORDER BY libelle")
        .fetch_all(pool)
        .await?;

    let mut ctx = context(&session);
    ctx.insert("bande_id".into(), json!(band_id));
    ctx.insert("code".into(), json!(bande.0));
    ctx.insert("date_mb".into(), json!(bande.1));
    ctx.insert("prevu".into(), json!(prevu.map(|d| d.to_string())));
    ctx.insert("date_proposee".into(), json!(date_proposee.to_string()));
    ctx.insert("aujourd_hui".into(), json!(aujourd_hui.to_string()));
    // Reprise d'une saisie faite ailleurs (saisie groupée de la fiche bande) :
    // nombres par truie, date, poids et sélection passés dans l'adresse.
    let reprise: Map<String, Value> = query
        .iter()
        .filter(|(cle, _)| {
            cle.starts_with("nb_") || cle.starts_with("sel_") || *cle == "nb" || *cle == "poids"
        })
        .map(|(cle, valeur)| (cle.clone(), json!(valeur)))
        .collect();
    let reprise_selection = reprise
        .keys()
        .any(|k| k.starts_with("sel_") || (k.starts_with("nb_")));
    let date_reprise = query
        .get("date")
        .and_then(|d| parse_stored_date(d))
        .filter(|d| *d <= aujourd_hui)
        .map(|d| d.to_string());
    if let Some(date) = date_reprise {
        ctx.insert("date_proposee".into(), json!(date));
    }
    ctx.insert("reprise".into(), Value::Object(reprise));
    ctx.insert("reprise_selection".into(), json!(reprise_selection));
    ctx.insert("portees".into(), Value::Array(portees));
    let total_deja: i64 = deja.iter().filter_map(|d| d["nb_sevres"].as_i64()).sum();
    ctx.insert("deja".into(), Value::Array(deja));
    ctx.insert("total_deja".into(), json!(total_deja));
    ctx.insert(
        "ok".into(),
        json!(query.get("ok").and_then(|v| v.parse::<i64>().ok())),
    );
    ctx.insert("cases".into(), Value::Array(cases));
    ctx.insert("causes".into(), json!(causes));
    ctx.insert("salles_truies".into(), Value::Array(salles_truies));
    render(&state, "sevrage_bande.html", Value::Object(ctx))
}

struct Ligne {
    destination: Option<i64>,
    truie_id: i64,
    numero: String,
    mise_bas_id: i64,
    date_mb: String,
    presents: i64,
    sevres: i64,
    poids: Option<f64>,
    case_source: Option<i64>,
}

/// Rapprochement des écarts de comptage (sevrés − présents, par truie) :
/// les porcelets en plus sont pris comme adoptions sur les truies qui en ont
/// en moins ; ce qui manque encore part en perte, ce qui reste en plus est un
/// surplus (nés vifs sous-estimés).
#[derive(Debug, Default, PartialEq)]
struct Rapprochement {
    /// (source, destination, nombre) en indices de lignes.
    adoptions: Vec<(usize, usize, i64)>,
    pertes: Vec<(usize, i64)>,
    surplus: Vec<(usize, i64)>,
}

fn rapprocher(ecarts: &[i64]) -> Rapprochement {
    let mut plus: Vec<(usize, i64)> = ecarts
        .iter()
        .enumerate()
        .filter(|(_, e)| **e > 0)
        .map(|(i, e)| (i, *e))
        .collect();
    let mut moins: Vec<(usize, i64)> = ecarts
        .iter()
        .enumerate()
        .filter(|(_, e)| **e < 0)
        .map(|(i, e)| (i, -*e))
        .collect();
    // Les plus gros écarts d'abord : moins d'adoptions, plus lisibles.
    plus.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    moins.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut r = Rapprochement::default();
    let mut k = 0;
    for (destination, mut besoin) in plus {
        while besoin > 0 && k < moins.len() {
            let pris = besoin.min(moins[k].1);
            r.adoptions.push((moins[k].0, destination, pris));
            besoin -= pris;
            moins[k].1 -= pris;
            if moins[k].1 == 0 {
                k += 1;
            }
        }
        if besoin > 0 {
            r.surplus.push((destination, besoin));
        }
    }
    r.pertes = moins.into_iter().filter(|(_, n)| *n > 0).collect();
    r.pertes.sort_unstable();
    r
}

pub(super) async fn enregistrer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(band_id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let pool = &state.pool;
    let code: String = sqlx::query_scalar("SELECT code FROM bande WHERE id=?")
        .bind(band_id)
        .fetch_optional(pool)
        .await?
        .ok_or(AppError::NotFound)?;
    let date = form_date_or_today(&form, "date")?;
    if date > today_iso() {
        return Err(AppError::Invalid(
            "Le sevrage ne peut pas être daté dans le futur.".into(),
        ));
    }
    let destination = form_i64(&form, "destination");
    let salle_truies = form_i64(&form, "salle_truies");
    let cause = form_text(&form, "cause_ecart");

    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let mut lignes = Vec::new();
    for cle in form.keys().filter(|k| k.starts_with("sel_")) {
        let truie_id: i64 = cle
            .trim_start_matches("sel_")
            .parse()
            .map_err(|_| AppError::Invalid("Truie invalide".into()))?;
        let portee = sqlx::query(
            "SELECT pe.id,pe.date,pe.presents,t.num_travail,COALESCE(e.case_id,t.case_id) AS case_source \
             FROM portee_effectif pe JOIN truie t ON t.id=pe.truie_id JOIN evenement e ON e.id=pe.id \
             WHERE pe.truie_id=? AND pe.bande_id=? AND pe.cloturee=0 AND pe.date<=date('now') \
             ORDER BY pe.date DESC,pe.id DESC LIMIT 1",
        )
        .bind(truie_id)
        .bind(band_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            AppError::Invalid("Une des truies cochées est déjà sevrée : rechargez la page.".into())
        })?;
        let numero: String = portee.try_get("num_travail")?;
        let presents: i64 = portee.try_get("presents")?;
        let sevres = form_i64(&form, &format!("nb_{truie_id}"))
            .filter(|n| *n >= 0)
            .ok_or_else(|| {
                AppError::Invalid(format!("Truie {numero} : nombre de sevrés manquant"))
            })?;
        let date_mb: String = portee.try_get("date")?;
        if date < date_mb {
            return Err(AppError::Invalid(format!(
                "Truie {numero} : le sevrage ne peut pas précéder la mise-bas"
            )));
        }
        let destination_truie = form_i64(&form, &format!("case_{truie_id}")).or(destination);
        lignes.push(Ligne {
            destination: destination_truie,
            truie_id,
            numero,
            mise_bas_id: portee.try_get("id")?,
            date_mb,
            presents,
            sevres,
            poids: form_f64(&form, &format!("poids_{truie_id}")).filter(|p| *p > 0.0 && *p < 30.0),
            case_source: portee
                .try_get::<Option<i64>, _>("case_source")
                .ok()
                .flatten(),
        });
    }
    if lignes.is_empty() {
        return Err(AppError::Invalid(
            "Cochez au moins une truie à sevrer".into(),
        ));
    }
    // Écarts entre comptage et présents : d'abord des adoptions entre truies
    // de la bande (les porcelets en plus viennent de celles qui en ont moins),
    // puis perte pour ce qui manque encore, et correction des nés vifs pour
    // un surplus restant si l'éleveur l'a choisi.
    let ecarts: Vec<i64> = lignes.iter().map(|l| l.sevres - l.presents).collect();
    let plan = rapprocher(&ecarts);
    let manquants: i64 = plan.pertes.iter().map(|(_, n)| n).sum();
    let surplus: i64 = plan.surplus.iter().map(|(_, n)| n).sum();
    if surplus > 0 && form_text(&form, "surplus").as_deref() != Some("corriger_nv") {
        let qui: Vec<String> = plan
            .surplus
            .iter()
            .map(|(i, n)| format!("{} (+{n})", lignes[*i].numero))
            .collect();
        return Err(AppError::Invalid(format!(
            "{surplus} porcelet(s) de plus que les présents sur l’ensemble de la bande ({}). \
             Aucune truie cochée n’en a assez en moins pour expliquer une adoption. Vérifiez le comptage, \
             ou choisissez « Nés vifs sous-estimés : corriger la mise-bas » pour les porcelets en plus.",
            qui.join(", ")
        )));
    }
    if manquants > 0 {
        let cause = cause.as_deref().ok_or_else(|| {
            AppError::Invalid(format!(
                "{manquants} porcelet(s) manquent par rapport aux présents : choisissez la cause de l’écart"
            ))
        })?;
        let connue: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM causeperte WHERE lower(libelle)=lower(?)")
                .bind(cause)
                .fetch_one(&mut *tx)
                .await?;
        if connue == 0 {
            return Err(AppError::Invalid("Choisissez une cause configurée".into()));
        }
    }
    let ecart_total = manquants;
    let total: i64 = lignes.iter().map(|l| l.sevres).sum();
    // Cases de destination : chacune doit être en post-sevrage et avoir la
    // place pour tout ce qu'on y envoie.
    let mut par_case: HashMap<i64, i64> = HashMap::new();
    for l in &lignes {
        if let Some(case_id) = l.destination {
            *par_case.entry(case_id).or_default() += l.sevres;
        }
    }
    let mut salle_de_case: HashMap<i64, i64> = HashMap::new();
    for (case_id, ajout) in &par_case {
        let ligne = sqlx::query(
            "SELECT c.salle_id,c.nom,c.nb_max_porcs FROM casesalle c JOIN salle s ON s.id=c.salle_id WHERE c.id=? \
             AND (lower(COALESCE(s.type,'')) LIKE '%sevr%' OR lower(s.nom) LIKE '%sevr%')",
        )
        .bind(case_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            AppError::Invalid("La destination doit être une case de post-sevrage".into())
        })?;
        let nom: String = ligne.try_get("nom")?;
        let max: Option<i64> = ligne
            .try_get::<Option<i64>, _>("nb_max_porcs")
            .ok()
            .flatten();
        let presents = case_pig_count_conn(&mut tx, *case_id).await?;
        if let Some(max) = max.filter(|m| *m > 0) {
            if presents + ajout > max {
                return Err(AppError::Invalid(format!(
                    "Case {nom} trop petite : {presents} présent(s) + {ajout} sevré(s) pour {max} places. \
                     Utilisez « Répartir automatiquement » ou choisissez une autre case pour certaines truies."
                )));
            }
        }
        salle_de_case.insert(*case_id, ligne.try_get("salle_id")?);
    }

    let mut notes: Vec<Vec<String>> = vec![Vec::new(); lignes.len()];
    for (source, destination, nombre) in &plan.adoptions {
        sqlx::query(
            "INSERT INTO adoptionporcelet(date,source_id,destination_id,nombre,note) VALUES(?,?,?,?,?)",
        )
        .bind(&date)
        .bind(lignes[*source].mise_bas_id)
        .bind(lignes[*destination].mise_bas_id)
        .bind(nombre)
        .bind("Adoption constatée au sevrage")
        .execute(&mut *tx)
        .await?;
        notes[*destination].push(format!("{nombre} adopté(s) de {}", lignes[*source].numero));
        notes[*source].push(format!(
            "{nombre} passé(s) à {}",
            lignes[*destination].numero
        ));
    }
    for (i, nombre) in &plan.pertes {
        let l = &lignes[*i];
        let age = match (parse_stored_date(&date), parse_stored_date(&l.date_mb)) {
            (Some(a), Some(b)) => Some((a - b).num_days()),
            _ => None,
        };
        sqlx::query(
            "INSERT INTO perteporcelet(truie_id,bande_id,age_j,nb,cause,date,evenement_id) VALUES(?,?,?,?,?,?,?)",
        )
        .bind(l.truie_id)
        .bind(band_id)
        .bind(age)
        .bind(nombre)
        .bind(cause.as_deref())
        .bind(&date)
        .bind(l.mise_bas_id)
        .execute(&mut *tx)
        .await?;
        notes[*i].push(format!("{nombre} en perte"));
    }
    for (i, nombre) in &plan.surplus {
        sqlx::query(
            "UPDATE evenement SET nes_vifs=COALESCE(nes_vifs,0)+?,nes_totaux=COALESCE(nes_totaux,0)+? WHERE id=?",
        )
        .bind(nombre)
        .bind(nombre)
        .bind(lignes[*i].mise_bas_id)
        .execute(&mut *tx)
        .await?;
        notes[*i].push(format!("nés vifs corrigés +{nombre}"));
    }
    for (index, l) in lignes.iter().enumerate() {
        // Contrôle final : la vue doit maintenant donner exactement le nombre
        // compté, sinon la base refuserait le sevrage.
        let presents: i64 = sqlx::query_scalar("SELECT presents FROM portee_effectif WHERE id=?")
            .bind(l.mise_bas_id)
            .fetch_one(&mut *tx)
            .await?;
        if presents != l.sevres {
            return Err(AppError::Invalid(format!(
                "Truie {} : effectif incohérent ({presents} présents après correction, {} sevrés). Rechargez la page.",
                l.numero, l.sevres
            )));
        }
        let note = if notes[index].is_empty() {
            "Sevrage de bande".to_string()
        } else {
            format!("Sevrage de bande · {}", notes[index].join(", "))
        };
        sqlx::query(
            "INSERT INTO evenement(type,date,truie_id,bande_id,nb_sevres,poids_moyen,note,suivi_actif) VALUES('sevrage',?,?,?,?,?,?,0)",
        )
        .bind(&date)
        .bind(l.truie_id)
        .bind(band_id)
        .bind(l.sevres)
        .bind(l.poids)
        .bind(note)
        .execute(&mut *tx)
        .await?;
        if let Some((case_id, salle_id)) = l
            .destination
            .and_then(|c| salle_de_case.get(&c).map(|s| (c, *s)))
        {
            if l.sevres > 0 {
                let salle_source: Option<i64> = match l.case_source {
                    Some(source) => {
                        sqlx::query_scalar("SELECT salle_id FROM casesalle WHERE id=?")
                            .bind(source)
                            .fetch_optional(&mut *tx)
                            .await?
                    }
                    None => None,
                };
                sqlx::query(
                    "INSERT INTO transfert(date,espece,bande_id,salle_source_id,salle_dest_id,case_source_id,case_dest_id,nombre,truie_id,note) \
                     VALUES(?,'porc',?,?,?,?,?,?,?,?)",
                )
                .bind(&date)
                .bind(band_id)
                .bind(salle_source)
                .bind(salle_id)
                .bind(l.case_source)
                .bind(case_id)
                .bind(l.sevres)
                .bind(l.truie_id)
                .bind("Sevrage : portée vers le post-sevrage")
                .execute(&mut *tx)
                .await?;
            }
        }
        // Les truies sevrées quittent la maternité vers la salle choisie
        // (verraterie en général) : leur place de maternité se libère.
        if let Some(salle_dest) = salle_truies {
            let salle_source: Option<i64> = sqlx::query_scalar(
                "SELECT COALESCE(t.salle_id,(SELECT c.salle_id FROM casesalle c WHERE c.id=t.case_id)) FROM truie t WHERE t.id=?",
            )
            .bind(l.truie_id)
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO transfert(date,espece,bande_id,salle_source_id,salle_dest_id,case_source_id,nombre,truie_id,note) \
                 VALUES(?,'truie',?,?,?,?,1,?,?)",
            )
            .bind(&date)
            .bind(band_id)
            .bind(salle_source)
            .bind(salle_dest)
            .bind(l.case_source)
            .bind(l.truie_id)
            .bind("Sevrage : sortie de maternité")
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE truie SET salle_id=?,case_id=NULL,updated_at=CURRENT_TIMESTAMP WHERE id=?",
            )
            .bind(salle_dest)
            .bind(l.truie_id)
            .execute(&mut *tx)
            .await?;
        }
        // Sevrage confirmé : la truie sort de la bande et rejoint la file
        // « Prochaine IA », comme au sevrage depuis la maternité.
        sqlx::query("UPDATE truie SET bande_code=NULL,updated_at=CURRENT_TIMESTAMP WHERE id=? AND bande_code=?")
            .bind(l.truie_id)
            .bind(&code)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query(
        "UPDATE bande SET cs_total_sevres=(SELECT CAST(COALESCE(SUM(nb_sevres),0) AS INTEGER) FROM evenement WHERE bande_id=? AND type='sevrage'),updated_at=CURRENT_TIMESTAMP WHERE id=?",
    )
    .bind(band_id)
    .bind(band_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    db::journal(
        pool,
        &session.nom,
        "sevrer",
        "bande",
        &format!(
            "{code} : {} truie(s), {total} porcelet(s), écart {ecart_total}",
            lignes.len()
        ),
        &format!("/bande/{band_id}/sevrage"),
    )
    .await;
    Ok(Redirect::to(&format!("/bande/{band_id}/sevrage?ok={}", lignes.len())).into_response())
}

/// Truie vide : la « mise-bas » enregistrée à 0 né est annulée et la truie
/// sort de la bande (file « Prochaine IA »). Refusé dès qu'un porcelet, une
/// perte, une adoption ou un sevrage est rattaché à cette mise-bas.
pub(super) async fn truie_vide(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path((band_id, mise_bas_id)): Path<(i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let pool = &state.pool;
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let ligne = sqlx::query(
        "SELECT e.truie_id,t.num_travail,b.code,COALESCE(e.nes_totaux,0) AS nt,COALESCE(e.nes_vifs,0) AS nv,\
         COALESCE(pe.pertes,0) AS pertes,COALESCE(pe.adoptes,0) AS adoptes,COALESCE(pe.retires,0) AS retires,pe.sevrage_id \
         FROM evenement e JOIN truie t ON t.id=e.truie_id JOIN bande b ON b.id=e.bande_id \
         LEFT JOIN portee_effectif pe ON pe.id=e.id WHERE e.id=? AND e.bande_id=? AND e.type='mise_bas'",
    )
    .bind(mise_bas_id)
    .bind(band_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let numero: String = ligne.try_get("num_travail")?;
    let code: String = ligne.try_get("code")?;
    let truie_id: i64 = ligne.try_get("truie_id")?;
    let bloquants = [
        ligne.try_get::<i64, _>("nt").unwrap_or(0),
        ligne.try_get::<i64, _>("nv").unwrap_or(0),
        ligne.try_get::<i64, _>("pertes").unwrap_or(0),
        ligne.try_get::<i64, _>("adoptes").unwrap_or(0),
        ligne.try_get::<i64, _>("retires").unwrap_or(0),
    ];
    let sevree = ligne
        .try_get::<Option<i64>, _>("sevrage_id")
        .ok()
        .flatten()
        .is_some();
    let adoptions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM adoptionporcelet WHERE source_id=?1 OR destination_id=?1",
    )
    .bind(mise_bas_id)
    .fetch_one(&mut *tx)
    .await?;
    if bloquants.iter().any(|n| *n != 0) || sevree || adoptions > 0 {
        return Err(AppError::Invalid(format!(
            "Truie {numero} : cette mise-bas a des porcelets, pertes, adoptions ou un sevrage enregistrés — elle ne peut pas être annulée comme « truie vide »."
        )));
    }
    sqlx::query("DELETE FROM soinportee WHERE evenement_id=? AND date_realisee IS NULL")
        .bind(mise_bas_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM evenement WHERE id=? AND type='mise_bas'")
        .bind(mise_bas_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE truie SET bande_code=NULL,updated_at=CURRENT_TIMESTAMP WHERE id=? AND bande_code=?",
    )
    .bind(truie_id)
    .bind(&code)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    db::journal(
        pool,
        &session.nom,
        "truie vide",
        "mise-bas",
        &format!("{code} : mise-bas à 0 né de la truie {numero} annulée, truie sortie de la bande"),
        &format!("/bande/{band_id}/sevrage"),
    )
    .await;
    let retour = form_text(&form, "retour")
        .filter(|r| r.starts_with('/') && !r.starts_with("//"))
        .unwrap_or_else(|| format!("/bande/{band_id}/sevrage"));
    Ok(Redirect::to(&retour).into_response())
}

#[cfg(test)]
mod tests {
    use super::super::documents::tests::{session, state};
    use super::*;

    fn formulaire(champs: &[(&str, &str)]) -> HashMap<String, String> {
        let mut f = HashMap::from([("csrf_token".to_string(), "test".to_string())]);
        f.extend(champs.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        f
    }

    async fn elevage() -> AppState {
        let state = state().await;
        sqlx::raw_sql(
            "INSERT INTO site(id,code) VALUES(1,'S'); \
             INSERT INTO salle(id,site_id,nom,type) VALUES(1,1,'Mater','maternite'),(2,1,'PS','post_sevrage'),(3,1,'Verraterie','verraterie'); \
             INSERT INTO casesalle(id,salle_id,nom) VALUES(1,1,'M1'),(2,2,'P1'); \
             INSERT INTO causeperte(libelle) VALUES('Autre'); \
             INSERT INTO bande(id,code,date_mb,active) VALUES(1,'B1.01',date('now','-28 day'),1); \
             INSERT INTO truie(id,num_travail,bande_code,case_id,salle_id,rang) VALUES(1,'T1','B1.01',1,1,2),(2,'T2','B1.01',1,1,3),(3,'T3','B1.01',1,1,1); \
             INSERT INTO evenement(id,type,date,truie_id,bande_id,nes_totaux,nes_vifs,case_id,suivi_actif) VALUES \
             (10,'mise_bas',date('now','-28 day'),1,1,14,13,1,0),(11,'mise_bas',date('now','-28 day'),2,1,12,12,1,0),(12,'mise_bas',date('now','-28 day'),3,1,10,10,1,0); \
             INSERT INTO perteporcelet(truie_id,bande_id,nb,cause,date,evenement_id) VALUES(1,1,1,'Autre',date('now','-20 day'),10);",
        )
        .execute(&state.pool)
        .await
        .unwrap();
        state
    }

    #[test]
    fn rapprochement_des_ecarts() {
        // Cas B3.19 simplifié : +6 et +2 compensés par des truies à -3,-3,-1,-1.
        let r = rapprocher(&[6, 2, -3, -3, -1, -1, 0]);
        assert!(r.pertes.is_empty() && r.surplus.is_empty());
        let recu = |d: usize| {
            r.adoptions
                .iter()
                .filter(|a| a.1 == d)
                .map(|a| a.2)
                .sum::<i64>()
        };
        assert_eq!((recu(0), recu(1)), (6, 2));
        // Manque net : perte ; surplus net : surplus.
        let r = rapprocher(&[2, -5]);
        assert_eq!(r.adoptions, vec![(1, 0, 2)]);
        assert_eq!(r.pertes, vec![(1, 3)]);
        let r = rapprocher(&[4, -1]);
        assert_eq!(r.surplus, vec![(0, 3)]);
    }

    #[tokio::test]
    async fn adoption_constatee_au_sevrage() -> anyhow::Result<()> {
        let state = elevage().await;
        // T1 : 12 présents, T2 : 12, T3 : 10. On compte T1=9, T2=12, T3=13 :
        // 3 porcelets passés de T1 à T3 sans adoption enregistrée.
        enregistrer(
            State(state.clone()),
            Extension(session()),
            Path(1),
            Form(formulaire(&[
                ("sel_1", "1"),
                ("nb_1", "9"),
                ("sel_2", "1"),
                ("nb_2", "12"),
                ("sel_3", "1"),
                ("nb_3", "13"),
            ])),
        )
        .await?;
        let adoption: (i64, i64, i64) =
            sqlx::query_as("SELECT source_id,destination_id,nombre FROM adoptionporcelet")
                .fetch_one(&state.pool)
                .await?;
        assert_eq!(adoption, (10, 12, 3));
        let sevres: i64 =
            sqlx::query_scalar("SELECT SUM(nb_sevres) FROM evenement WHERE type='sevrage'")
                .fetch_one(&state.pool)
                .await?;
        assert_eq!(sevres, 34);
        let pertes: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM perteporcelet WHERE date=date('now')")
                .fetch_one(&state.pool)
                .await?;
        assert_eq!(pertes, 0);
        Ok(())
    }

    #[tokio::test]
    async fn surplus_net_corrige_les_nes_vifs_sur_demande() -> anyhow::Result<()> {
        let state = elevage().await;
        let champs = [("sel_3", "1"), ("nb_3", "11")];
        assert!(enregistrer(
            State(state.clone()),
            Extension(session()),
            Path(1),
            Form(formulaire(&champs)),
        )
        .await
        .is_err());
        let mut avec = champs.to_vec();
        avec.push(("surplus", "corriger_nv"));
        enregistrer(
            State(state.clone()),
            Extension(session()),
            Path(1),
            Form(formulaire(&avec)),
        )
        .await?;
        let nv: i64 = sqlx::query_scalar("SELECT nes_vifs FROM evenement WHERE id=12")
            .fetch_one(&state.pool)
            .await?;
        assert_eq!(nv, 11);
        Ok(())
    }

    #[tokio::test]
    async fn truie_vide_annule_seulement_une_mise_bas_a_zero() -> anyhow::Result<()> {
        let state = elevage().await;
        sqlx::raw_sql(
            "INSERT INTO truie(id,num_travail,bande_code,rang) VALUES(4,'T4','B1.01',2); \
             INSERT INTO evenement(id,type,date,truie_id,bande_id,nes_totaux,nes_vifs,suivi_actif) VALUES(13,'mise_bas',date('now','-20 day'),4,1,0,0,0);",
        )
        .execute(&state.pool)
        .await?;
        // Une vraie portée (T1, 13 nés vifs) est refusée.
        assert!(truie_vide(
            State(state.clone()),
            Extension(session()),
            Path((1, 10)),
            Form(formulaire(&[])),
        )
        .await
        .is_err());
        truie_vide(
            State(state.clone()),
            Extension(session()),
            Path((1, 13)),
            Form(formulaire(&[])),
        )
        .await?;
        let reste: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM evenement WHERE id=13")
            .fetch_one(&state.pool)
            .await?;
        assert_eq!(reste, 0);
        let bande: Option<String> = sqlx::query_scalar("SELECT bande_code FROM truie WHERE id=4")
            .fetch_one(&state.pool)
            .await?;
        assert_eq!(bande, None);
        let t1: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM evenement WHERE id=10")
            .fetch_one(&state.pool)
            .await?;
        assert_eq!(t1, 1);
        Ok(())
    }

    #[tokio::test]
    async fn ia_sans_bande_rattachee_par_la_date_puis_par_affectation() -> anyhow::Result<()> {
        let state = elevage().await;
        // Bande dont l'IA prévue tombe aujourd'hui (gestation par défaut 115 j).
        sqlx::raw_sql(
            "INSERT INTO bande(id,code,date_mb,active) VALUES(2,'B2.33',date('now','+115 day'),1); \
             INSERT INTO truie(id,num_travail,rang) VALUES(5,'T5',2);",
        )
        .execute(&state.pool)
        .await?;
        let mut conn = state.pool.acquire().await?;
        let aujourd_hui = today_iso();
        assert_eq!(
            bande_pour_ia(&mut conn, 5, &aujourd_hui, 115).await?,
            Some(2)
        );
        // Loin de toute date d'IA prévue : pas de bande inventée.
        assert_eq!(bande_pour_ia(&mut conn, 5, "2020-01-01", 115).await?, None);
        // IA saisie sans bande, puis truie affectée à la bande : rattachée.
        sqlx::query("INSERT INTO evenement(type,date,truie_id,suivi_actif) VALUES('ia',date('now','-3 day'),5,0)")
            .execute(&mut *conn)
            .await?;
        sqlx::query("UPDATE truie SET bande_code='B2.33' WHERE id=5")
            .execute(&mut *conn)
            .await?;
        rattacher_cycle_a_la_bande(&mut conn, 5).await?;
        let bande: Option<i64> =
            sqlx::query_scalar("SELECT bande_id FROM evenement WHERE truie_id=5 AND type='ia'")
                .fetch_one(&mut *conn)
                .await?;
        assert_eq!(bande, Some(2));
        Ok(())
    }

    #[tokio::test]
    async fn ecart_en_perte_sevrage_et_sortie_de_bande() -> anyhow::Result<()> {
        let state = elevage().await;
        // T1 : 13 nés vifs − 1 perte = 12 présents, 10 comptés → écart de 2.
        // T2 : 12 présents, 12 sevrés. T3 non cochée.
        enregistrer(
            State(state.clone()),
            Extension(session()),
            Path(1),
            Form(formulaire(&[
                ("sel_1", "1"),
                ("nb_1", "10"),
                ("poids_1", "7,4"),
                ("sel_2", "1"),
                ("nb_2", "12"),
                ("destination", "2"),
                ("salle_truies", "3"),
                ("cause_ecart", "Autre"),
            ])),
        )
        .await?;
        let sevres: i64 = sqlx::query_scalar(
            "SELECT SUM(nb_sevres) FROM evenement WHERE type='sevrage' AND bande_id=1",
        )
        .fetch_one(&state.pool)
        .await?;
        assert_eq!(sevres, 22);
        let pertes_t1: i64 =
            sqlx::query_scalar("SELECT SUM(nb) FROM perteporcelet WHERE truie_id=1")
                .fetch_one(&state.pool)
                .await?;
        assert_eq!(pertes_t1, 3);
        let poids: Option<f64> = sqlx::query_scalar(
            "SELECT poids_moyen FROM evenement WHERE type='sevrage' AND truie_id=1",
        )
        .fetch_one(&state.pool)
        .await?;
        assert_eq!(poids, Some(7.4));
        assert_eq!(case_pig_count(&state.pool, 2).await?, 22);
        // Les truies sevrées sortent de la bande et de la maternité ; T3 reste.
        let dans_bande: Vec<String> = sqlx::query_scalar(
            "SELECT num_travail FROM truie WHERE bande_code='B1.01' ORDER BY num_travail",
        )
        .fetch_all(&state.pool)
        .await?;
        assert_eq!(dans_bande, vec!["T3".to_string()]);
        let salle_t1: i64 = sqlx::query_scalar("SELECT salle_id FROM truie WHERE id=1")
            .fetch_one(&state.pool)
            .await?;
        assert_eq!(salle_t1, 3);
        // Une truie déjà sevrée ne peut pas l'être deux fois.
        assert!(enregistrer(
            State(state.clone()),
            Extension(session()),
            Path(1),
            Form(formulaire(&[("sel_2", "1"), ("nb_2", "12")])),
        )
        .await
        .is_err());
        Ok(())
    }

    #[tokio::test]
    async fn plus_de_sevres_que_de_presents_ou_ecart_sans_cause_n_ecrivent_rien(
    ) -> anyhow::Result<()> {
        let state = elevage().await;
        for champs in [
            vec![("sel_3", "1"), ("nb_3", "11")],
            vec![("sel_3", "1"), ("nb_3", "8")],
        ] {
            assert!(enregistrer(
                State(state.clone()),
                Extension(session()),
                Path(1),
                Form(formulaire(&champs)),
            )
            .await
            .is_err());
        }
        let ecrits: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM evenement WHERE type='sevrage')+(SELECT COUNT(*) FROM perteporcelet WHERE truie_id=3)",
        )
        .fetch_one(&state.pool)
        .await?;
        assert_eq!(ecrits, 0);
        Ok(())
    }

    /// Les bases héritées de la version Python ont `evenement.suivi_actif`
    /// NOT NULL sans valeur par défaut : toute saisie qui l'omet échoue en
    /// production alors qu'elle passe sur une base de test neuve. Découvert
    /// le 23/09/2026 en sevrant B3.19 sur une copie de la base réelle.
    #[test]
    fn chaque_saisie_d_evenement_renseigne_suivi_actif() {
        for (fichier, source) in [
            ("parity.rs", include_str!("parity.rs")),
            ("mod.rs", include_str!("mod.rs")),
            ("sevrage.rs", include_str!("sevrage.rs")),
            ("adoptions.rs", include_str!("adoptions.rs")),
            ("import_historique.rs", include_str!("import_historique.rs")),
            ("maternite_suivi.rs", include_str!("maternite_suivi.rs")),
            ("ameliorations.rs", include_str!("ameliorations.rs")),
        ] {
            for (n, ligne) in source.lines().enumerate() {
                let fixture = ligne.contains("'2026-")
                    || ligne.contains("'2025-")
                    || ligne.contains("'2999-")
                    || ligne.contains("raw_sql")
                    || ligne.contains("date('now'")
                    || ligne.trim_start().starts_with("//")
                    || ligne.contains("fixture");
                if ligne.contains(&["INSERT INTO evenement", "("].concat()) && !fixture {
                    assert!(
                        ligne.contains("suivi_actif"),
                        "{fichier}:{} : INSERT INTO evenement sans suivi_actif",
                        n + 1
                    );
                }
            }
        }
    }
}
