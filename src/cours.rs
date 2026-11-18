//! Suivi des cours (page Quotidien), relevés aux sources d'origine :
//! - prix de base Cooperl : la saisie hebdomadaire faite dans EO_Compare
//!   (adresse `/api/cours`, paramètre `eo_compare_adresse`) ;
//! - cadran de Plérin : Réseau des Nouvelles des Marchés de FranceAgriMer ;
//! - céréales : note hebdomadaire « Situation des cours et marchés » du RNM
//!   (FranceAgriMer), publiée le jeudi en PDF ;
//! - blé et maïs Euronext (MATIF), échéance la plus proche : cours du marché
//!   (différé de 15 min), relevé toutes les 30 min pendant la séance.
//! Relevé automatique une fois par jour, ou à la demande depuis la page.

use chrono::{Local, NaiveDate, Timelike};
use regex::Regex;
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::time::Duration;

/// Adresse d'EO_Compare par défaut (paramètre `eo_compare_adresse`).
const EO_COMPARE: &str = "https://eo-elevage.fr";
/// Historique du cadran de Plérin (porc charcutier base 56 TMP, € les 100 kg).
const RNM_PRIX: &str = "https://rnm.franceagrimer.fr/prix";
const RNM_PLERIN: &str = "70178";
/// Dernière note hebdomadaire « Situation des cours et marchés » (PDF).
const RNM_NOTE: &str = "https://rnm.franceagrimer.fr/prix?MESSAGE=1779";
/// Tableau des échéances d'un contrat à terme Euronext (code produit).
const EURONEXT: &str = "https://live.euronext.com/fr/ajax/getPricesFutures/commodities-futures";
const EURONEXT_BLE: &str = "https://live.euronext.com/fr/product/commodities-futures/EBM-DPAR";
const EURONEXT_MAIS: &str = "https://live.euronext.com/fr/product/commodities-futures/EMA-DPAR";
/// Contrats Euronext suivis : (produit, code du contrat).
const MATIF: [(&str, &str); 2] = [("ble_matif", "EBM"), ("mais_matif", "EMA")];

pub struct Produit {
    pub code: &'static str,
    pub libelle: &'static str,
    pub unite: &'static str,
    pub source: &'static str,
    pub lien: &'static str,
}

/// Ordre d'affichage sur la page.
pub const PRODUITS: [Produit; 7] = [
    Produit { code: "porc_cooperl", libelle: "Prix de base Cooperl", unite: "€/kg", source: "Saisie dans EO_Compare", lien: EO_COMPARE },
    Produit { code: "porc_plerin", libelle: "Cadran de Plérin, porc base 56 TMP", unite: "€/kg", source: "FranceAgriMer (RNM)", lien: RNM_PRIX },
    Produit { code: "ble_matif", libelle: "Blé meunier Euronext (MATIF)", unite: "€/t", source: "Euronext, toutes les 30 min", lien: EURONEXT_BLE },
    Produit { code: "mais_matif", libelle: "Maïs Euronext (MATIF)", unite: "€/t", source: "Euronext, toutes les 30 min", lien: EURONEXT_MAIS },
    Produit { code: "ble_rouen", libelle: "Blé tendre FOB Rouen", unite: "€/t", source: "FranceAgriMer (RNM), hebdomadaire", lien: RNM_NOTE },
    Produit { code: "orge_rouen", libelle: "Orge fourragère rendu Rouen", unite: "€/t", source: "FranceAgriMer (RNM), hebdomadaire", lien: RNM_NOTE },
    Produit { code: "mais_bordeaux", libelle: "Maïs rendu Bordeaux", unite: "€/t", source: "FranceAgriMer (RNM), hebdomadaire", lien: RNM_NOTE },
];

/// Céréales de la note du RNM : (produit, début du libellé dans la note).
const CEREALES: [(&str, &str); 3] = [
    ("ble_rouen", "Blé tendre French Channel Wheat FOB Rouen"),
    ("orge_rouen", "Orge fourragère rendu Rouen"),
    ("mais_bordeaux", "Maïs rendu Bordeaux"),
];

#[derive(Debug, PartialEq)]
struct Releve {
    produit: &'static str,
    date: NaiveDate,
    prix: f64,
}

fn nombre(s: &str) -> Option<f64> {
    s.trim().replace([' ', '\u{a0}', '\u{202f}'], "").replace(',', ".").parse().ok()
}

/// Historique du cadran sur la page du RNM : une ligne par marché (date,
/// prix moyen en € HT les 100 kg, …).
fn lire_plerin(page: &str) -> Vec<Releve> {
    let ligne = Regex::new(r#"<tr><td class="tdcotr">(\d{2})/(\d{2})/(\d{2}) </td>\s*<td class="tdcotr">([0-9.]+) </td>"#).unwrap();
    ligne
        .captures_iter(page)
        .filter_map(|c| {
            let date = NaiveDate::from_ymd_opt(2000 + c[3].parse::<i32>().ok()?, c[2].parse().ok()?, c[1].parse().ok()?)?;
            let prix = (c[4].parse::<f64>().ok()? * 10.0).round() / 1000.0;
            // Garde-fou : un cours hors de ces bornes est une erreur de lecture.
            (0.3..=4.0).contains(&prix).then_some(Releve { produit: "porc_plerin", date, prix })
        })
        .collect()
}

const MOIS: [&str; 12] = [
    "janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre",
];

/// Note hebdomadaire du RNM (texte de `pdftotext -layout`) : « jeudi 22
/// octobre 2026 » en tête, puis des lignes « libellé  €/t  sem. N-1  sem. N ».
/// La semaine N est datée du jour de la note, la semaine N-1 sept jours avant.
fn lire_note(texte: &str) -> Vec<Releve> {
    let entete = Regex::new(r"(\d{1,2}) ([a-zéû]+) (\d{4})").unwrap();
    let Some(date) = entete.captures(texte).and_then(|c| {
        let mois = MOIS.iter().position(|m| *m == &c[2])? as u32 + 1;
        NaiveDate::from_ymd_opt(c[3].parse().ok()?, mois, c[1].parse().ok()?)
    }) else {
        return Vec::new();
    };
    let ligne = Regex::new(r"^(.*?)\s*€/t\s+([0-9][0-9 ]*,[0-9]+)\s+([0-9][0-9 ]*,[0-9]+)").unwrap();
    let lignes: Vec<&str> = texte.lines().collect();
    let mut releves = Vec::new();
    for (i, l) in lignes.iter().enumerate() {
        let Some(c) = ligne.captures(l) else { continue };
        // Le libellé est parfois imprimé sous les chiffres (colonne de gauche
        // de l'une des lignes suivantes).
        let mut libelle = c[1].trim().to_string();
        if libelle.is_empty() {
            libelle = lignes[i + 1..]
                .iter()
                .take(12)
                .map(|s| s.trim_start().split("   ").next().unwrap_or("").trim())
                .find(|s| s.chars().any(|ch| ch.is_alphabetic()) && !s.starts_with("€/t"))
                .unwrap_or("")
                .to_string();
        }
        let Some((produit, _)) = CEREALES.iter().find(|(_, debut)| libelle.starts_with(debut)) else { continue };
        if releves.iter().any(|r: &Releve| r.produit == *produit) {
            continue;
        }
        for (jours, valeur) in [(7, &c[2]), (0, &c[3])] {
            if let Some(prix) = nombre(valeur).filter(|p| (50.0..=1000.0).contains(p)) {
                releves.push(Releve { produit, date: date - chrono::Duration::days(jours), prix });
            }
        }
    }
    releves
}

/// Réponse de `/api/cours` d'EO_Compare : `{"cooperl": [{"date","prix"}…]}`.
fn lire_eo_compare(json: &Value) -> Vec<Releve> {
    json["cooperl"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| {
            let date = NaiveDate::parse_from_str(l["date"].as_str()?.get(..10)?, "%Y-%m-%d").ok()?;
            let prix = l["prix"].as_f64().filter(|p| (0.3..=4.0).contains(p))?;
            Some(Releve { produit: "porc_cooperl", date, prix })
        })
        .collect()
}

/// Tableau des échéances Euronext : la première échéance cotée, au dernier
/// cours de la séance s'il y en a un, sinon au cours de compensation de la
/// séance précédente. Rend le relevé et l'échéance (« Dec 2026 »).
fn lire_euronext(page: &str, produit: &'static str, aujourdhui: NaiveDate) -> Option<(Releve, String)> {
    let echeance = Regex::new(r"md=\d{2}-\d{2}-\d{4}[^>]*>([^<]+)</a>").unwrap();
    let cellule = Regex::new(r#"<td class="[^"]*text-right[^"]*">\s*([^<]*?)\s*</td>"#).unwrap();
    for ligne in page.split("<tr").skip(1) {
        let Some(nom) = echeance.captures(ligne).map(|c| c[1].trim().to_string()) else { continue };
        let cases: Vec<&str> = cellule.captures_iter(ligne).map(|c| c.get(1).map_or("", |m| m.as_str())).collect();
        // Achat, Vente, Dernier, Heure, +/-, Volume, Ouvert, Haut, Bas, Compensation.
        let dernier = cases.get(2).and_then(|v| nombre(v));
        let compensation = cases.get(9).and_then(|v| nombre(v));
        let (prix, date) = match (dernier, compensation) {
            (Some(p), _) => (p, aujourdhui),
            (None, Some(p)) => (p, seance_precedente(aujourdhui)),
            _ => continue,
        };
        if (50.0..=1000.0).contains(&prix) {
            return Some((Releve { produit, date, prix }, nom));
        }
    }
    None
}

/// Jour ouvré précédent (le cours de compensation date de la dernière séance).
fn seance_precedente(jour: NaiveDate) -> NaiveDate {
    use chrono::Datelike;
    let mut d = jour - chrono::Duration::days(1);
    while d.weekday().number_from_monday() > 5 {
        d -= chrono::Duration::days(1);
    }
    d
}

/// Séance Euronext des céréales : 10 h 45 – 18 h 30 en semaine (heure de Paris).
fn seance_ouverte(maintenant: chrono::NaiveDateTime) -> bool {
    use chrono::Datelike;
    let minutes = maintenant.hour() * 60 + maintenant.minute();
    maintenant.weekday().number_from_monday() <= 5 && (10 * 60 + 45..=18 * 60 + 45).contains(&minutes)
}

async fn relever_matif(client: &reqwest::Client, pool: &SqlitePool, erreurs: &mut Vec<String>) -> Vec<Releve> {
    let mut releves = Vec::new();
    let aujourdhui = Local::now().date_naive();
    for (produit, code) in MATIF {
        let url = format!("{EURONEXT}/{code}/DPAR");
        match reponse(client.get(&url).send().await, "Euronext").await {
            Ok(r) => {
                let page = r.text().await.unwrap_or_default();
                match lire_euronext(&page, produit, aujourdhui) {
                    Some((releve, echeance)) => {
                        let _ = crate::sms_free::ecrire_parametre(pool, &format!("cours_echeance_{produit}"), &echeance).await;
                        releves.push(releve);
                    }
                    None => erreurs.push(format!("Euronext : cours {code} introuvable")),
                }
            }
            Err(e) => erreurs.push(e),
        }
    }
    releves
}

async fn enregistrer(pool: &SqlitePool, releves: &[Releve], maintenant: &str, erreurs: &mut Vec<String>) {
    for r in releves {
        let res = sqlx::query(
            "INSERT INTO cours_releve(produit,date,prix,releve_le) VALUES(?,?,?,?)
             ON CONFLICT(produit,date) DO UPDATE SET prix=excluded.prix, releve_le=excluded.releve_le",
        )
        .bind(r.produit)
        .bind(r.date.format("%Y-%m-%d").to_string())
        .bind(r.prix)
        .bind(maintenant)
        .execute(pool)
        .await;
        if let Err(e) = res {
            erreurs.push(e.to_string());
        }
    }
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("Mozilla/5.0 (compatible; EO_Sevel suivi des cours)")
        .build()
        .map_err(|e| e.to_string())
}

/// Relevé des seuls cours Euronext, pendant la séance.
pub async fn actualiser_matif(pool: &SqlitePool) {
    let Ok(client) = client() else { return };
    let mut erreurs = Vec::new();
    let releves = relever_matif(&client, pool, &mut erreurs).await;
    let maintenant = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    enregistrer(pool, &releves, &maintenant, &mut erreurs).await;
    if !erreurs.is_empty() {
        tracing::warn!(erreurs=%erreurs.join(" ; "), "Relevé des cours Euronext");
    }
}

fn pdf_en_texte(pdf: Vec<u8>) -> Result<String, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut enfant = Command::new("pdftotext")
        .args(["-layout", "-l", "4", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("pdftotext : {e}"))?;
    enfant.stdin.take().ok_or("pdftotext")?.write_all(&pdf).map_err(|e| e.to_string())?;
    let sortie = enfant.wait_with_output().map_err(|e| e.to_string())?;
    if !sortie.status.success() {
        return Err("pdftotext : lecture de la note impossible".into());
    }
    Ok(String::from_utf8_lossy(&sortie.stdout).into_owned())
}

async fn reponse(r: Result<reqwest::Response, reqwest::Error>, nom: &str) -> Result<reqwest::Response, String> {
    let r = r.map_err(|e| format!("{nom} : {e}"))?;
    if !r.status().is_success() {
        return Err(format!("{nom} : réponse {}", r.status()));
    }
    Ok(r)
}

async fn relever_sources(client: &reqwest::Client, pool: &SqlitePool, erreurs: &mut Vec<String>) -> Vec<Releve> {
    let mut releves = Vec::new();
    // Cadran de Plérin.
    match reponse(client.post(RNM_PRIX).form(&[("LAST", ""), ("LIBCOD", RNM_PLERIN)]).send().await, "FranceAgriMer, Plérin").await {
        Ok(r) => {
            let page = r.bytes().await.map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
            let lus = lire_plerin(&page);
            if lus.is_empty() {
                erreurs.push("FranceAgriMer : cours de Plérin introuvables".into());
            }
            releves.extend(lus.into_iter().take(30));
        }
        Err(e) => erreurs.push(e),
    }
    // Céréales : note hebdomadaire du RNM.
    match reponse(client.get(RNM_NOTE).send().await, "FranceAgriMer, note hebdomadaire").await {
        Ok(r) => match r.bytes().await {
            Ok(pdf) => match tokio::task::spawn_blocking(move || pdf_en_texte(pdf.to_vec())).await {
                Ok(Ok(texte)) => {
                    let lus = lire_note(&texte);
                    for (produit, debut) in CEREALES {
                        if !lus.iter().any(|r| r.produit == produit) {
                            erreurs.push(format!("FranceAgriMer : « {debut} » absent de la note"));
                        }
                    }
                    releves.extend(lus);
                }
                Ok(Err(e)) => erreurs.push(e),
                Err(e) => erreurs.push(e.to_string()),
            },
            Err(e) => erreurs.push(e.to_string()),
        },
        Err(e) => erreurs.push(e),
    }
    // Prix de base Cooperl saisi dans EO_Compare.
    let adresse = crate::proxmox::parametre(pool, "eo_compare_adresse")
        .await
        .unwrap_or_else(|| EO_COMPARE.to_string());
    let url = format!("{}/api/cours", adresse.trim_end_matches('/'));
    match reponse(client.get(&url).send().await, "EO_Compare").await {
        Ok(r) => match r.json::<Value>().await {
            Ok(json) => {
                let lus = lire_eo_compare(&json);
                if lus.is_empty() {
                    erreurs.push("EO_Compare : aucun prix de base Cooperl saisi".into());
                }
                releves.extend(lus);
            }
            Err(e) => erreurs.push(format!("EO_Compare : {e}")),
        },
        Err(e) => erreurs.push(e),
    }
    releves
}

/// Relève tous les cours et les enregistre. Rend les erreurs rencontrées
/// (une source en panne n'empêche pas de relever les autres).
pub async fn actualiser(pool: &SqlitePool) -> Vec<String> {
    let mut erreurs = Vec::new();
    let client = match client() {
        Ok(c) => c,
        Err(e) => return vec![e],
    };
    let mut releves = relever_sources(&client, pool, &mut erreurs).await;
    releves.extend(relever_matif(&client, pool, &mut erreurs).await);
    let maintenant = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    enregistrer(pool, &releves, &maintenant, &mut erreurs).await;
    let _ = crate::sms_free::ecrire_parametre(pool, "cours_dernier_releve", &maintenant).await;
    let _ = crate::sms_free::ecrire_parametre(pool, "cours_erreurs", &erreurs.join(" ; ")).await;
    if !erreurs.is_empty() {
        tracing::warn!(erreurs=%erreurs.join(" ; "), "Relevé des cours");
    }
    erreurs
}

/// Boucle lancée au démarrage : un relevé par jour à partir de 14 h (cours
/// de Plérin publiés en fin de matinée), ou tout de suite si le dernier
/// relevé a plus d'un jour.
pub async fn boucle(pool: SqlitePool) {
    tokio::time::sleep(Duration::from_secs(120)).await;
    loop {
        let maintenant = Local::now().naive_local();
        let dernier = crate::proxmox::parametre(&pool, "cours_dernier_releve")
            .await
            .and_then(|d| chrono::NaiveDateTime::parse_from_str(&d, "%Y-%m-%d %H:%M:%S").ok());
        let a_faire = match dernier {
            None => true,
            Some(d) => {
                (maintenant - d).num_hours() >= 24
                    || (maintenant.hour() >= 14 && (d.date() < maintenant.date() || d.hour() < 14))
            }
        };
        if a_faire {
            actualiser(&pool).await;
        } else if seance_ouverte(maintenant) {
            actualiser_matif(&pool).await;
        }
        tokio::time::sleep(Duration::from_secs(1800)).await;
    }
}

/// Données de la page : dernier cours, variation, courbe et historique.
pub async fn suivi(pool: &SqlitePool) -> sqlx::Result<Value> {
    let mut lignes = Vec::new();
    for p in &PRODUITS {
        let rows: Vec<(String, f64, Option<String>)> =
            sqlx::query_as("SELECT date, prix, releve_le FROM cours_releve WHERE produit=? ORDER BY date DESC LIMIT 20")
                .bind(p.code)
                .fetch_all(pool)
                .await?;
        let variation = match (rows.first(), rows.get(1)) {
            (Some(a), Some(b)) => Some(a.1 - b.1),
            _ => None,
        };
        // Courbe SVG (120 × 28) du plus ancien au plus récent relevé.
        let (min, max) = rows.iter().fold((f64::MAX, f64::MIN), |(lo, hi), r| (lo.min(r.1), hi.max(r.1)));
        let n = rows.len();
        let courbe = if n >= 2 {
            let ecart = if max > min { max - min } else { 1.0 };
            rows.iter()
                .rev()
                .enumerate()
                .map(|(i, r)| format!("{:.1},{:.1}", i as f64 * 120.0 / (n - 1) as f64, 26.0 - (r.1 - min) / ecart * 24.0))
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            String::new()
        };
        let porc = p.unite == "€/kg";
        let echeance = crate::proxmox::parametre(pool, &format!("cours_echeance_{}", p.code)).await;
        // Graphique de l'historique : jusqu'à un an de relevés, du plus ancien au plus récent.
        let serie: Vec<(String, f64)> = sqlx::query_as(
            "SELECT date, prix FROM (SELECT date, prix FROM cours_releve WHERE produit=? ORDER BY date DESC LIMIT 260) ORDER BY date",
        )
        .bind(p.code)
        .fetch_all(pool)
        .await?;
        lignes.push(json!({
            "code": p.code,
            "libelle": p.libelle,
            "unite": p.unite,
            "source": p.source,
            "lien": p.lien,
            "porc": porc,
            "date": rows.first().map(|r| r.0.clone()),
            // Cours Euronext : l'heure du relevé compte (cours de la séance).
            "heure": if p.code.ends_with("_matif") { rows.first().and_then(|r| r.2.as_ref()).and_then(|h| h.get(11..16)).map(str::to_string) } else { None },
            "echeance": echeance,
            "serie": serie.iter().map(|r| json!({"date": r.0, "prix": r.1})).collect::<Vec<_>>(),
            "prix": rows.first().map(|r| r.1),
            "variation": variation,
            "courbe": courbe,
            "historique": rows.iter().map(|r| json!({"date": r.0, "prix": r.1})).collect::<Vec<_>>(),
        }));
    }
    let dernier = crate::proxmox::parametre(pool, "cours_dernier_releve").await;
    let erreurs = crate::proxmox::parametre(pool, "cours_erreurs").await;
    Ok(json!({"lignes": lignes, "dernier_releve": dernier, "erreurs": erreurs}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(a: i32, m: u32, j: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(a, m, j).unwrap()
    }

    #[test]
    fn lit_les_sources() {
        let note = "Semaine 42 / 2026                     jeudi 22 octobre 2026\n\
                    Céréales\n\
                    Blé tendre French Channel Wheat FOB Rouen A2          €/t        254,19   258,54      =\n\
                    Orge fourragère rendu Rouen                           €/t        227,40   224,10      =\n\
                    \n\
                    Blé dur rendu Port La Nouvelle                        €/t        295,00   301,00   2,0%     400\n\
                                                                          €/t        282,30   290,83   3,0%\n\
                    \n\
                                                                                                          €/t\n\
                    Maïs rendu Bordeaux                                                                    200\n\
                    PORC (carcasse) France classe S entrée abattoir      €/kg         1,98     1,92       -\n";
        let r = lire_note(note);
        assert_eq!(r.len(), 6);
        assert_eq!(r[0], Releve { produit: "ble_rouen", date: d(2026, 10, 15), prix: 254.19 });
        assert_eq!(r[1], Releve { produit: "ble_rouen", date: d(2026, 10, 22), prix: 258.54 });
        assert_eq!(r[3].prix, 224.10);
        assert_eq!(r[5], Releve { produit: "mais_bordeaux", date: d(2026, 10, 22), prix: 290.83 });

        let page = r#"<tr><td class="tdcotr">22/10/26 </td> <td class="tdcotr">156.20 </td><td class="tdcotr">156.20 </td>"#;
        assert_eq!(lire_plerin(page), vec![Releve { produit: "porc_plerin", date: d(2026, 10, 22), prix: 1.562 }]);

        // Euronext avant l'ouverture : pas de dernier cours, compensation de
        // la séance précédente (lundi → vendredi).
        let ligne = |dernier: &str| format!(
            r#"<tr data-lasttradesdate="-"><td class="text-left"><a href="/fr/x?md=01-12-2026" class="b">Dec 2026</a></td>
            <td class="text-right"> 235,75 </td><td class="text-right"> 234,25 </td><td class="text-right"> {dernier} </td>
            <td class="text-right"> - </td><td class="text-ui-pistachio-green text-right">0.00</td><td class="text-right"> - </td>
            <td class="text-right"> - </td><td class="text-right"> - </td><td class="text-right"> - </td>
            <td class="text-right"> 233,50 </td><td class="text-right"> 108 754 </td></tr>"#);
        let lundi = d(2026, 11, 16);
        assert_eq!(
            lire_euronext(&format!("<table>{}", ligne("-")), "ble_matif", lundi),
            Some((Releve { produit: "ble_matif", date: d(2026, 11, 13), prix: 233.5 }, "Dec 2026".into()))
        );
        assert_eq!(
            lire_euronext(&format!("<table>{}", ligne("236,25")), "ble_matif", lundi).map(|r| r.0),
            Some(Releve { produit: "ble_matif", date: lundi, prix: 236.25 })
        );
        assert!(seance_ouverte(d(2026, 11, 18).and_hms_opt(11, 0, 0).unwrap()));
        assert!(!seance_ouverte(d(2026, 11, 21).and_hms_opt(11, 0, 0).unwrap()));

        let json = json!({"cooperl": [{"date": "2026-10-19", "prix": 1.5}], "porc": []});
        assert_eq!(lire_eo_compare(&json), vec![Releve { produit: "porc_cooperl", date: d(2026, 10, 19), prix: 1.5 }]);
    }
}
