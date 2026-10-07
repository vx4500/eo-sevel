//! Accueil au choix (Paramètres → Page d'accueil, réglage `accueil_style`,
//! le même pour toute l'exploitation) : « classique » (dashboard.html),
//! « cockpit », « tournee » ou « flux ». Les trois nouvelles pages reprennent
//! le contexte du tableau de bord et y ajoutent salles, alertes, machine à
//! soupe et planning des bandes.

use super::*;
use chrono::Datelike;

pub(super) const STYLES: [(&str, &str); 4] = [
    ("classique", "Classique (tableau de bord actuel)"),
    ("cockpit", "Cockpit : tout sur une page"),
    ("tournee", "Ma tournée : salle par salle"),
    ("flux", "Flux : planning des bandes"),
];

pub(super) async fn style(pool: &SqlitePool) -> AppResult<String> {
    let v: Option<String> = sqlx::query_scalar("SELECT valeur FROM parametre WHERE cle='accueil_style'")
        .fetch_optional(pool)
        .await?
        .flatten();
    Ok(v.filter(|v| STYLES.iter().any(|(c, _)| c == v)).unwrap_or_else(|| "classique".into()))
}

fn date(v: &Value) -> Option<NaiveDate> {
    v.as_str().and_then(parse_stored_date)
}

/// Frise des bandes : fenêtre de 18 mois (7 avant aujourd'hui, 11 après),
/// positions en % pour le gabarit.
fn planning(bandes: &mut [Value]) -> Value {
    let today = Local::now().date_naive();
    let debut = (today - Duration::days(213)).with_day(1).unwrap_or(today);
    let fin = debut + Duration::days(548);
    let total = (fin - debut).num_days() as f64;
    let pct = |d: NaiveDate| (((d - debut).num_days() as f64 / total) * 100.0).clamp(0.0, 100.0);
    // Étapes (index dans `etapes`) qui bornent chaque période.
    let periodes = [
        ("Gestation", 0usize, 3usize, "gestation"),
        ("Maternité", 3, 5, "maternite"),
        ("Post-sevrage", 5, 6, "post-sevrage"),
        ("Engraissement", 6, 7, "engraissement"),
        ("Finition", 7, 8, "finition"),
    ];
    for b in bandes.iter_mut() {
        let etapes = b["etapes"].as_array().cloned().unwrap_or_default();
        let dates: Vec<Option<NaiveDate>> = etapes.iter().map(|e| date(&e["date"])).collect();
        let mut segments = Vec::new();
        for (nom, a, z, classe) in periodes {
            let (Some(Some(da)), Some(Some(dz))) = (dates.get(a), dates.get(z)) else { continue };
            let (l, r) = (pct(*da), pct(*dz));
            if r - l > 0.1 {
                segments.push(json!({"nom": nom, "classe": classe, "gauche": format!("{l:.2}"), "largeur": format!("{:.2}", r - l),
                    "passe": *dz < today, "en_cours": *da <= today && today < *dz, "du": da.to_string(), "au": dz.to_string()}));
            }
        }
        b["segments"] = Value::Array(segments);
        b["prochaine_pct"] = date(&b["prochaine_date"]).map(|d| json!(format!("{:.2}", pct(d)))).unwrap_or(Value::Null);
    }
    let mois_noms = ["janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc."];
    let mut mois = Vec::new();
    let mut m = debut;
    while m < fin {
        let suivant = if m.month() == 12 { NaiveDate::from_ymd_opt(m.year() + 1, 1, 1) } else { NaiveDate::from_ymd_opt(m.year(), m.month() + 1, 1) }.unwrap_or(fin);
        let nom = if m.month() == 1 { format!("{} {}", mois_noms[0], m.year() % 100) } else { mois_noms[m.month0() as usize].to_string() };
        mois.push(json!({"nom": nom, "largeur": format!("{:.2}", pct(suivant.min(fin)) - pct(m))}));
        m = suivant;
    }
    json!({"mois": mois, "aujourdhui": format!("{:.2}", pct(today))})
}

/// Frise des bandes actives pour une page qui n'a pas le contexte du tableau
/// de bord (Places et flux) : mêmes vues de bande, mêmes segments.
pub(super) async fn frise_bandes(pool: &SqlitePool) -> AppResult<(Value, Vec<Value>)> {
    let schedule = load_band_schedule(pool).await?;
    let mut bandes = Vec::new();
    for band in sqlx::query_as::<_, Bande>(BAND_SELECT_ACTIVE).fetch_all(pool).await? {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM truie WHERE bande_code=? AND reformee=0")
            .bind(&band.code)
            .fetch_one(pool)
            .await?;
        let flux = band_flow_summary(pool, &band).await?;
        let actual_stage = actual_band_stage(pool, &band).await?;
        let reproduction = band_reproduction_stats(pool, &band).await?;
        bandes.push(serde_json::to_value(band_view(&band, count, schedule, flux, actual_stage, reproduction)).unwrap_or_default());
    }
    let frise = planning(&mut bandes);
    Ok((frise, bandes))
}

pub(super) async fn enrichir(pool: &SqlitePool, ctx: &mut Map<String, Value>, mut bandes: Vec<Value>) -> AppResult<()> {
    // Salles suivies par la régulation, avec les animaux présents d'après les
    // emplacements des bandes (mêmes calculs que le tableau classique).
    let mut presents: HashMap<i64, (i64, Vec<String>)> = HashMap::new();
    for b in bandes.iter_mut() {
        let porcs: i64 = b["flux"]["emplacements"].as_array().into_iter().flatten()
            .filter(|e| e["animaux"] == "Porcs").filter_map(|e| e["effectif"].as_i64()).sum();
        b["porcs"] = json!(porcs);
    }
    for b in &bandes {
        let code = b["code"].as_str().unwrap_or("").to_string();
        for e in b["flux"]["emplacements"].as_array().into_iter().flatten() {
            if let Some(id) = e["salle_id"].as_i64() {
                let p = presents.entry(id).or_default();
                p.0 += e["effectif"].as_i64().unwrap_or(0);
                if !p.1.contains(&code) {
                    p.1.push(code.clone());
                }
            }
        }
    }
    let mut salles = rows_to_json(
        sqlx::query(
            "SELECT s.id,s.nom,e.t,e.h,e.consigne,e.ventil,e.sonde_ok,e.ts FROM ambiance_etat e JOIN salle s ON s.id=e.salle_id \
             ORDER BY s.ordre,length(s.nom),s.nom",
        )
        .fetch_all(pool)
        .await?,
    )?;
    for s in salles.iter_mut() {
        let id = s["id"].as_i64().unwrap_or(0);
        let (n, codes) = presents.get(&id).cloned().unwrap_or_default();
        s["effectif"] = json!(n);
        s["bandes"] = json!(codes.join(", "));
        s["bandes_liste"] = json!(codes);
        let ecart = match (s["t"].as_f64(), s["consigne"].as_f64()) {
            (Some(t), Some(c)) if c > 0.0 => Some(((t - c) * 10.0).round() / 10.0),
            _ => None,
        };
        s["ecart"] = json!(ecart);
        s["hors_consigne"] = json!(ecart.is_some_and(|e| e.abs() >= 3.0));
    }
    // Salles occupées d'abord (ordre de la tournée), puis les vides.
    salles.sort_by_key(|s| (s["effectif"].as_i64().unwrap_or(0) == 0) as u8);
    ctx.insert("salles_suivies".into(), Value::Array(salles));

    let alertes = farm_alerts(pool).await?;
    ctx.insert("alertes_elevage".into(), alertes);

    let soupe = if module_actif(pool, "module_machine_soupe", false).await? {
        sqlx::query_as::<_, (i64, i64, i64, i64)>("SELECT ts,phase,poids,cumul FROM soupe_etat WHERE id=1")
            .fetch_optional(pool)
            .await?
            .map(|(ts, phase, poids, cumul)| json!({"ts": ts, "phase": crate::soupe::libelle_phase(phase), "en_cours": phase != 0, "poids": poids, "cumul": cumul}))
            .unwrap_or_else(|| json!({"phase": "Pas encore de nouvelles", "en_cours": false}))
    } else {
        Value::Null
    };
    ctx.insert("soupe".into(), soupe);

    let frise = planning(&mut bandes);
    ctx.insert("frise".into(), frise);
    let mut interventions: Vec<Value> = bandes
        .iter()
        .filter(|b| b["prochaine_date"].is_string())
        .map(|b| json!({"bande_id": b["id"], "code": b["code"], "nom": b["prochaine"], "date": b["prochaine_date"], "delai": b["prochaine_delai"], "urgence": b["urgence"]}))
        .collect();
    interventions.sort_by(|a, b| a["date"].as_str().cmp(&b["date"].as_str()));
    ctx.insert("interventions".into(), Value::Array(interventions));
    // Effectifs par stade pour la page Flux.
    let mut stades: Vec<(String, i64, Vec<String>)> = Vec::new();
    for b in &bandes {
        let st = b["stade"].as_str().unwrap_or("—").to_string();
        let n = b["truies"].as_i64().unwrap_or(0) + b["porcs"].as_i64().unwrap_or(0);
        let code = b["code"].as_str().unwrap_or("").to_string();
        match stades.iter_mut().find(|s| s.0 == st) {
            Some(s) => {
                s.1 += n;
                s.2.push(code);
            }
            None => stades.push((st, n, vec![code])),
        }
    }
    ctx.insert(
        "stades".into(),
        Value::Array(stades.into_iter().map(|(s, n, c)| json!({"stade": s, "animaux": n, "bandes": c.join(", ")})).collect()),
    );
    ctx.insert("bandes".into(), Value::Array(bandes));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_frise_place_chaque_periode_dans_la_fenetre() {
        let today = Local::now().date_naive();
        let d = |j: i64| json!((today + Duration::days(j)).to_string());
        let mut bandes = vec![json!({
            "code": "T", "prochaine_date": d(10),
            "etapes": [{"date": d(-100)}, {"date": d(-72)}, {"date": d(-72)}, {"date": d(10)}, {"date": d(15)}, {"date": d(43)}, {"date": d(86)}, {"date": d(155)}, {"date": d(230)}]
        })];
        let f = planning(&mut bandes);
        let segs = bandes[0]["segments"].as_array().unwrap();
        assert_eq!(segs.len(), 5);
        assert_eq!(segs[0]["nom"], "Gestation");
        let auj: f64 = f["aujourdhui"].as_str().unwrap().parse().unwrap();
        assert!(auj > 30.0 && auj < 45.0);
        let total: f64 = f["mois"].as_array().unwrap().iter().map(|m| m["largeur"].as_str().unwrap().parse::<f64>().unwrap()).sum();
        assert!((total - 100.0).abs() < 0.5);
    }

    /// Rend les quatre accueils sur une copie de la base réelle et les écrit
    /// dans le dossier de la copie (accueil_<style>.html) :
    /// `EO_TEST_BASE=/var/tmp/copie/elevage.db cargo test -- --ignored accueils_sur_copie`.
    #[tokio::test]
    #[ignore]
    async fn accueils_sur_copie_de_la_base_reelle() -> anyhow::Result<()> {
        let chemin = std::env::var("EO_TEST_BASE")?;
        let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&chemin).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await?;
        crate::db::init(&pool).await?;
        let state = AppState::new(
            Config { bind: "127.0.0.1:8080".parse()?, db_path: chemin.clone().into(), secure_cookies: false },
            pool,
            crate::templates::build()?,
        );
        let dossier = std::path::Path::new(&chemin).parent().unwrap().to_path_buf();
        for (code, _) in STYLES {
            sqlx::query("INSERT INTO parametre(cle,valeur) VALUES('accueil_style',?) ON CONFLICT(cle) DO UPDATE SET valeur=excluded.valeur")
                .bind(code).execute(&state.pool).await?;
            let r = dashboard(State(state.clone()), Extension(super::super::documents::tests::session())).await?;
            assert_eq!(r.status(), StatusCode::OK, "{code}");
            let html = String::from_utf8(axum::body::to_bytes(r.into_body(), usize::MAX).await?.to_vec())?;
            std::fs::write(dossier.join(format!("accueil_{code}.html")), &html)?;
        }
        Ok(())
    }
}

/// `EO_TEST_BASE=/var/tmp/copie.db cargo test planning_sur_copie -- --ignored --nocapture`
#[cfg(test)]
mod tests_planning_copie {
    #[tokio::test]
    #[ignore]
    async fn planning_sur_copie() -> anyhow::Result<()> {
        let chemin = std::env::var("EO_TEST_BASE")?;
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect(&format!("sqlite://{chemin}?mode=ro")).await?;
        let (frise, bandes) = super::frise_bandes(&pool).await.map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let env = crate::templates::build()?;
        let page = env.get_template("planning.html")?.render(serde_json::json!({
            "session": {"pages": []}, "frise": frise, "frise_bandes": bandes, "evenements": [], "taches": [], "echeances": []
        }))?;
        let debut = page.find("Les bandes en détail").unwrap();
        let extrait: String = page[debut..].chars().take(3000).collect();
        println!("{extrait}");
        assert!(page.contains("<em>Gestation</em>") && page.contains("en cours"));
        Ok(())
    }
}
