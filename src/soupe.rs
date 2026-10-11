//! Machine à soupe Asserva (Exa-Feeder GX, logiciel PC Mega-feeder NT), suivie
//! en écoute seule.
//!
//! Le PC Windows XP interroge la machine toutes les 1,5 s par la liaison série
//! (câble FTDI passé à la VM, puis Network Modem Asserva et RS485). Un petit
//! service sur l'hôte Proxmox (`soupe-ecoute`, script servi par
//! /api/soupe/script) lit ce trafic avec usbmon et poste les trames ici.
//! EO_Sevel ne parle jamais à la machine.
//!
//! Trame : STX (question) ou SOH (réponse), adresse sur 2 chiffres, commande,
//! sous-commande, longueur sur 4 chiffres, contenu (valeurs séparées par
//! « ; »), contrôle sur 4 chiffres (XOR des octets en 3 chiffres + dernier
//! chiffre de leur somme), CR. Le service vérifie le contrôle avant d'envoyer.
//!
//! Réponse d'état `c0`, champs relevés le 28/09/2026 sur un repas complet
//! comparé aux écrans d'Exa-Feeder : voir [`Etat::depuis_champs`].

use anyhow::Result;
use chrono::Local;
use sqlx::SqlitePool;
use std::sync::Mutex;

/// Sans trame depuis ce délai, le PC Mega-feeder ou le câble est muet.

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Etat {
    pub poids: i64,
    pub sequence: i64,
    pub formule: i64,
    pub temps_min: i64,
    pub vanne: i64,
    pub consigne: f64,
    pub controle: f64,
    pub debit: f64,
    pub rpm: i64,
    pub passage: i64,
    pub produit: i64,
    pub doit: i64,
    pub seuil: i64,
    pub cumul: i64,
    pub phase: i64,
}

impl Etat {
    /// Champs de la réponse `c0` (après l'en-tête de 8 caractères) :
    /// 0 poids cuve (kg) · 2 séquence · 3 formule · 4 temps (min) · 5 vanne en
    /// cours · 6 consigne ×10 kg · 7 contrôle ×10 kg · 8 débit ×10 · 9 vitesse
    /// mélangeur (%rpm) · 10 remise en circuit (passe à 1 après l'eau
    /// poussante ; ce n'est PAS le n° de passage, compté à part) · 14 cumul distribué (kg) ·
    /// 15 produit en cours (0 = eau propre) · 16 « Doit » · 17 « Seuil » ·
    /// 19 phase.
    pub fn depuis_champs(champs: &[&str]) -> Option<Self> {
        if champs.len() < 20 {
            return None;
        }
        let n = |i: usize| champs[i].trim().parse::<i64>().unwrap_or(0);
        Some(Self {
            poids: n(0),
            sequence: n(2),
            formule: n(3),
            temps_min: n(4),
            vanne: n(5),
            consigne: n(6) as f64 / 10.0,
            controle: n(7) as f64 / 10.0,
            debit: n(8) as f64 / 10.0,
            rpm: n(9),
            passage: n(10),
            cumul: n(14),
            produit: n(15),
            doit: n(16),
            seuil: n(17),
            phase: n(19),
        })
    }

    fn en_distribution(&self) -> bool {
        matches!(self.phase, 31 | 35 | 45)
    }
}

/// Libellé d'une phase, tel qu'Exa-Feeder l'affiche.
pub fn libelle_phase(phase: i64) -> String {
    match phase {
        0 => "Automate à l'arrêt".into(),
        1 => "Dosage produit".into(),
        7 => "Alerte dosage".into(),
        37 => "Alarme débit distribution".into(),
        9 => "Contrôle / attente".into(),
        13 => "Mélange".into(),
        15 => "Test circuit".into(),
        23..=29 => "Remplissage circuit".into(),
        31 => "Dosage eau poussante".into(),
        35 => "Distribution".into(),
        45 => "Remplissage queue de circuit".into(),
        49 => "Dosage eau de rinçage".into(),
        n => format!("Étape {n}"),
    }
}

/// Noms des produits (réponse `X`, dans l'ordre des numéros de produit, 0 = eau).
pub async fn produits(pool: &SqlitePool) -> Vec<String> {
    sqlx::query_scalar::<_, String>("SELECT contenu FROM soupe_donnee WHERE commande='X'")
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .map(|c| c.split(';').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default()
}

/// Découpe une trame (sans STX/SOH ni CR, contrôle compris) :
/// (sens, commande, sous-commande, champs du contenu).
fn decouper(trame: &str) -> Option<(char, char, Vec<&str>)> {
    let cmd = trame.chars().nth(2)?;
    let sous = trame.chars().nth(3)?;
    let contenu = trame.get(8..trame.len().checked_sub(4)?)?;
    let mut champs: Vec<&str> = contenu.split(';').collect();
    if champs.last() == Some(&"") {
        champs.pop();
    }
    Some((cmd, sous, champs))
}

/// État précédent en mémoire, pour détecter les fins de vanne et les
/// changements de phase sans relire la base à chaque trame.
static PRECEDENT: Mutex<Option<Etat>> = Mutex::new(None);
/// Cumul distribué (champ 14) au début de la vanne en cours : la machine
/// l'incrémente à chaque changement de vanne, la différence donne la quantité
/// exacte servie, plus fiable que le dernier « contrôle » lu.
static CUMUL_DEBUT: Mutex<i64> = Mutex::new(0);

/// Traite un lot de lignes « horodatage<TAB>sens<TAB>trame » envoyé par le
/// service d'écoute (sens `<` = réponse de la machine, `>` = question du PC).
/// Durée d'alarme débit distribution avant l'alerte (et le SMS).
const ALARME_DISTRIBUTION_S: i64 = 180;

pub async fn recevoir(pool: &SqlitePool, lot: &str) -> Result<usize> {
    let mut traitees = 0;
    for ligne in lot.lines() {
        let mut morceaux = ligne.splitn(3, '\t');
        let (Some(ts), Some(sens), Some(trame)) = (morceaux.next(), morceaux.next(), morceaux.next()) else {
            continue;
        };
        let Ok(ts) = ts.trim().parse::<f64>() else { continue };
        let ts = ts as i64;
        let Some((cmd, sous, champs)) = decouper(trame.trim()) else { continue };
        match (sens, cmd, sous) {
            ("<", 'c', '0') => {
                if let Some(etat) = Etat::depuis_champs(&champs) {
                    traiter_etat(pool, ts, etat).await?;
                    traitees += 1;
                }
            }
            // Autres réponses (synchronisation du PC) : gardées telles quelles,
            // par commande et sous-commande (« R1 » = fiche vanne 1…).
            ("<", c, _) if c != 'c' && c.is_ascii_alphabetic() => {
                let cle = match c {
                    'R' | 'S' => format!("{c}{}", champs.first().copied().unwrap_or("")),
                    _ => c.to_string(),
                };
                sqlx::query(
                    "INSERT INTO soupe_donnee(commande,ts,contenu) VALUES(?,?,?) \
                     ON CONFLICT(commande) DO UPDATE SET ts=excluded.ts, contenu=excluded.contenu",
                )
                .bind(cle)
                .bind(ts)
                .bind(champs.join(";"))
                .execute(pool)
                .await?;
            }
            (">", 'c', '2') => evenement(pool, ts, None, "Alarme acquittée depuis le PC").await?,
            (">", 'c', '1') => evenement(pool, ts, None, "Formule annulée depuis le PC").await?,
            (">", 'b', _) => evenement(pool, ts, None, "Séquence lancée depuis le PC").await?,
            (">", 'q', _) => evenement(pool, ts, None, "Formules modifiées depuis le PC").await?,
            _ => {}
        }
    }
    Ok(traitees)
}

async fn traiter_etat(pool: &SqlitePool, ts: i64, etat: Etat) -> Result<()> {
    let precedent = PRECEDENT.lock().unwrap().replace(etat.clone());
    if let Some(avant) = &precedent {
        // Fin d'une vanne : la vanne ou le passage change, ou le contrôle
        // repart à zéro sur la même vanne (2e passage), ou la distribution
        // s'arrête. On garde la dernière quantité relevée pour cette vanne.
        let vanne_finie = avant.en_distribution()
            && avant.vanne > 0
            && avant.consigne > 0.0
            && (etat.vanne != avant.vanne
                || etat.passage != avant.passage
                || etat.controle + 0.5 < avant.controle
                || !etat.en_distribution());
        if !avant.en_distribution() && etat.en_distribution() {
            *CUMUL_DEBUT.lock().unwrap() = etat.cumul;
        }
        if vanne_finie {
            // Le cumul repart à zéro en fin de repas : garder le dernier lu.
            let fin = etat.cumul.max(avant.cumul);
            let debut = std::mem::replace(&mut *CUMUL_DEBUT.lock().unwrap(), if etat.cumul < avant.cumul { 0 } else { fin });
            let servi = if fin > debut { (fin - debut) as f64 } else { avant.controle };
            // Passage = rang de cette vanne dans le repas en cours (2 h).
            let jour = Local::now().format("%Y-%m-%d").to_string();
            sqlx::query(
                "INSERT INTO soupe_distribution(ts,jour,sequence,formule,passage,vanne,consigne,distribue) \
                 VALUES(?1,?2,?3,?4,(SELECT COUNT(*)+1 FROM soupe_distribution WHERE jour=?2 AND sequence=?3 AND formule=?4 AND vanne=?5 AND ts>?1-7200),?5,?6,?7)",
            )
            .bind(ts)
            .bind(&jour)
            .bind(avant.sequence)
            .bind(avant.formule)
            .bind(avant.vanne)
            .bind(avant.consigne)
            .bind(servi)
            .execute(pool)
            .await?;
        }
    }
    if precedent.as_ref().map(|p| p.phase) != Some(etat.phase) {
        evenement(pool, ts, Some(etat.phase), &libelle_phase(etat.phase)).await?;
        if etat.phase == 7 {
            ouvrir_alerte(
                pool,
                "soupe_alerte",
                &format!("Machine à soupe : alerte dosage (séquence {}, formule {})", etat.sequence, etat.formule),
            )
            .await?;
        } else {
            fermer_alerte(pool, "soupe_alerte").await?;
        }
    }
    // Alarme débit distribution (phase 37) : souvent acquittée en quelques
    // secondes au PC. Au-delà de quelques minutes, personne n'est là : la
    // machine reste en sécurité et les porcs ne sont plus servis.
    if etat.phase == 37 {
        let debut: Option<i64> = sqlx::query_scalar(
            "SELECT ts FROM soupe_evenement WHERE phase IS NOT NULL ORDER BY ts DESC, id DESC LIMIT 1",
        )
        .fetch_optional(pool)
        .await?;
        if debut.is_some_and(|d| ts - d >= ALARME_DISTRIBUTION_S) {
            ouvrir_alerte(
                pool,
                "soupe_distribution",
                &format!(
                    "Machine à soupe en sécurité : alarme débit distribution, vanne {} ({:.1} kg servis sur {:.1}). Les porcs ne sont plus servis.",
                    etat.vanne, etat.controle, etat.consigne
                ),
            )
            .await?;
        }
    } else {
        fermer_alerte(pool, "soupe_distribution").await?;
    }
    fermer_alerte(pool, "soupe_silence").await?;
    sqlx::query(
        "INSERT INTO soupe_etat(id,ts,phase,poids,sequence,formule,temps_min,vanne,consigne,controle,debit,rpm,passage,produit,doit,seuil,cumul) \
         VALUES(1,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET \
         ts=excluded.ts,phase=excluded.phase,poids=excluded.poids,sequence=excluded.sequence,formule=excluded.formule,\
         temps_min=excluded.temps_min,vanne=excluded.vanne,consigne=excluded.consigne,controle=excluded.controle,\
         debit=excluded.debit,rpm=excluded.rpm,passage=excluded.passage,produit=excluded.produit,doit=excluded.doit,\
         seuil=excluded.seuil,cumul=excluded.cumul",
    )
    .bind(ts)
    .bind(etat.phase)
    .bind(etat.poids)
    .bind(etat.sequence)
    .bind(etat.formule)
    .bind(etat.temps_min)
    .bind(etat.vanne)
    .bind(etat.consigne)
    .bind(etat.controle)
    .bind(etat.debit)
    .bind(etat.rpm)
    .bind(etat.passage + 1)
    .bind(etat.produit)
    .bind(etat.doit)
    .bind(etat.seuil)
    .bind(etat.cumul)
    .execute(pool)
    .await?;
    Ok(())
}

async fn evenement(pool: &SqlitePool, ts: i64, phase: Option<i64>, message: &str) -> Result<()> {
    sqlx::query("INSERT INTO soupe_evenement(ts,phase,message) VALUES(?,?,?)")
        .bind(ts)
        .bind(phase)
        .bind(message)
        .execute(pool)
        .await?;
    Ok(())
}

/// Alerte dans `ambiance_alerte` (sans salle) : la boucle SMS l'envoie comme
/// les autres alertes de l'élevage.
async fn ouvrir_alerte(pool: &SqlitePool, type_alerte: &str, message: &str) -> Result<()> {
    let ouverte: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ambiance_alerte WHERE type=? AND fin IS NULL")
        .bind(type_alerte)
        .fetch_one(pool)
        .await?;
    if ouverte == 0 {
        sqlx::query(
            "INSERT INTO ambiance_alerte(salle_id,type,gravite,debut,message) VALUES(NULL,?,'critique',datetime('now','localtime'),?)",
        )
        .bind(type_alerte)
        .bind(message)
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn fermer_alerte(pool: &SqlitePool, type_alerte: &str) -> Result<()> {
    sqlx::query("UPDATE ambiance_alerte SET fin=datetime('now','localtime') WHERE type=? AND fin IS NULL")
        .bind(type_alerte)
        .execute(pool)
        .await?;
    Ok(())
}

/// La machine ne parle au PC que pendant ses cycles : son silence est normal
/// et ne donne pas d'alerte. Seule l'alerte de sécurité (phase 7) prévient.
/// On ferme ici une éventuelle ancienne alerte « plus aucune nouvelle ».
pub async fn boucle(pool: SqlitePool) {
    if let Err(e) = fermer_alerte(&pool, "soupe_silence").await {
        tracing::warn!(error = %e, "fermeture alerte machine à soupe impossible");
    }
}

#[cfg(test)]
pub(crate) fn reinitialiser() {
    *PRECEDENT.lock().unwrap() = None;
    *CUMUL_DEBUT.lock().unwrap() = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trame réelle du 28/09/2026 à 20:29:12 : vanne 6, consigne 32,3 kg,
    // contrôle 2,6 kg, débit 2,0, 46 %rpm, distribution, poids 91 kg.
    const DISTRIBUTION: &str = "01c0011591;0;4;3;5;6;323;26;20;46;0;0;0;495;325;2;134;495;0;35;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;1;0000";

    #[test]
    fn une_trame_d_etat_se_lit_comme_l_ecran() {
        let (cmd, sous, champs) = decouper(DISTRIBUTION).unwrap();
        assert_eq!((cmd, sous), ('c', '0'));
        let e = Etat::depuis_champs(&champs).unwrap();
        assert_eq!((e.poids, e.sequence, e.formule, e.temps_min, e.vanne), (91, 4, 3, 5, 6));
        assert_eq!((e.consigne, e.controle, e.debit, e.rpm), (32.3, 2.6, 2.0, 46));
        assert_eq!((e.produit, e.doit, e.seuil, e.phase), (2, 134, 495, 35));
        assert_eq!(libelle_phase(e.phase), "Distribution");
        assert_eq!(libelle_phase(27), "Remplissage circuit");
    }

    fn trame(vanne: i64, controle: i64, cumul: i64, phase: i64) -> String {
        format!("01c00115100;0;4;3;5;{vanne};323;{controle};20;46;0;0;0;495;{cumul};2;134;495;0;{phase};0;0000")
    }

    #[tokio::test]
    async fn chaque_vanne_servie_est_enregistree_avec_sa_quantite() {
        reinitialiser();
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(
            "CREATE TABLE IF NOT EXISTS ambiance_alerte (id INTEGER PRIMARY KEY AUTOINCREMENT, salle_id INTEGER, type TEXT NOT NULL, \
             gravite TEXT NOT NULL DEFAULT 'alerte', debut TEXT NOT NULL, fin TEXT, message TEXT NOT NULL, acquittee INTEGER NOT NULL DEFAULT 0);",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql(include_str!("../migrations/0024_soupe.sql")).execute(&pool).await.unwrap();
        let lot = [
            trame(1, 14, 0, 35),
            trame(1, 126, 0, 35),
            trame(2, 24, 16, 35),
            trame(2, 150, 16, 35),
            // 2e passage sur la vanne 2 : le contrôle repart de zéro.
            trame(2, 9, 32, 35),
            trame(2, 132, 32, 35),
            // Fin de repas : le cumul retombe à zéro, on garde le dernier lu.
            trame(0, 0, 0, 0),
        ]
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{}\t<\t{t}", 1_000 + i))
        .collect::<Vec<_>>()
        .join("\n");
        assert_eq!(recevoir(&pool, &lot).await.unwrap(), 7);
        let servies: Vec<(i64, i64, f64)> =
            sqlx::query_as("SELECT vanne, passage, distribue FROM soupe_distribution ORDER BY id").fetch_all(&pool).await.unwrap();
        // Cumul 0 → 16 → 32, puis 13,2 lu faute de nouveau cumul en fin de repas.
        assert_eq!(servies, vec![(1, 1, 16.0), (2, 1, 16.0), (2, 2, 13.2)]);
        let phase: i64 = sqlx::query_scalar("SELECT phase FROM soupe_etat").fetch_one(&pool).await.unwrap();
        assert_eq!(phase, 0);

        // Alarme débit distribution : rien sous 3 min, alerte ensuite, fermée à la reprise.
        let courte = format!("3000\t<\t{}\n3100\t<\t{}\n3101\t<\t{}", trame(6, 64, 0, 35), trame(6, 64, 0, 37), trame(6, 64, 0, 37));
        recevoir(&pool, &courte).await.unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ambiance_alerte WHERE type='soupe_distribution'").fetch_one(&pool).await.unwrap();
        assert_eq!(n, 0);
        let longue = format!("3400\t<\t{}\n3401\t<\t{}", trame(6, 64, 0, 37), trame(6, 64, 0, 35));
        recevoir(&pool, &longue).await.unwrap();
        let (n, ouvertes): (i64, i64) = sqlx::query_as("SELECT COUNT(*), COALESCE(SUM(fin IS NULL),0) FROM ambiance_alerte WHERE type='soupe_distribution'").fetch_one(&pool).await.unwrap();
        assert_eq!((n, ouvertes), (1, 0));
        // Alerte dosage : ouverte une seule fois, fermée à la phase suivante.
        let alerte = format!("2000\t<\t{}\n2001\t<\t{}\n2002\t<\t{}", trame(0, 0, 0, 7), trame(0, 0, 0, 7), trame(0, 0, 0, 1));
        recevoir(&pool, &alerte).await.unwrap();
        let (n, ouvertes): (i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), SUM(fin IS NULL) FROM ambiance_alerte WHERE type='soupe_alerte'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((n, ouvertes), (1, 0));
    }
}
