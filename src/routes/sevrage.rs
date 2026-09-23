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
        if sevres > presents {
            return Err(AppError::Invalid(format!(
                "Truie {numero} : {sevres} sevrés pour {presents} présents. \
                 Enregistrez d’abord l’adoption des porcelets reçus (page Maternité)."
            )));
        }
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
    let ecart_total: i64 = lignes.iter().map(|l| l.presents - l.sevres).sum();
    if ecart_total > 0 {
        let cause = cause.as_deref().ok_or_else(|| {
            AppError::Invalid(format!(
                "{ecart_total} porcelet(s) manquent par rapport aux présents : choisissez la cause de l’écart"
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

    for l in &lignes {
        let ecart = l.presents - l.sevres;
        if ecart > 0 {
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
            .bind(ecart)
            .bind(cause.as_deref())
            .bind(&date)
            .bind(l.mise_bas_id)
            .execute(&mut *tx)
            .await?;
        }
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
        let note = if ecart > 0 {
            format!("Sevrage de bande · écart de {ecart} enregistré en perte")
        } else {
            "Sevrage de bande".to_string()
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
            ("vocal.rs", include_str!("vocal.rs")),
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
