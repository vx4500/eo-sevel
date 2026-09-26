use super::documents::tests::{session, state};
use super::*;

fn form(fields: &[(&str, &str)]) -> HashMap<String, String> {
    let mut result = HashMap::from([("csrf_token".into(), "test".into())]);
    result.extend(fields.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    result
}

async fn farm() -> AppState {
    let state = state().await;
    sqlx::raw_sql("INSERT INTO site(id,code) VALUES(1,'S'); INSERT INTO salle(id,site_id,nom) VALUES(1,1,'R'); INSERT INTO casesalle(id,salle_id,nom) VALUES(1,1,'C1'),(2,1,'C2'); INSERT INTO bande(id,code) VALUES(1,'B'); INSERT INTO truie(id,num_travail,bande_code,case_id,salle_id) VALUES(1,'T1','B',1,1),(2,'T2','B',1,1); INSERT INTO produitpharmacie(produit,stock_actuel,unite) VALUES('A',100,'doses'),('B',100,'doses');")
        .execute(&state.pool).await.unwrap();
    state
}

fn treatment(product: &str, end: &str) -> HashMap<String, String> {
    form(&[
        ("type", "traitement"),
        ("truie_id", "1"),
        ("date", "2026-09-01"),
        ("date_fin", end),
        ("produit", product),
    ])
}

async fn stock(state: &AppState, product: &str) -> f64 {
    sqlx::query_scalar("SELECT CAST(stock_actuel AS REAL) FROM produitpharmacie WHERE produit=?")
        .bind(product)
        .fetch_one(&state.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn traitement_correction_produit_duree_et_suppression() -> anyhow::Result<()> {
    let state = farm().await;
    evenement_ajouter(
        State(state.clone()),
        Extension(session()),
        Form(treatment("A", "2026-09-18")),
    )
    .await?;
    assert_eq!(stock(&state, "A").await, 82.0);
    evenement_modifier(
        State(state.clone()),
        Extension(session()),
        Path(1),
        Form(treatment("A", "2026-09-01")),
    )
    .await?;
    assert_eq!(stock(&state, "A").await, 99.0);
    evenement_modifier(
        State(state.clone()),
        Extension(session()),
        Path(1),
        Form(treatment("B", "2026-09-03")),
    )
    .await?;
    assert_eq!(stock(&state, "A").await, 100.0);
    assert_eq!(stock(&state, "B").await, 97.0);
    evenement_supprimer(
        State(state.clone()),
        Extension(session()),
        Path(1),
        Form(form(&[])),
    )
    .await?;
    assert_eq!(stock(&state, "B").await, 100.0);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mouvementpharmacie")
        .fetch_one(&state.pool)
        .await?;
    assert_eq!(count, 0);
    Ok(())
}

#[tokio::test]
async fn traitement_stock_insuffisant_ne_restitue_pas_des_doses_inexistantes() -> anyhow::Result<()>
{
    let state = farm().await;
    sqlx::query("UPDATE produitpharmacie SET stock_actuel=2 WHERE produit='A'")
        .execute(&state.pool)
        .await?;
    evenement_ajouter(
        State(state.clone()),
        Extension(session()),
        Form(treatment("A", "2026-09-18")),
    )
    .await?;
    assert_eq!(stock(&state, "A").await, 0.0);
    evenement_supprimer(
        State(state.clone()),
        Extension(session()),
        Path(1),
        Form(form(&[])),
    )
    .await?;
    assert_eq!(stock(&state, "A").await, 2.0);
    Ok(())
}

#[tokio::test]
async fn traitement_erreur_pharmacie_annule_aussi_evenement() -> anyhow::Result<()> {
    let state = farm().await;
    sqlx::query("CREATE TRIGGER refuser_sortie BEFORE INSERT ON mouvementpharmacie BEGIN SELECT RAISE(ABORT,'test stock indisponible'); END").execute(&state.pool).await?;
    assert!(evenement_ajouter(
        State(state.clone()),
        Extension(session()),
        Form(treatment("A", "2026-09-18"))
    )
    .await
    .is_err());
    assert_eq!(stock(&state, "A").await, 100.0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM evenement")
            .fetch_one(&state.pool)
            .await?,
        0
    );
    Ok(())
}

#[tokio::test]
async fn traitement_date_invalide_et_inventaire_preserves() -> anyhow::Result<()> {
    let state = farm().await;
    assert!(evenement_ajouter(
        State(state.clone()),
        Extension(session()),
        Form(treatment("A", "2026-08-31"))
    )
    .await
    .is_err());
    evenement_ajouter(
        State(state.clone()),
        Extension(session()),
        Form(treatment("A", "2026-09-18")),
    )
    .await?;
    assert!(evenement_modifier(
        State(state.clone()),
        Extension(session()),
        Path(1),
        Form(treatment("A", "2026-08-31"))
    )
    .await
    .is_err());
    assert_eq!(stock(&state, "A").await, 82.0);
    pharmacie_mouvement(
        State(state.clone()),
        Extension(session()),
        Form(form(&[
            ("produit", "A"),
            ("type", "inventaire"),
            ("quantite", "40"),
        ])),
    )
    .await?;
    // A note-only edit never touches an inventory or the original stock movement.
    let mut edit = treatment("A", "2026-09-18");
    edit.insert("note".into(), "Observation corrigée".into());
    evenement_modifier(
        State(state.clone()),
        Extension(session()),
        Path(1),
        Form(edit),
    )
    .await?;
    assert!(evenement_supprimer(
        State(state.clone()),
        Extension(session()),
        Path(1),
        Form(form(&[]))
    )
    .await
    .is_err());
    assert_eq!(stock(&state, "A").await, 40.0);
    evenement_supprimer(
        State(state.clone()),
        Extension(session()),
        Path(1),
        Form(form(&[("stock_manuel", "1")])),
    )
    .await?;
    assert_eq!(stock(&state, "A").await, 40.0);
    Ok(())
}

#[tokio::test]
async fn traitement_ancien_ne_subit_pas_une_seconde_sortie() -> anyhow::Result<()> {
    let state = farm().await;
    sqlx::query("INSERT INTO evenement(id,type,date,date_fin,truie_id,produit) VALUES(1,'traitement','2026-09-01','2026-09-18',1,'A')").execute(&state.pool).await?;
    assert!(evenement_modifier(
        State(state.clone()),
        Extension(session()),
        Path(1),
        Form(treatment("A", "2026-09-03"))
    )
    .await
    .is_err());
    assert_eq!(stock(&state, "A").await, 100.0);
    let mut edit = treatment("A", "2026-09-03");
    edit.insert("stock_manuel".into(), "1".into());
    evenement_modifier(
        State(state.clone()),
        Extension(session()),
        Path(1),
        Form(edit),
    )
    .await?;
    assert_eq!(stock(&state, "A").await, 100.0);
    Ok(())
}

fn weaning(selected: &str, distribution: &str) -> HashMap<String, String> {
    form(&[
        ("type", "sevrage"),
        ("bande_id", "1"),
        ("date", "2026-09-01"),
        ("sevrage_truies", selected),
        ("sevrage_distribution", distribution),
    ])
}

#[tokio::test]
async fn sevrage_plusieurs_truies_sans_capacite_et_sans_double_comptage() -> anyhow::Result<()> {
    let state = farm().await;
    let request = weaning(
        r#"[{"id":1,"nb":3},{"id":2,"nb":4}]"#,
        r#"[{"case_id":2,"nombre":7}]"#,
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        parity::saisie_rapide(
            State(state.clone()),
            Extension(session()),
            Form(request.clone()),
        ),
    )
    .await??;
    assert_eq!(case_pig_count(&state.pool, 2).await?, 7);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT cs_total_sevres FROM bande WHERE id=1")
            .fetch_one(&state.pool)
            .await?,
        7
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT SUM(nb_sevres) FROM evenement WHERE type='sevrage'")
            .fetch_one(&state.pool)
            .await?,
        7
    );
    assert!(
        parity::saisie_rapide(State(state.clone()), Extension(session()), Form(request))
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT cs_total_sevres FROM bande WHERE id=1")
            .fetch_one(&state.pool)
            .await?,
        7
    );
    Ok(())
}

#[tokio::test]
async fn sevrage_invalide_et_case_inconnue_n_ecrivent_rien() -> anyhow::Result<()> {
    let state = farm().await;
    for request in [
        weaning(r#"[{"id":1,"nb":-3}]"#, r#"[{"case_id":2,"nombre":-3}]"#),
        weaning(
            r#"[{"id":1,"nb":3},{"id":1,"nb":3}]"#,
            r#"[{"case_id":2,"nombre":6}]"#,
        ),
        weaning(r#"[{"id":1,"nb":3}]"#, r#"[{"case_id":999,"nombre":3}]"#),
    ] {
        assert!(
            parity::saisie_rapide(State(state.clone()), Extension(session()), Form(request))
                .await
                .is_err()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM evenement")
                .fetch_one(&state.pool)
                .await?,
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT bande_code FROM truie WHERE id=1")
                .fetch_one(&state.pool)
                .await?,
            "B"
        );
    }
    Ok(())
}

#[tokio::test]
async fn transferts_dates_et_suppression_historique_preservent_la_case() -> anyhow::Result<()> {
    let state = farm().await;
    let today = today_iso();
    let future = (Local::now().date_naive() + Duration::days(1)).to_string();
    let past = (Local::now().date_naive() - Duration::days(1)).to_string();
    let request = |date: &str| form(&[("date", date), ("case_dest_id", "2"), ("truie_1", "1")]);
    assert!(transferts_truies(
        State(state.clone()),
        Extension(session()),
        Form(request(&future))
    )
    .await
    .is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT case_id FROM truie WHERE id=1")
            .fetch_one(&state.pool)
            .await?,
        1
    );
    transferts_truies(
        State(state.clone()),
        Extension(session()),
        Form(request(&today)),
    )
    .await?;
    assert!(transferts_truies(
        State(state.clone()),
        Extension(session()),
        Form(request(&past))
    )
    .await
    .is_err());
    let historic = sqlx::query("INSERT INTO transfert(date,espece,truie_id,case_source_id,case_dest_id,nombre) VALUES(?,'truie',1,2,1,1)").bind(&past).execute(&state.pool).await?.last_insert_rowid();
    transfert_supprimer(
        State(state.clone()),
        Extension(session()),
        Path(historic),
        Form(form(&[])),
    )
    .await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT case_id FROM truie WHERE id=1")
            .fetch_one(&state.pool)
            .await?,
        2
    );
    transfert_supprimer(
        State(state.clone()),
        Extension(session()),
        Path(1),
        Form(form(&[])),
    )
    .await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT case_id FROM truie WHERE id=1")
            .fetch_one(&state.pool)
            .await?,
        1
    );
    Ok(())
}

#[tokio::test]
async fn tableau_de_bord_delai_depuis_fin_traitement() -> anyhow::Result<()> {
    let state = state().await;
    sqlx::raw_sql(include_str!("../../migrations/0003_portee_effectif.sql"))
        .execute(&state.pool)
        .await?;
    sqlx::raw_sql("INSERT INTO truie(id,num_travail) VALUES(1,'T1'); INSERT INTO evenement(type,date,date_fin,truie_id,produit,delai_attente) VALUES('traitement',date('now','-20 days'),date('now','-1 day'),1,'ALERTE_FIN_TRAITEMENT',5)").execute(&state.pool).await?;
    let response = dashboard(State(state), Extension(session())).await?;
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
    let html = String::from_utf8(body.to_vec())?;
    assert!(html.contains("ALERTE_FIN_TRAITEMENT"));
    let end = (chrono::Utc::now().date_naive() + Duration::days(4))
        .format("%d/%m/%Y")
        .to_string();
    assert!(
        html.replace("&#x2f;", "/").contains(&end),
        "Date attendue : {end}"
    );
    Ok(())
}

#[tokio::test]
async fn supprimer_une_case_occupee_envoie_tout_vers_la_case_choisie() -> anyhow::Result<()> {
    let state = farm().await;
    sqlx::raw_sql(include_str!("../../migrations/0012_observation_comportement.sql")).execute(&state.pool).await?;
    // Case 1 : truies T1 et T2 + 10 porcs ; case 2 : 5 porcs.
    sqlx::raw_sql(
        "INSERT INTO inventairecase(case_id,date,nombre) VALUES(1,'2026-09-01',10),(2,'2026-09-01',5); \
         INSERT INTO declarationmort(horodatage,date,case_id,nombre) VALUES('2026-09-02','2026-09-02',1,1);",
    )
    .execute(&state.pool)
    .await?;
    let avant = case_pig_count(&state.pool, 1).await?;
    assert_eq!(avant, 9);
    // Sans case de reprise : refusé.
    assert!(structure_case_supprimer(State(state.clone()), Extension(session()), Path(1), Form(form(&[]))).await.is_err());
    structure_case_supprimer(State(state.clone()), Extension(session()), Path(1), Form(form(&[("case_cible_id", "2")]))).await?;
    assert_eq!(case_pig_count(&state.pool, 2).await?, 14, "9 repris + 5 présents");
    let truies: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM truie WHERE case_id=2").fetch_one(&state.pool).await?;
    assert_eq!(truies, 2);
    let pertes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM declarationmort WHERE case_id=2").fetch_one(&state.pool).await?;
    assert_eq!(pertes, 1, "l'historique suit");
    let reste: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM casesalle WHERE id=1").fetch_one(&state.pool).await?;
    assert_eq!(reste, 0);
    Ok(())
}
