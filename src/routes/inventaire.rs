//! Inventaire terrain (/inventaire, menu Ma gestion) : le logiciel prépare,
//! à une date donnée, la liste de ce qui devrait être présent (truies et
//! cochettes une par une, verrats, portées sous la mère, porcs par case),
//! rangée par section puis par salle. La feuille s'imprime avec une colonne
//! « Trouvé » vide pour le tour de l'élevage ; les quantités trouvées se
//! reportent ensuite sur la même feuille, qui calcule les écarts.
//!
//! Les quantités attendues sont figées à la création : corriger une donnée
//! après coup ne change pas un inventaire déjà fait.

use super::*;

/// Une ligne de feuille avant enregistrement.
struct Ligne {
    section: &'static str,
    site: Option<String>,
    salle: Option<String>,
    emplacement: Option<String>,
    designation: String,
    detail: Option<String>,
    attendu: i64,
}

/// Rang des sections sur la feuille, dans l'ordre du parcours des animaux.
const SECTIONS: [&str; 8] = [
    "Truies et cochettes",
    "Verrats",
    "Porcelets sous la mère",
    "Post-sevrage",
    "Post-engraissement",
    "Engraissement",
    "Autres porcs",
    "Hors case",
];

fn rang_section(section: &str) -> usize {
    SECTIONS.iter().position(|s| *s == section).unwrap_or(SECTIONS.len())
}

/// Porcs d'une case au soir du jour `date` : même calcul que
/// `case_pig_count` (dernier inventaire de case, puis transferts et morts),
/// limité aux mouvements jusqu'à cette date.
async fn porcs_case_au(pool: &SqlitePool, case_id: i64, date: &str) -> AppResult<i64> {
    let inventaire: Option<(String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT date,nombre,dernier_transfert_id FROM inventairecase WHERE case_id=? AND date<=? ORDER BY date DESC,id DESC LIMIT 1",
    )
    .bind(case_id)
    .bind(date)
    .fetch_optional(pool)
    .await?;
    let (depuis, base, dernier) = inventaire
        .map(|(d, n, dernier)| (Some(d), n, dernier))
        .unwrap_or((None, 0, None));
    let mouvements: i64 = sqlx::query_scalar(
        "SELECT CAST(COALESCE(SUM(CASE WHEN case_dest_id=case_source_id THEN 0 WHEN case_dest_id=?1 THEN COALESCE(nombre,0) \
         WHEN id IN (SELECT transfert_id FROM sortienourrice WHERE transfert_id IS NOT NULL) THEN 0 ELSE -COALESCE(nombre,0) END),0) AS INTEGER) \
         FROM transfert WHERE espece='porc' AND (case_dest_id=?1 OR case_source_id=?1) AND substr(date,1,10)<=?2 \
         AND (?3 IS NULL OR date>?3 OR (date=?3 AND id>?4))",
    )
    .bind(case_id)
    .bind(date)
    .bind(&depuis)
    .bind(dernier)
    .fetch_one(pool)
    .await?;
    let morts: i64 = sqlx::query_scalar(
        "SELECT CAST(COALESCE(SUM(nombre),0) AS INTEGER) FROM declarationmort WHERE case_id=?1 AND date<=?2 AND (?3 IS NULL OR date>?3)",
    )
    .bind(case_id)
    .bind(date)
    .bind(&depuis)
    .fetch_one(pool)
    .await?;
    Ok((base + mouvements - morts).max(0))
}

/// Ce qui devrait être présent le jour `date`.
async fn lignes_attendues(pool: &SqlitePool, date: &str) -> AppResult<Vec<Ligne>> {
    let mut lignes = Vec::new();

    // Truies et cochettes présentes ce jour-là (entrées, ni réformées ni
    // mortes avant), rangées à leur case actuelle.
    let truies: Vec<(String, Option<i64>, Option<String>, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT t.num_travail, t.rang, t.bande_code, si.nom, s.nom, c.nom \
         FROM truie t \
         LEFT JOIN casesalle c ON c.id=t.case_id \
         LEFT JOIN salle s ON s.id=COALESCE(c.salle_id,t.salle_id) \
         LEFT JOIN site si ON si.id=s.site_id \
         WHERE (t.date_entree IS NULL OR t.date_entree<=?1) \
           AND NOT ((t.reformee=1 OR t.statut IN ('reformee','morte')) AND COALESCE(t.date_reforme,'')<=?1) \
         ORDER BY si.id, s.ordre, s.nom, c.id, t.num_travail",
    )
    .bind(date)
    .fetch_all(pool)
    .await?;
    for (numero, rang, bande, site, salle, case) in truies {
        let cochette = rang.unwrap_or(0) == 0;
        lignes.push(Ligne {
            section: if salle.is_some() { "Truies et cochettes" } else { "Hors case" },
            site,
            salle,
            emplacement: case,
            designation: format!("{} {numero}", if cochette { "Cochette" } else { "Truie" }),
            detail: bande.map(|b| format!("Bande {b}")),
            attendu: 1,
        });
    }

    let verrats: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT code, race FROM verrat WHERE actif=1 ORDER BY code")
            .fetch_all(pool)
            .await?;
    let verraterie: Option<(Option<String>, String)> = sqlx::query_as(
        "SELECT si.nom, s.nom FROM salle s LEFT JOIN site si ON si.id=s.site_id \
         WHERE lower(s.type) LIKE '%verrat%' OR lower(s.nom) LIKE '%verrat%' ORDER BY s.ordre LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    for (code, race) in verrats {
        lignes.push(Ligne {
            section: "Verrats",
            site: verraterie.as_ref().and_then(|v| v.0.clone()),
            salle: verraterie.as_ref().map(|v| v.1.clone()),
            emplacement: None,
            designation: format!("Verrat {code}"),
            detail: race,
            attendu: 1,
        });
    }

    // Portées sous la mère : mise-bas faite, pas encore sevrée ce jour-là.
    let portees: Vec<(String, Option<String>, Option<String>, Option<String>, Option<String>, String, i64)> = sqlx::query_as(
        "SELECT t.num_travail, b.code, si.nom, s.nom, c.nom, e.date, \
           COALESCE(e.nes_vifs,0) \
           + COALESCE((SELECT SUM(a.nombre) FROM adoptionporcelet a WHERE a.destination_id=e.id AND a.date<=?1),0) \
           - COALESCE((SELECT SUM(a.nombre) FROM adoptionporcelet a WHERE a.source_id=e.id AND a.date<=?1),0) \
           - COALESCE((SELECT SUM(p.nb) FROM perteporcelet p WHERE p.date<=?1 AND (p.evenement_id=e.id \
               OR (p.evenement_id IS NULL AND p.truie_id=e.truie_id AND p.date>=e.date))),0) \
         FROM evenement e \
         JOIN truie t ON t.id=e.truie_id \
         LEFT JOIN bande b ON b.id=e.bande_id \
         LEFT JOIN casesalle c ON c.id=t.case_id \
         LEFT JOIN salle s ON s.id=COALESCE(c.salle_id,t.salle_id) \
         LEFT JOIN site si ON si.id=s.site_id \
         WHERE e.type='mise_bas' AND e.date<=?1 \
           AND NOT EXISTS (SELECT 1 FROM evenement x WHERE x.truie_id=e.truie_id AND x.type IN ('sevrage','mise_bas') \
                           AND x.id<>e.id AND x.date>=e.date AND x.date<=?1) \
         ORDER BY si.id, s.ordre, s.nom, c.id, t.num_travail",
    )
    .bind(date)
    .fetch_all(pool)
    .await?;
    for (numero, bande, site, salle, case, mise_bas, presents) in portees {
        if presents <= 0 {
            continue;
        }
        lignes.push(Ligne {
            section: "Porcelets sous la mère",
            site,
            salle,
            emplacement: case,
            designation: format!("Portée de {numero}"),
            detail: Some(format!(
                "Née le {}{}",
                mise_bas.get(..10).map(|d| format!("{}/{}", &d[8..10], &d[5..7])).unwrap_or(mise_bas.clone()),
                bande.map(|b| format!(" · bande {b}")).unwrap_or_default()
            )),
            attendu: presents,
        });
    }

    // Porcs sevrés, case par case.
    let cases: Vec<(i64, String, Option<String>, String, String)> = sqlx::query_as(
        "SELECT c.id, c.nom, si.nom, s.nom, COALESCE(s.type,'') \
         FROM casesalle c JOIN salle s ON s.id=c.salle_id LEFT JOIN site si ON si.id=s.site_id \
         ORDER BY si.id, s.ordre, s.nom, c.id",
    )
    .fetch_all(pool)
    .await?;
    for (case_id, case, site, salle, type_salle) in cases {
        let (_, stade) = super::salles::stade(&type_salle, &salle);
        if stade == "Maternité" || stade == "Gestante" || stade == "Verraterie" {
            continue;
        }
        let porcs = porcs_case_au(pool, case_id, date).await?;
        if porcs <= 0 {
            continue;
        }
        let bande: Option<String> = sqlx::query_scalar(
            "SELECT b.code FROM transfert t JOIN bande b ON b.id=t.bande_id \
             WHERE t.case_dest_id=? AND substr(t.date,1,10)<=? ORDER BY t.date DESC,t.id DESC LIMIT 1",
        )
        .bind(case_id)
        .bind(date)
        .fetch_optional(pool)
        .await?;
        lignes.push(Ligne {
            section: match stade {
                "Post-sevrage" | "Post-engraissement" | "Engraissement" => stade,
                _ => "Autres porcs",
            },
            site,
            emplacement: (case != salle).then_some(case),
            salle: Some(salle),
            designation: "Porcs".into(),
            detail: bande.map(|b| format!("Bande {b}")),
            attendu: porcs,
        });
    }

    // Tri stable : l'ordre des requêtes (site, salle, case) est gardé à
    // l'intérieur de chaque section.
    lignes.sort_by_key(|l| rang_section(l.section));
    Ok(lignes)
}

pub(super) async fn liste(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
) -> AppResult<Html<String>> {
    let inventaires: Vec<(i64, String, Option<String>, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT i.id, i.date, i.note, COUNT(l.id), COUNT(l.trouve), \
           CAST(COALESCE(SUM(l.attendu),0) AS INTEGER), \
           CAST(COALESCE(SUM(CASE WHEN l.trouve IS NOT NULL THEN ABS(l.trouve-l.attendu) END),0) AS INTEGER) \
         FROM inventaire i LEFT JOIN inventaire_ligne l ON l.inventaire_id=i.id \
         GROUP BY i.id ORDER BY i.date DESC, i.id DESC",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut ctx = context(&session);
    ctx.insert("aujourdhui".into(), json!(today_iso()));
    ctx.insert(
        "inventaires".into(),
        Value::Array(
            inventaires
                .into_iter()
                .map(|(id, date, note, lignes, comptees, attendu, ecarts)| {
                    json!({"id": id, "date": date, "note": note, "lignes": lignes,
                           "comptees": comptees, "attendu": attendu, "ecarts": ecarts})
                })
                .collect(),
        ),
    );
    render(&state, "inventaire.html", Value::Object(ctx))
}

pub(super) async fn creer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let date = form_date_or_today(&form, "date")?;
    if date > today_iso() {
        return Err(AppError::Invalid("La date de l'inventaire ne peut pas être dans le futur".into()));
    }
    let lignes = lignes_attendues(&state.pool, &date).await?;
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let id = sqlx::query("INSERT INTO inventaire(date,note,cree_par) VALUES(?,?,?)")
        .bind(&date)
        .bind(form_text(&form, "note"))
        .bind(&session.identifiant)
        .execute(&mut *tx)
        .await?
        .last_insert_rowid();
    for (ordre, l) in lignes.iter().enumerate() {
        sqlx::query(
            "INSERT INTO inventaire_ligne(inventaire_id,ordre,section,site,salle,emplacement,designation,detail,attendu) \
             VALUES(?,?,?,?,?,?,?,?,?)",
        )
        .bind(id)
        .bind(ordre as i64)
        .bind(l.section)
        .bind(&l.site)
        .bind(&l.salle)
        .bind(&l.emplacement)
        .bind(&l.designation)
        .bind(&l.detail)
        .bind(l.attendu)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    db::journal(&state.pool, &session.nom, "créer", "inventaire", &format!("{date} · {} lignes", lignes.len()), "/inventaire").await;
    Ok(Redirect::to(&format!("/inventaire/{id}")).into_response())
}

pub(super) async fn feuille(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let (date, note): (String, Option<String>) =
        sqlx::query_as("SELECT date, note FROM inventaire WHERE id=?")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or(AppError::NotFound)?;
    #[allow(clippy::type_complexity)]
    let lignes: Vec<(i64, String, Option<String>, Option<String>, Option<String>, String, Option<String>, i64, Option<i64>, Option<String>)> =
        sqlx::query_as(
            "SELECT id,section,site,salle,emplacement,designation,detail,attendu,trouve,remarque \
             FROM inventaire_ligne WHERE inventaire_id=? ORDER BY ordre",
        )
        .bind(id)
        .fetch_all(&state.pool)
        .await?;
    let plusieurs_sites = lignes
        .iter()
        .filter_map(|l| l.2.as_deref())
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        > 1;
    let mut sections: Vec<Value> = Vec::new();
    for (ligne_id, section, site, salle, emplacement, designation, detail, attendu, trouve, remarque) in lignes {
        if sections.last().and_then(|s| s["nom"].as_str()) != Some(section.as_str()) {
            sections.push(json!({"nom": section, "lignes": [], "attendu": 0, "trouve": 0, "comptees": 0}));
        }
        let courante = sections.last_mut().expect("section ajoutée juste au-dessus");
        courante["attendu"] = json!(courante["attendu"].as_i64().unwrap_or(0) + attendu);
        if let Some(t) = trouve {
            courante["trouve"] = json!(courante["trouve"].as_i64().unwrap_or(0) + t);
            courante["comptees"] = json!(courante["comptees"].as_i64().unwrap_or(0) + 1);
        }
        let lieu = [
            site.filter(|_| plusieurs_sites),
            salle,
            emplacement,
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        if let Some(liste) = courante["lignes"].as_array_mut() {
            liste.push(json!({
                "id": ligne_id, "lieu": lieu, "designation": designation, "detail": detail,
                "attendu": attendu, "trouve": trouve, "ecart": trouve.map(|t| t - attendu), "remarque": remarque,
            }));
        }
    }
    for s in &mut sections {
        let n = s["lignes"].as_array().map_or(0, Vec::len) as i64;
        s["complete"] = json!(s["comptees"].as_i64() == Some(n));
    }
    let mut ctx = context(&session);
    ctx.insert("inventaire".into(), json!({"id": id, "date": date, "note": note}));
    ctx.insert("sections".into(), Value::Array(sections));
    ctx.insert("ok".into(), json!(query.contains_key("ok")));
    render(&state, "inventaire_feuille.html", Value::Object(ctx))
}

pub(super) async fn enregistrer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM inventaire_ligne WHERE inventaire_id=?")
        .bind(id)
        .fetch_all(&state.pool)
        .await?;
    if ids.is_empty() {
        return Err(AppError::NotFound);
    }
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    for ligne in ids {
        let trouve = match form_text(&form, &format!("trouve_{ligne}")) {
            Some(texte) => Some(
                texte
                    .parse::<i64>()
                    .ok()
                    .filter(|n| *n >= 0)
                    .ok_or_else(|| AppError::Invalid(format!("Quantité trouvée invalide : « {texte} »")))?,
            ),
            None => None,
        };
        sqlx::query("UPDATE inventaire_ligne SET trouve=?, remarque=? WHERE id=? AND inventaire_id=?")
            .bind(trouve)
            .bind(form_text(&form, &format!("remarque_{ligne}")))
            .bind(ligne)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(note) = form.get("note") {
        sqlx::query("UPDATE inventaire SET note=? WHERE id=?")
            .bind(Some(note.trim()).filter(|n| !n.is_empty()))
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    db::journal(&state.pool, &session.nom, "compter", "inventaire", &id.to_string(), &format!("/inventaire/{id}")).await;
    Ok(Redirect::to(&format!("/inventaire/{id}?ok=1")).into_response())
}

pub(super) async fn supprimer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query("DELETE FROM inventaire_ligne WHERE inventaire_id=?").bind(id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM inventaire WHERE id=?").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    db::journal(&state.pool, &session.nom, "supprimer", "inventaire", &id.to_string(), "/inventaire").await;
    Ok(Redirect::to("/inventaire").into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn base() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE site(id INTEGER PRIMARY KEY, code TEXT, nom TEXT);
             CREATE TABLE salle(id INTEGER PRIMARY KEY, site_id INTEGER, nom TEXT, type TEXT, ordre INTEGER);
             CREATE TABLE casesalle(id INTEGER PRIMARY KEY, salle_id INTEGER, nom TEXT);
             CREATE TABLE bande(id INTEGER PRIMARY KEY, code TEXT);
             CREATE TABLE truie(id INTEGER PRIMARY KEY, num_travail TEXT, rang INTEGER, bande_code TEXT, statut TEXT,
               reformee INTEGER DEFAULT 0, date_reforme TEXT, date_entree TEXT, salle_id INTEGER, case_id INTEGER);
             CREATE TABLE verrat(id INTEGER PRIMARY KEY, code TEXT, race TEXT, actif INTEGER);
             CREATE TABLE evenement(id INTEGER PRIMARY KEY, type TEXT, date TEXT, truie_id INTEGER, bande_id INTEGER, nes_vifs INTEGER);
             CREATE TABLE adoptionporcelet(id INTEGER PRIMARY KEY, date TEXT, source_id INTEGER, destination_id INTEGER, nombre INTEGER);
             CREATE TABLE perteporcelet(id INTEGER PRIMARY KEY, truie_id INTEGER, bande_id INTEGER, nb INTEGER, date TEXT, evenement_id INTEGER);
             CREATE TABLE inventairecase(id INTEGER PRIMARY KEY, case_id INTEGER, date TEXT, nombre INTEGER, dernier_transfert_id INTEGER);
             CREATE TABLE transfert(id INTEGER PRIMARY KEY, date TEXT, espece TEXT, bande_id INTEGER, case_source_id INTEGER, case_dest_id INTEGER, nombre INTEGER);
             CREATE TABLE sortienourrice(transfert_id INTEGER);
             CREATE TABLE declarationmort(id INTEGER PRIMARY KEY, case_id INTEGER, date TEXT, nombre INTEGER);
             INSERT INTO site VALUES(1,'35DA9','Basse Chevrie');
             INSERT INTO salle VALUES(1,1,'Verraterie','verraterie',0),(2,1,'Mater 1','maternite',1),(3,1,'Engraissement 1','engraissement',2);
             INSERT INTO casesalle VALUES(10,1,'Case V'),(20,2,'Case 1'),(30,3,'Case 1');
             INSERT INTO bande VALUES(7,'B3.14');
             INSERT INTO truie VALUES(1,'500711',3,'B2.33','active',0,NULL,'2025-01-01',2,20),
                                     (2,'600001',0,NULL,'active',0,NULL,'2026-10-01',1,10),
                                     (3,'400100',5,NULL,'reformee',1,'2026-09-01','2024-01-01',NULL,NULL);
             INSERT INTO verrat VALUES(1,'PIE','Piétrain',1);
             INSERT INTO evenement VALUES(1,'mise_bas','2026-10-01',1,7,12);
             INSERT INTO perteporcelet VALUES(1,1,7,1,'2026-10-03',1);
             INSERT INTO transfert VALUES(1,'2026-08-01','porc',7,NULL,30,100),(2,'2026-10-26','porc',7,30,NULL,66);
             INSERT INTO declarationmort VALUES(1,30,'2026-09-01',4);",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn feuille_a_une_date_donnee() {
        let pool = base().await;
        let avant = lignes_attendues(&pool, "2026-10-02").await.unwrap();
        let resume: Vec<_> = avant.iter().map(|l| (l.section, l.designation.as_str(), l.attendu)).collect();
        assert_eq!(
            resume,
            vec![
                ("Truies et cochettes", "Cochette 600001", 1),
                ("Truies et cochettes", "Truie 500711", 1),
                ("Verrats", "Verrat PIE", 1),
                ("Porcelets sous la mère", "Portée de 500711", 12),
                ("Engraissement", "Porcs", 96),
            ]
        );
        // Après le départ du 26/10 et la perte du 03/10.
        let apres = lignes_attendues(&pool, "2026-10-27").await.unwrap();
        assert_eq!(apres.iter().find(|l| l.designation == "Porcs").map(|l| l.attendu), Some(30));
        assert_eq!(apres.iter().find(|l| l.section == "Porcelets sous la mère").map(|l| l.attendu), Some(11));
        // La truie réformée le 01/09 figure encore sur un inventaire du mois d'août.
        let aout = lignes_attendues(&pool, "2026-08-15").await.unwrap();
        assert!(aout.iter().any(|l| l.designation == "Truie 400100"));
        assert!(!aout.iter().any(|l| l.designation == "Cochette 600001"));
    }

    /// EO_TEST_BASE=copie.db cargo test --bin eo-sevel -- --ignored inventaire_sur_copie --nocapture
    #[tokio::test]
    #[ignore]
    async fn inventaire_sur_copie() {
        let pool = SqlitePool::connect(&format!("sqlite://{}?mode=ro", std::env::var("EO_TEST_BASE").unwrap())).await.unwrap();
        let date = std::env::var("EO_TEST_DATE").unwrap_or_else(|_| today_iso());
        for l in lignes_attendues(&pool, &date).await.unwrap() {
            println!("{} | {} | {} | {} | {} | {}", l.section, l.salle.unwrap_or_default(), l.emplacement.unwrap_or_default(), l.designation, l.detail.unwrap_or_default(), l.attendu);
        }
    }

    #[test]
    fn les_pages_se_rendent() {
        let env = crate::templates::build().expect("gabarits");
        let session = json!({"csrf": "x", "peut_modifier": true, "pages": [], "identifiant": "test"});
        env.get_template("inventaire.html").unwrap().render(json!({
            "session": session, "aujourdhui": "2026-10-27",
            "inventaires": [{"id": 1, "date": "2026-10-27", "note": "Tour du matin", "lignes": 5, "comptees": 2, "attendu": 110, "ecarts": 1}]
        })).expect("inventaire.html");
        env.get_template("inventaire_feuille.html").unwrap().render(json!({
            "session": session, "inventaire": {"id": 1, "date": "2026-10-27", "note": null},
            "sections": [{"nom": "Engraissement", "attendu": 96, "trouve": 95, "comptees": 1, "complete": true,
              "lignes": [{"id": 3, "lieu": "Engraissement 1 · Case 1", "designation": "Porcs", "detail": "Bande B3.14",
                          "attendu": 96, "trouve": 95, "ecart": -1, "remarque": null}]}]
        })).expect("inventaire_feuille.html");
    }
}
