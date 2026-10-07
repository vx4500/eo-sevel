//! Consommation électrique : jour (courbe de puissance et kWh par heure),
//! semaine, mois et année (kWh par jour ou par mois), par compteur Zigbee.
//!
//! Les kWh se calculent en intégrant la puissance relevée toutes les deux
//! minutes environ (`zigbee_mesure`) ; contrôlé contre les relevés d'index
//! du 05/10 : 136,0 kWh calculés pour 136,2 relevés. Chaque journée finie
//! est gardée dans `elec_jour`, qui survit à la purge des mesures (90 jours).

use super::*;
use chrono::{Datelike, TimeZone};

/// Un trou de plus de 10 min entre deux mesures n'est pas compté.
const TROU_MAX_S: i64 = 600;

/// Compteurs : (ieee, nom affiché, grandeur de puissance historisée).
async fn compteurs(pool: &SqlitePool) -> AppResult<Vec<(String, String, String)>> {
    let lignes: Vec<(String, String, Option<String>, i64)> = sqlx::query_as(
        "SELECT a.ieee, a.nom, \
           (SELECT c.nom FROM ipx_point p JOIN compteur_energie c ON c.id=p.compteur_id WHERE p.hote='zigbee' AND p.chemin=a.ieee||'/energy' LIMIT 1), \
           EXISTS(SELECT 1 FROM zigbee_mesure m WHERE m.ieee=a.ieee AND m.cle='total_power') \
         FROM zigbee_appareil a WHERE a.categorie='compteur' AND a.present=1 ORDER BY a.nom COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await?;
    Ok(lignes
        .into_iter()
        .map(|(ieee, nom, releve, total)| {
            let nom = releve.map(|r| r.trim_start_matches("Électricité — ").to_string()).unwrap_or(nom);
            (ieee, nom, if total != 0 { "total_power".into() } else { "power".into() })
        })
        .collect())
}

async fn mesures(pool: &SqlitePool, ieee: &str, cle: &str, t1: i64, t2: i64) -> AppResult<Vec<(i64, f64)>> {
    Ok(sqlx::query_as("SELECT ts, valeur FROM zigbee_mesure WHERE ieee=? AND cle=? AND ts>=? AND ts<? ORDER BY ts")
        .bind(ieee)
        .bind(cle)
        .bind(t1)
        .bind(t2)
        .fetch_all(pool)
        .await?)
}

/// Énergie en kWh entre `t1` et `t2`, et part du temps couverte par des mesures.
fn integrer(m: &[(i64, f64)], t1: i64, t2: i64) -> (f64, f64) {
    let mut wh = 0.0;
    let mut couvert = 0;
    for (i, (ts, w)) in m.iter().enumerate() {
        if *ts < t1 || *ts >= t2 {
            continue;
        }
        let suivant = m.get(i + 1).map(|s| s.0).unwrap_or(t2).min(t2);
        let dt = (suivant - ts).clamp(0, TROU_MAX_S);
        wh += w.max(0.0) * dt as f64 / 3600.0;
        couvert += dt;
    }
    (wh / 1000.0, couvert as f64 / (t2 - t1).max(1) as f64)
}

fn debut_jour(jour: NaiveDate) -> i64 {
    jour.and_hms_opt(0, 0, 0)
        .and_then(|d| Local.from_local_datetime(&d).earliest())
        .map(|d| d.timestamp())
        .unwrap_or(0)
}

async fn kwh_jour(pool: &SqlitePool, ieee: &str, cle: &str, jour: NaiveDate) -> AppResult<Option<(f64, f64)>> {
    let (t1, t2) = (debut_jour(jour), debut_jour(jour + Duration::days(1)));
    let m = mesures(pool, ieee, cle, t1, t2).await?;
    Ok((!m.is_empty()).then(|| integrer(&m, t1, t2)))
}

/// Calcule et garde les journées finies qui manquent dans `elec_jour`.
pub(crate) async fn completer_jours(pool: &SqlitePool) -> AppResult<usize> {
    let hier = Local::now().date_naive() - Duration::days(1);
    let mut faits = 0;
    for (ieee, _, cle) in compteurs(pool).await? {
        let premier: Option<i64> = sqlx::query_scalar("SELECT MIN(ts) FROM zigbee_mesure WHERE ieee=? AND cle=?")
            .bind(&ieee)
            .bind(&cle)
            .fetch_one(pool)
            .await?;
        let Some(premier) = premier else { continue };
        let Some(mut jour) = Local.timestamp_opt(premier, 0).single().map(|d| d.date_naive()) else { continue };
        let connus: HashSet<String> = sqlx::query_scalar("SELECT jour FROM elec_jour WHERE ieee=?")
            .bind(&ieee)
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();
        while jour <= hier {
            if !connus.contains(&jour.to_string()) {
                if let Some((kwh, couverture)) = kwh_jour(pool, &ieee, &cle, jour).await? {
                    sqlx::query("INSERT OR REPLACE INTO elec_jour(ieee,jour,kwh,couverture) VALUES(?,?,?,?)")
                        .bind(&ieee)
                        .bind(jour.to_string())
                        .bind(kwh)
                        .bind(couverture)
                        .execute(pool)
                        .await?;
                    faits += 1;
                }
            }
            jour += Duration::days(1);
        }
    }
    Ok(faits)
}

/// Boucle lancée au démarrage : journées complétées toutes les 6 h.
pub(crate) async fn boucle(pool: SqlitePool) {
    loop {
        if let Err(e) = completer_jours(&pool).await {
            tracing::warn!(error = ?e, "consommation électrique : calcul des journées");
        }
        tokio::time::sleep(std::time::Duration::from_secs(6 * 3600)).await;
    }
}

/// kWh par jour entre deux dates (incluses) : `elec_jour`, et le jour en
/// cours calculé à la volée.
async fn par_jour(pool: &SqlitePool, ieee: &str, cle: &str, du: NaiveDate, au: NaiveDate) -> AppResult<HashMap<NaiveDate, (f64, f64)>> {
    let mut r: HashMap<NaiveDate, (f64, f64)> = HashMap::new();
    let lignes: Vec<(String, f64, f64)> = sqlx::query_as("SELECT jour,kwh,couverture FROM elec_jour WHERE ieee=? AND jour BETWEEN ? AND ?")
        .bind(ieee)
        .bind(du.to_string())
        .bind(au.to_string())
        .fetch_all(pool)
        .await?;
    for (j, k, c) in lignes {
        if let Ok(j) = NaiveDate::parse_from_str(&j, "%Y-%m-%d") {
            r.insert(j, (k, c));
        }
    }
    let auj = Local::now().date_naive();
    if du <= auj && auj <= au {
        let t1 = debut_jour(auj);
        let maintenant = Local::now().timestamp();
        let m = mesures(pool, ieee, cle, t1, maintenant).await?;
        if !m.is_empty() {
            let (k, c) = integrer(&m, t1, maintenant);
            r.insert(auj, (k, c));
        }
    }
    Ok(r)
}

fn barres(valeurs: Vec<(String, String, Option<f64>, bool)>) -> Value {
    let max = valeurs.iter().filter_map(|v| v.2).fold(0.0_f64, f64::max).max(0.001);
    Value::Array(
        valeurs
            .into_iter()
            .map(|(court, titre, kwh, partiel)| {
                json!({"court": court, "titre": titre, "kwh": kwh.map(|k| (k * 10.0).round() / 10.0),
                       "h": kwh.map(|k| (k / max * 100.0).round()).unwrap_or(0.0), "partiel": partiel})
            })
            .collect(),
    )
}

const MOIS: [&str; 12] = ["janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc."];
const JOURS: [&str; 7] = ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."];

fn debut_mois(d: NaiveDate) -> NaiveDate {
    d.with_day(1).unwrap_or(d)
}
fn mois_suivant(d: NaiveDate) -> NaiveDate {
    let d = debut_mois(d);
    if d.month() == 12 { NaiveDate::from_ymd_opt(d.year() + 1, 1, 1) } else { NaiveDate::from_ymd_opt(d.year(), d.month() + 1, 1) }.unwrap_or(d)
}
fn mois_precedent(d: NaiveDate) -> NaiveDate {
    let d = debut_mois(d);
    if d.month() == 1 { NaiveDate::from_ymd_opt(d.year() - 1, 12, 1) } else { NaiveDate::from_ymd_opt(d.year(), d.month() - 1, 1) }.unwrap_or(d)
}

/// (début, fin incluse, période précédente début, titre).
fn periode(vue: &str, d: NaiveDate) -> (NaiveDate, NaiveDate, NaiveDate, String) {
    match vue {
        "semaine" => {
            let lundi = d - Duration::days(i64::from(d.weekday().num_days_from_monday()));
            (lundi, lundi + Duration::days(6), lundi - Duration::days(7), format!("Semaine du {}", lundi.format("%d/%m/%Y")))
        }
        "mois" => {
            let m = debut_mois(d);
            (m, mois_suivant(m) - Duration::days(1), mois_precedent(m), format!("{} {}", MOIS[m.month0() as usize], m.year()))
        }
        "annee" => {
            let a = NaiveDate::from_ymd_opt(d.year(), 1, 1).unwrap_or(d);
            (a, NaiveDate::from_ymd_opt(d.year(), 12, 31).unwrap_or(d), NaiveDate::from_ymd_opt(d.year() - 1, 1, 1).unwrap_or(d), format!("Année {}", d.year()))
        }
        _ => (d, d, d - Duration::days(1), format!("{} {}", JOURS[d.weekday().num_days_from_monday() as usize], d.format("%d/%m/%Y"))),
    }
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(q): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let pool = &state.pool;
    let _ = completer_jours(pool).await;
    let liste = compteurs(pool).await?;
    let mut ctx = context(&session);
    let auj = Local::now().date_naive();
    let vue = match q.get("vue").map(String::as_str) {
        Some(v @ ("semaine" | "mois" | "annee")) => v,
        _ => "jour",
    };
    let d = q.get("d").and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()).unwrap_or(auj).min(auj);
    let choisi = q.get("c").and_then(|c| liste.iter().find(|l| &l.0 == c)).or(liste.first()).cloned();
    ctx.insert("compteurs".into(), json!(liste.iter().map(|(i, n, _)| json!({"ieee": i, "nom": n})).collect::<Vec<_>>()));
    ctx.insert("vue".into(), json!(vue));
    let prix: Option<f64> = crate::proxmox::parametre(pool, "elec_prix_kwh").await.and_then(|p| p.replace(',', ".").parse().ok());
    ctx.insert("prix".into(), json!(prix));
    let Some((ieee, nom, cle)) = choisi else {
        return render(&state, "electricite.html", Value::Object(ctx));
    };
    ctx.insert("ieee".into(), json!(ieee));
    ctx.insert("nom".into(), json!(nom));

    let (du, au, avant, titre) = periode(vue, d);
    let (precedent, suivant) = match vue {
        "semaine" => (d - Duration::days(7), d + Duration::days(7)),
        "mois" => (mois_precedent(d), mois_suivant(d)),
        "annee" => (NaiveDate::from_ymd_opt(d.year() - 1, 1, 1).unwrap_or(d), NaiveDate::from_ymd_opt(d.year() + 1, 1, 1).unwrap_or(d)),
        _ => (d - Duration::days(1), d + Duration::days(1)),
    };
    ctx.insert("titre".into(), json!(titre));
    ctx.insert("d".into(), json!(d.to_string()));
    ctx.insert("precedent".into(), json!(precedent.to_string()));
    ctx.insert("suivant".into(), json!((suivant <= auj).then(|| suivant.to_string())));

    let jours = par_jour(pool, &ieee, &cle, avant.min(du), au).await?;
    let somme = |a: NaiveDate, b: NaiveDate| -> Option<f64> {
        let v: Vec<f64> = jours.iter().filter(|(j, _)| **j >= a && **j <= b).map(|(_, (k, _))| *k).collect();
        (!v.is_empty()).then(|| v.iter().sum())
    };
    let total = somme(du, au);
    // Période précédente de même longueur, arrêtée au même point si la période est en cours.
    let ecoule = (auj.min(au) - du).num_days();
    let total_avant = somme(avant, avant + Duration::days(ecoule));
    ctx.insert("total".into(), json!(total.map(|t| (t * 10.0).round() / 10.0)));
    ctx.insert("total_avant".into(), json!(total_avant.map(|t| (t * 10.0).round() / 10.0)));
    ctx.insert("ecart".into(), json!(total.zip(total_avant).filter(|(_, a)| *a > 0.0).map(|(t, a)| ((t / a - 1.0) * 100.0).round())));
    ctx.insert("cout".into(), json!(total.zip(prix).map(|(t, p)| (t * p * 100.0).round() / 100.0)));

    if vue == "jour" {
        let (t1, t2) = (debut_jour(d), debut_jour(d + Duration::days(1)));
        let m = mesures(pool, &ieee, &cle, t1, t2).await?;
        // Courbe : moyenne par quart d'heure, SVG 720 × 180.
        let mut quarts: Vec<(i64, f64, u32)> = Vec::new();
        for (ts, w) in &m {
            let q = (ts - t1) / 900;
            match quarts.last_mut() {
                Some((s, somme, n)) if *s == q => { *somme += w; *n += 1; }
                _ => quarts.push((q, *w, 1)),
            }
        }
        let pmax = m.iter().map(|x| x.1).fold(0.0_f64, f64::max);
        let echelle = (pmax * 1.1).max(100.0);
        let points: Vec<String> = quarts
            .iter()
            .map(|(q, s, n)| format!("{:.1},{:.1}", (*q as f64 + 0.5) / 96.0 * 720.0, 175.0 - s / f64::from(*n) / echelle * 165.0))
            .collect();
        ctx.insert("trace".into(), json!(points.join(" ")));
        ctx.insert("echelle_kw".into(), json!((echelle / 100.0).round() / 10.0));
        if let Some((ts_max, w)) = m.iter().max_by(|a, b| a.1.total_cmp(&b.1)) {
            ctx.insert("pic".into(), json!({"kw": (w / 100.0).round() / 10.0, "heure": Local.timestamp_opt(*ts_max, 0).single().map(|t| t.format("%Hh%M").to_string())}));
        }
        // Sans mesures de puissance : consommation par heure importée.
        let importees: Vec<(i64, f64, bool)> = if m.is_empty() {
            sqlx::query_as("SELECT ts,kwh,estime FROM elec_heure WHERE ieee=? AND ts>=? AND ts<? ORDER BY ts")
                .bind(&ieee)
                .bind(t1)
                .bind(t2)
                .fetch_all(pool)
                .await?
        } else {
            Vec::new()
        };
        ctx.insert("par_heure_seulement".into(), json!(!importees.is_empty()));
        // Heures réelles du jour (23 ou 25 lors des changements d'heure).
        let nb_heures = ((t2 - t1) / 3600).clamp(23, 25);
        let heures = (0..nb_heures)
            .map(|h| {
                let (a, b) = (t1 + h * 3600, t1 + (h + 1) * 3600);
                let etiquette = Local.timestamp_opt(a, 0).single().map(|t| t.format("%H").to_string().trim_start_matches('0').to_string()).unwrap_or_default();
                let etiquette = if etiquette.is_empty() { "0".to_string() } else { etiquette };
                let titre = format!("{etiquette}h");
                if !importees.is_empty() {
                    let dans: Vec<&(i64, f64, bool)> = importees.iter().filter(|x| x.0 >= a && x.0 < b).collect();
                    let k = (!dans.is_empty()).then(|| dans.iter().map(|x| x.1).sum::<f64>());
                    return (etiquette, titre, k, dans.iter().any(|x| x.2));
                }
                let dans: Vec<(i64, f64)> = m.iter().filter(|x| x.0 >= a && x.0 < b).cloned().collect();
                let k = (!dans.is_empty()).then(|| integrer(&m, a, b));
                (etiquette, titre, k.map(|k| k.0), k.is_some_and(|k| k.1 < 0.9))
            })
            .collect();
        ctx.insert("barres".into(), barres(heures));
    } else if vue == "annee" {
        let mut v = Vec::new();
        let mut m = du;
        while m <= au {
            let fin = mois_suivant(m) - Duration::days(1);
            let jours_mois: Vec<&(f64, f64)> = jours.iter().filter(|(j, _)| **j >= m && **j <= fin).map(|(_, x)| x).collect();
            let k = (!jours_mois.is_empty()).then(|| jours_mois.iter().map(|x| x.0).sum());
            let complet = jours_mois.len() as i64 >= (fin.min(auj) - m).num_days() + 1 && jours_mois.iter().all(|x| x.1 >= 0.9);
            v.push((MOIS[m.month0() as usize].to_string(), format!("{} {}", MOIS[m.month0() as usize], m.year()), k, k.is_some() && !complet));
            m = mois_suivant(m);
        }
        ctx.insert("barres".into(), barres(v));
    } else {
        let mut v = Vec::new();
        let mut j = du;
        while j <= au {
            let x = jours.get(&j);
            let court = if vue == "semaine" { JOURS[j.weekday().num_days_from_monday() as usize].to_string() } else { j.day().to_string() };
            v.push((court, j.format("%d/%m").to_string(), x.map(|x| x.0), x.is_some_and(|x| x.1 < 0.9)));
            j += Duration::days(1);
        }
        ctx.insert("barres".into(), barres(v));
    }
    render(&state, "electricite.html", Value::Object(ctx))
}

pub(super) async fn prix(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let p = form_text(&form, "prix").unwrap_or_default().replace(',', ".");
    if !p.is_empty() && !p.parse::<f64>().is_ok_and(|v| (0.0..10.0).contains(&v)) {
        return Err(AppError::Invalid("Prix du kWh attendu en euros, par exemple 0,21".into()));
    }
    crate::sms_free::ecrire_parametre(&state.pool, "elec_prix_kwh", &p).await?;
    let retour = form_text(&form, "retour").filter(|r| r.starts_with("/electricite")).unwrap_or_else(|| "/electricite".into());
    Ok(Redirect::to(&retour).into_response())
}

#[cfg(test)]
mod tests {
    use super::integrer;

    #[test]
    fn integration_de_la_puissance() {
        // 1 000 W pendant 1 h, mesuré toutes les 2 min.
        let m: Vec<(i64, f64)> = (0..30).map(|i| (i * 120, 1000.0)).collect();
        let (kwh, couv) = integrer(&m, 0, 3600);
        assert!((kwh - 1.0).abs() < 1e-9 && (couv - 1.0).abs() < 1e-9);
        // Un trou de 30 min n'est compté que pour 10 min.
        let m = vec![(0, 1200.0), (1800, 1200.0)];
        let (kwh, couv) = integrer(&m, 0, 3600);
        assert!((kwh - 0.4).abs() < 1e-9 && (couv - 1200.0 / 3600.0).abs() < 1e-9);
    }
}

/// `EO_TEST_BASE=/var/tmp/copie.db cargo test electricite_sur_copie -- --ignored --nocapture` (écrit elec_jour dans la copie).
#[cfg(test)]
mod tests_copie {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn electricite_sur_copie() -> anyhow::Result<()> {
        let chemin = std::env::var("EO_TEST_BASE")?;
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect(&format!("sqlite://{chemin}")).await?;
        crate::db::init(&pool).await?;
        let n = completer_jours(&pool).await.map_err(|e| anyhow::anyhow!("{e:?}"))?;
        println!("{n} journées calculées");
        let l: Vec<(String, String, f64, f64)> = sqlx::query_as("SELECT ieee,jour,round(kwh,1),round(couverture,2) FROM elec_jour ORDER BY ieee,jour").fetch_all(&pool).await?;
        for x in l { println!("{x:?}"); }
        for (ieee, nom, cle) in compteurs(&pool).await.map_err(|e| anyhow::anyhow!("{e:?}"))? {
            let j = par_jour(&pool, &ieee, &cle, Local::now().date_naive() - Duration::days(2), Local::now().date_naive()).await.map_err(|e| anyhow::anyhow!("{e:?}"))?;
            println!("{nom} ({cle}) : {j:?}");
        }
        let env = crate::templates::build()?;
        let page = env.get_template("electricite.html")?.render(json!({
            "session": {"peut_modifier": true, "csrf": "x", "pages": []},
            "compteurs": [{"ieee": "a", "nom": "Général"}, {"ieee": "b", "nom": "Soupe"}], "ieee": "a", "nom": "Général",
            "vue": "jour", "d": "2026-10-06", "precedent": "2026-10-05", "suivant": "2026-10-07", "titre": "mar. 06/10/2026",
            "total": 136.0, "total_avant": 240.1, "ecart": -43.0, "cout": 28.56, "prix": 0.21, "pic": {"kw": 27.1, "heure": "07h12"},
            "trace": "0,170 720,20", "echelle_kw": 29.8,
            "barres": [{"court": "0", "titre": "0h–1h", "kwh": 4.2, "h": 50, "partiel": false}, {"court": "1", "titre": "1h–2h", "kwh": null, "h": 0, "partiel": false}]
        }))?;
        assert!(page.contains("136,0 kWh") || page.contains("136.0 kWh"), "{page}");
        Ok(())
    }
}
