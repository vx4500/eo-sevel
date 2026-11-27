//! Diagramme résultat élevage (/diagramme-resultat) : l'arbre de marge sur
//! coût alimentaire et renouvellement, case par case (produits, aliment,
//! renouvellement et les chiffres techniques qui les expliquent), avec pour
//! chaque case la valeur de l'élevage, une valeur de comparaison et l'écart.
//!
//! La comparaison se choisit : la période précédente de même longueur,
//! les références nationales IFIP de la GTE, ou les chiffres de référence
//! Cooperl saisis à la main (enregistrés dans `arbre_reference`).
//!
//! Les chiffres viennent des mêmes calculs que la GTE (`gte_periode`) :
//! le diagramme et la GTE donnent la même réponse sur la même période.

use super::*;
use std::collections::BTreeMap;

/// Une case du diagramme : clé, libellé, unité, décimales, sens (1 : plus
/// c'est haut mieux c'est, -1 : l'inverse, 0 : ni l'un ni l'autre),
/// famille (couleur) et clé de la référence IFIP s'il y en a une.
struct Case {
    cle: &'static str,
    libelle: &'static str,
    unite: &'static str,
    decimales: usize,
    sens: i8,
    famille: &'static str,
    ifip: Option<&'static str>,
    aide: &'static str,
}

const CASES: &[Case] = &[
    Case { cle: "marge", libelle: "Marge sur coût alimentaire et renouvellement", unite: "€", decimales: 0, sens: 1, famille: "marge", ifip: None, aide: "Produits animaux − aliment − renouvellement, par truie présente et par an." },
    Case { cle: "produit", libelle: "Produits animaux", unite: "€", decimales: 0, sens: 1, famille: "produit", ifip: None, aide: "Ventes + variation des porcs en bâtiment, par truie présente et par an." },
    Case { cle: "kg_vif", libelle: "Kg vif / truie", unite: "kg", decimales: 0, sens: 1, famille: "produit", ifip: Some("kg_vifs_truie_an"), aide: "Kilos vifs produits par truie présente et par an." },
    Case { cle: "porcs", libelle: "Porcs produits / truie", unite: "", decimales: 1, sens: 1, famille: "produit2", ifip: Some("porcs_truie_an"), aide: "Porcs produits par truie présente et par an." },
    Case { cle: "sevres_portee", libelle: "Sevrés / portée", unite: "", decimales: 1, sens: 1, famille: "produit2", ifip: None, aide: "Moyenne des porcelets sevrés par portée sevrée sur la période." },
    Case { cle: "pertes_sv", libelle: "% pertes sevrage-vente", unite: "%", decimales: 1, sens: -1, famille: "produit2", ifip: Some("sv_taux_pertes"), aide: "Morts et saisies du sevrage à la vente, sur les porcelets sevrés." },
    Case { cle: "poids_vente", libelle: "Poids de vente", unite: "kg", decimales: 1, sens: 1, famille: "produit2", ifip: Some("engr_poids_sortie"), aide: "Poids vif moyen des porcs vendus." },
    Case { cle: "gmq_sv", libelle: "GMQ sevrage-vente", unite: "g/j", decimales: 0, sens: 1, famille: "produit2", ifip: Some("sv_gmq_8_115"), aide: "Croissance du sevrage à la vente (durée selon les réglages de conduite)." },
    Case { cle: "prix_vente", libelle: "Prix de vente", unite: "€/kg", decimales: 3, sens: 1, famille: "produit", ifip: None, aide: "Montant HT des apports divisé par le poids carcasse (truies de réforme à part)." },
    Case { cle: "gamme", libelle: "% gamme", unite: "%", decimales: 1, sens: 1, famille: "produit2", ifip: Some("sv_pct_gamme"), aide: "Porcs dans la gamme de poids, d'après les bons d'apport." },
    Case { cle: "tmp", libelle: "TMP", unite: "", decimales: 1, sens: 1, famille: "produit2", ifip: Some("sv_tmp"), aide: "Taux de muscle des pièces moyen, d'après les bons d'apport." },
    Case { cle: "plus_value", libelle: "Plus-value", unite: "€/kg", decimales: 3, sens: 1, famille: "produit2", ifip: None, aide: "Plus-value moyenne des bons d'apport, pondérée par le poids." },
    Case { cle: "aliment", libelle: "Aliment", unite: "€", decimales: 0, sens: -1, famille: "aliment", ifip: None, aide: "Factures d'aliment par truie présente et par an." },
    Case { cle: "ic_global", libelle: "Indice global", unite: "", decimales: 2, sens: -1, famille: "aliment", ifip: Some("ic_global"), aide: "Tout l'aliment de l'élevage divisé par les kilos produits." },
    Case { cle: "aliment_total_truie", libelle: "Aliment / truie + porcs", unite: "kg", decimales: 0, sens: 0, famille: "aliment2", ifip: None, aide: "Tout l'aliment livré, par truie présente et par an." },
    Case { cle: "conso_truie", libelle: "Aliment truies / truie", unite: "kg", decimales: 0, sens: -1, famille: "aliment2", ifip: Some("aliment_truie_an"), aide: "Aliment gestante et allaitante par truie présente et par an." },
    Case { cle: "ic_sv", libelle: "IC sevrage-vente", unite: "", decimales: 2, sens: -1, famille: "aliment2", ifip: Some("sv_ic_8_115"), aide: "Aliment post-sevrage et engraissement divisé par les kilos produits." },
    Case { cle: "prix_aliment", libelle: "Prix aliment", unite: "€/t", decimales: 0, sens: -1, famille: "aliment", ifip: None, aide: "Prix moyen payé par tonne d'aliment livrée." },
    Case { cle: "cout_tonne_croit", libelle: "Coût aliment / tonne produite", unite: "€", decimales: 0, sens: -1, famille: "aliment2", ifip: None, aide: "Coût de l'aliment pour produire une tonne de porc vif." },
    Case { cle: "renouvellement", libelle: "Renouvellement", unite: "€", decimales: 0, sens: -1, famille: "renouv", ifip: None, aide: "Cochettes et semence par truie présente et par an." },
    Case { cle: "cout_ia", libelle: "Coût IA / truie", unite: "€", decimales: 0, sens: -1, famille: "renouv2", ifip: None, aide: "Factures de semence par truie présente et par an." },
    Case { cle: "pertes_truies", libelle: "% pertes truies / an", unite: "%", decimales: 1, sens: -1, famille: "renouv2", ifip: None, aide: "Truies mortes, euthanasiées ou accidentées, ramenées à l'année." },
    Case { cle: "cochette", libelle: "Cochettes (€/U)", unite: "€", decimales: 0, sens: -1, famille: "renouv2", ifip: None, aide: "Prix moyen d'une cochette achetée." },
    Case { cle: "reforme", libelle: "Réforme (€/U)", unite: "€", decimales: 0, sens: 1, famille: "renouv2", ifip: None, aide: "Prix moyen d'une truie de réforme vendue." },
];

/// Forme de l'arbre : chaque case et ses cases filles, de gauche à droite.
const ENFANTS: &[(&str, &[&str])] = &[
    ("marge", &["produit", "aliment", "renouvellement"]),
    ("produit", &["kg_vif", "prix_vente"]),
    ("kg_vif", &["porcs", "poids_vente"]),
    ("porcs", &["sevres_portee", "pertes_sv"]),
    ("poids_vente", &["gmq_sv"]),
    ("prix_vente", &["gamme", "tmp", "plus_value"]),
    ("aliment", &["ic_global", "prix_aliment"]),
    ("ic_global", &["aliment_total_truie", "conso_truie", "ic_sv"]),
    ("prix_aliment", &["cout_tonne_croit"]),
    ("renouvellement", &["cout_ia", "pertes_truies", "cochette", "reforme"]),
];

const COMPARAISONS: [(&str, &str); 3] = [
    ("precedente", "Période précédente"),
    ("ifip", "Références IFIP (moyenne nationale)"),
    ("cooperl", "Chiffres Cooperl (saisis)"),
];

type Valeurs = BTreeMap<&'static str, Option<f64>>;

/// Nombre écrit à la française : espace pour les milliers, virgule.
pub(super) fn nombre_fr(v: f64, decimales: usize) -> String {
    let brut = format!("{:.*}", decimales, v.abs());
    let (entier, decimal) = match brut.split_once('.') {
        Some((e, d)) => (e.to_string(), Some(d.to_string())),
        None => (brut.clone(), None),
    };
    let mut groupes = String::new();
    for (i, c) in entier.chars().enumerate() {
        if i > 0 && (entier.len() - i) % 3 == 0 {
            groupes.push('\u{202f}');
        }
        groupes.push(c);
    }
    let signe = if v < 0.0 && brut.chars().any(|c| c.is_ascii_digit() && c != '0') { "-" } else { "" };
    match decimal {
        Some(d) => format!("{signe}{groupes},{d}"),
        None => format!("{signe}{groupes}"),
    }
}

fn avec_unite(v: f64, case: &Case, signe: bool) -> String {
    let n = nombre_fr(v, case.decimales);
    let n = if signe && v > 0.0 && n.chars().any(|c| c.is_ascii_digit() && c != '0') { format!("+{n}") } else { n };
    if case.unite.is_empty() { n } else if case.unite == "%" { format!("{n} %") } else { format!("{n} {}", case.unite) }
}

/// Classe de couleur de l'écart : « bon », « mauvais » ou « neutre ».
pub(super) fn classe_ecart(ecart: f64, sens: i8, decimales: usize) -> &'static str {
    let seuil = 0.5 * 10_f64.powi(-(decimales as i32));
    if ecart.abs() < seuil || sens == 0 {
        "neutre"
    } else if (ecart > 0.0) == (sens > 0) {
        "bon"
    } else {
        "mauvais"
    }
}

const EXTRAS_SQL: &str = "SELECT \
    (SELECT CAST(AVG(nb_sevres) AS REAL) FROM evenement WHERE type='sevrage' AND nb_sevres>0 AND date BETWEEN ?1 AND ?2),\
    (SELECT CAST(SUM(montant_ht) AS REAL)/NULLIF(SUM(poids_total),0) FROM venteapport WHERE date BETWEEN ?1 AND ?2 AND montant_ht>0 AND poids_total>0 AND COALESCE(lots_json,'') NOT LIKE '%\"reforme\":true%'),\
    (SELECT CAST(SUM(plus_value*poids_total) AS REAL)/NULLIF(SUM(poids_total),0) FROM venteapport WHERE date BETWEEN ?1 AND ?2 AND plus_value IS NOT NULL AND poids_total>0 AND COALESCE(lots_json,'') NOT LIKE '%\"reforme\":true%'),\
    (SELECT COUNT(*) FROM truie WHERE reformee=1 AND date(date_reforme) BETWEEN ?1 AND ?2 AND (lower(motif_sortie) LIKE '%mort%' OR lower(motif_sortie) LIKE '%euthan%' OR lower(motif_sortie) LIKE '%accident%')),\
    (SELECT CAST(SUM(montant_ht) AS REAL)/NULLIF(SUM(nb_animaux),0) FROM achatgenetique WHERE date BETWEEN ?1 AND ?2 AND nb_animaux>0 AND montant_ht>0),\
    (SELECT CAST(SUM(montant_ht) AS REAL)/NULLIF(SUM(nb_porcs),0) FROM venteapport WHERE date BETWEEN ?1 AND ?2 AND montant_ht>0 AND nb_porcs>0 AND COALESCE(lots_json,'') LIKE '%\"reforme\":true%')";

/// Valeur d'une ligne de la GTE, retrouvée par le titre du bloc et le début
/// du libellé.
fn ligne_gte(blocs: &[Value], titre: &str, debut: &str) -> Option<f64> {
    blocs
        .iter()
        .find(|b| b["titre"] == titre)?
        .get("lignes")?
        .as_array()?
        .iter()
        .find(|l| l["libelle"].as_str().is_some_and(|x| x.starts_with(debut)))?
        .get("valeur")?
        .as_f64()
}

/// Toutes les cases de l'élevage pour une période.
pub(super) async fn indicateurs(
    pool: &SqlitePool,
    debut: &str,
    fin: &str,
    rendement: f64,
    conduite: gte_periode::Conduite,
    orientation: &Value,
) -> AppResult<Valeurs> {
    let jours = gte_periode::jours_periode(debut, fin).unwrap_or(0);
    let d = gte_periode::charger(pool, debut, fin).await?;
    let mut c = gte_periode::charger_couts(pool, debut, fin).await?;
    if let Some(kg_vifs) = gte_periode::poids_vif(d.poids_carcasse_kg, rendement).filter(|kg| *kg > 0.0) {
        c.produit_animaux += c.produit_animaux / kg_vifs * d.kg_variation_stock;
    }
    let blocs = gte_periode::tableau(&d, jours, rendement, conduite, orientation, true);
    let truies = d.truies_presentes();
    let an = |x: f64| gte_periode::par_truie_et_par_an(x, truies, jours);
    let sans_trou = d.aliment_trou.is_none();

    let produit = an(c.produit_animaux);
    let aliment = if sans_trou { an(c.aliment) } else { None };
    let renouvellement = an(c.genetique + c.semence);
    let marge = match (produit, aliment, renouvellement) {
        (Some(p), Some(a), Some(r)) => Some(p - a - r),
        _ => None,
    };
    let kg_vif = ligne_gte(&blocs, "Résultats techniques", "Kilos vifs");
    let (sevres_portee, prix_vente, plus_value, truies_mortes, cochette, reforme): (
        Option<f64>,
        Option<f64>,
        Option<f64>,
        i64,
        Option<f64>,
        Option<f64>,
    ) = sqlx::query_as(EXTRAS_SQL).bind(debut).bind(fin).fetch_one(pool).await?;
    let pertes_truies = (truies > 0.0 && jours > 0).then(|| truies_mortes as f64 / truies * 365.0 / jours as f64 * 100.0);
    let prix_aliment = (sans_trou && d.aliment_total_kg > 0.0).then(|| c.aliment / (d.aliment_total_kg / 1000.0));
    let cout_tonne_croit = aliment.zip(kg_vif).filter(|(_, kg)| *kg > 0.0).map(|(a, kg)| a / kg * 1000.0);

    let mut v = Valeurs::new();
    v.insert("marge", marge);
    v.insert("produit", produit);
    v.insert("kg_vif", kg_vif);
    v.insert("porcs", ligne_gte(&blocs, "Résultats techniques", "Porcs produits"));
    v.insert("sevres_portee", sevres_portee);
    v.insert("pertes_sv", ligne_gte(&blocs, "Sevrage-vente", "Taux de pertes"));
    v.insert("poids_vente", ligne_gte(&blocs, "Engraissement", "Poids moyen de sortie"));
    v.insert("gmq_sv", ligne_gte(&blocs, "Sevrage-vente", "GMQ réel"));
    v.insert("prix_vente", prix_vente);
    v.insert("gamme", d.pct_gamme);
    v.insert("tmp", d.tmp);
    v.insert("plus_value", plus_value);
    v.insert("aliment", aliment);
    v.insert("ic_global", ligne_gte(&blocs, "Résultats techniques", "Indice de consommation global"));
    v.insert("aliment_total_truie", if sans_trou { an(d.aliment_total_kg) } else { None });
    v.insert("conso_truie", ligne_gte(&blocs, "Résultats techniques", "Consommation aliment truies"));
    v.insert("ic_sv", ligne_gte(&blocs, "Sevrage-vente", "Indice de consommation réel"));
    v.insert("prix_aliment", prix_aliment);
    v.insert("cout_tonne_croit", cout_tonne_croit);
    v.insert("renouvellement", renouvellement);
    v.insert("cout_ia", an(c.semence));
    v.insert("pertes_truies", pertes_truies);
    v.insert("cochette", cochette);
    v.insert("reforme", reforme);
    // Une valeur non finie (division par zéro) ne s'affiche pas.
    for valeur in v.values_mut() {
        if valeur.is_some_and(|x| !x.is_finite()) {
            *valeur = None;
        }
    }
    Ok(v)
}

/// Construit l'arbre (récursif) pour le gabarit.
fn noeud(cle: &str, moi: &Valeurs, comp: &Valeurs) -> Value {
    let case = CASES.iter().find(|c| c.cle == cle).expect("case connue");
    let a = moi.get(case.cle).copied().flatten();
    let b = comp.get(case.cle).copied().flatten();
    let ecart = a.zip(b).map(|(a, b)| a - b);
    let enfants: Vec<Value> = ENFANTS
        .iter()
        .find(|(p, _)| *p == cle)
        .map(|(_, e)| e.iter().map(|k| noeud(k, moi, comp)).collect())
        .unwrap_or_default();
    json!({
        "cle": case.cle,
        "libelle": case.libelle,
        "famille": case.famille,
        "aide": case.aide,
        "moi": a.map(|x| avec_unite(x, case, false)),
        "comp": b.map(|x| avec_unite(x, case, false)),
        "ecart": ecart.map(|x| avec_unite(x, case, true)),
        "classe": ecart.map(|x| classe_ecart(x, case.sens, case.decimales)).unwrap_or("neutre"),
        "enfants": enfants,
    })
}

async fn reference_cooperl(pool: &SqlitePool) -> AppResult<(Valeurs, Option<String>)> {
    let lignes: Vec<(String, Option<f64>)> =
        sqlx::query_as("SELECT cle,CAST(valeur AS REAL) FROM arbre_reference").fetch_all(pool).await?;
    let mut v = Valeurs::new();
    for case in CASES {
        v.insert(case.cle, lignes.iter().find(|(k, _)| k == case.cle).and_then(|(_, x)| *x));
    }
    let libelle: Option<String> =
        sqlx::query_scalar("SELECT valeur FROM parametre WHERE cle='arbre_reference_libelle'")
            .fetch_optional(pool)
            .await?
            .flatten();
    Ok((v, libelle))
}

fn date_fr(iso: &str) -> String {
    NaiveDate::parse_from_str(iso, "%Y-%m-%d").map(|d| d.format("%d/%m/%Y").to_string()).unwrap_or_else(|_| iso.to_string())
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let lire = |nom: &str| -> AppResult<Option<String>> {
        match query.get(nom).map(|v| v.trim()).filter(|v| !v.is_empty()) {
            Some(v) => parse_stored_date(v)
                .map(|d| Some(d.format("%Y-%m-%d").to_string()))
                .ok_or_else(|| AppError::Invalid(format!("Date {nom} invalide"))),
            None => Ok(None),
        }
    };
    let (defaut_debut, defaut_fin) = gte_periode::periode_par_defaut();
    let debut = lire("debut")?.unwrap_or(defaut_debut);
    let fin = lire("fin")?.unwrap_or(defaut_fin);
    if debut > fin {
        return Err(AppError::Invalid("La date de début doit précéder la date de fin".into()));
    }
    let comparer = query
        .get("comparer")
        .map(String::as_str)
        .filter(|c| COMPARAISONS.iter().any(|(k, _)| k == c))
        .unwrap_or("precedente")
        .to_string();

    let (orientation, _livret, rendement_defaut) = gte_periode::references(&session.type_elevage)?;
    let rendement = reglage_i64(&state.pool, "rendement_carcasse_pct", 0)
        .await
        .ok()
        .filter(|v| *v > 0)
        .map(|v| v as f64)
        .unwrap_or(rendement_defaut);
    let conduite = gte_periode::Conduite {
        sevrage_j: reglage_i64(&state.pool, "sevrage", 28).await?,
        transfert_engraissement_j: reglage_i64(&state.pool, "transfert_engr", 71).await?,
        depart_j: reglage_i64(&state.pool, "depart", 215).await?,
    };
    let moi = indicateurs(&state.pool, &debut, &fin, rendement, conduite, &orientation).await?;

    let (cooperl, cooperl_libelle) = reference_cooperl(&state.pool).await?;
    let (comp, comp_libelle) = match comparer.as_str() {
        "ifip" => {
            let mut v = Valeurs::new();
            for case in CASES {
                v.insert(
                    case.cle,
                    case.ifip.and_then(|k| orientation.get("valeurs")?.get(k)?.as_f64()),
                );
            }
            (v, "IFIP GT-Porc 2024".to_string())
        }
        "cooperl" => (
            cooperl.clone(),
            cooperl_libelle.clone().unwrap_or_else(|| "Référence Cooperl".into()),
        ),
        _ => {
            let periodes = gte_periode::periodes_precedentes(&debut, &fin, 2);
            match periodes.get(1) {
                Some((d, f)) => (
                    indicateurs(&state.pool, d, f, rendement, conduite, &orientation).await?,
                    format!("Du {} au {}", date_fr(d), date_fr(f)),
                ),
                None => (Valeurs::new(), "Période précédente".into()),
            }
        }
    };

    let arbre = noeud("marge", &moi, &comp);
    let saisie: Vec<Value> = CASES
        .iter()
        .map(|c| {
            json!({
                "cle": c.cle,
                "libelle": c.libelle,
                "unite": c.unite,
                "valeur": cooperl.get(c.cle).copied().flatten(),
            })
        })
        .collect();
    let nom_elevage: Option<String> =
        sqlx::query_scalar("SELECT valeur FROM parametre WHERE cle='nom_elevage'")
            .fetch_optional(&state.pool)
            .await?
            .flatten();

    let mut ctx = context(&session);
    ctx.insert("arbre".into(), arbre);
    ctx.insert("debut".into(), json!(debut));
    ctx.insert("fin".into(), json!(fin));
    ctx.insert("debut_fr".into(), json!(date_fr(&debut)));
    ctx.insert("fin_fr".into(), json!(date_fr(&fin)));
    ctx.insert("comparer".into(), json!(comparer));
    ctx.insert(
        "comparaisons".into(),
        json!(COMPARAISONS.iter().map(|(k, l)| json!({"cle": k, "libelle": l})).collect::<Vec<_>>()),
    );
    ctx.insert("comp_libelle".into(), json!(comp_libelle));
    ctx.insert("saisie".into(), json!(saisie));
    ctx.insert("cooperl_libelle".into(), json!(cooperl_libelle));
    ctx.insert("cooperl_vide".into(), json!(cooperl.values().all(Option::is_none)));
    ctx.insert("nom_elevage".into(), json!(nom_elevage));
    render(&state, "diagramme_resultat.html", Value::Object(ctx))
}

/// Enregistre les chiffres de référence Cooperl (une valeur par case ; une
/// case vide efface la valeur).
pub(super) async fn enregistrer_reference(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let mut tx = state.pool.begin().await?;
    for case in CASES {
        match form_f64(&form, &format!("ref_{}", case.cle)) {
            Some(v) => {
                sqlx::query("INSERT INTO arbre_reference(cle,valeur) VALUES(?,?) ON CONFLICT(cle) DO UPDATE SET valeur=excluded.valeur")
                    .bind(case.cle)
                    .bind(v)
                    .execute(&mut *tx)
                    .await?;
            }
            None => {
                sqlx::query("DELETE FROM arbre_reference WHERE cle=?").bind(case.cle).execute(&mut *tx).await?;
            }
        }
    }
    match form_text(&form, "libelle") {
        Some(l) => {
            sqlx::query("INSERT INTO parametre(cle,valeur) VALUES('arbre_reference_libelle',?) ON CONFLICT(cle) DO UPDATE SET valeur=excluded.valeur")
                .bind(l)
                .execute(&mut *tx)
                .await?;
        }
        None => {
            sqlx::query("DELETE FROM parametre WHERE cle='arbre_reference_libelle'").execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    let retour = |k: &str| form_text(&form, k).unwrap_or_default();
    let url = format!(
        "/diagramme-resultat?debut={}&fin={}&comparer=cooperl",
        retour("debut"),
        retour("fin")
    );
    Ok(Redirect::to(&url).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nombres_et_couleurs_des_ecarts() {
        assert_eq!(nombre_fr(2684.4, 0), "2\u{202f}684");
        assert_eq!(nombre_fr(-1482.0, 0), "-1\u{202f}482");
        assert_eq!(nombre_fr(1.7514, 3), "1,751");
        assert_eq!(nombre_fr(-0.0001, 2), "0,00");
        // Indice global plus haut que la référence : mauvais.
        assert_eq!(classe_ecart(0.18, -1, 2), "mauvais");
        // Prix de vente plus haut : bon.
        assert_eq!(classe_ecart(0.083, 1, 3), "bon");
        assert_eq!(classe_ecart(0.0004, 1, 3), "neutre");
        assert_eq!(classe_ecart(500.0, 0, 0), "neutre");
    }

    #[test]
    fn chaque_case_de_l_arbre_existe_une_seule_fois() {
        let mut vues = vec!["marge"];
        for (parent, enfants) in ENFANTS {
            assert!(CASES.iter().any(|c| c.cle == *parent), "{parent}");
            for e in *enfants {
                assert!(CASES.iter().any(|c| c.cle == *e), "{e}");
                assert!(!vues.contains(e), "{e} deux fois");
                vues.push(e);
            }
        }
        assert_eq!(vues.len(), CASES.len(), "toutes les cases sont dans l'arbre");
    }

    #[tokio::test]
    async fn calcul_sur_une_base_avec_ventes_et_achats() {
        let state = crate::routes::documents::tests::state().await;
        let p = &state.pool;
        sqlx::raw_sql(
            "INSERT INTO venteapport(id,date,nb_porcs,poids_total,montant_ht,plus_value,tmp,lots_json) VALUES\
             (1,'2026-05-10',60,5700,10000,0.25,61.5,'[]'),(2,'2026-05-20',2,400,500,NULL,NULL,'[{\"reforme\":true}]');\
             INSERT INTO achatgenetique(id,date,designation,nb_animaux,montant_ht) VALUES(1,'2026-05-02','Cochettes',4,1400);",
        )
        .execute(p)
        .await
        .unwrap();
        let (orientation, _, rendement) = gte_periode::references("naisseur_engraisseur").unwrap();
        let conduite = gte_periode::Conduite { sevrage_j: 28, transfert_engraissement_j: 71, depart_j: 215 };
        let v = indicateurs(p, "2026-04-01", "2026-09-30", rendement, conduite, &orientation).await.unwrap();
        assert!((v["prix_vente"].unwrap() - 10000.0 / 5700.0).abs() < 1e-9);
        assert!((v["plus_value"].unwrap() - 0.25).abs() < 1e-9);
        assert_eq!(v["cochette"], Some(350.0));
        assert_eq!(v["reforme"], Some(250.0));
        assert_eq!(v.len(), CASES.len());
        let arbre = noeud("marge", &v, &Valeurs::new());
        assert_eq!(arbre["enfants"].as_array().unwrap().len(), 3);
    }

    fn session() -> SessionData {
        SessionData {
            uid: 1,
            identifiant: "test".into(),
            nom: "Test".into(),
            role: "admin".into(),
            sections: vec![],
            csrf: "test".into(),
            doit_changer_mdp: false,
            type_elevage: "naisseur_engraisseur".into(),
            module_genetique: false,
            module_prestataires: false,
            module_charcutiers_rfid: false,
            module_vente_directe: false,
            module_equipements: false,
        }
    }

    #[tokio::test]
    async fn la_page_s_affiche_pour_les_trois_comparaisons() {
        let state = crate::routes::documents::tests::state().await;
        for comparer in ["precedente", "ifip", "cooperl"] {
            let q: HashMap<String, String> = [
                ("debut".to_string(), "2026-04-01".to_string()),
                ("fin".to_string(), "2026-10-12".to_string()),
                ("comparer".to_string(), comparer.to_string()),
            ]
            .into_iter()
            .collect();
            let Html(page_html) = page(State(state.clone()), Extension(session()), Query(q)).await.unwrap();
            assert!(page_html.contains("Marge sur coût alimentaire et renouvellement"), "{comparer}");
            assert!(page_html.contains("Cochettes"), "{comparer}");
        }
    }
}
