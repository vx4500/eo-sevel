//! Lecture des codes GS1 des emballages (Data Matrix, code-barres).
//!
//! Une boîte de médicament vétérinaire porte le plus souvent un Data Matrix
//! GS1 qui contient, en une seule chaîne, le code produit, le numéro de lot et
//! la date de péremption. C'est exactement ce qu'il faut reporter au registre
//! d'élevage, et c'est exactement ce qu'on recopie à la main aujourd'hui.
//!
//! Ce module ne fait que décoder la chaîne. Il ne sait pas ce qu'est le
//! produit : un GTIN est un numéro, pas un nom, et il n'existe pas de base
//! publique fiable côté vétérinaire. La correspondance code → produit
//! s'apprend au premier scan et se retient ensuite.
//!
//! Aucune dépendance : le décodage de l'image est fait par le téléphone, on ne
//! reçoit ici que le texte déjà décodé.

// Ce module est compilé dans l'application ; certaines fonctions utilitaires
// ne servent qu'aux tests ou aux futurs appels de l'application mobile.
#![allow(dead_code)]

/// Caractère de séparation des champs de longueur variable (FNC1, encodé en
/// ASCII 29). Les lecteurs le transmettent tel quel ; certains le remplacent
/// par un autre caractère, d'où la tolérance plus bas.
const SEPARATEUR: char = '\u{001D}';

/// Identifiants d'application de longueur fixe, avec leur longueur de données.
/// Tout ce qui n'est pas listé ici est traité comme variable.
const LONGUEURS_FIXES: &[(&str, usize)] = &[
    ("00", 18), // SSCC
    ("01", 14), // GTIN
    ("02", 14), // GTIN des contenus
    ("11", 6),  // date de fabrication
    ("12", 6),  // date d'échéance
    ("13", 6),  // date de conditionnement
    ("15", 6),  // date de durabilité minimale
    ("16", 6),  // date de vente conseillée
    ("17", 6),  // date de péremption
    ("20", 2),  // variante produit
];

/// Ce qu'on a su tirer d'un code scanné.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CodeLu {
    /// Code produit (AI 01). C'est la clé d'apprentissage.
    pub gtin: Option<String>,
    /// Numéro de lot (AI 10).
    pub lot: Option<String>,
    /// Péremption (AI 17), normalisée en AAAA-MM-JJ.
    pub peremption: Option<String>,
    /// Date de fabrication (AI 11), normalisée de même.
    pub fabrication: Option<String>,
    /// Numéro de série (AI 21).
    pub serie: Option<String>,
    /// Quantité (AI 30), quand elle est présente.
    pub quantite: Option<f64>,
    /// Chaîne d'origine, conservée telle quelle : c'est elle qu'on stocke pour
    /// reconnaître un code déjà vu, même si on n'a pas su tout interpréter.
    pub brut: String,
    /// Vrai si la chaîne a été comprise comme du GS1.
    pub gs1: bool,
    /// Identifiants rencontrés mais non interprétés, pour diagnostic.
    pub autres: Vec<(String, String)>,
}

impl CodeLu {
    /// Clé d'identification du produit : le GTIN quand il existe, sinon la
    /// chaîne brute. Un code-barres EAN simple n'a pas de structure GS1 mais
    /// identifie quand même le produit.
    pub fn cle(&self) -> String {
        self.gtin.clone().unwrap_or_else(|| self.brut.clone())
    }

    /// Vrai si le code apporte quelque chose de plus que l'identité du produit.
    pub fn a_tracabilite(&self) -> bool {
        self.lot.is_some() || self.peremption.is_some()
    }
}

/// Normalise une date GS1 (AAMMJJ) en AAAA-MM-JJ.
///
/// Le siècle se déduit selon la règle GS1 : un écart de plus de cinquante ans
/// bascule le siècle. En pratique, une péremption est toujours proche, mais une
/// date de fabrication peut être un peu ancienne.
///
/// Le jour `00` est autorisé par la norme et signifie « fin de mois » : on le
/// ramène au dernier jour, sinon la date serait invalide.
pub fn date_gs1(brut: &str, annee_courante: i32) -> Option<String> {
    let chiffres: String = brut.chars().filter(|c| c.is_ascii_digit()).collect();
    if chiffres.len() != 6 {
        return None;
    }
    let aa: i32 = chiffres[0..2].parse().ok()?;
    let mm: u32 = chiffres[2..4].parse().ok()?;
    let jj: u32 = chiffres[4..6].parse().ok()?;
    if !(1..=12).contains(&mm) || jj > 31 {
        return None;
    }
    let siecle = annee_courante / 100;
    let mut annee = siecle * 100 + aa;
    if annee - annee_courante > 50 {
        annee -= 100;
    } else if annee_courante - annee > 50 {
        annee += 100;
    }
    let jour = if jj == 0 { dernier_jour(annee, mm) } else { jj };
    Some(format!("{annee:04}-{mm:02}-{jour:02}"))
}

fn est_bissextile(annee: i32) -> bool {
    (annee % 4 == 0 && annee % 100 != 0) || annee % 400 == 0
}

fn dernier_jour(annee: i32, mois: u32) -> u32 {
    match mois {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if est_bissextile(annee) => 29,
        _ => 28,
    }
}

fn longueur_fixe(ai: &str) -> Option<usize> {
    LONGUEURS_FIXES
        .iter()
        .find(|(code, _)| *code == ai)
        .map(|(_, longueur)| *longueur)
}

/// Retire le préfixe de symbologie que certains lecteurs ajoutent (`]d2` pour
/// un Data Matrix GS1, `]C1` pour un GS1-128, `]Q3` pour un QR GS1).
fn sans_prefixe(texte: &str) -> &str {
    for prefixe in ["]d2", "]C1", "]Q3", "]e0", "]d1"] {
        if let Some(reste) = texte.strip_prefix(prefixe) {
            return reste;
        }
    }
    texte
}

/// Décode une chaîne scannée.
///
/// Trois formes sont acceptées, parce que les lecteurs ne se comportent pas
/// tous pareil : avec séparateurs FNC1, avec les identifiants entre
/// parenthèses (forme lisible, fréquente en saisie manuelle), ou collés sans
/// séparateur quand seuls des champs de longueur fixe se suivent.
///
/// Une chaîne incompréhensible n'est jamais une erreur : on renvoie un `CodeLu`
/// qui ne porte que le brut. C'est suffisant pour reconnaître un code déjà
/// associé à un produit, et c'est mieux que de refuser un scan valide parce
/// qu'on n'a pas su l'analyser.
pub fn decoder(texte: &str, annee_courante: i32) -> CodeLu {
    let brut = texte.trim().to_string();
    let mut lu = CodeLu {
        brut: brut.clone(),
        ..CodeLu::default()
    };
    if brut.is_empty() {
        return lu;
    }

    let travail = sans_prefixe(&brut);
    // Forme entre parenthèses : on la ramène à la forme à séparateurs, ce qui
    // évite d'écrire deux analyseurs.
    let normalise = if travail.contains('(') {
        let mut sortie = String::with_capacity(travail.len());
        let mut dans_ai = false;
        for caractere in travail.chars() {
            match caractere {
                '(' => {
                    if !sortie.is_empty() && !sortie.ends_with(SEPARATEUR) {
                        sortie.push(SEPARATEUR);
                    }
                    dans_ai = true;
                }
                ')' => dans_ai = false,
                autre => {
                    if dans_ai && !autre.is_ascii_digit() {
                        continue;
                    }
                    sortie.push(autre);
                }
            }
        }
        sortie
    } else {
        travail.to_string()
    };

    let caracteres: Vec<char> = normalise.chars().collect();
    let mut position = 0usize;
    let mut trouve_quelque_chose = false;

    while position + 2 <= caracteres.len() {
        // Un séparateur en tête : on l'avale et on continue.
        if caracteres[position] == SEPARATEUR {
            position += 1;
            continue;
        }
        let ai: String = caracteres[position..position + 2].iter().collect();
        if !ai.chars().all(|c| c.is_ascii_digit()) {
            break;
        }
        position += 2;

        let valeur: String = match longueur_fixe(&ai) {
            Some(longueur) => {
                if position + longueur > caracteres.len() {
                    // Champ tronqué : on s'arrête là plutôt que de lire n'importe quoi.
                    break;
                }
                let valeur: String = caracteres[position..position + longueur].iter().collect();
                position += longueur;
                valeur
            }
            None => {
                // Longueur variable : jusqu'au séparateur, ou jusqu'à la fin.
                let debut = position;
                while position < caracteres.len() && caracteres[position] != SEPARATEUR {
                    position += 1;
                }
                let valeur: String = caracteres[debut..position].iter().collect();
                if position < caracteres.len() {
                    position += 1; // on consomme le séparateur
                }
                valeur
            }
        };

        if valeur.is_empty() {
            continue;
        }
        trouve_quelque_chose = true;
        match ai.as_str() {
            "01" | "02" => lu.gtin = Some(valeur),
            "10" => lu.lot = Some(valeur),
            "21" => lu.serie = Some(valeur),
            "17" => lu.peremption = date_gs1(&valeur, annee_courante),
            "11" => lu.fabrication = date_gs1(&valeur, annee_courante),
            "30" => lu.quantite = valeur.trim().parse().ok(),
            _ => lu.autres.push((ai, valeur)),
        }
    }

    // Un code-barres EAN-13 ou EAN-8 seul : ce ne sont que des chiffres, sans
    // identifiant d'application. Il identifie le produit, sans traçabilité.
    let chiffres_seuls = brut.chars().all(|c| c.is_ascii_digit());
    if chiffres_seuls && matches!(brut.len(), 8 | 12 | 13 | 14) {
        return CodeLu {
            gtin: Some(format!("{:0>14}", brut)),
            brut,
            gs1: false,
            ..CodeLu::default()
        };
    }

    lu.gs1 = trouve_quelque_chose && lu.gtin.is_some();
    if !lu.gs1 {
        // Analyse non concluante : on ne garde que le brut, pour ne pas
        // présenter des champs devinés comme s'ils étaient sûrs.
        return CodeLu {
            brut: lu.brut,
            ..CodeLu::default()
        };
    }
    lu
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANNEE: i32 = 2026;

    #[test]
    fn un_datamatrix_complet_donne_produit_lot_et_peremption() {
        // GTIN + péremption + lot, séparés par FNC1.
        let code = "010340093000000117281231\u{001D}10LOT4521";
        let lu = decoder(code, ANNEE);
        assert!(lu.gs1);
        assert_eq!(lu.gtin.as_deref(), Some("03400930000001"));
        assert_eq!(lu.peremption.as_deref(), Some("2028-12-31"));
        assert_eq!(lu.lot.as_deref(), Some("LOT4521"));
        assert!(lu.a_tracabilite());
    }

    #[test]
    fn la_forme_entre_parentheses_est_acceptee() {
        let lu = decoder("(01)03400930000001(17)281231(10)LOT4521", ANNEE);
        assert_eq!(lu.gtin.as_deref(), Some("03400930000001"));
        assert_eq!(lu.lot.as_deref(), Some("LOT4521"));
        assert_eq!(lu.peremption.as_deref(), Some("2028-12-31"));
    }

    #[test]
    fn le_prefixe_de_symbologie_est_retire() {
        let lu = decoder("]d2010340093000000117281231\u{001D}10LOT1", ANNEE);
        assert_eq!(lu.gtin.as_deref(), Some("03400930000001"));
        assert_eq!(lu.lot.as_deref(), Some("LOT1"));
    }

    #[test]
    fn le_numero_de_serie_est_reconnu() {
        let lu = decoder(
            "010340093000000121SERIE123\u{001D}10LOT9\u{001D}17270630",
            ANNEE,
        );
        assert_eq!(lu.serie.as_deref(), Some("SERIE123"));
        assert_eq!(lu.lot.as_deref(), Some("LOT9"));
        assert_eq!(lu.peremption.as_deref(), Some("2027-06-30"));
    }

    /// Les champs de longueur fixe peuvent se suivre sans séparateur : c'est
    /// permis par la norme et fréquent en pratique.
    #[test]
    fn les_champs_fixes_collés_sont_lus() {
        let lu = decoder("010340093000000117281231", ANNEE);
        assert_eq!(lu.gtin.as_deref(), Some("03400930000001"));
        assert_eq!(lu.peremption.as_deref(), Some("2028-12-31"));
        assert!(lu.lot.is_none());
    }

    #[test]
    fn le_jour_zero_signifie_fin_de_mois() {
        assert_eq!(date_gs1("280200", ANNEE).as_deref(), Some("2028-02-29"));
        assert_eq!(date_gs1("270200", ANNEE).as_deref(), Some("2027-02-28"));
        assert_eq!(date_gs1("281100", ANNEE).as_deref(), Some("2028-11-30"));
    }

    #[test]
    fn le_siecle_est_choisi_par_proximite() {
        assert_eq!(date_gs1("990101", ANNEE).as_deref(), Some("1999-01-01"));
        assert_eq!(date_gs1("300101", ANNEE).as_deref(), Some("2030-01-01"));
    }

    #[test]
    fn une_date_invalide_est_rejetee_sans_planter() {
        assert!(date_gs1("281301", ANNEE).is_none());
        assert!(date_gs1("28123", ANNEE).is_none());
        assert!(date_gs1("", ANNEE).is_none());
        assert!(date_gs1("ABCDEF", ANNEE).is_none());
    }

    #[test]
    fn un_ean13_seul_identifie_le_produit_sans_tracabilite() {
        let lu = decoder("3401579876543", ANNEE);
        assert_eq!(lu.gtin.as_deref(), Some("03401579876543"));
        assert!(!lu.a_tracabilite());
        assert!(!lu.gs1);
    }

    /// Un code qu'on ne sait pas analyser doit rester utilisable : c'est le
    /// brut qui sert de clé, et l'association manuelle prend le relais.
    #[test]
    fn un_code_inconnu_garde_son_brut_et_ne_devine_rien() {
        let lu = decoder("XYZ-BIDON-42", ANNEE);
        assert!(lu.gtin.is_none());
        assert!(lu.lot.is_none());
        assert!(lu.peremption.is_none());
        assert_eq!(lu.brut, "XYZ-BIDON-42");
        assert_eq!(lu.cle(), "XYZ-BIDON-42");
    }

    #[test]
    fn une_chaine_vide_ne_plante_pas() {
        let lu = decoder("   ", ANNEE);
        assert!(lu.brut.is_empty());
        assert!(lu.gtin.is_none());
    }

    /// Un champ fixe tronqué ne doit pas faire lire les caractères suivants
    /// comme s'ils appartenaient au champ.
    #[test]
    fn un_champ_tronque_ne_produit_pas_de_valeur_fausse() {
        let lu = decoder("0103400930", ANNEE);
        assert!(lu.gtin.is_none());
        assert_eq!(lu.brut, "0103400930");
    }

    #[test]
    fn la_quantite_est_lue_quand_elle_est_presente() {
        let lu = decoder("010340093000000130012\u{001D}10LOT3", ANNEE);
        assert_eq!(lu.quantite, Some(12.0));
        assert_eq!(lu.lot.as_deref(), Some("LOT3"));
    }

    #[test]
    fn les_identifiants_non_interpretes_sont_conserves_pour_diagnostic() {
        let lu = decoder("010340093000000137100\u{001D}10LOT7", ANNEE);
        assert_eq!(lu.lot.as_deref(), Some("LOT7"));
        assert!(lu.autres.iter().any(|(ai, _)| ai == "37"));
    }

    /// Cas réel : vaccin HIPRA, relevé sur une boîte de l'élevage.
    /// GTIN 08427711153207, lot 766G-2, péremption 19/02/2028.
    #[test]
    fn cas_reel_vaccin_hipra() {
        let lu = decoder("010842771115320717280219\u{001D}10766G-2", ANNEE);
        assert!(lu.gs1);
        assert_eq!(lu.gtin.as_deref(), Some("08427711153207"));
        assert_eq!(lu.lot.as_deref(), Some("766G-2"));
        assert_eq!(lu.peremption.as_deref(), Some("2028-02-19"));
    }

    /// Cas réel : boîte dont l'étiquette n'imprime que « Exp. 03/2028 ».
    /// Le code encode alors le jour 00, qu'il faut ramener au dernier jour du
    /// mois — sinon la date est refusée et le scan perd tout son intérêt.
    #[test]
    fn cas_reel_peremption_au_mois_seulement() {
        let lu = decoder("010842771115320717280300\u{001D}10235333A", ANNEE);
        assert_eq!(lu.lot.as_deref(), Some("235333A"));
        assert_eq!(lu.peremption.as_deref(), Some("2028-03-31"));
    }

    /// Certaines boîtes portent le numéro de série avant le lot, d'autres
    /// après : l'ordre des identifiants ne doit rien changer au résultat.
    #[test]
    fn lordre_des_identifiants_est_indifferent() {
        let attendu = decoder("010842771115320717280219\u{001D}10766G-2", ANNEE);
        let inverse = decoder("10766G-2\u{001D}17280219\u{001D}0108427711153207", ANNEE);
        assert_eq!(inverse.lot, attendu.lot);
        assert_eq!(inverse.peremption, attendu.peremption);
        assert_eq!(inverse.gtin, attendu.gtin);
    }

    #[test]
    fn la_cle_retombe_sur_le_brut_sans_gtin() {
        let lu = decoder("PAS-UN-GS1", ANNEE);
        assert_eq!(lu.cle(), "PAS-UN-GS1");
    }
}
