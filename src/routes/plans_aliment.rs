//! Présentation des plans d'alimentation (Paramètres) : regroupement par
//! catégorie, frise des jours et contrôle de couverture.
//!
//! Chaque catégorie a son jour 0 (insémination, mise-bas, naissance) et une
//! durée attendue tirée des réglages de conduite. Une période sans aucun
//! aliment prévu est signalée : c'est le plus souvent un oubli de saisie
//! (ex. rien entre la fin du 2e âge et le début du 3e).

use super::*;

struct Categorie {
    code: &'static str,
    libelle: &'static str,
    reference: &'static str,
    /// Tableau d'affichage : les stades d'un même animal partagent un tableau.
    tableau: &'static str,
}

const TRUIES: &str = "Truies et cochettes";

const CATEGORIES: [Categorie; 7] = [
    Categorie {
        code: "cochette",
        libelle: "Cochettes",
        reference: "J0 = entrée",
        tableau: TRUIES,
    },
    Categorie {
        code: "truie_ia",
        libelle: "Truies à l’IA",
        reference: "J0 = sevrage",
        tableau: TRUIES,
    },
    Categorie {
        code: "truie_gestante",
        libelle: "Truies gestantes",
        reference: "J0 = insémination",
        tableau: TRUIES,
    },
    Categorie {
        code: "truie_allaitante",
        libelle: "Truies allaitantes",
        reference: "J0 = mise-bas",
        tableau: TRUIES,
    },
    Categorie {
        code: "porcelet",
        libelle: "Porcelets et porcs",
        reference: "J0 = naissance",
        tableau: "",
    },
    Categorie {
        code: "engraissement",
        libelle: "Porcs charcutiers",
        reference: "J0 = naissance",
        tableau: "",
    },
    Categorie {
        code: "verrat",
        libelle: "Verrats",
        reference: "",
        tableau: "",
    },
];

fn libelle(code: &str) -> String {
    CATEGORIES
        .iter()
        .find(|c| c.code == code)
        .map(|c| c.libelle.to_string())
        .unwrap_or_else(|| {
            let texte = code.replace('_', " ");
            let mut lettres = texte.chars();
            lettres
                .next()
                .map(|p| p.to_uppercase().collect::<String>() + lettres.as_str())
                .unwrap_or_default()
        })
}

/// Dernier jour qu'un plan devrait couvrir pour la catégorie.
fn fin_attendue(code: &str, c: BandSchedule) -> Option<i64> {
    match code {
        "truie_gestante" => Some(c.gestation - c.maternity_before_farrowing - 1),
        "truie_allaitante" => Some(c.weaning - 1),
        "porcelet" | "engraissement" => Some(c.departure),
        _ => None,
    }
}

fn periodes(debut: i64, fin: i64) -> String {
    if debut == fin {
        format!("J{debut}")
    } else {
        format!("J{debut} à J{fin}")
    }
}

/// Trous de couverture entre 0 et `fin` (inclus), lignes triées par début.
fn trous(lignes: &[(i64, i64)], fin: Option<i64>) -> Vec<(i64, i64)> {
    let mut sortie = Vec::new();
    let mut couvert = -1;
    for (debut, fin_ligne) in lignes {
        if *debut > couvert + 1 {
            sortie.push((couvert + 1, debut - 1));
        }
        couvert = couvert.max(*fin_ligne);
    }
    if let Some(fin) = fin {
        if couvert < fin {
            sortie.push((couvert + 1, fin));
        }
    }
    sortie
}

pub(super) fn grouper(plans: &[Value], c: BandSchedule) -> Vec<Value> {
    let mut codes: Vec<String> = plans
        .iter()
        .filter_map(|p| p["categorie"].as_str().map(str::to_string))
        // Les stades des truies sont toujours proposés, même encore vides.
        .chain(
            CATEGORIES
                .iter()
                .filter(|c| c.tableau == TRUIES)
                .map(|c| c.code.to_string()),
        )
        .collect();
    codes.sort_by_key(|code| {
        (
            CATEGORIES.iter().position(|c| c.code == code).unwrap_or(99),
            code.clone(),
        )
    });
    codes.dedup();
    codes
        .into_iter()
        .map(|code| {
            let mut lignes: Vec<&Value> = plans
                .iter()
                .filter(|p| p["categorie"].as_str() == Some(code.as_str()))
                .collect();
            lignes.sort_by_key(|p| {
                (
                    p["jour_debut"].as_i64().unwrap_or(0),
                    p["jour_fin"].as_i64().unwrap_or(0),
                )
            });
            let bornes: Vec<(i64, i64)> = lignes
                .iter()
                .map(|p| {
                    let debut = p["jour_debut"].as_i64().unwrap_or(0);
                    (debut, p["jour_fin"].as_i64().unwrap_or(debut).max(debut))
                })
                .collect();
            let attendue = fin_attendue(&code, c);
            let echelle = bornes
                .iter()
                .map(|(_, f)| *f)
                .chain(attendue)
                .max()
                .unwrap_or(1)
                .max(1) as f64
                + 1.0;
            let lignes_json: Vec<Value> = lignes
                .iter()
                .zip(&bornes)
                .enumerate()
                .map(|(i, (p, (debut, fin)))| {
                    let mut ligne = (*p).clone();
                    if let Some(objet) = ligne.as_object_mut() {
                        objet.insert("gauche".into(), json!((100.0 * *debut as f64 / echelle * 10.0).round() / 10.0));
                        objet.insert(
                            "largeur".into(),
                            json!((100.0 * (fin - debut + 1) as f64 / echelle * 10.0).round() / 10.0),
                        );
                        objet.insert("couleur".into(), json!(i % 8));
                        objet.insert("jours".into(), json!(periodes(*debut, *fin)));
                    }
                    ligne
                })
                .collect();
            let alertes: Vec<String> = trous(&bornes, attendue)
                .into_iter()
                .map(|(debut, fin)| {
                    if Some(fin) == attendue && debut > 0 {
                        format!(
                            "Rien de prévu de J{debut} à J{fin} (fin de période attendue d’après la conduite)"
                        )
                    } else {
                        format!("Rien de prévu {}", if debut == fin { format!("le J{debut}") } else { format!("de J{debut} à J{fin}") })
                    }
                })
                .collect();
            let reperes: Vec<Value> = (0..=(echelle as i64))
                .step_by(if echelle > 120.0 { 28 } else if echelle > 40.0 { 14 } else { 7 })
                .map(|j| json!({"jour": j, "gauche": (100.0 * j as f64 / echelle * 10.0).round() / 10.0}))
                .collect();
            let info = CATEGORIES.iter().find(|x| x.code == code);
            json!({
                "code": code,
                "libelle": libelle(&code),
                "reference": info.map(|x| x.reference).unwrap_or(""),
                "tableau": info.map(|x| x.tableau).filter(|t| !t.is_empty()),
                "fin_attendue": attendue,
                "attendu_gauche": attendue.map(|f| (100.0 * (f + 1) as f64 / echelle * 10.0).round() / 10.0),
                "lignes": lignes_json,
                "alertes": alertes,
                "reperes": reperes,
            })
        })
        .collect()
}

/// Regroupe les plans par tableau d'affichage (les stades des truies ensemble).
pub(super) fn tableaux(plans: &[Value], c: BandSchedule) -> Vec<Value> {
    let mut sortie: Vec<(String, Vec<Value>)> = Vec::new();
    for groupe in grouper(plans, c) {
        let nom = groupe["tableau"]
            .as_str()
            .or(groupe["libelle"].as_str())
            .unwrap_or_default()
            .to_string();
        match sortie.iter_mut().find(|(n, _)| *n == nom) {
            Some((_, liste)) => liste.push(groupe),
            None => sortie.push((nom, vec![groupe])),
        }
    }
    sortie
        .into_iter()
        .map(|(libelle, plans)| json!({"libelle": libelle, "plans": plans}))
        .collect()
}

/// Catégories connues qui n'ont encore aucun tableau affiché.
pub(super) fn categories_sans_plan(plans: &[Value]) -> Vec<Value> {
    CATEGORIES
        .iter()
        .filter(|c| c.tableau != TRUIES)
        .filter(|c| !plans.iter().any(|p| p["categorie"].as_str() == Some(c.code)))
        .map(|c| json!({"code": c.code, "libelle": c.libelle}))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trous_de_couverture() {
        // Porcelets de l'élevage au 23/09/2026 : rien entre J36 et J41.
        let lignes = [(0, 28), (4, 10), (19, 35), (42, 70), (71, 140), (141, 210)];
        assert_eq!(trous(&lignes, Some(215)), vec![(36, 41), (211, 215)]);
        // Allaitantes : J4-J5 vides, puis rien après J22 jusqu'au sevrage.
        assert_eq!(trous(&[(0, 3), (6, 22)], Some(27)), vec![(4, 5), (23, 27)]);
        assert!(trous(&[(0, 10), (5, 20)], Some(20)).is_empty());
    }

    #[test]
    fn regroupement_et_libelles() {
        let plans = vec![
            json!({"id":1,"categorie":"porcelet","jour_debut":42,"jour_fin":70,"aliment":"B"}),
            json!({"id":2,"categorie":"truie_gestante","jour_debut":0,"jour_fin":84,"aliment":"G"}),
            json!({"id":3,"categorie":"porcelet","jour_debut":0,"jour_fin":28,"aliment":"A"}),
        ];
        let groupes = grouper(&plans, BandSchedule::default());
        let groupe = |code: &str| groupes.iter().find(|g| g["code"] == code).unwrap();
        assert_eq!(groupe("truie_gestante")["libelle"], "Truies gestantes");
        // Triés par jour de début, pas par ordre de saisie.
        assert_eq!(groupe("porcelet")["lignes"][0]["aliment"], "A");
        assert_eq!(libelle("post_sevrage"), "Post sevrage");
        // Un seul tableau pour cochettes, IA, gestantes et allaitantes, même vides.
        let tableaux = tableaux(&plans, BandSchedule::default());
        assert_eq!(tableaux[0]["libelle"], "Truies et cochettes");
        assert_eq!(tableaux[0]["plans"].as_array().unwrap().len(), 4);
        assert_eq!(tableaux[1]["libelle"], "Porcelets et porcs");
        assert_eq!(categories_sans_plan(&plans).len(), 2);
        // Gestantes : attendu jusqu'à l'entrée en maternité (J109).
        assert!(groupe("truie_gestante")["alertes"][0]
            .as_str()
            .unwrap()
            .contains("J85 à J109"));
    }
}
