//! Noyau de calcul de la régulation de ventilation.
//!
//! Ce fichier ne dépend de rien : ni base de données, ni réseau, ni async. Il
//! est compilé à la fois dans l'application et dans le démon `eo-regul`, ce qui
//! garantit que les deux affichent et appliquent exactement le même calcul.
//!
//! Règle de conception à ne pas transgresser : le calcul qui décide du
//! pourcentage envoyé au variateur reste une fonction pure, bornée, lisible et
//! testable. Toute « intelligence » (apprentissage, comparaison entre salles,
//! corrélations avec les performances) se place *autour* de cette fonction,
//! pour produire des consignes ou lever des alertes — jamais à l'intérieur.

// Ce fichier est compilé dans deux binaires qui n'en utilisent pas les mêmes
// parties : l'application n'a besoin que de l'interpolation de consigne, le
// démon se sert de tout. Sans cette autorisation, le code inutilisé côté
// application ferait échouer `cargo clippy -- -D warnings`.
#![allow(dead_code)]

/// Un point de courbe : à `jour` jours de présence de la bande dans la salle,
/// la consigne vaut `t_cible`, avec éventuellement des bornes de ventilation
/// propres à ce stade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointCourbe {
    pub jour: i64,
    pub t_cible: f64,
    pub ventil_min: Option<f64>,
    pub ventil_max: Option<f64>,
}

/// Consigne issue d'une courbe, éventuellement assortie de bornes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Consigne {
    pub t_cible: f64,
    pub ventil_min: Option<f64>,
    pub ventil_max: Option<f64>,
}

/// Réglages de régulation d'une salle, tels que stockés dans `ambiance_salle`.
#[derive(Clone, Debug)]
pub struct Reglages {
    pub plage: f64,
    pub ventil_min: f64,
    pub ventil_max: f64,
    pub hygro_cible: Option<f64>,
    pub hygro_influence: f64,
    pub anticipation: f64,
    pub t_plausible_min: f64,
    pub t_plausible_max: f64,
    pub h_plausible_min: f64,
    pub h_plausible_max: f64,
    pub fraicheur_max_s: i64,
    pub ventil_securite: f64,
}

impl Default for Reglages {
    fn default() -> Self {
        Self {
            plage: 3.0,
            ventil_min: 20.0,
            ventil_max: 100.0,
            hygro_cible: None,
            hygro_influence: 0.0,
            anticipation: 0.0,
            t_plausible_min: 2.0,
            t_plausible_max: 45.0,
            h_plausible_min: 5.0,
            h_plausible_max: 99.0,
            fraicheur_max_s: 1800,
            ventil_securite: 60.0,
        }
    }
}

/// Une valeur lue sur une sonde, avec son âge en secondes. Une sonde qui
/// répond toujours la même chose n'est pas une sonde qui va bien : l'âge
/// compte autant que la valeur.
#[derive(Clone, Copy, Debug, Default)]
pub struct Lecture {
    pub valeur: Option<f64>,
    pub age_s: i64,
}

impl Lecture {
    pub fn neuve(valeur: f64) -> Self {
        Self {
            valeur: Some(valeur),
            age_s: 0,
        }
    }

    pub fn absente() -> Self {
        Self {
            valeur: None,
            age_s: i64::MAX,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Régulation normale à partir d'une mesure jugée valide.
    Auto,
    /// Mesure inutilisable : repli sur le pourcentage de sécurité.
    Securite,
    /// Salle non pilotée (configuration inactive).
    Arret,
    /// Salle sans animaux : ventilation tenue au réglage « salle vide ».
    Vide,
}

impl Mode {
    pub fn code(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Securite => "securite",
            Mode::Arret => "arret",
            Mode::Vide => "vide",
        }
    }
}

/// Résultat d'un cycle de régulation.
#[derive(Clone, Debug)]
pub struct Decision {
    pub ventil: f64,
    pub mode: Mode,
    pub sonde_ok: bool,
    pub message: Option<String>,
}

/// Consigne interpolée pour un jour de présence donné.
///
/// Avant le premier point et après le dernier, la valeur extrême est tenue :
/// on n'extrapole jamais une courbe de température. Une bande gardée plus
/// longtemps que prévu doit rester sur la dernière consigne connue, pas
/// dériver vers une valeur que personne n'a écrite.
pub fn consigne_courbe(points: &[PointCourbe], jour: i64) -> Option<Consigne> {
    if points.is_empty() {
        return None;
    }
    let mut tries: Vec<PointCourbe> = points.to_vec();
    tries.sort_by_key(|point| point.jour);

    let premier = tries[0];
    if jour <= premier.jour {
        return Some(Consigne {
            t_cible: premier.t_cible,
            ventil_min: premier.ventil_min,
            ventil_max: premier.ventil_max,
        });
    }
    let dernier = tries[tries.len() - 1];
    if jour >= dernier.jour {
        return Some(Consigne {
            t_cible: dernier.t_cible,
            ventil_min: dernier.ventil_min,
            ventil_max: dernier.ventil_max,
        });
    }
    for paire in tries.windows(2) {
        let (bas, haut) = (paire[0], paire[1]);
        if jour >= bas.jour && jour <= haut.jour {
            let ecart_jours = (haut.jour - bas.jour) as f64;
            if ecart_jours <= 0.0 {
                return Some(Consigne {
                    t_cible: bas.t_cible,
                    ventil_min: bas.ventil_min,
                    ventil_max: bas.ventil_max,
                });
            }
            let part = (jour - bas.jour) as f64 / ecart_jours;
            return Some(Consigne {
                t_cible: bas.t_cible + (haut.t_cible - bas.t_cible) * part,
                // Les bornes ne s'interpolent pas : elles décrivent un palier
                // de conduite, pas une rampe. On garde celles du point courant.
                ventil_min: bas.ventil_min.or(haut.ventil_min),
                ventil_max: bas.ventil_max.or(haut.ventil_max),
            });
        }
    }
    None
}

/// Vrai si la mesure est exploitable pour réguler.
fn mesure_valide(lecture: Lecture, mini: f64, maxi: f64, fraicheur_max_s: i64) -> bool {
    match lecture.valeur {
        None => false,
        Some(valeur) => {
            valeur.is_finite()
                && valeur >= mini
                && valeur <= maxi
                && lecture.age_s >= 0
                && lecture.age_s <= fraicheur_max_s
        }
    }
}

fn borner(valeur: f64, mini: f64, maxi: f64) -> f64 {
    if maxi < mini {
        return mini;
    }
    valeur.max(mini).min(maxi)
}

fn arrondi_dixieme(valeur: f64) -> f64 {
    (valeur * 10.0).round() / 10.0
}

/// Calcule le pourcentage de ventilation à appliquer.
///
/// `gradient` est la pente de température observée en °C/h : positive quand la
/// salle se réchauffe. Multipliée par `anticipation`, elle avance la réaction
/// pour éviter de ne réagir qu'une fois la consigne dépassée.
///
/// Le repli de sécurité n'est jamais une valeur calculée : si la mesure est
/// absente, figée ou aberrante, la salle passe à `ventil_securite`. Une sonde
/// qui renvoie 0 °C ne doit pas pouvoir faire couper la ventilation d'une
/// maternité pleine.
pub fn calculer(
    reglages: &Reglages,
    consigne: f64,
    temperature: Lecture,
    humidite: Lecture,
    gradient: f64,
) -> Decision {
    if !mesure_valide(
        temperature,
        reglages.t_plausible_min,
        reglages.t_plausible_max,
        reglages.fraicheur_max_s,
    ) {
        let motif = match temperature.valeur {
            None => "aucune mesure de température".to_string(),
            Some(valeur)
                if valeur < reglages.t_plausible_min || valeur > reglages.t_plausible_max =>
            {
                format!("température hors plage plausible ({valeur:.1} °C)")
            }
            Some(_) => format!(
                "mesure de température figée depuis {} min",
                temperature.age_s / 60
            ),
        };
        return Decision {
            ventil: borner(reglages.ventil_securite, 0.0, 100.0),
            mode: Mode::Securite,
            sonde_ok: false,
            message: Some(format!("Repli de sécurité : {motif}")),
        };
    }

    let mesure = temperature.valeur.unwrap_or(consigne);
    let plage = if reglages.plage > 0.0 {
        reglages.plage
    } else {
        1.0
    };

    // Écart réel, avancé de la part d'anticipation retenue. Un bâtiment a
    // plusieurs heures d'inertie : attendre le dépassement, c'est toujours
    // rattraper avec un cran de retard.
    let ecart = (mesure - consigne) + reglages.anticipation * gradient;
    let ratio = borner(ecart / plage, 0.0, 1.0);

    let ventil_min = borner(reglages.ventil_min, 0.0, 100.0);
    let ventil_max = borner(reglages.ventil_max, ventil_min, 100.0);
    let mut ventil = ventil_min + ratio * (ventil_max - ventil_min);

    // Correction d'humidité : elle ne peut que *renforcer* la ventilation, et
    // seulement au-dessus de la cible. Un air sec ne justifie pas de fermer en
    // dessous du minimum d'hygiène, qui existe pour évacuer l'ammoniac.
    let mut note = None;
    if let (Some(cible), true) = (reglages.hygro_cible, reglages.hygro_influence > 0.0) {
        if mesure_valide(
            humidite,
            reglages.h_plausible_min,
            reglages.h_plausible_max,
            reglages.fraicheur_max_s,
        ) {
            let humide = humidite.valeur.unwrap_or(cible);
            if humide > cible {
                let bonus = borner((humide - cible) * reglages.hygro_influence, 0.0, 20.0);
                if bonus > 0.0 {
                    ventil += bonus;
                    note = Some(format!("+{bonus:.0} % hygrométrie"));
                }
            }
        } else if humidite.valeur.is_some() {
            note = Some("hygrométrie ignorée (mesure douteuse)".to_string());
        }
    }

    Decision {
        ventil: arrondi_dixieme(borner(ventil, ventil_min, ventil_max)),
        mode: Mode::Auto,
        sonde_ok: true,
        message: note,
    }
}

/// Salle vide : pas d'animaux à protéger, la ventilation est tenue au
/// pourcentage « salle vide » pour économiser l'énergie, quelle que soit la
/// température.
pub fn salle_vide(ventil_vide: f64) -> Decision {
    Decision {
        ventil: arrondi_dixieme(borner(ventil_vide, 0.0, 100.0)),
        mode: Mode::Vide,
        sonde_ok: true,
        message: Some("salle vide : ventilation réduite".to_string()),
    }
}

/// Faut-il réécrire la sortie sur l'automate ?
///
/// Écrire à chaque cycle sur treize salles, c'est saturer l'IPX pour rien et
/// user la liaison. On n'écrit que si la consigne a bougé d'au moins
/// `deadband`, ou si la dernière écriture date de plus de `relance_s` — cette
/// relance périodique sert de rafraîchissement après un redémarrage de
/// l'automate, qui aurait pu perdre sa valeur.
pub fn doit_ecrire(
    derniere: Option<f64>,
    voulue: f64,
    deadband: f64,
    age_ecriture_s: i64,
    relance_s: i64,
) -> bool {
    match derniere {
        None => true,
        Some(precedente) => {
            (voulue - precedente).abs() >= deadband.max(0.0) || age_ecriture_s >= relance_s.max(1)
        }
    }
}

/// Faut-il enregistrer cette mesure ?
///
/// Même principe appliqué au stockage : une température de maternité ne change
/// pas toutes les minutes. On écrit sur variation, avec une écriture forcée de
/// temps en temps pour que les courbes ne présentent pas de trou.
pub fn doit_enregistrer(
    derniere: Option<f64>,
    valeur: f64,
    deadband: f64,
    age_s: i64,
    intervalle_max_s: i64,
) -> bool {
    match derniere {
        None => true,
        Some(precedente) => {
            (valeur - precedente).abs() >= deadband.max(0.0) || age_s >= intervalle_max_s.max(1)
        }
    }
}

/// Conversion vers le format de stockage : dixièmes d'unité dans un entier.
pub fn en_dixiemes(valeur: f64) -> i64 {
    (valeur * 10.0).round() as i64
}

/// Conversion inverse.
pub fn depuis_dixiemes(valeur: i64) -> f64 {
    valeur as f64 / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points() -> Vec<PointCourbe> {
        vec![
            PointCourbe {
                jour: 0,
                t_cible: 28.0,
                ventil_min: Some(25.0),
                ventil_max: Some(100.0),
            },
            PointCourbe {
                jour: 10,
                t_cible: 26.0,
                ventil_min: Some(25.0),
                ventil_max: Some(100.0),
            },
            PointCourbe {
                jour: 20,
                t_cible: 24.0,
                ventil_min: Some(25.0),
                ventil_max: Some(100.0),
            },
        ]
    }

    #[test]
    fn la_courbe_interpole_entre_deux_points() {
        let consigne = consigne_courbe(&points(), 5).unwrap();
        assert!((consigne.t_cible - 27.0).abs() < 1e-9);
    }

    #[test]
    fn la_courbe_tient_les_extremes_sans_extrapoler() {
        assert!((consigne_courbe(&points(), -3).unwrap().t_cible - 28.0).abs() < 1e-9);
        assert!((consigne_courbe(&points(), 400).unwrap().t_cible - 24.0).abs() < 1e-9);
    }

    #[test]
    fn une_salle_vide_tient_la_ventilation_reduite() {
        let d = salle_vide(8.0);
        assert_eq!((d.ventil, d.mode, d.mode.code()), (8.0, Mode::Vide, "vide"));
        assert_eq!(salle_vide(150.0).ventil, 100.0);
    }

    #[test]
    fn la_courbe_vide_ne_donne_pas_de_consigne() {
        assert!(consigne_courbe(&[], 4).is_none());
    }

    #[test]
    fn a_la_consigne_on_reste_au_minimum() {
        let reglages = Reglages::default();
        let decision = calculer(
            &reglages,
            24.0,
            Lecture::neuve(24.0),
            Lecture::absente(),
            0.0,
        );
        assert_eq!(decision.mode, Mode::Auto);
        assert!((decision.ventil - reglages.ventil_min).abs() < 1e-9);
    }

    #[test]
    fn au_dela_de_la_plage_on_plafonne_au_maximum() {
        let reglages = Reglages::default();
        let decision = calculer(
            &reglages,
            24.0,
            Lecture::neuve(40.0),
            Lecture::absente(),
            0.0,
        );
        assert!((decision.ventil - reglages.ventil_max).abs() < 1e-9);
    }

    #[test]
    fn au_milieu_de_la_plage_on_est_a_mi_course() {
        let reglages = Reglages::default();
        let decision = calculer(
            &reglages,
            24.0,
            Lecture::neuve(25.5),
            Lecture::absente(),
            0.0,
        );
        assert!((decision.ventil - 60.0).abs() < 1e-6);
    }

    /// Le cas qui justifie tout le reste : une sonde qui renvoie 0 °C ne doit
    /// jamais faire couper la ventilation d'une salle pleine.
    #[test]
    fn une_sonde_a_zero_declenche_le_repli_et_non_larret() {
        let reglages = Reglages::default();
        let decision = calculer(
            &reglages,
            24.0,
            Lecture::neuve(0.0),
            Lecture::absente(),
            0.0,
        );
        assert_eq!(decision.mode, Mode::Securite);
        assert!(!decision.sonde_ok);
        assert!((decision.ventil - reglages.ventil_securite).abs() < 1e-9);
    }

    #[test]
    fn une_mesure_figee_declenche_le_repli() {
        let reglages = Reglages::default();
        let temperature = Lecture {
            valeur: Some(24.0),
            age_s: 5_000,
        };
        let decision = calculer(&reglages, 24.0, temperature, Lecture::absente(), 0.0);
        assert_eq!(decision.mode, Mode::Securite);
    }

    #[test]
    fn une_sonde_absente_declenche_le_repli() {
        let reglages = Reglages::default();
        let decision = calculer(&reglages, 24.0, Lecture::absente(), Lecture::absente(), 0.0);
        assert_eq!(decision.mode, Mode::Securite);
    }

    #[test]
    fn lhygrometrie_ne_fait_que_renforcer_la_ventilation() {
        let reglages = Reglages {
            hygro_cible: Some(70.0),
            hygro_influence: 1.0,
            ..Reglages::default()
        };
        let sec = calculer(
            &reglages,
            24.0,
            Lecture::neuve(24.0),
            Lecture::neuve(40.0),
            0.0,
        );
        let humide = calculer(
            &reglages,
            24.0,
            Lecture::neuve(24.0),
            Lecture::neuve(85.0),
            0.0,
        );
        assert!((sec.ventil - reglages.ventil_min).abs() < 1e-9);
        assert!(humide.ventil > sec.ventil);
    }

    #[test]
    fn lhygrometrie_aberrante_est_ignoree_sans_couper_la_regulation() {
        let reglages = Reglages {
            hygro_cible: Some(70.0),
            hygro_influence: 1.0,
            ..Reglages::default()
        };
        let decision = calculer(
            &reglages,
            24.0,
            Lecture::neuve(25.5),
            Lecture::neuve(99.99),
            0.0,
        );
        assert_eq!(decision.mode, Mode::Auto);
        assert!((decision.ventil - 60.0).abs() < 1e-6);
    }

    #[test]
    fn lanticipation_avance_la_reaction_sans_depasser_les_bornes() {
        let reglages = Reglages {
            anticipation: 1.0,
            ..Reglages::default()
        };
        let stable = calculer(
            &reglages,
            24.0,
            Lecture::neuve(24.0),
            Lecture::absente(),
            0.0,
        );
        let monte = calculer(
            &reglages,
            24.0,
            Lecture::neuve(24.0),
            Lecture::absente(),
            1.5,
        );
        assert!(monte.ventil > stable.ventil);
        assert!(monte.ventil <= reglages.ventil_max);
    }

    #[test]
    fn la_ventilation_reste_toujours_dans_les_bornes() {
        let reglages = Reglages {
            ventil_min: 30.0,
            ventil_max: 80.0,
            hygro_cible: Some(60.0),
            hygro_influence: 5.0,
            ..Reglages::default()
        };
        for temperature in [-50.0, 0.0, 10.0, 24.0, 30.0, 44.0] {
            for humidite in [10.0, 50.0, 95.0] {
                let decision = calculer(
                    &reglages,
                    24.0,
                    Lecture::neuve(temperature),
                    Lecture::neuve(humidite),
                    0.0,
                );
                assert!(decision.ventil >= 0.0 && decision.ventil <= 100.0);
                if decision.mode == Mode::Auto {
                    assert!(decision.ventil >= reglages.ventil_min - 1e-9);
                    assert!(decision.ventil <= reglages.ventil_max + 1e-9);
                }
            }
        }
    }

    #[test]
    fn une_plage_nulle_ne_fait_pas_exploser_le_calcul() {
        let reglages = Reglages {
            plage: 0.0,
            ..Reglages::default()
        };
        let decision = calculer(
            &reglages,
            24.0,
            Lecture::neuve(26.0),
            Lecture::absente(),
            0.0,
        );
        assert!(decision.ventil.is_finite());
        assert!(decision.ventil <= reglages.ventil_max);
    }

    #[test]
    fn on_ecrit_sur_variation_ou_sur_relance() {
        assert!(doit_ecrire(None, 40.0, 2.0, 0, 300));
        assert!(!doit_ecrire(Some(40.0), 41.0, 2.0, 10, 300));
        assert!(doit_ecrire(Some(40.0), 43.0, 2.0, 10, 300));
        assert!(doit_ecrire(Some(40.0), 40.0, 2.0, 400, 300));
    }

    #[test]
    fn on_enregistre_sur_variation_ou_sur_intervalle() {
        assert!(doit_enregistrer(None, 24.0, 0.1, 0, 600));
        assert!(!doit_enregistrer(Some(24.0), 24.05, 0.1, 30, 600));
        assert!(doit_enregistrer(Some(24.0), 24.2, 0.1, 30, 600));
        assert!(doit_enregistrer(Some(24.0), 24.0, 0.1, 900, 600));
    }

    #[test]
    fn les_dixiemes_font_un_aller_retour_exact() {
        for valeur in [-12.3_f64, 0.0, 24.7, 99.9, 100.0] {
            assert!((depuis_dixiemes(en_dixiemes(valeur)) - valeur).abs() < 1e-9);
        }
    }
}
