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
        reference: "J0 = insémination",
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

fn messages_trous(bornes: &[(i64, i64)], attendue: Option<i64>) -> Vec<String> {
    trous(bornes, attendue)
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
        .collect()
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
            let alertes = messages_trous(&bornes, attendue);
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
        .map(|(libelle, plans)| {
            let choix: Vec<Value> = plans
                .iter()
                .map(|g| json!({"code": g["code"], "libelle": g["libelle"]}))
                .collect();
            let plans = if plans.len() > 1 {
                vec![fusionner(&libelle, &plans)]
            } else {
                plans
            };
            json!({"libelle": libelle, "plans": plans, "choix": choix})
        })
        .collect()
}

/// Met bout à bout les stades d'un même animal sur une seule frise. L'IA et
/// la gestation comptent leurs jours depuis la même insémination : elles
/// partagent le même J0 (IA J0 à J20, gestation J21 à J115 par exemple).
fn fusionner(libelle: &str, plans: &[Value]) -> Value {
    let axe = |g: &Value| match g["code"].as_str().unwrap_or_default() {
        "truie_ia" | "truie_gestante" => "insemination".to_string(),
        code => code.to_string(),
    };
    let mut troncons: Vec<Vec<&Value>> = Vec::new();
    for g in plans {
        match troncons.last_mut() {
            Some(t) if axe(t[0]) == axe(g) => t.push(g),
            _ => troncons.push(vec![g]),
        }
    }
    let mut depart = 0_i64;
    let mut stades: Vec<(String, String, i64, i64)> = Vec::new();
    let mut durees: Vec<(i64, i64)> = Vec::new();
    let mut vides: Vec<String> = Vec::new();
    let mut lignes: Vec<(i64, i64, String, Value)> = Vec::new();
    let mut alertes: Vec<String> = Vec::new();
    for troncon in troncons {
        let mut etiquettes: Vec<(String, String, i64)> = Vec::new();
        let mut du_troncon: Vec<(i64, i64, String, Value)> = Vec::new();
        let mut attendue: Option<i64> = None;
        let mut couvert = -1_i64;
        for g in troncon {
            let nom = g["libelle"].as_str().unwrap_or_default().to_string();
            let du_stade = g["lignes"].as_array().cloned().unwrap_or_default();
            let fin_stade = g["fin_attendue"].as_i64();
            if du_stade.is_empty() && fin_stade.is_none() {
                vides.push(nom);
                continue;
            }
            let bornes: Vec<(i64, i64)> = du_stade
                .iter()
                .map(|p| {
                    let debut = p["jour_debut"].as_i64().unwrap_or(0);
                    (debut, p["jour_fin"].as_i64().unwrap_or(debut).max(debut))
                })
                .collect();
            let debut_stade = if etiquettes.is_empty() {
                0
            } else {
                bornes.iter().map(|b| b.0).min().unwrap_or(couvert + 1)
            };
            let reference = g["reference"].as_str().unwrap_or_default().to_string();
            etiquettes.push((nom.clone(), reference, debut_stade));
            couvert = bornes.iter().map(|b| b.1).fold(couvert, i64::max);
            attendue = attendue.max(fin_stade);
            for (p, (debut, fin)) in du_stade.into_iter().zip(bornes) {
                du_troncon.push((debut, fin, nom.clone(), p));
            }
        }
        if etiquettes.is_empty() {
            continue;
        }
        du_troncon.sort_by_key(|l| (l.0, l.1));
        let bornes: Vec<(i64, i64)> = du_troncon.iter().map(|l| (l.0, l.1)).collect();
        let noms: Vec<&str> = etiquettes.iter().map(|e| e.0.as_str()).collect();
        for message in messages_trous(&bornes, attendue) {
            alertes.push(format!("{} : {message}", noms.join(" et ")));
        }
        let duree = couvert.max(attendue.unwrap_or(0)) + 1;
        for (i, (nom, reference, debut)) in etiquettes.iter().enumerate() {
            let suivant = etiquettes.get(i + 1).map(|e| e.2).unwrap_or(duree);
            stades.push((nom.clone(), reference.clone(), depart + debut, (suivant - debut).max(1)));
        }
        for (debut, fin, nom, p) in du_troncon {
            lignes.push((depart + debut, depart + fin, nom, p));
        }
        durees.push((depart, duree));
        depart += duree;
    }
    let echelle = depart.max(1) as f64;
    let pct = |v: i64| (100.0 * v as f64 / echelle * 10.0).round() / 10.0;
    let pas: i64 = if echelle > 120.0 { 28 } else if echelle > 40.0 { 14 } else { 7 };
    let reperes: Vec<Value> = durees
        .iter()
        .flat_map(|(debut, duree)| {
            (0..*duree)
                .step_by(pas as usize)
                .filter(move |j| *j == 0 || duree - j >= pas / 2)
                .map(move |j| (j, debut + j))
        })
        .map(|(j, position)| json!({"jour": j, "gauche": pct(position)}))
        .collect();
    let lignes: Vec<Value> = lignes
        .into_iter()
        .enumerate()
        .map(|(i, (debut, fin, stade, mut ligne))| {
            if let Some(objet) = ligne.as_object_mut() {
                objet.insert("gauche".into(), json!(pct(debut)));
                objet.insert("largeur".into(), json!(pct(fin - debut + 1)));
                objet.insert("couleur".into(), json!(i % 8));
                objet.insert("stade".into(), json!(stade));
            }
            ligne
        })
        .collect();
    let stades: Vec<Value> = stades
        .into_iter()
        .map(|(nom, reference, debut, duree)| {
            json!({"libelle": nom, "reference": reference, "gauche": pct(debut), "largeur": pct(duree)})
        })
        .collect();
    json!({
        "code": "",
        "libelle": libelle,
        "reference": "cycle complet : IA et gestation comptées depuis l’insémination",
        "fin_attendue": null,
        "attendu_gauche": null,
        "lignes": lignes,
        "alertes": alertes,
        "reperes": reperes,
        "stades": stades,
        "vides": vides,
    })
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
        // Un seul tableau pour cochettes, IA, gestantes et allaitantes.
        let tableaux = tableaux(&plans, BandSchedule::default());
        assert_eq!(tableaux[0]["libelle"], "Truies et cochettes");
        assert_eq!(tableaux[0]["choix"].as_array().unwrap().len(), 4);
        // Une seule frise : gestation (J0 à J109) puis maternité à la suite.
        let cycle = &tableaux[0]["plans"][0];
        assert_eq!(tableaux[0]["plans"].as_array().unwrap().len(), 1);
        assert_eq!(cycle["stades"].as_array().unwrap().len(), 2);
        assert_eq!(cycle["vides"].as_array().unwrap().len(), 2);
        assert_eq!(cycle["lignes"][0]["stade"], "Truies gestantes");
        assert_eq!(cycle["stades"][1]["libelle"], "Truies allaitantes");
        assert_eq!(tableaux[1]["libelle"], "Porcelets et porcs");
        assert_eq!(categories_sans_plan(&plans).len(), 2);
        // Gestantes : attendu jusqu'à l'entrée en maternité (J109).
        assert!(groupe("truie_gestante")["alertes"][0]
            .as_str()
            .unwrap()
            .contains("J85 à J109"));
        {
            // IA et gestation partagent le J0 de l'insémination : pas de trou J0-J20.
            let plans = vec![
                json!({"id":1,"categorie":"truie_ia","jour_debut":0,"jour_fin":20,"aliment":"I"}),
                json!({"id":2,"categorie":"truie_gestante","jour_debut":21,"jour_fin":115,"aliment":"G"}),
                json!({"id":3,"categorie":"truie_allaitante","jour_debut":0,"jour_fin":28,"aliment":"L"}),
            ];
            let tableaux = super::tableaux(&plans, BandSchedule::default());
            let cycle = &tableaux[0]["plans"][0];
            assert!(cycle["alertes"].as_array().unwrap().is_empty());
            assert_eq!(cycle["stades"].as_array().unwrap().len(), 3);
            assert_eq!(cycle["lignes"][1]["gauche"], json!(14.5));
        }
    }
}
