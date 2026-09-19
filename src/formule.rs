//! Évaluateur de formule pour convertir la valeur brute d'une entrée
//! analogique (compte ADC) en unité physique — comme une sonde TC100 dont la
//! fiche donne directement `((X * 0.00323) - 0.25) / 0.028` plutôt qu'un
//! coefficient et un décalage déjà réduits.
//!
//! Volontairement une grammaire minuscule et sans variables ni fonctions au
//! delà de `X` : ce n'est pas un langage de script, c'est une formule de
//! fiche technique. Aucune évaluation de code arbitraire n'a lieu — juste
//! + - * / ^ et des parenthèses sur des nombres et une seule variable.

/// Évalue `formule` en substituant `x` à la variable `X` (insensible à la
/// casse). Renvoie `None` si la formule est vide, mal formée, ou si le calcul
/// produit une valeur non finie (ex. division par zéro) — jamais un résultat
/// inventé.
pub fn evaluer(formule: &str, x: f64) -> Option<f64> {
    let jetons = decouper(formule)?;
    if jetons.is_empty() {
        return None;
    }
    let mut position = 0;
    let resultat = expression(&jetons, &mut position, x)?;
    if position != jetons.len() {
        return None; // jetons en trop : formule mal formée
    }
    resultat.is_finite().then_some(resultat)
}

#[derive(Debug, Clone, PartialEq)]
enum Jeton {
    Nombre(f64),
    Variable,
    Plus,
    Moins,
    Fois,
    Divise,
    Puissance,
    ParentheseOuvrante,
    ParentheseFermante,
}

fn decouper(source: &str) -> Option<Vec<Jeton>> {
    let mut jetons = Vec::new();
    let caracteres: Vec<char> = source.chars().collect();
    let mut i = 0;
    while i < caracteres.len() {
        let c = caracteres[i];
        match c {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '+' => {
                jetons.push(Jeton::Plus);
                i += 1;
            }
            '-' => {
                jetons.push(Jeton::Moins);
                i += 1;
            }
            '*' => {
                jetons.push(Jeton::Fois);
                i += 1;
            }
            '/' => {
                jetons.push(Jeton::Divise);
                i += 1;
            }
            '^' => {
                jetons.push(Jeton::Puissance);
                i += 1;
            }
            '(' => {
                jetons.push(Jeton::ParentheseOuvrante);
                i += 1;
            }
            ')' => {
                jetons.push(Jeton::ParentheseFermante);
                i += 1;
            }
            'x' | 'X' => {
                jetons.push(Jeton::Variable);
                i += 1;
            }
            c if c.is_ascii_digit() || c == '.' || c == ',' => {
                let debut = i;
                while i < caracteres.len()
                    && (caracteres[i].is_ascii_digit()
                        || caracteres[i] == '.'
                        || caracteres[i] == ',')
                {
                    i += 1;
                }
                let texte: String = caracteres[debut..i]
                    .iter()
                    .collect::<String>()
                    .replace(',', ".");
                jetons.push(Jeton::Nombre(texte.parse().ok()?));
            }
            _ => return None, // caractère non reconnu : formule refusée entièrement
        }
    }
    Some(jetons)
}

// Grammaire (priorités croissantes) :
//   expression := terme (('+' | '-') terme)*
//   terme      := facteur (('*' | '/') facteur)*
//   facteur    := puissance ('^' facteur)?      -- associativité à droite
//   puissance  := '-' puissance | primaire
//   primaire   := nombre | 'X' | '(' expression ')'

fn expression(jetons: &[Jeton], position: &mut usize, x: f64) -> Option<f64> {
    let mut valeur = terme(jetons, position, x)?;
    loop {
        match jetons.get(*position) {
            Some(Jeton::Plus) => {
                *position += 1;
                valeur += terme(jetons, position, x)?;
            }
            Some(Jeton::Moins) => {
                *position += 1;
                valeur -= terme(jetons, position, x)?;
            }
            _ => break,
        }
    }
    Some(valeur)
}

fn terme(jetons: &[Jeton], position: &mut usize, x: f64) -> Option<f64> {
    let mut valeur = facteur(jetons, position, x)?;
    loop {
        match jetons.get(*position) {
            Some(Jeton::Fois) => {
                *position += 1;
                valeur *= facteur(jetons, position, x)?;
            }
            Some(Jeton::Divise) => {
                *position += 1;
                let diviseur = facteur(jetons, position, x)?;
                if diviseur == 0.0 {
                    return None;
                }
                valeur /= diviseur;
            }
            _ => break,
        }
    }
    Some(valeur)
}

fn facteur(jetons: &[Jeton], position: &mut usize, x: f64) -> Option<f64> {
    let base = puissance(jetons, position, x)?;
    if matches!(jetons.get(*position), Some(Jeton::Puissance)) {
        *position += 1;
        let exposant = facteur(jetons, position, x)?;
        return Some(base.powf(exposant));
    }
    Some(base)
}

fn puissance(jetons: &[Jeton], position: &mut usize, x: f64) -> Option<f64> {
    if matches!(jetons.get(*position), Some(Jeton::Moins)) {
        *position += 1;
        return puissance(jetons, position, x).map(|v| -v);
    }
    primaire(jetons, position, x)
}

fn primaire(jetons: &[Jeton], position: &mut usize, x: f64) -> Option<f64> {
    match jetons.get(*position) {
        Some(Jeton::Nombre(valeur)) => {
            *position += 1;
            Some(*valeur)
        }
        Some(Jeton::Variable) => {
            *position += 1;
            Some(x)
        }
        Some(Jeton::ParentheseOuvrante) => {
            *position += 1;
            let valeur = expression(jetons, position, x)?;
            if matches!(jetons.get(*position), Some(Jeton::ParentheseFermante)) {
                *position += 1;
                Some(valeur)
            } else {
                None // parenthèse jamais refermée
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_formule_de_la_fiche_tc100_se_calcule_correctement() {
        // ((X * 0.00323) - 0.25) / 0.028, X = 18023 (compte ADC de l'exemple).
        let resultat = evaluer("((X * 0.00323) - 0.25) / 0.028", 18023.0).unwrap();
        let attendu = ((18023.0 * 0.00323) - 0.25) / 0.028;
        assert!((resultat - attendu).abs() < 1e-9);
    }

    #[test]
    fn priorite_des_operateurs_respectee_sans_parentheses() {
        assert_eq!(evaluer("2 + 3 * 4", 0.0), Some(14.0));
        assert_eq!(evaluer("X + 1", 5.0), Some(6.0));
        assert_eq!(evaluer("2 ^ 3 ^ 2", 0.0), Some(512.0)); // associativité à droite
    }

    #[test]
    fn les_decimales_a_la_francaise_sont_acceptees() {
        assert_eq!(evaluer("X * 0,5", 10.0), Some(5.0));
    }

    #[test]
    fn une_formule_vide_ou_mal_formee_ne_renvoie_rien() {
        assert_eq!(evaluer("", 1.0), None);
        assert_eq!(evaluer("X +", 1.0), None);
        assert_eq!(evaluer("(X + 1", 1.0), None);
        assert_eq!(evaluer("X + 1)", 1.0), None);
        assert_eq!(evaluer("X # 1", 1.0), None);
    }

    #[test]
    fn une_division_par_zero_ne_fabrique_pas_de_valeur() {
        assert_eq!(evaluer("X / 0", 1.0), None);
        assert_eq!(evaluer("X / (2 - 2)", 1.0), None);
    }

    #[test]
    fn le_moins_unaire_fonctionne_seul_et_apres_une_parenthese() {
        assert_eq!(evaluer("-X", 4.0), Some(-4.0));
        assert_eq!(evaluer("2 * -X", 4.0), Some(-8.0));
    }
}
