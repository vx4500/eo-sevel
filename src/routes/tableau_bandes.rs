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

    let objectifs = generic_rows(pool, "SELECT cle,valeur,sens FROM objectif WHERE actif=1").await?;
    let objectif = |cle: &str| objectifs.iter().find(|o| o["cle"] == cle);

    let mut lignes: Vec<Value> = Vec::new();
    for cle in COLONNES {
        let Some((libelle, decimales)) = objective_definition(cle) else { continue };
        let valeurs: Vec<Option<f64>> = bandes.iter().map(|b| nombre(&b[cle])).collect();
        if valeurs.iter().all(Option::is_none) {
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
