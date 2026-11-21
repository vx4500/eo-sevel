//! Croissance d'une bande recalée sur les poids réellement relevés.
//!
//! Les poids saisis pour quelques porcs (pesée de la bande, poids au
//! sevrage, porc mort pesé, apports à l'abattoir) disent si la bande est en
//! avance ou en retard sur la courbe de croissance type
//! (`gte_periode::poids_a_lage`). Ce décalage, exprimé en jours, sert à
//! estimer le poids actuel des autres porcs de la bande et la date à
//! laquelle ils atteindront le poids de départ.
//!
//! Sans aucun poids pour la bande, le décalage moyen de l'élevage (tiré des
//! apports à l'abattoir des bandes passées) est utilisé.

use super::*;

/// Âge (jours) auquel la courbe type atteint `kg` : inverse de
/// `gte_periode::poids_a_lage`.
pub(super) fn age_pour_poids(kg: f64) -> f64 {
    // La courbe est croissante : dichotomie sur 0–400 jours.
    let (mut bas, mut haut) = (0.0_f64, 400.0_f64);
    if kg >= super::gte_periode::poids_a_lage(haut) {
        // Au-delà de la courbe (130 kg à 250 j) : prolongée à 0,5 kg/j.
        return 250.0 + (kg - super::gte_periode::poids_a_lage(250.0)) / 0.5;
    }
    for _ in 0..50 {
        let milieu = (bas + haut) / 2.0;
        if super::gte_periode::poids_a_lage(milieu) < kg {
            bas = milieu;
        } else {
            haut = milieu;
        }
    }
    (bas + haut) / 2.0
}

/// Un poids relevé : date, poids vif moyen par porc, nombre de porcs, source.
#[derive(Clone, Debug)]
pub(super) struct Releve {
    pub date: NaiveDate,
    pub kg: f64,
    pub nombre: f64,
    pub source: &'static str,
}

/// Importance d'une source : une pesée de la bande vaut plus qu'un porc mort
/// (souvent plus léger que les autres) ; les porcs vendus sont les plus
/// lourds de la bande.
fn poids_source(source: &str) -> f64 {
    match source {
        "pesee" => 1.0,
        "vente" => 0.7,
        "sevrage" => 0.5,
        _ => 0.3,
    }
}

/// Avance (jours, positif = en avance) de la bande sur la courbe type,
/// moyenne pondérée des relevés (nombre de porcs, source, ancienneté).
pub(super) fn avance_jours(releves: &[Releve], date_mb: NaiveDate, aujourd_hui: NaiveDate) -> Option<f64> {
    let mut somme = 0.0;
    let mut poids_total = 0.0;
    for r in releves {
        let age = (r.date - date_mb).num_days() as f64;
        if age < 14.0 || r.kg <= 0.0 {
            continue;
        }
        let attendu = super::gte_periode::poids_a_lage(age);
        // Un porc mort très en dessous de la courbe (chétif, malade) ne dit
        // rien des autres.
        if r.source == "mort" && r.kg < attendu * 0.6 {
            continue;
        }
        let ecart = (age_pour_poids(r.kg) - age).clamp(-60.0, 60.0);
        let anciennete = (aujourd_hui - r.date).num_days().max(0) as f64;
        let w = poids_source(r.source) * r.nombre.max(1.0).sqrt() * (-anciennete / 60.0).exp();
        somme += ecart * w;
        poids_total += w;
    }
    (poids_total > 0.0).then(|| somme / poids_total)
}

fn rendement(valeur: Option<String>) -> f64 {
    valeur
        .and_then(|v| v.replace(',', ".").parse::<f64>().ok())
        .filter(|v| *v > 30.0 && *v <= 100.0)
        .unwrap_or(76.5)
}

/// Apports à l'abattoir (hors réformes), en poids vif par porc : (bande,
/// date, kg vif par porc, nombre).
async fn ventes(pool: &SqlitePool) -> AppResult<Vec<(i64, NaiveDate, f64, f64)>> {
    let rendement = rendement(sqlx::query_scalar("SELECT valeur FROM parametre WHERE cle='rendement_carcasse_pct'").fetch_optional(pool).await?);
    let lignes: Vec<(Option<String>, Option<i64>, Option<i64>, Option<f64>, Option<String>)> =
        sqlx::query_as("SELECT date,bande_id,nb_porcs,poids_moyen,lots_json FROM venteapport WHERE date IS NOT NULL")
            .fetch_all(pool)
            .await?;
    let mut sortie = Vec::new();
    for (date, bande_id, nb, moyen, lots) in lignes {
        let Some(date) = date.as_deref().and_then(parse_stored_date) else { continue };
        let lots: Vec<Value> = lots
            .as_deref()
            .and_then(|l| serde_json::from_str::<Value>(l).ok())
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        if lots.is_empty() {
            if let (Some(b), Some(n), Some(m)) = (bande_id, nb, moyen) {
                if n > 0 && m > 20.0 {
                    sortie.push((b, date, m * 100.0 / rendement, n as f64));
                }
            }
            continue;
        }
        for lot in lots {
            if lot["reforme"] == true {
                continue;
            }
            let b = lot["bande_id"].as_i64().or(bande_id);
            let n = lot["nb_porcs"].as_f64().unwrap_or(0.0);
            let poids = lot["poids"].as_f64().unwrap_or(0.0);
            if let Some(b) = b {
                if n > 0.0 && poids / n > 20.0 {
                    sortie.push((b, date, poids / n * 100.0 / rendement, n));
                }
            }
        }
    }
    Ok(sortie)
}

/// Tous les poids connus d'une bande.
pub(super) async fn releves(pool: &SqlitePool, bande_id: i64, code: &str) -> AppResult<Vec<Releve>> {
    let mut r = Vec::new();
    let pesees: Vec<(String, f64, i64)> = sqlx::query_as("SELECT date,poids_moyen,nombre FROM pesee_bande WHERE bande_code=?")
        .bind(code)
        .fetch_all(pool)
        .await?;
    for (d, kg, n) in pesees {
        if let Some(date) = parse_stored_date(&d) {
            r.push(Releve { date, kg, nombre: n as f64, source: "pesee" });
        }
    }
    let morts: Vec<(String, f64, i64)> = sqlx::query_as(
        "SELECT date,poids,COALESCE(nombre,1) FROM declarationmort WHERE bande_code=? AND poids IS NOT NULL AND poids>0 AND date IS NOT NULL",
    )
    .bind(code)
    .fetch_all(pool)
    .await?;
    for (d, poids, n) in morts {
        if let Some(date) = parse_stored_date(&d) {
            let n = n.max(1) as f64;
            r.push(Releve { date, kg: poids / n, nombre: n, source: "mort" });
        }
    }
    let sevrages: Vec<(String, f64, Option<i64>)> = sqlx::query_as(
        "SELECT date,poids_moyen,nb_sevres FROM evenement WHERE bande_id=? AND type='sevrage' AND poids_moyen IS NOT NULL AND poids_moyen>0",
    )
    .bind(bande_id)
    .fetch_all(pool)
    .await?;
    for (d, kg, n) in sevrages {
        if let Some(date) = parse_stored_date(&d) {
            r.push(Releve { date, kg, nombre: n.unwrap_or(1) as f64, source: "sevrage" });
        }
    }
    for (b, date, kg, n) in ventes(pool).await? {
        if b == bande_id {
            r.push(Releve { date, kg, nombre: n, source: "vente" });
        }
    }
    r.sort_by_key(|x| x.date);
    Ok(r)
}

/// Décalage moyen de l'élevage et poids vif moyen à la vente, d'après les
/// apports des 12 derniers mois.
async fn reference_elevage(pool: &SqlitePool, aujourd_hui: NaiveDate) -> AppResult<(Option<f64>, Option<f64>)> {
    let naissances: HashMap<i64, NaiveDate> = sqlx::query_as::<_, (i64, Option<String>)>("SELECT id,date_mb FROM bande")
        .fetch_all(pool)
        .await?
        .into_iter()
        .filter_map(|(id, d)| d.as_deref().and_then(parse_stored_date).map(|d| (id, d)))
        .collect();
    let (mut s_ecart, mut s_kg, mut n_total) = (0.0, 0.0, 0.0);
    for (b, date, kg, n) in ventes(pool).await? {
        if (aujourd_hui - date).num_days() > 365 {
            continue;
        }
        let Some(mb) = naissances.get(&b) else { continue };
        let age = (date - *mb).num_days() as f64;
        if !(100.0..=320.0).contains(&age) {
            continue;
        }
        s_ecart += (age_pour_poids(kg) - age).clamp(-60.0, 60.0) * n;
        s_kg += kg * n;
        n_total += n;
    }
    Ok(if n_total > 0.0 { (Some(s_ecart / n_total), Some(s_kg / n_total)) } else { (None, None) })
}

fn libelle_source(s: &str) -> &'static str {
    match s {
        "pesee" => "pesée",
        "vente" => "apport abattoir",
        "sevrage" => "sevrage",
        _ => "porc mort pesé",
    }
}

/// Estimation pour une bande : poids actuel, avance, date de départ au
/// poids cible et écart avec le calendrier de conduite.
pub(super) async fn estimation(pool: &SqlitePool, bande_id: i64, code: &str, date_mb: Option<&str>, poids_cible: Option<f64>) -> AppResult<Option<Value>> {
    let Some(mb) = date_mb.and_then(parse_stored_date) else { return Ok(None) };
    let aujourd_hui = Local::now().date_naive();
    let age = (aujourd_hui - mb).num_days() as f64;
    // Ni avant le sevrage, ni pour une bande pas encore née.
    if age < 21.0 {
        return Ok(None);
    }
    let liste = releves(pool, bande_id, code).await?;
    let (avance_elevage, vente_moyenne) = reference_elevage(pool, aujourd_hui).await?;
    let propre = avance_jours(&liste, mb, aujourd_hui);
    let Some(avance) = propre.or(avance_elevage) else { return Ok(None) };
    let cible = poids_cible.filter(|c| *c > 20.0).or(vente_moyenne).unwrap_or(118.0);
    let poids = super::gte_periode::poids_a_lage(age + avance);
    let age_depart = age_pour_poids(cible) - avance;
    let date_depart = mb + Duration::days(age_depart.round() as i64);
    // Comparé aux bandes habituelles de l'élevage (apports de l'année) :
    // positif = cette bande pousse plus vite.
    let ecart_elevage = propre.zip(avance_elevage).map(|(p, e)| (p - e).round() as i64);
    let ventes_faites = liste.iter().any(|r| r.source == "vente");
    let derniere = liste.iter().filter(|r| (r.date - mb).num_days() >= 14).max_by_key(|r| r.date);
    let mut sources: Vec<String> = Vec::new();
    for s in ["pesee", "vente", "mort", "sevrage"] {
        let n = liste.iter().filter(|r| r.source == s && (r.date - mb).num_days() >= 14).count();
        if n > 0 {
            sources.push(format!("{n} {}{}", libelle_source(s), if n > 1 { "s" } else { "" }));
        }
    }
    Ok(Some(json!({
        "poids": poids.round(),
        "avance_j": avance.round(),
        "cible": cible.round(),
        "date_depart": date_depart.to_string(),
        "depart_passe": date_depart <= aujourd_hui,
        "age_depart": age_depart.round(),
        "ecart_elevage": ecart_elevage,
        "ventes_faites": ventes_faites,
        "cible_bande": poids_cible.is_some_and(|c| c > 20.0),
        "propre": propre.is_some(),
        "base": if propre.is_some() { sources.join(", ") } else { "moyenne des apports de l'élevage (aucun poids pour cette bande)".to_string() },
        "derniere": derniere.map(|r| json!({"date": r.date.to_string(), "kg": (r.kg * 10.0).round() / 10.0, "source": libelle_source(r.source)})),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn l_age_pour_un_poids_inverse_la_courbe() {
        for age in [30.0, 70.0, 120.0, 185.0, 240.0] {
            let kg = super::super::gte_periode::poids_a_lage(age);
            assert!((age_pour_poids(kg) - age).abs() < 0.5, "{age}");
        }
        assert!(age_pour_poids(140.0) > 250.0);
    }

    #[test]
    fn un_porc_plus_lourd_que_la_courbe_met_la_bande_en_avance() {
        let mb = d("2026-06-01");
        // À 130 j la courbe type donne 72 kg ; 80 kg = environ 10 j d'avance.
        let r = vec![Releve { date: d("2026-10-09"), kg: 80.0, nombre: 1.0, source: "pesee" }];
        let a = avance_jours(&r, mb, d("2026-10-10")).unwrap();
        assert!((8.0..13.0).contains(&a), "{a}");
        // Un porc mort chétif (30 kg à 130 j) est ignoré.
        let r = vec![Releve { date: d("2026-10-09"), kg: 30.0, nombre: 1.0, source: "mort" }];
        assert!(avance_jours(&r, mb, d("2026-10-10")).is_none());
        // Plus léger que la courbe : en retard.
        let r = vec![Releve { date: d("2026-10-09"), kg: 60.0, nombre: 5.0, source: "vente" }];
        assert!(avance_jours(&r, mb, d("2026-10-10")).unwrap() < -5.0);
    }

    /// Estimation de chaque bande active sur une copie de la base réelle :
    /// `EO_TEST_BASE=… cargo test -- --ignored croissance_sur_copie --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn croissance_sur_copie() -> anyhow::Result<()> {
        let chemin = std::env::var("EO_TEST_BASE")?;
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect(&format!("sqlite://{chemin}?mode=ro")).await?;
        let (avance, vente) = reference_elevage(&pool, Local::now().date_naive()).await.map_err(|e| anyhow::anyhow!("{e:?}"))?;
        println!("élevage : avance {avance:?} j, vente moyenne {vente:?} kg vif");
        let bandes: Vec<(i64, String, Option<String>, Option<f64>)> =
            sqlx::query_as("SELECT id,code,date_mb,poids_cible FROM bande WHERE active=1 ORDER BY date_mb").fetch_all(&pool).await?;
        for (id, code, mb, cible) in bandes {
            let r = releves(&pool, id, &code).await.map_err(|e| anyhow::anyhow!("{e:?}"))?;
            let e = estimation(&pool, id, &code, mb.as_deref(), cible).await.map_err(|e| anyhow::anyhow!("{e:?}"))?;
            println!("{code} ({mb:?}) : {} relevé(s) {:?}\n   → {e:?}", r.len(), r.iter().map(|x| format!("{} {} {:.0}kg×{}", x.date, x.source, x.kg, x.nombre)).collect::<Vec<_>>());
        }
        Ok(())
    }

    /// Pages Bande et Planning sur une copie : la carte « Poids estimé » se
    /// rend sans erreur. Écrit `bande_<code>.html` et `planning.html` dans
    /// le dossier de la copie.
    #[tokio::test]
    #[ignore]
    async fn pages_croissance_sur_copie() -> anyhow::Result<()> {
        use super::super::documents::tests::session;
        let chemin = std::env::var("EO_TEST_BASE")?;
        let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&chemin).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await?;
        crate::db::init(&pool).await?;
        let state = AppState::new(
            Config { bind: "127.0.0.1:8080".parse()?, db_path: chemin.clone().into(), secure_cookies: false },
            pool.clone(),
            crate::templates::build()?,
        );
        let dossier = std::path::Path::new(&chemin).parent().unwrap().to_path_buf();
        let bandes: Vec<(i64, String)> = sqlx::query_as("SELECT id,code FROM bande WHERE active=1").fetch_all(&pool).await?;
        for (id, code) in bandes {
            let html = super::super::bande_detail(State(state.clone()), Extension(session()), Path(id)).await.map_err(|e| anyhow::anyhow!("{code}: {e:?}"))?.0;
            assert!(html.contains("Poids estimé de la bande"), "{code}");
            std::fs::write(dossier.join(format!("bande_{code}.html")), html)?;
        }
        let html = super::super::planning(State(state.clone()), Extension(session()), Query(HashMap::new())).await.map_err(|e| anyhow::anyhow!("{e:?}"))?.0;
        std::fs::write(dossier.join("planning.html"), html)?;
        Ok(())
    }
}
