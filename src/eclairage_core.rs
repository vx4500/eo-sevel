//! Noyau de décision des programmes d'éclairage.
//!
//! Comme `regul_core`, ce fichier ne dépend de rien : ni base, ni réseau, ni
//! async. Il est compilé dans l'application (pour dire sur la page ce que le
//! programme ferait maintenant) et dans le démon `eo-regul` (qui l'applique).
//!
//! Principe retenu : le programme agit sur des **fronts**, pas en continu.
//! Il allume quand sa demande passe de « éteint » à « allumé », il éteint
//! dans l'autre sens, et entre les deux il ne réécrit rien. Conséquence
//! voulue : quelqu'un qui allume ou éteint à la main dans l'élevage n'est pas
//! contredit trente secondes plus tard — son geste tient jusqu'au prochain
//! changement prévu (fin de plage, jour qui se lève, minuterie).

// Compilé dans deux binaires qui n'en utilisent pas les mêmes parties.
#![allow(dead_code)]

/// Une plage horaire : jours concernés (lundi en premier) et heures en
/// minutes depuis minuit. Une fin plus tôt que le début passe minuit
/// (22:00 → 06:00) ; un début égal à la fin couvre la journée entière.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plage {
    pub jours: [bool; 7],
    pub debut: u32,
    pub fin: u32,
}

impl Plage {
    /// `jour` : 0 = lundi … 6 = dimanche ; `minute` : 0 … 1439.
    pub fn couvre(&self, jour: usize, minute: u32) -> bool {
        let jour = jour % 7;
        let veille = (jour + 6) % 7;
        if self.debut == self.fin {
            return self.jours[jour];
        }
        if self.debut < self.fin {
            self.jours[jour] && minute >= self.debut && minute < self.fin
        } else {
            // Plage de nuit : commencée ce soir, ou commencée hier soir.
            (self.jours[jour] && minute >= self.debut) || (self.jours[veille] && minute < self.fin)
        }
    }
}

/// Jours stockés en texte : sept caractères '0'/'1', lundi en premier.
pub fn jours_depuis_texte(texte: &str) -> [bool; 7] {
    let mut jours = [false; 7];
    for (index, caractere) in texte.chars().take(7).enumerate() {
        jours[index] = caractere == '1';
    }
    jours
}

pub fn jours_en_texte(jours: &[bool; 7]) -> String {
    jours.iter().map(|jour| if *jour { '1' } else { '0' }).collect()
}

/// « HH:MM » → minutes depuis minuit. « 24:00 » est accepté comme minuit.
pub fn minutes_depuis_heure(texte: &str) -> Option<u32> {
    let texte = texte.trim().replace(['h', 'H'], ":");
    let (heures, minutes) = match texte.split_once(':') {
        Some((heures, minutes)) => (heures.trim(), minutes.trim()),
        None => (texte.as_str(), "0"),
    };
    let heures: u32 = heures.parse().ok()?;
    let minutes: u32 = if minutes.is_empty() { 0 } else { minutes.parse().ok()? };
    if minutes >= 60 || heures > 24 || (heures == 24 && minutes > 0) {
        return None;
    }
    Some((heures * 60 + minutes) % 1440)
}

pub fn heure_depuis_minutes(minutes: u32) -> String {
    format!("{:02}:{:02}", (minutes / 60) % 24, minutes % 60)
}

/// Aucune plage : pas de contrainte horaire.
pub fn dans_horaire(plages: &[Plage], jour: usize, minute: u32) -> bool {
    plages.is_empty() || plages.iter().any(|plage| plage.couvre(jour, minute))
}

/// Que faire quand le capteur de luminosité ne répond pas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiCapteurMuet {
    /// Ignorer la luminosité et suivre les horaires seuls. Le choix par
    /// défaut : des animaux ne restent pas dans le noir parce qu'une sonde a
    /// lâché.
    Horaire,
    /// Considérer qu'il fait jour : rien ne s'allume sur la luminosité.
    Eteindre,
    /// Garder le dernier verdict du capteur.
    Garder,
}

impl SiCapteurMuet {
    pub fn depuis_code(code: &str) -> Self {
        match code {
            "eteindre" => Self::Eteindre,
            "garder" => Self::Garder,
            _ => Self::Horaire,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Luminosite {
    /// Il fait sombre en dessous de ce seuil : on allume.
    pub allumer_sous: f64,
    /// Il fait jour au-dessus de ce seuil : on éteint. Plus haut que le
    /// premier, pour qu'une lampe allumée qui éclaire le capteur ne
    /// s'éteigne pas aussitôt (hystérésis).
    pub eteindre_au_dessus: f64,
    /// La condition doit tenir tout ce temps avant de basculer : un nuage
    /// qui passe ne fait pas clignoter une salle.
    pub temporisation_s: i64,
    pub si_muet: SiCapteurMuet,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Programme {
    pub plages: Vec<Plage>,
    pub luminosite: Option<Luminosite>,
    /// Extinction automatique après ce temps d'allumage.
    pub minuterie_s: Option<i64>,
    /// La minuterie s'applique aussi quand la lumière a été allumée à la
    /// main (bouton dans le bâtiment ou dans l'application).
    pub minuterie_manuel: bool,
}

impl Programme {
    /// Sans plage ni luminosité, le programme n'allume jamais de lui-même :
    /// c'est une simple minuterie d'extinction.
    pub fn allume_de_lui_meme(&self) -> bool {
        !self.plages.is_empty() || self.luminosite.is_some()
    }
}

/// Mémoire d'une sortie d'un cycle à l'autre. Le démon la conserve en base
/// (`eclairage_etat`) : un redémarrage ne fait ni rallumer ni oublier une
/// minuterie en cours.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Memoire {
    /// Dernière demande calculée (allumé / éteint). `None` : jamais calculée,
    /// ou remise à zéro après une modification du programme — la prochaine
    /// demande est alors appliquée d'office.
    pub demande: Option<bool>,
    /// Verdict retenu pour la luminosité (vrai : il fait sombre).
    pub sombre: Option<bool>,
    /// Depuis quand la luminosité réclame de basculer.
    pub bascule_depuis: Option<i64>,
    /// Dernier état connu de la sortie.
    pub allume: Option<bool>,
    pub allume_depuis: Option<i64>,
    /// Vrai quand c'est le programme qui a allumé.
    pub par_programme: bool,
    /// Vrai pendant un forçage manuel depuis la page.
    pub force: bool,
}

/// Forçage demandé depuis la page : allumé ou éteint jusqu'à `fin`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Forcage {
    pub allume: bool,
    pub fin: i64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entree {
    pub ts: i64,
    /// 0 = lundi.
    pub jour: usize,
    pub minute: u32,
    pub lux: Option<f64>,
    /// État réellement lu sur l'automate, s'il a pu l'être.
    pub etat_lu: Option<bool>,
    pub forcage: Option<Forcage>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    /// Ce qu'il faut envoyer à la sortie, s'il y a quelque chose à envoyer.
    pub commande: Option<bool>,
    /// Ce que le programme veut en ce moment (affiché sur la page).
    pub demande: Option<bool>,
    /// Phrase courte expliquant l'état, pour l'éleveur.
    pub raison: String,
    pub memoire: Memoire,
}

pub fn decider(programme: &Programme, entree: &Entree, precedente: &Memoire) -> Decision {
    let mut memoire = precedente.clone();
    let ts = entree.ts;

    // 1. Ce qu'on sait de la sortie. Une lumière trouvée allumée alors que le
    //    programme la croyait éteinte a été allumée à la main : la minuterie
    //    part de maintenant.
    if let Some(lu) = entree.etat_lu {
        if lu && memoire.allume != Some(true) {
            memoire.allume_depuis = Some(ts);
            memoire.par_programme = false;
        }
        if !lu {
            memoire.allume_depuis = None;
            memoire.par_programme = false;
        }
        memoire.allume = Some(lu);
    }
    let etat = memoire.allume;

    // 2. Luminosité : évaluée même hors plage, pour que l'hystérésis soit
    //    déjà juste au moment où une plage commence.
    let mut raison_lux: Option<String> = None;
    let lux_ok = match &programme.luminosite {
        None => true,
        Some(reglage) => match entree.lux {
            None => {
                memoire.bascule_depuis = None;
                raison_lux = Some("capteur de luminosité muet".into());
                match reglage.si_muet {
                    SiCapteurMuet::Horaire => true,
                    SiCapteurMuet::Eteindre => false,
                    SiCapteurMuet::Garder => memoire.sombre.unwrap_or(true),
                }
            }
            Some(lux) => {
                let mut sombre = memoire.sombre.unwrap_or(lux < reglage.allumer_sous);
                let voudrait = if sombre {
                    lux <= reglage.eteindre_au_dessus
                } else {
                    lux < reglage.allumer_sous
                };
                if voudrait == sombre {
                    memoire.bascule_depuis = None;
                } else {
                    let depuis = *memoire.bascule_depuis.get_or_insert(ts);
                    if ts - depuis >= reglage.temporisation_s {
                        sombre = voudrait;
                        memoire.bascule_depuis = None;
                    }
                }
                memoire.sombre = Some(sombre);
                raison_lux = Some(format!(
                    "{} lux, {}",
                    lux.round(),
                    if sombre { "sombre" } else { "assez clair" }
                ));
                sombre
            }
        },
    };

    let horaire = dans_horaire(&programme.plages, entree.jour, entree.minute);

    // 3. Forçage manuel depuis la page : prioritaire sur tout le reste, y
    //    compris la minuterie. À sa fin, la demande normale est réappliquée.
    let forcage = entree.forcage.filter(|forcage| forcage.fin > ts);
    let sortie_de_forcage = memoire.force && forcage.is_none();
    memoire.force = forcage.is_some();
    if sortie_de_forcage {
        memoire.demande = None;
    }

    let demande: Option<bool> = if let Some(forcage) = forcage {
        Some(forcage.allume)
    } else if programme.allume_de_lui_meme() {
        Some(horaire && lux_ok)
    } else {
        None
    };

    let mut commande = None;
    let mut raison = match (forcage, demande) {
        (Some(forcage), _) => format!(
            "forcé {} par l'application",
            if forcage.allume { "allumé" } else { "éteint" }
        ),
        (None, None) => "allumage à la main, extinction par minuterie".to_string(),
        (None, Some(true)) => "dans la plage horaire".to_string(),
        (None, Some(false)) if !horaire => "hors des plages horaires".to_string(),
        (None, Some(false)) => "il fait assez clair".to_string(),
    };
    if let Some(detail) = raison_lux.filter(|_| forcage.is_none()) {
        raison = format!("{raison} — {detail}");
    }

    if let Some(voulu) = demande {
        if memoire.demande != Some(voulu) {
            memoire.demande = Some(voulu);
            // Front : on n'envoie que si la sortie n'y est pas déjà.
            if etat != Some(voulu) {
                commande = Some(voulu);
            }
        }
    } else {
        memoire.demande = None;
    }

    // 4. Minuterie : éteint ce qui est allumé depuis trop longtemps. Pas de
    //    rallumage ensuite, puisque la demande n'a pas changé de front.
    if commande.is_none() && forcage.is_none() {
        if let (Some(duree), Some(true), Some(depuis)) =
            (programme.minuterie_s, etat, memoire.allume_depuis)
        {
            let concernee = memoire.par_programme || programme.minuterie_manuel;
            if concernee && ts - depuis >= duree {
                commande = Some(false);
                raison = format!("minuterie écoulée ({} min)", duree / 60);
            }
        }
    }

    // 5. On retient l'effet attendu de la commande. Si l'envoi échoue, le
    //    démon garde l'ancienne mémoire et la commande repart au cycle suivant.
    if let Some(voulu) = commande {
        memoire.allume = Some(voulu);
        if voulu {
            memoire.allume_depuis = Some(ts);
            memoire.par_programme = true;
        } else {
            memoire.allume_depuis = None;
            memoire.par_programme = false;
        }
    }

    Decision {
        commande,
        demande,
        raison,
        memoire,
    }
}

/// Prochain changement prévu par les plages, en minutes à partir de
/// maintenant (au plus une semaine), avec l'état qui suivra. Sert à écrire
/// « s'éteindra à 20:00 » sur la page.
pub fn prochain_changement(plages: &[Plage], jour: usize, minute: u32) -> Option<(u32, bool)> {
    if plages.is_empty() {
        return None;
    }
    let actuel = dans_horaire(plages, jour, minute);
    for pas in 1..=(7 * 1440) {
        let total = minute + pas;
        let j = (jour + (total / 1440) as usize) % 7;
        let m = total % 1440;
        if dans_horaire(plages, j, m) != actuel {
            return Some((pas, !actuel));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOUS: [bool; 7] = [true; 7];

    fn plage(debut: &str, fin: &str) -> Plage {
        Plage {
            jours: TOUS,
            debut: minutes_depuis_heure(debut).unwrap(),
            fin: minutes_depuis_heure(fin).unwrap(),
        }
    }

    fn horaire(debut: &str, fin: &str) -> Programme {
        Programme {
            plages: vec![plage(debut, fin)],
            luminosite: None,
            minuterie_s: None,
            minuterie_manuel: true,
        }
    }

    fn entree(ts: i64, heure: &str) -> Entree {
        Entree {
            ts,
            jour: 0,
            minute: minutes_depuis_heure(heure).unwrap(),
            lux: None,
            etat_lu: None,
            forcage: None,
        }
    }

    #[test]
    fn les_heures_se_lisent_sous_plusieurs_formes() {
        assert_eq!(minutes_depuis_heure("06:30"), Some(390));
        assert_eq!(minutes_depuis_heure("6h30"), Some(390));
        assert_eq!(minutes_depuis_heure("20"), Some(1200));
        assert_eq!(minutes_depuis_heure("24:00"), Some(0));
        assert_eq!(minutes_depuis_heure("25:00"), None);
        assert_eq!(minutes_depuis_heure("12:75"), None);
        assert_eq!(heure_depuis_minutes(390), "06:30");
    }

    #[test]
    fn une_plage_de_nuit_passe_minuit_et_respecte_la_veille() {
        let mut nuit = plage("22:00", "06:00");
        assert!(nuit.couvre(0, 23 * 60));
        assert!(nuit.couvre(1, 5 * 60));
        assert!(!nuit.couvre(1, 7 * 60));
        // Seulement le vendredi soir : le samedi matin est couvert, pas le
        // vendredi matin.
        nuit.jours = [false, false, false, false, true, false, false];
        assert!(nuit.couvre(5, 3 * 60));
        assert!(!nuit.couvre(4, 3 * 60));
        assert!(nuit.couvre(4, 23 * 60));
    }

    #[test]
    fn debut_egal_fin_couvre_la_journee() {
        let jour = plage("00:00", "00:00");
        assert!(jour.couvre(3, 0));
        assert!(jour.couvre(3, 1439));
    }

    #[test]
    fn allume_au_debut_de_la_plage_et_eteint_a_la_fin() {
        let programme = horaire("06:30", "20:00");
        let d = decider(&programme, &entree(0, "06:00"), &Memoire::default());
        assert_eq!(d.commande, Some(false));
        let d = decider(&programme, &entree(60, "06:15"), &d.memoire);
        assert_eq!(d.commande, None);
        let d = decider(&programme, &entree(120, "06:30"), &d.memoire);
        assert_eq!(d.commande, Some(true));
        let d = decider(&programme, &entree(180, "12:00"), &d.memoire);
        assert_eq!(d.commande, None);
        let d = decider(&programme, &entree(240, "20:00"), &d.memoire);
        assert_eq!(d.commande, Some(false));
    }

    #[test]
    fn un_geste_a_la_main_n_est_pas_contredit_avant_le_prochain_changement() {
        let programme = horaire("06:30", "20:00");
        let mut e = entree(0, "07:00");
        e.etat_lu = Some(true);
        let d = decider(&programme, &e, &Memoire::default());
        assert_eq!(d.commande, None, "déjà allumée : rien à envoyer");
        // Quelqu'un éteint à 8 h : le programme ne rallume pas.
        let mut e = entree(3600, "08:00");
        e.etat_lu = Some(false);
        let d = decider(&programme, &e, &d.memoire);
        assert_eq!(d.commande, None);
        // Mais à 20 h, il éteint bien s'il trouve la lumière rallumée.
        let mut e = entree(7200, "20:00");
        e.etat_lu = Some(true);
        let d = decider(&programme, &e, &d.memoire);
        assert_eq!(d.commande, Some(false));
    }

    #[test]
    fn la_luminosite_a_une_hysteresis_et_une_temporisation() {
        let programme = Programme {
            plages: vec![],
            luminosite: Some(Luminosite {
                allumer_sous: 100.0,
                eteindre_au_dessus: 300.0,
                temporisation_s: 300,
                si_muet: SiCapteurMuet::Horaire,
            }),
            minuterie_s: None,
            minuterie_manuel: true,
        };
        let mut e = entree(0, "12:00");
        e.lux = Some(500.0);
        let d = decider(&programme, &e, &Memoire::default());
        assert_eq!(d.commande, Some(false));
        // Un nuage : 80 lux pendant deux minutes seulement → rien.
        e.ts = 60;
        e.lux = Some(80.0);
        let d = decider(&programme, &e, &d.memoire);
        assert_eq!(d.commande, None);
        e.ts = 180;
        e.lux = Some(500.0);
        let d = decider(&programme, &e, &d.memoire);
        assert_eq!(d.commande, None);
        // Nuit qui tombe : sombre plus de cinq minutes → allumage.
        e.ts = 1000;
        e.lux = Some(50.0);
        let d = decider(&programme, &e, &d.memoire);
        assert_eq!(d.commande, None);
        e.ts = 1300;
        let d = decider(&programme, &e, &d.memoire);
        assert_eq!(d.commande, Some(true));
        // La lampe éclaire le capteur à 200 lux : sous le seuil haut, reste
        // allumée.
        e.ts = 2000;
        e.lux = Some(200.0);
        let d = decider(&programme, &e, &d.memoire);
        assert_eq!(d.commande, None);
        assert_eq!(d.memoire.sombre, Some(true));
    }

    #[test]
    fn capteur_muet_suit_l_horaire_par_defaut() {
        let mut programme = horaire("06:00", "20:00");
        programme.luminosite = Some(Luminosite {
            allumer_sous: 100.0,
            eteindre_au_dessus: 300.0,
            temporisation_s: 0,
            si_muet: SiCapteurMuet::Horaire,
        });
        let d = decider(&programme, &entree(0, "07:00"), &Memoire::default());
        assert_eq!(d.commande, Some(true));
        programme.luminosite.as_mut().unwrap().si_muet = SiCapteurMuet::Eteindre;
        let d = decider(&programme, &entree(0, "07:00"), &Memoire::default());
        assert_eq!(d.commande, Some(false));
    }

    #[test]
    fn la_minuterie_eteint_sans_rallumer() {
        let mut programme = horaire("06:00", "20:00");
        programme.minuterie_s = Some(30 * 60);
        let d = decider(&programme, &entree(0, "06:00"), &Memoire::default());
        assert_eq!(d.commande, Some(true));
        let d = decider(&programme, &entree(29 * 60, "06:29"), &d.memoire);
        assert_eq!(d.commande, None);
        let d = decider(&programme, &entree(30 * 60, "06:30"), &d.memoire);
        assert_eq!(d.commande, Some(false));
        let d = decider(&programme, &entree(40 * 60, "06:40"), &d.memoire);
        assert_eq!(d.commande, None, "pas de rallumage dans la même plage");
    }

    #[test]
    fn minuterie_seule_eteint_un_allumage_a_la_main() {
        let mut programme = Programme {
            plages: vec![],
            luminosite: None,
            minuterie_s: Some(15 * 60),
            minuterie_manuel: true,
        };
        let mut e = entree(0, "10:00");
        e.etat_lu = Some(false);
        let d = decider(&programme, &e, &Memoire::default());
        assert_eq!(d.commande, None, "une minuterie seule n'allume jamais");
        e.ts = 100;
        e.etat_lu = Some(true);
        let d = decider(&programme, &e, &d.memoire);
        assert_eq!(d.commande, None);
        e.ts = 100 + 15 * 60;
        let d = decider(&programme, &e, &d.memoire);
        assert_eq!(d.commande, Some(false));
        // Sans l'option « aussi à la main », on ne touche pas.
        programme.minuterie_manuel = false;
        e.ts = 100;
        let m = Memoire {
            allume: Some(false),
            ..Memoire::default()
        };
        let d = decider(&programme, &e, &m);
        e.ts = 100 + 15 * 60;
        let d = decider(&programme, &e, &d.memoire);
        assert_eq!(d.commande, None);
    }

    #[test]
    fn un_forcage_prime_puis_le_programme_reprend() {
        let programme = horaire("06:00", "20:00");
        let d = decider(&programme, &entree(0, "22:00"), &Memoire::default());
        assert_eq!(d.commande, Some(false));
        let mut e = entree(60, "22:01");
        e.forcage = Some(Forcage {
            allume: true,
            fin: 3600,
        });
        let d = decider(&programme, &e, &d.memoire);
        assert_eq!(d.commande, Some(true));
        e.ts = 3601;
        let d = decider(&programme, &e, &d.memoire);
        assert_eq!(d.commande, Some(false), "fin du forçage : retour au programme");
    }

    #[test]
    fn prochain_changement_trouve_la_fin_de_plage() {
        let plages = vec![plage("06:30", "20:00")];
        assert_eq!(prochain_changement(&plages, 0, 12 * 60), Some((8 * 60, false)));
        assert_eq!(prochain_changement(&plages, 0, 21 * 60), Some((9 * 60 + 30, true)));
        assert_eq!(prochain_changement(&[], 0, 0), None);
    }
}
