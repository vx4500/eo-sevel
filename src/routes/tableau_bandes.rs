//! Tableau des bandes façon « analyse par bande » : une colonne par bande,
//! une ligne par indicateur, puis la moyenne et l'objectif de l'élevage.
//! Les valeurs sont celles enregistrées sur chaque bande (`bande.cs_*`), les
//! objectifs ceux de la page Objectifs. Export PDF par l'impression du
//! navigateur.

use super::*;

/// Indicateurs repris d'une colonne `bande.cs_*` du même nom.
const COLONNES: [&str; 15] = [
    "cs_truies_saillies",
    "cs_pleines",
    "cs_truies_mb",
    "cs_nt_portee",
    "cs_nv_portee",
    "cs_mn_portee",
    "cs_sevres_portee",
    "cs_tx_pertes_nv",
    "cs_total_sevres",
    "cs_poids_sevrage",
    "cs_adoptes",
    "cs_retires",
    "cs_gmq_ps",
    "cs_gmq_engr",
    "cs_gmq_nv",
];

fn nombre(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().replace(',', ".").parse().ok()))
}

/// Nombre à la française avec le nombre de décimales de l'indicateur.
fn texte(v: Option<f64>, decimales: i64) -> Value {
    match v {
        Some(v) => json!(format!("{:.*}", decimales.max(0) as usize, v).replace('.', ",")),
        None => Value::Null,
    }
}

/// « bon », « mauvais » ou rien, selon le sens de l'objectif.
fn ecart(valeur: Option<f64>, objectif: Option<f64>, sens: &str) -> &'static str {
    match (valeur, objectif) {
        (Some(v), Some(o)) => {
            let atteint = if sens == "bas" { v <= o } else { v >= o };
            if atteint { "bon" } else { "mauvais" }
        }
        _ => "",
    }
}

/// Aliment des porcs (post-sevrage + engraissement) affecté à la bande, en kg,
/// et indice de consommation du sevrage à la vente. Les kilos produits
/// comptent chaque porc jusqu'à sa sortie : vendus au poids vif d'abattoir,
/// morts au poids estimé à leur âge le jour de la mort, restants au poids
/// estimé aujourd'hui ; moins les kilos entrés au sevrage. `None` tant que
/// la bande n'a rien vendu : l'IC d'une bande en croissance ne veut rien dire.
async fn ic_bande(
    pool: &SqlitePool,
    id: i64,
    code: &str,
    date_mb: &str,
    poids_sevrage: Option<f64>,
) -> AppResult<(Option<f64>, Option<f64>)> {
    let aliment: f64 = sqlx::query_scalar(&format!(
        "SELECT CAST(COALESCE(SUM(tonnage*1000.0*COALESCE(af.part,1.0/(SELECT COUNT(*) FROM affectationfacturebande n \
         WHERE n.categorie='aliment' AND n.facture_id=livraisonaliment.id))),0) AS REAL) \
         FROM affectationfacturebande af JOIN livraisonaliment ON livraisonaliment.id=af.facture_id \
         WHERE af.categorie='aliment' AND af.bande_id=? AND {} IN ('ps','croissance','finition')",
        gte_periode::STADE_EFFECTIF
    ))
    .bind(id)
    .fetch_one(pool)
    .await?;
    let aliment = (aliment > 0.0).then_some(aliment);
    let Ok(naissance) = NaiveDate::parse_from_str(date_mb.get(..10).unwrap_or(date_mb), "%Y-%m-%d") else {
        return Ok((aliment, None));
    };
    let sevres: i64 = sqlx::query_scalar(
        "SELECT CAST(COALESCE(SUM(nb_sevres),0) AS INTEGER) FROM evenement WHERE bande_id=? AND type='sevrage'",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    let (vendus, carcasse): (i64, f64) = sqlx::query_as(
        "SELECT CAST(COALESCE(SUM(nb_porcs),0) AS INTEGER),CAST(COALESCE(SUM(poids_total),0) AS REAL) FROM venteapport WHERE bande_id=?",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    if vendus == 0 || sevres == 0 {
        return Ok((aliment, None));
    }
    let morts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT date,CAST(COALESCE(nombre,0) AS INTEGER) FROM declarationmort WHERE bande_code=?1 AND date IS NOT NULL \
         UNION ALL SELECT date,CAST(COALESCE(nombre,0) AS INTEGER) FROM mouvementstock WHERE bande_code=?1 AND code_ifip IN('39','49') AND date IS NOT NULL",
    )
    .bind(code)
    .fetch_all(pool)
    .await?;
    let age = |d: NaiveDate| (d - naissance).num_days() as f64;
    // Morts plafonnés aux porcs jamais vendus (déclarations de « clôture »).
    let mut reste = (sevres - vendus).max(0);
    let mut kg_morts = 0.0;
    for (date, n) in morts {
        let Ok(d) = NaiveDate::parse_from_str(date.get(..10).unwrap_or(&date), "%Y-%m-%d") else { continue };
        let n = n.clamp(0, reste);
        reste -= n;
        kg_morts += n as f64 * gte_periode::poids_a_lage(age(d));
    }
    let kg_restants = reste as f64 * gte_periode::poids_a_lage(age(Local::now().date_naive()));
    let kg_vendus = gte_periode::poids_vif(carcasse, 76.5).unwrap_or(0.0);
    let kg_entres = sevres as f64 * poids_sevrage.filter(|p| *p > 0.0).unwrap_or(8.0);
    let produits = kg_vendus + kg_morts + kg_restants - kg_entres;
    Ok((aliment, aliment.and_then(|a| (produits > 0.0).then(|| a / produits))))
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let pool = &state.pool;
    let nb: i64 = query.get("n").and_then(|n| n.parse().ok()).filter(|n| (2..=24).contains(n)).unwrap_or(8);
    // Bandes qui ont commencé (au moins des saillies), les plus récentes, puis
    // remises dans l'ordre chronologique comme dans l'écran d'analyse.
    let mut bandes = generic_rows(
        pool,
        &format!(
            "SELECT id,code,date_mb,active,{} FROM bande \
             WHERE date_mb IS NOT NULL AND COALESCE(cs_truies_saillies,0)>0 \
             ORDER BY date_mb DESC,id DESC LIMIT {nb}",
            COLONNES.join(",")
        ),
    )
    .await?;
    bandes.reverse();
    // Ce que la bande a réellement donné : les porcs partis à l'abattoir
    // (apports rattachés à la bande). Partiel tant que la bande se vend.
    for b in bandes.iter_mut() {
        let id = b["id"].as_i64().unwrap_or_default();
        let vendus: i64 = sqlx::query_scalar(conduite::VENDUS_BANDE).bind(id).fetch_one(pool).await?;
        b["vendus"] = json!((vendus > 0).then_some(vendus));
        let code = b["code"].as_str().unwrap_or_default().to_string();
        let date_mb = b["date_mb"].as_str().unwrap_or_default().to_string();
        let (aliment, ic) = ic_bande(pool, id, &code, &date_mb, nombre(&b["cs_poids_sevrage"])).await?;
        b["aliment_porcs_t"] = json!(aliment.map(|kg| kg / 1000.0));
        b["ic"] = json!(ic);
    }

    let objectifs = generic_rows(pool, "SELECT cle,valeur,sens FROM objectif WHERE actif=1").await?;
    let objectif = |cle: &str| objectifs.iter().find(|o| o["cle"] == cle);

    let mut lignes: Vec<Value> = Vec::new();
    for cle in COLONNES {
        let Some((libelle, decimales)) = objective_definition(cle) else { continue };
        let valeurs: Vec<Option<f64>> = bandes.iter().map(|b| nombre(&b[cle])).collect();
        if valeurs.iter().all(Option::is_none) {
            continue;
        }
        // Dans une bande, chaque porcelet adopté par une truie a été retiré à
        // une autre : les deux totaux sont égaux, la seconde ligne n'apprend
        // rien. On ne la garde que si elle diffère (échanges entre bandes).
        if cle == "cs_retires"
            && bandes.iter().all(|b| nombre(&b["cs_retires"]) == nombre(&b["cs_adoptes"]))
        {
            continue;
        }
        let presentes: Vec<f64> = valeurs.iter().flatten().copied().collect();
        let moyenne = (!presentes.is_empty()).then(|| presentes.iter().sum::<f64>() / presentes.len() as f64);
        let (cible, sens) = objectif(cle)
            .map(|o| (nombre(&o["valeur"]), o["sens"].as_str().unwrap_or("haut").to_string()))
            .unwrap_or((None, "haut".into()));
        lignes.push(json!({
            "libelle": libelle,
            "decimales": decimales,
            "cellules": valeurs.iter().map(|v| json!({"v": texte(*v, decimales), "etat": ecart(*v, cible, &sens)})).collect::<Vec<_>>(),
            "moyenne": {"v": texte(moyenne, decimales), "etat": ecart(moyenne, cible, &sens)},
            "objectif": texte(cible, decimales),
            "sens": sens,
        }));
        // Taux de pleines juste après « Pleines à l'écho », comme CSuite.
        if cle == "cs_total_sevres" {
            let ratio = |num: &str, den: &str, facteur: f64| -> Vec<Option<f64>> {
                bandes
                    .iter()
                    .map(|b| match (nombre(&b[num]), nombre(&b[den])) {
                        (Some(n), Some(d)) if d > 0.0 => Some(n / d * facteur),
                        _ => None,
                    })
                    .collect()
            };
            let vendus: Vec<Option<f64>> = bandes.iter().map(|b| nombre(&b["vendus"])).collect();
            for (libelle, decimales, valeurs) in [
                ("Porcs vendus (abattoir)", 0, vendus),
                ("Vendus / truie mise bas", 1, ratio("vendus", "cs_truies_mb", 1.0)),
                ("Vendus / sevrés (%)", 1, ratio("vendus", "cs_total_sevres", 100.0)),
                (
                    "Aliment post-sevrage + engraissement (t)",
                    1,
                    bandes.iter().map(|b| nombre(&b["aliment_porcs_t"])).collect(),
                ),
                ("IC sevrage → vente", 2, bandes.iter().map(|b| nombre(&b["ic"])).collect()),
            ] {
                let v: Vec<f64> = valeurs.iter().flatten().copied().collect();
                let moy = (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64);
                lignes.push(json!({
                    "libelle": libelle, "decimales": decimales, "vente": true,
                    "cellules": valeurs.iter().map(|x| json!({"v": texte(*x, decimales), "etat": ""})).collect::<Vec<_>>(),
                    "moyenne": {"v": texte(moy, decimales), "etat": ""}, "objectif": null, "sens": "haut",
                }));
            }
        }
        if cle == "cs_pleines" {
            let taux: Vec<Option<f64>> = bandes
                .iter()
                .map(|b| match (nombre(&b["cs_pleines"]), nombre(&b["cs_truies_saillies"])) {
                    (Some(p), Some(s)) if s > 0.0 => Some(p / s * 100.0),
                    _ => None,
                })
                .collect();
            let t: Vec<f64> = taux.iter().flatten().copied().collect();
            let moy = (!t.is_empty()).then(|| t.iter().sum::<f64>() / t.len() as f64);
            lignes.push(json!({
                "libelle": "Taux de pleines à l'écho (%)", "decimales": 1,
                "cellules": taux.iter().map(|v| json!({"v": texte(*v, 1), "etat": ""})).collect::<Vec<_>>(),
                "moyenne": {"v": texte(moy, 1), "etat": ""}, "objectif": null, "sens": "haut",
            }));
        }
    }

    let mut ctx = context(&session);
    ctx.insert("bandes".into(), Value::Array(bandes));
    ctx.insert("lignes".into(), Value::Array(lignes));
    ctx.insert("nb".into(), json!(nb));
    render(&state, "tableau_bandes.html", Value::Object(ctx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecart_suit_le_sens_de_l_objectif() {
        assert_eq!(ecart(Some(15.0), Some(14.0), "haut"), "bon");
        assert_eq!(ecart(Some(13.0), Some(14.0), "haut"), "mauvais");
        assert_eq!(ecart(Some(0.8), Some(1.0), "bas"), "bon");
        assert_eq!(ecart(Some(1.2), Some(1.0), "bas"), "mauvais");
        assert_eq!(ecart(None, Some(1.0), "bas"), "");
        assert_eq!(ecart(Some(1.0), None, "haut"), "");
    }

    #[test]
    fn texte_a_la_francaise() {
        assert_eq!(texte(Some(16.456), 2), json!("16,46"));
        assert_eq!(texte(Some(31.0), 0), json!("31"));
        assert_eq!(texte(None, 1), Value::Null);
    }

    #[test]
    fn chaque_colonne_a_un_libelle() {
        for cle in COLONNES {
            assert!(objective_definition(cle).is_some(), "{cle}");
        }
    }
}
