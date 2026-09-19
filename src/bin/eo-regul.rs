//! Démon de régulation de ventilation d'EO_Sevel.
//!
//! Binaire séparé, volontairement. Il ne partage aucun code avec le serveur
//! web : il lit sa configuration et ses consignes dans la base, il y écrit les
//! mesures et l'état. Conséquence recherchée — on peut redéployer
//! l'application, migrer son schéma ou la faire planter sans que la
//! ventilation s'arrête, et on peut arrêter le démon sans empêcher la
//! consultation des données.
//!
//! Trois garde-fous portent toute la sécurité de l'ensemble :
//!
//! 1. Aucune mesure douteuse n'entre dans le calcul. Absente, figée ou
//!    aberrante, elle fait passer la salle au pourcentage de repli.
//! 2. Le démon écrit un battement de cœur à chaque cycle. L'automate surveille
//!    ce signal de son côté et bascule les salles en repli s'il s'arrête —
//!    c'est ce qui rend acceptable de faire tourner ce démon sur une machine
//!    virtuelle qui peut redémarrer.
//! 3. Une erreur, quelle qu'elle soit, ne fait jamais sortir de la boucle. On
//!    journalise, on compte, on continue.
//!
//! Variables d'environnement :
//!   ELEVAGE_DATA        dossier de la base (défaut « data »)
//!   EO_REGUL_INTERVALLE secondes entre deux cycles (défaut 30)
//!   EO_REGUL_SIMULATION 1 pour calculer et journaliser sans jamais écrire sur
//!                       l'automate — le mode à utiliser pour la phase de
//!                       comparaison en parallèle de l'installation existante.

use anyhow::Result;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

#[path = "../regul_core.rs"]
mod regul_core;

use regul_core::{
    calculer, consigne_courbe, depuis_dixiemes, doit_ecrire, doit_enregistrer, en_dixiemes,
    Decision, Lecture, Mode, PointCourbe, Reglages,
};

/// Gabarits d'accès à l'automate. Ils vivent dans la table `parametre` plutôt
/// qu'en dur : l'API de l'IPX800 évolue avec son firmware, et on ne veut pas
/// recompiler le démon le jour où un chemin change.
struct AccesIpx {
    gabarit_lecture: String,
    gabarit_ecriture: String,
    champ_valeur: String,
    champ_ecriture: String,
    cle: String,
    simulation: bool,
}

const GABARIT_LECTURE_DEFAUT: &str = "http://{hote}/{id}?ApiKey={cle}";
const GABARIT_ECRITURE_DEFAUT: &str = "http://{hote}/{id}?ApiKey={cle}";

/// Configuration de régulation d'une salle, telle que lue en base.
struct SalleRegulee {
    salle_id: i64,
    nom: String,
    pilote: bool,
    ipx_hote: Option<String>,
    entree_t: Option<String>,
    entree_h: Option<String>,
    entree_lux: Option<String>,
    sortie_ventil: Option<String>,
    courbe_id: Option<i64>,
    t_cible_fixe: f64,
    reglages: Reglages,
    t_alerte_min: Option<f64>,
    t_alerte_max: Option<f64>,
    h_alerte_max: Option<f64>,
    deadband_pct: f64,
    relance_s: i64,
}

/// Ce que le démon garde en mémoire d'un cycle à l'autre. Rien d'essentiel :
/// après un redémarrage, la première écriture est simplement forcée.
#[derive(Default)]
struct Memoire {
    derniere_ecriture: Option<f64>,
    ts_derniere_ecriture: i64,
    derniere_mesure: HashMap<String, (f64, i64)>,
    derniere_temperature: Option<(f64, i64)>,
    /// Dernier *changement* observé par grandeur : valeur et horodatage. Une
    /// lecture réussie ne prouve pas qu'une sonde est vivante — une sonde
    /// débranchée ou gelée répond souvent la même chose indéfiniment. C'est
    /// l'immobilité de la valeur qui trahit la panne, pas l'absence de
    /// réponse.
    dernier_changement: HashMap<String, (f64, i64)>,
}

impl Memoire {
    /// Âge de la mesure au sens « depuis combien de temps n'a-t-elle pas
    /// bougé ». Met à jour la référence au passage.
    fn age_mesure(&mut self, grandeur: &str, valeur: Option<f64>, ts: i64) -> Lecture {
        let Some(valeur) = valeur else {
            return Lecture::absente();
        };
        let age = match self.dernier_changement.get(grandeur).copied() {
            Some((precedente, ts_precedent)) if (valeur - precedente).abs() < 0.005 => {
                (ts - ts_precedent).max(0)
            }
            _ => {
                self.dernier_changement
                    .insert(grandeur.to_string(), (valeur, ts));
                0
            }
        };
        Lecture {
            valeur: Some(valeur),
            age_s: age,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "eo_regul=info".into()),
        )
        .init();

    let data_dir = std::env::var_os("ELEVAGE_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data"));
    let db_path = data_dir.join("elevage.db");
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path.display()))?
        .create_if_missing(false)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(10));
    // Deux connexions suffisent et bornent la contention avec le serveur web,
    // qui écrit dans la même base.
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await?;

    let intervalle = std::env::var("EO_REGUL_INTERVALLE")
        .ok()
        .and_then(|valeur| valeur.parse::<u64>().ok())
        .filter(|valeur| *valeur >= 5)
        .unwrap_or(30);
    let simulation = matches!(
        std::env::var("EO_REGUL_SIMULATION")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "oui" | "on"
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    tracing::info!(
        base = %db_path.display(),
        intervalle,
        simulation,
        "EO_Sevel — démon de régulation démarré"
    );
    if simulation {
        tracing::warn!(
            "Mode simulation : les consignes sont calculées et enregistrées, \
             aucune écriture n'est envoyée à l'automate"
        );
    }

    let mut memoire: HashMap<i64, Memoire> = HashMap::new();
    let mut cycles: i64 = 0;
    let mut erreurs: i64 = 0;

    loop {
        cycles += 1;
        match cycle(&pool, &client, &mut memoire, simulation).await {
            Ok(traitees) => {
                tracing::debug!(cycles, traitees, "cycle terminé");
            }
            Err(error) => {
                erreurs += 1;
                // Une erreur de cycle ne doit jamais arrêter la régulation :
                // la salle concernée retombera d'elle-même en repli côté
                // automate si la situation dure.
                tracing::error!(error = %error, cycles, erreurs, "cycle en échec");
            }
        }
        if let Err(error) = battement(&pool, cycles, erreurs).await {
            tracing::error!(error = %error, "battement de cœur non enregistré");
        }
        tokio::time::sleep(Duration::from_secs(intervalle)).await;
    }
}

/// Enregistre le battement de cœur. C'est ce signal que l'automate surveille.
async fn battement(pool: &SqlitePool, cycles: i64, erreurs: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO ambiance_heartbeat(id,ts,version,cycles,erreurs) VALUES(1,?,?,?,?) \
         ON CONFLICT(id) DO UPDATE SET ts=excluded.ts,version=excluded.version,\
         cycles=excluded.cycles,erreurs=excluded.erreurs",
    )
    .bind(maintenant())
    .bind(env!("CARGO_PKG_VERSION"))
    .bind(cycles)
    .bind(erreurs)
    .execute(pool)
    .await?;
    Ok(())
}

fn maintenant() -> i64 {
    chrono::Utc::now().timestamp()
}

async fn parametre(pool: &SqlitePool, cle: &str, defaut: &str) -> String {
    let valeur: Option<Option<String>> =
        sqlx::query_scalar("SELECT valeur FROM parametre WHERE cle=?")
            .bind(cle)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    valeur
        .flatten()
        .map(|texte| texte.trim().to_string())
        .filter(|texte| !texte.is_empty())
        .unwrap_or_else(|| defaut.to_string())
}

async fn acces_ipx(pool: &SqlitePool, simulation: bool) -> AccesIpx {
    AccesIpx {
        gabarit_lecture: parametre(pool, "ambiance_ipx_url_lecture", GABARIT_LECTURE_DEFAUT).await,
        gabarit_ecriture: parametre(pool, "ambiance_ipx_url_ecriture", GABARIT_ECRITURE_DEFAUT)
            .await,
        champ_valeur: parametre(pool, "ambiance_ipx_champ_valeur", "value").await,
        champ_ecriture: parametre(pool, "ambiance_ipx_champ_ecriture", "value").await,
        cle: parametre(pool, "ambiance_ipx_cle", "").await,
        simulation,
    }
}

async fn cycle(
    pool: &SqlitePool,
    client: &reqwest::Client,
    memoire: &mut HashMap<i64, Memoire>,
    simulation: bool,
) -> Result<usize> {
    let acces = acces_ipx(pool, simulation).await;
    let salles = charger_salles(pool).await?;
    let mut traitees = 0usize;
    for salle in &salles {
        if let Err(error) = traiter_salle(pool, client, &acces, salle, memoire).await {
            tracing::error!(salle = %salle.nom, error = %error, "salle non régulée ce cycle");
            let _ = ouvrir_alerte(
                pool,
                salle.salle_id,
                "liaison",
                "alerte",
                &format!("Communication impossible avec l'automate : {error}"),
            )
            .await;
        } else {
            traitees += 1;
        }
    }
    // Les compteurs d'eau et d'électricité raccordés à l'automate n'ont pas
    // besoin de la mécanique d'ambiance : ils alimentent les compteurs déjà
    // suivis par l'application, avec leur répartition de coût par bande.
    if let Err(error) = relever_compteurs(pool, client, &acces).await {
        tracing::error!(error = %error, "relevé des compteurs impossible");
    }
    Ok(traitees)
}

/// Relève une fois par jour les points de type compteur.
///
/// Un index de compteur n'a pas besoin d'être enregistré toutes les trente
/// secondes : ce qui compte est l'écart entre deux relevés. Un par jour, à
/// l'heure choisie, donne exactement la même information pour mille fois moins
/// de lignes — et se compare directement à un relevé fait à la main.
async fn relever_compteurs(
    pool: &SqlitePool,
    client: &reqwest::Client,
    acces: &AccesIpx,
) -> Result<()> {
    let maintenant_local = chrono::Local::now();
    let aujourdhui = maintenant_local.format("%Y-%m-%d").to_string();
    let heure = {
        use chrono::Timelike;
        maintenant_local.hour() as i64
    };

    let lignes = sqlx::query(
        "SELECT id, hote, chemin, nom, coefficient, decalage, compteur_id, releve_heure, \
                dernier_releve \
         FROM ipx_point \
         WHERE actif=1 AND compteur_id IS NOT NULL AND role IN ('compteur_eau','compteur_elec')",
    )
    .fetch_all(pool)
    .await?;

    for ligne in lignes {
        let point_id: i64 = ligne.try_get("id")?;
        let releve_heure: i64 = ligne.try_get("releve_heure").unwrap_or(6);
        let dernier: Option<String> = ligne.try_get("dernier_releve").ok();
        // Déjà relevé aujourd'hui, ou l'heure choisie n'est pas encore passée.
        if dernier.as_deref() == Some(aujourdhui.as_str()) || heure < releve_heure {
            continue;
        }
        let compteur_id: i64 = ligne.try_get("compteur_id")?;
        let hote: String = ligne.try_get("hote").unwrap_or_default();
        let chemin: String = ligne.try_get("chemin").unwrap_or_default();
        let coefficient: f64 = ligne.try_get("coefficient").unwrap_or(1.0);
        let decalage: f64 = ligne.try_get("decalage").unwrap_or(0.0);
        let nom: String = ligne.try_get("nom").unwrap_or_default();

        let lecture = lire(client, acces, &hote, Some(&chemin)).await;
        let Some(brut) = lecture.valeur else {
            tracing::warn!(point = %nom, "compteur illisible, relevé reporté");
            continue;
        };
        let index = brut * coefficient + decalage;

        // Un relevé saisi à la main le même jour l'emporte : on ne vient pas
        // écraser ce que l'éleveur a constaté sur le compteur lui-même.
        let existe: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM releve_compteur WHERE compteur_id=? AND date_releve=?",
        )
        .bind(compteur_id)
        .bind(&aujourdhui)
        .fetch_one(pool)
        .await?;
        if existe == 0 {
            sqlx::query(
                "INSERT INTO releve_compteur(compteur_id,date_releve,valeur_index,note,\
                 remplacement_compteur) VALUES(?,?,?,?,0)",
            )
            .bind(compteur_id)
            .bind(&aujourdhui)
            .bind(index)
            .bind(format!("Relevé automatique depuis l'automate ({nom})"))
            .execute(pool)
            .await?;
            tracing::info!(point = %nom, index, "relevé de compteur enregistré");
        }
        sqlx::query("UPDATE ipx_point SET dernier_releve=? WHERE id=?")
            .bind(&aujourdhui)
            .bind(point_id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

async fn charger_salles(pool: &SqlitePool) -> Result<Vec<SalleRegulee>> {
    let lignes = sqlx::query(
        "SELECT a.salle_id, s.nom, a.pilote, a.ipx_hote, a.entree_t, a.entree_h, a.entree_lux, \
                a.sortie_ventil, a.courbe_id, a.t_cible_fixe, a.plage, a.ventil_min, a.ventil_max, \
                a.hygro_cible, a.hygro_influence, a.anticipation, a.t_plausible_min, \
                a.t_plausible_max, a.h_plausible_min, a.h_plausible_max, a.fraicheur_max_s, \
                a.ventil_securite, a.t_alerte_min, a.t_alerte_max, a.h_alerte_max, \
                a.deadband_pct, a.relance_s \
         FROM ambiance_salle a JOIN salle s ON s.id=a.salle_id \
         WHERE a.actif=1 ORDER BY s.ordre, s.nom",
    )
    .fetch_all(pool)
    .await?;

    let mut salles = Vec::with_capacity(lignes.len());
    for ligne in lignes {
        salles.push(SalleRegulee {
            salle_id: ligne.try_get("salle_id")?,
            nom: ligne.try_get("nom")?,
            pilote: ligne.try_get::<i64, _>("pilote").unwrap_or(0) != 0,
            ipx_hote: ligne.try_get("ipx_hote").ok(),
            entree_t: ligne.try_get("entree_t").ok(),
            entree_h: ligne.try_get("entree_h").ok(),
            entree_lux: ligne.try_get("entree_lux").ok(),
            sortie_ventil: ligne.try_get("sortie_ventil").ok(),
            courbe_id: ligne.try_get("courbe_id").ok(),
            t_cible_fixe: ligne.try_get("t_cible_fixe")?,
            reglages: Reglages {
                plage: ligne.try_get("plage")?,
                ventil_min: ligne.try_get("ventil_min")?,
                ventil_max: ligne.try_get("ventil_max")?,
                hygro_cible: ligne.try_get("hygro_cible").ok(),
                hygro_influence: ligne.try_get("hygro_influence")?,
                anticipation: ligne.try_get("anticipation")?,
                t_plausible_min: ligne.try_get("t_plausible_min")?,
                t_plausible_max: ligne.try_get("t_plausible_max")?,
                h_plausible_min: ligne.try_get("h_plausible_min")?,
                h_plausible_max: ligne.try_get("h_plausible_max")?,
                fraicheur_max_s: ligne.try_get("fraicheur_max_s")?,
                ventil_securite: ligne.try_get("ventil_securite")?,
            },
            t_alerte_min: ligne.try_get("t_alerte_min").ok(),
            t_alerte_max: ligne.try_get("t_alerte_max").ok(),
            h_alerte_max: ligne.try_get("h_alerte_max").ok(),
            deadband_pct: ligne.try_get("deadband_pct")?,
            relance_s: ligne.try_get("relance_s")?,
        });
    }
    Ok(salles)
}

/// Bande présente dans la salle et son nombre de jours de présence.
///
/// La source de vérité est le dernier transfert enregistré vers cette salle.
/// C'est exactement ce qu'aucun automate ne peut savoir seul, et c'est ce qui
/// permet à la consigne de descendre toute seule au fil du lot.
async fn bande_en_place(pool: &SqlitePool, salle_id: i64) -> Result<Option<(i64, i64)>> {
    let ligne = sqlx::query(
        "SELECT t.bande_id, CAST(julianday('now') - julianday(t.date) AS INTEGER) AS jours \
         FROM transfert t JOIN bande b ON b.id=t.bande_id \
         WHERE t.salle_dest_id=? AND t.bande_id IS NOT NULL AND t.date IS NOT NULL AND b.active=1 \
         ORDER BY t.date DESC, t.id DESC LIMIT 1",
    )
    .bind(salle_id)
    .fetch_optional(pool)
    .await?;
    let Some(ligne) = ligne else {
        return Ok(None);
    };
    let bande_id: i64 = ligne.try_get("bande_id")?;
    let jours: i64 = ligne.try_get("jours").unwrap_or(0);
    Ok(Some((bande_id, jours.max(0))))
}

/// Bande présente dans la salle, via la date de mise-bas réelle des truies
/// qui s'y trouvent, quand aucun transfert de bande n'existe pour cette
/// salle.
///
/// En maternité, les truies entrent une par une (mouvement individuel) —
/// aucun `transfert` avec `bande_id` renseigné n'est jamais créé pour ce
/// type de salle, donc `bande_en_place` n'y trouve jamais rien. La date de
/// mise-bas, elle, est toujours connue avec précision et par truie : c'est
/// le vrai jour 0 d'une maternité, tel que l'éleveur le vit.
async fn bande_via_mise_bas(pool: &SqlitePool, salle_id: i64) -> Result<Option<(i64, i64)>> {
    let ligne = sqlx::query(
        "SELECT e.bande_id, CAST(julianday('now') - julianday(e.date) AS INTEGER) AS jours \
         FROM evenement e JOIN truie t ON t.id=e.truie_id JOIN bande b ON b.id=e.bande_id \
         WHERE e.type='mise_bas' AND t.salle_id=? AND t.reformee=0 AND e.bande_id IS NOT NULL \
         AND b.active=1 ORDER BY e.date DESC, e.id DESC LIMIT 1",
    )
    .bind(salle_id)
    .fetch_optional(pool)
    .await?;
    let Some(ligne) = ligne else {
        return Ok(None);
    };
    let bande_id: i64 = ligne.try_get("bande_id")?;
    let jours: i64 = ligne.try_get("jours").unwrap_or(0);
    Ok(Some((bande_id, jours.max(0))))
}

/// Marge après l'heure d'arrivée prévue au-delà de laquelle le
/// préconditionnement n'a plus d'effet : une arrivée qui glisse d'un jour ne
/// doit pas laisser une salle vide chauffée indéfiniment.
const PRECONDITIONNEMENT_GRACE_H: f64 = 48.0;

/// Bande attendue dans une salle encore vide, si le préconditionnement en
/// cours le justifie.
///
/// N'est consulté que si `bande_en_place` n'a rien trouvé : un vrai transfert
/// enregistré a toujours priorité sur une simple hypothèse de planification.
/// Renvoie jour 0 pendant toute la fenêtre de préchauffage — la salle se
/// comporte comme si la bande venait d'arriver, jusqu'à l'arrivée réelle.
async fn preconditionnement_en_attente(
    pool: &SqlitePool,
    salle_id: i64,
) -> Result<Option<(i64, i64)>> {
    let ligne = sqlx::query(
        "SELECT bande_id FROM ambiance_preconditionnement \
         WHERE salle_id=? AND bande_id IS NOT NULL \
         AND julianday('now') >= julianday(arrivee_prevue) - heures_avant/24.0 \
         AND julianday('now') < julianday(arrivee_prevue) + ?/24.0",
    )
    .bind(salle_id)
    .bind(PRECONDITIONNEMENT_GRACE_H)
    .fetch_optional(pool)
    .await?;
    let Some(ligne) = ligne else {
        return Ok(None);
    };
    Ok(Some((ligne.try_get("bande_id")?, 0)))
}

async fn points_courbe(pool: &SqlitePool, courbe_id: i64) -> Result<Vec<PointCourbe>> {
    let lignes = sqlx::query(
        "SELECT jour,t_cible,ventil_min,ventil_max FROM ambiance_courbe_point \
         WHERE courbe_id=? ORDER BY jour",
    )
    .bind(courbe_id)
    .fetch_all(pool)
    .await?;
    Ok(lignes
        .into_iter()
        .map(|ligne| PointCourbe {
            jour: ligne.try_get("jour").unwrap_or(0),
            t_cible: ligne.try_get("t_cible").unwrap_or(20.0),
            ventil_min: ligne.try_get("ventil_min").ok(),
            ventil_max: ligne.try_get("ventil_max").ok(),
        })
        .collect())
}

async fn traiter_salle(
    pool: &SqlitePool,
    client: &reqwest::Client,
    acces: &AccesIpx,
    salle: &SalleRegulee,
    memoire: &mut HashMap<i64, Memoire>,
) -> Result<()> {
    let ts = maintenant();
    let hote = salle.ipx_hote.clone().unwrap_or_default();

    let brut_t = lire(client, acces, &hote, salle.entree_t.as_deref()).await;
    let brut_h = lire(client, acces, &hote, salle.entree_h.as_deref()).await;
    let brut_lux = lire(client, acces, &hote, salle.entree_lux.as_deref()).await;

    // Consigne : la courbe liée à la bande en place l'emporte sur la valeur
    // fixe. Sans bande identifiée, on retombe sur la valeur fixe plutôt que de
    // deviner — une salle vide n'a pas d'âge. Trois sources, dans cet ordre
    // de priorité : un vrai transfert de bande vers la salle ; à défaut, la
    // date de mise-bas des truies qui s'y trouvent (le cas des maternités,
    // où les entrées sont individuelles et ne créent jamais de transfert de
    // bande) ; à défaut, un préconditionnement en cours (arrivée prévue
    // proche, voir 0013) pour démarrer le préchauffage au jour 0.
    let bande = match bande_en_place(pool, salle.salle_id).await.unwrap_or(None) {
        Some(bande) => Some(bande),
        None => match bande_via_mise_bas(pool, salle.salle_id)
            .await
            .unwrap_or(None)
        {
            Some(bande) => Some(bande),
            None => preconditionnement_en_attente(pool, salle.salle_id)
                .await
                .unwrap_or(None),
        },
    };
    let mut reglages = salle.reglages.clone();
    let mut consigne = salle.t_cible_fixe;
    if let (Some(courbe_id), Some((_, jours))) = (salle.courbe_id, bande) {
        if let Ok(points) = points_courbe(pool, courbe_id).await {
            if let Some(valeur) = consigne_courbe(&points, jours) {
                consigne = valeur.t_cible;
                if let Some(mini) = valeur.ventil_min {
                    reglages.ventil_min = mini;
                }
                if let Some(maxi) = valeur.ventil_max {
                    reglages.ventil_max = maxi;
                }
            }
        }
    }

    let etat = memoire.entry(salle.salle_id).or_default();

    // L'âge d'une mesure se compte depuis son dernier changement, pas depuis
    // la dernière requête réussie.
    let temperature = etat.age_mesure("t", brut_t.valeur, ts);
    let humidite = etat.age_mesure("h", brut_h.valeur, ts);
    let luminosite = brut_lux;

    // Gradient en °C/h, calculé sur la mesure précédente gardée en mémoire.
    let gradient = match (etat.derniere_temperature, temperature.valeur) {
        (Some((precedente, ts_precedent)), Some(actuelle)) if ts > ts_precedent => {
            let heures = (ts - ts_precedent) as f64 / 3600.0;
            if heures > 0.0 {
                ((actuelle - precedente) / heures).clamp(-20.0, 20.0)
            } else {
                0.0
            }
        }
        _ => 0.0,
    };
    if let Some(valeur) = temperature.valeur {
        etat.derniere_temperature = Some((valeur, ts));
    }

    let decision = calculer(&reglages, consigne, temperature, humidite, gradient);

    // Écriture sur l'automate : seulement si la salle est déclarée pilotée, si
    // la consigne a bougé assez, et jamais en mode simulation.
    let mut applique = false;
    let mut echec_ecriture: Option<String> = None;
    if salle.pilote && !acces.simulation {
        if let Some(sortie) = salle.sortie_ventil.as_deref() {
            let age = if etat.ts_derniere_ecriture == 0 {
                i64::MAX
            } else {
                ts - etat.ts_derniere_ecriture
            };
            if doit_ecrire(
                etat.derniere_ecriture,
                decision.ventil,
                salle.deadband_pct,
                age,
                salle.relance_s,
            ) {
                // Une écriture qui échoue ne doit pas faire perdre la mesure du
                // cycle : on note l'échec, on archive quand même, et on lève
                // l'alerte de liaison plus bas. Sortir ici reviendrait à créer
                // un trou dans l'historique au moment précis où il compte.
                match ecrire(client, acces, &hote, sortie, decision.ventil).await {
                    Ok(()) => {
                        etat.derniere_ecriture = Some(decision.ventil);
                        etat.ts_derniere_ecriture = ts;
                        applique = true;
                    }
                    Err(error) => {
                        echec_ecriture = Some(error.to_string());
                    }
                }
            }
        }
    }
    if applique {
        tracing::info!(
            salle = %salle.nom,
            consigne = format!("{consigne:.1}"),
            ventil = format!("{:.1}", decision.ventil),
            mode = decision.mode.code(),
            "consigne appliquée"
        );
    }

    enregistrer_mesures(
        pool,
        salle,
        etat,
        ts,
        bande.map(|(id, _)| id),
        temperature.valeur,
        humidite.valeur,
        luminosite.valeur,
        &decision,
        consigne,
    )
    .await?;

    ecrire_etat(
        pool,
        salle,
        ts,
        temperature.valeur,
        humidite.valeur,
        luminosite.valeur,
        consigne,
        &decision,
        bande,
        acces.simulation,
    )
    .await?;

    surveiller(pool, salle, temperature.valeur, humidite.valeur, &decision).await;

    if let Some(erreur) = echec_ecriture {
        // Signalé après l'archivage, et remonté comme une erreur de cycle : la
        // salle continue de calculer, mais l'automate n'a pas reçu la consigne.
        ouvrir_alerte(
            pool,
            salle.salle_id,
            "liaison",
            "critique",
            &format!("Consigne non transmise à l'automate : {erreur}"),
        )
        .await?;
        anyhow::bail!("écriture de la consigne impossible : {erreur}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn enregistrer_mesures(
    pool: &SqlitePool,
    salle: &SalleRegulee,
    etat: &mut Memoire,
    ts: i64,
    bande_id: Option<i64>,
    temperature: Option<f64>,
    humidite: Option<f64>,
    luminosite: Option<f64>,
    decision: &Decision,
    consigne: f64,
) -> Result<()> {
    // Seuils d'enregistrement par grandeur : une température au dixième, une
    // hygrométrie au demi-point, la lumière et la ventilation au point. Avec
    // l'écriture forcée toutes les dix minutes, les courbes restent continues
    // sans stocker des millions de points identiques.
    let grandeurs: [(&str, Option<f64>, f64); 5] = [
        ("t", temperature, 0.1),
        ("h", humidite, 0.5),
        ("lux", luminosite, 1.0),
        ("ventil", Some(decision.ventil), 1.0),
        ("consigne", Some(consigne), 0.1),
    ];
    for (grandeur, valeur, deadband) in grandeurs {
        let Some(valeur) = valeur else { continue };
        if !valeur.is_finite() {
            continue;
        }
        let precedente = etat.derniere_mesure.get(grandeur).copied();
        let age = precedente
            .map(|(_, ts_prec)| ts - ts_prec)
            .unwrap_or(i64::MAX);
        if !doit_enregistrer(
            precedente.map(|(valeur, _)| valeur),
            valeur,
            deadband,
            age,
            600,
        ) {
            continue;
        }
        sqlx::query(
            "INSERT INTO ambiance_mesure(salle_id,grandeur,ts,valeur,bande_id) \
             VALUES(?,?,?,?,?) ON CONFLICT(salle_id,grandeur,ts) DO UPDATE \
             SET valeur=excluded.valeur,bande_id=excluded.bande_id",
        )
        .bind(salle.salle_id)
        .bind(grandeur)
        .bind(ts)
        .bind(en_dixiemes(valeur))
        .bind(bande_id)
        .execute(pool)
        .await?;
        etat.derniere_mesure
            .insert(grandeur.to_string(), (valeur, ts));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn ecrire_etat(
    pool: &SqlitePool,
    salle: &SalleRegulee,
    ts: i64,
    temperature: Option<f64>,
    humidite: Option<f64>,
    luminosite: Option<f64>,
    consigne: f64,
    decision: &Decision,
    bande: Option<(i64, i64)>,
    simulation: bool,
) -> Result<()> {
    let mode = if simulation {
        "simulation"
    } else if !salle.pilote {
        "observation"
    } else {
        decision.mode.code()
    };
    sqlx::query(
        "INSERT INTO ambiance_etat(salle_id,ts,t,h,lux,consigne,ventil,bande_id,jour_bande,mode,sonde_ok,message) \
         VALUES(?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(salle_id) DO UPDATE SET \
         ts=excluded.ts,t=excluded.t,h=excluded.h,lux=excluded.lux,consigne=excluded.consigne,\
         ventil=excluded.ventil,bande_id=excluded.bande_id,jour_bande=excluded.jour_bande,\
         mode=excluded.mode,sonde_ok=excluded.sonde_ok,message=excluded.message",
    )
    .bind(salle.salle_id)
    .bind(ts)
    .bind(temperature)
    .bind(humidite)
    .bind(luminosite)
    .bind(consigne)
    .bind(decision.ventil)
    .bind(bande.map(|(id, _)| id))
    .bind(bande.map(|(_, jours)| jours))
    .bind(mode)
    .bind(i64::from(decision.sonde_ok))
    .bind(decision.message.clone())
    .execute(pool)
    .await?;
    Ok(())
}

/// Alertes d'ambiance. Une alerte du même type reste ouverte tant que la cause
/// dure : on ne recrée pas une ligne à chaque cycle, sinon la table devient
/// illisible au moment précis où elle servirait.
async fn surveiller(
    pool: &SqlitePool,
    salle: &SalleRegulee,
    temperature: Option<f64>,
    humidite: Option<f64>,
    decision: &Decision,
) {
    if decision.mode == Mode::Securite {
        let _ = ouvrir_alerte(
            pool,
            salle.salle_id,
            "sonde",
            "critique",
            decision
                .message
                .as_deref()
                .unwrap_or("Mesure inutilisable, ventilation en repli de sécurité"),
        )
        .await;
    } else {
        let _ = fermer_alerte(pool, salle.salle_id, "sonde").await;
        let _ = fermer_alerte(pool, salle.salle_id, "liaison").await;
    }

    match (temperature, salle.t_alerte_max) {
        (Some(valeur), Some(seuil)) if valeur >= seuil => {
            let _ = ouvrir_alerte(
                pool,
                salle.salle_id,
                "t_haute",
                "critique",
                &format!("Température haute : {valeur:.1} °C (seuil {seuil:.1} °C)"),
            )
            .await;
        }
        _ => {
            let _ = fermer_alerte(pool, salle.salle_id, "t_haute").await;
        }
    }
    match (temperature, salle.t_alerte_min) {
        (Some(valeur), Some(seuil)) if valeur <= seuil => {
            let _ = ouvrir_alerte(
                pool,
                salle.salle_id,
                "t_basse",
                "critique",
                &format!("Température basse : {valeur:.1} °C (seuil {seuil:.1} °C)"),
            )
            .await;
        }
        _ => {
            let _ = fermer_alerte(pool, salle.salle_id, "t_basse").await;
        }
    }
    match (humidite, salle.h_alerte_max) {
        (Some(valeur), Some(seuil)) if valeur >= seuil => {
            let _ = ouvrir_alerte(
                pool,
                salle.salle_id,
                "h_haute",
                "alerte",
                &format!("Hygrométrie haute : {valeur:.0} % (seuil {seuil:.0} %)"),
            )
            .await;
        }
        _ => {
            let _ = fermer_alerte(pool, salle.salle_id, "h_haute").await;
        }
    }

    // Ventilation à fond depuis un moment sans que la température redescende :
    // c'est la signature d'un ventilateur arrêté, d'un gradateur mort ou d'une
    // trappe bloquée. Aucun capteur ne le dirait — seul l'effet le trahit.
    let _ = detecter_ventilation_inefficace(pool, salle, temperature, decision).await;
}

async fn detecter_ventilation_inefficace(
    pool: &SqlitePool,
    salle: &SalleRegulee,
    temperature: Option<f64>,
    decision: &Decision,
) -> Result<()> {
    let Some(actuelle) = temperature else {
        return Ok(());
    };
    if decision.mode != Mode::Auto || decision.ventil < 95.0 {
        let _ = fermer_alerte(pool, salle.salle_id, "ventilation_inefficace").await;
        return Ok(());
    }
    let depuis = maintenant() - 20 * 60;
    let ligne = sqlx::query(
        "SELECT MIN(valeur) AS mini_ventil FROM ambiance_mesure \
         WHERE salle_id=? AND grandeur='ventil' AND ts>=?",
    )
    .bind(salle.salle_id)
    .bind(depuis)
    .fetch_optional(pool)
    .await?;
    let mini_ventil: Option<i64> = ligne.and_then(|ligne| ligne.try_get("mini_ventil").ok());
    // Il faut que la commande soit restée à fond sur toute la fenêtre, sinon
    // on observe un simple à-coup de régulation — ou une charge de chaleur
    // que la ventilation n'a pas encore fini de compenser — pas une panne.
    let Some(mini_ventil) = mini_ventil else {
        return Ok(());
    };
    if depuis_dixiemes(mini_ventil) < 95.0 {
        return Ok(());
    }
    let ligne = sqlx::query(
        "SELECT valeur FROM ambiance_mesure WHERE salle_id=? AND grandeur='t' AND ts>=? \
         ORDER BY ts LIMIT 1",
    )
    .bind(salle.salle_id)
    .bind(depuis)
    .fetch_optional(pool)
    .await?;
    let Some(ligne) = ligne else {
        return Ok(());
    };
    let ancienne = depuis_dixiemes(ligne.try_get::<i64, _>("valeur").unwrap_or(0));
    if actuelle > ancienne - 0.5 {
        let _ = ouvrir_alerte(
            pool,
            salle.salle_id,
            "ventilation_inefficace",
            "critique",
            &format!(
                "Ventilation commandée à {:.0} % depuis 20 min sans effet \
                 ({ancienne:.1} °C → {actuelle:.1} °C) : vérifier ventilateur, \
                 gradateur et trappes",
                decision.ventil
            ),
        )
        .await;
    } else {
        let _ = fermer_alerte(pool, salle.salle_id, "ventilation_inefficace").await;
    }
    Ok(())
}

async fn ouvrir_alerte(
    pool: &SqlitePool,
    salle_id: i64,
    type_alerte: &str,
    gravite: &str,
    message: &str,
) -> Result<()> {
    let ouverte: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ambiance_alerte WHERE salle_id=? AND type=? AND fin IS NULL",
    )
    .bind(salle_id)
    .bind(type_alerte)
    .fetch_one(pool)
    .await?;
    if ouverte > 0 {
        sqlx::query(
            "UPDATE ambiance_alerte SET message=? WHERE salle_id=? AND type=? AND fin IS NULL",
        )
        .bind(message)
        .bind(salle_id)
        .bind(type_alerte)
        .execute(pool)
        .await?;
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO ambiance_alerte(salle_id,type,gravite,debut,message) \
         VALUES(?,?,?,CURRENT_TIMESTAMP,?)",
    )
    .bind(salle_id)
    .bind(type_alerte)
    .bind(gravite)
    .bind(message)
    .execute(pool)
    .await?;
    tracing::warn!(salle_id, type_alerte, message, "alerte d'ambiance ouverte");
    Ok(())
}

async fn fermer_alerte(pool: &SqlitePool, salle_id: i64, type_alerte: &str) -> Result<()> {
    sqlx::query(
        "UPDATE ambiance_alerte SET fin=CURRENT_TIMESTAMP \
         WHERE salle_id=? AND type=? AND fin IS NULL",
    )
    .bind(salle_id)
    .bind(type_alerte)
    .execute(pool)
    .await?;
    Ok(())
}

fn url(gabarit: &str, hote: &str, id: &str, cle: &str) -> String {
    gabarit
        .replace("{hote}", hote)
        .replace("{id}", id)
        .replace("{cle}", cle)
}

/// Lecture d'une entrée. Toute erreur se traduit par une absence de mesure —
/// jamais par une valeur par défaut, qui serait indiscernable d'une vraie
/// lecture et pourrait faire réguler dans le vide.
async fn lire(client: &reqwest::Client, acces: &AccesIpx, hote: &str, id: Option<&str>) -> Lecture {
    let Some(id) = id.filter(|valeur| !valeur.trim().is_empty()) else {
        return Lecture::absente();
    };
    if hote.trim().is_empty() {
        return Lecture::absente();
    }
    let adresse = url(&acces.gabarit_lecture, hote, id.trim(), &acces.cle);
    let reponse = match client.get(&adresse).send().await {
        Ok(reponse) => reponse,
        Err(error) => {
            tracing::debug!(adresse, error = %error, "lecture impossible");
            return Lecture::absente();
        }
    };
    if !reponse.status().is_success() {
        tracing::debug!(adresse, statut = %reponse.status(), "lecture refusée");
        return Lecture::absente();
    }
    let corps: serde_json::Value = match reponse.json().await {
        Ok(corps) => corps,
        Err(error) => {
            tracing::debug!(adresse, error = %error, "réponse illisible");
            return Lecture::absente();
        }
    };
    match extraire_nombre(&corps, &acces.champ_valeur) {
        Some(valeur) => Lecture::neuve(valeur),
        None => {
            tracing::debug!(adresse, "aucune valeur numérique trouvée dans la réponse");
            Lecture::absente()
        }
    }
}

/// Cherche le champ attendu dans la réponse, à n'importe quelle profondeur, et
/// se rabat sur le premier nombre rencontré. L'API de l'automate a déjà changé
/// de forme entre deux firmwares ; mieux vaut une extraction tolérante qu'un
/// démon à recompiler.
fn extraire_nombre(valeur: &serde_json::Value, champ: &str) -> Option<f64> {
    fn en_nombre(valeur: &serde_json::Value) -> Option<f64> {
        match valeur {
            serde_json::Value::Number(nombre) => nombre.as_f64(),
            serde_json::Value::String(texte) => texte.trim().replace(',', ".").parse().ok(),
            serde_json::Value::Bool(vrai) => Some(if *vrai { 1.0 } else { 0.0 }),
            _ => None,
        }
    }
    match valeur {
        serde_json::Value::Object(table) => {
            if let Some(trouve) = table.get(champ).and_then(en_nombre) {
                return Some(trouve);
            }
            for imbrique in table.values() {
                if let Some(trouve) = extraire_nombre(imbrique, champ) {
                    return Some(trouve);
                }
            }
            None
        }
        serde_json::Value::Array(elements) => {
            for element in elements {
                if let Some(trouve) = extraire_nombre(element, champ) {
                    return Some(trouve);
                }
            }
            None
        }
        autre => en_nombre(autre),
    }
}

async fn ecrire(
    client: &reqwest::Client,
    acces: &AccesIpx,
    hote: &str,
    id: &str,
    valeur: f64,
) -> Result<()> {
    let adresse = url(&acces.gabarit_ecriture, hote, id.trim(), &acces.cle);
    let mut objet = serde_json::Map::new();
    objet.insert(
        acces.champ_ecriture.clone(),
        serde_json::Value::from(valeur),
    );
    let corps = serde_json::Value::Object(objet);
    let reponse = client.put(&adresse).json(&corps).send().await?;
    anyhow::ensure!(
        reponse.status().is_success(),
        "écriture refusée par l'automate (statut {})",
        reponse.status()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lurl_remplace_les_trois_jetons() {
        let resultat = url(
            "http://{hote}/api/core/{id}?key={cle}",
            "192.168.1.250",
            "ana12",
            "SECRET",
        );
        assert_eq!(resultat, "http://192.168.1.250/api/core/ana12?key=SECRET");
    }

    #[test]
    fn on_extrait_le_champ_attendu_meme_imbrique() {
        let corps = serde_json::json!({"data": {"ana12": {"value": 24.7}}});
        assert_eq!(extraire_nombre(&corps, "value"), Some(24.7));
    }

    #[test]
    fn on_accepte_un_nombre_transmis_en_texte() {
        let corps = serde_json::json!({"value": "24,7"});
        assert_eq!(extraire_nombre(&corps, "value"), Some(24.7));
    }

    #[test]
    fn on_se_rabat_sur_le_premier_nombre_si_le_champ_manque() {
        let corps = serde_json::json!({"autreChamp": 18.25});
        assert_eq!(extraire_nombre(&corps, "value"), Some(18.25));
    }

    #[test]
    fn un_corps_sans_nombre_ne_donne_pas_de_mesure() {
        let corps = serde_json::json!({"etat": "ok", "detail": null});
        assert_eq!(extraire_nombre(&corps, "value"), None);
    }
}

#[cfg(test)]
mod bande_via_mise_bas_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// Situation réelle qui a révélé le besoin de ce repli : une salle de
    /// maternité où les truies entrent une par une, sans jamais créer de
    /// `transfert` avec `bande_id` renseigné.
    #[tokio::test]
    async fn mise_bas_recente_donne_le_jour_de_presence_sans_transfert() -> Result<()> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::raw_sql(include_str!("../../migrations/0001_schema.sql"))
            .execute(&pool)
            .await?;
        let site = sqlx::query("INSERT INTO site(code,nom) VALUES('S1','Site test')")
            .execute(&pool)
            .await?
            .last_insert_rowid();
        let salle = sqlx::query(
            "INSERT INTO salle(site_id,nom,type,nb_cases,ordre) VALUES(?,'Mater 1','Maternité',1,1)",
        )
        .bind(site)
        .execute(&pool)
        .await?
        .last_insert_rowid();
        let bande =
            sqlx::query("INSERT INTO bande(code,date_mb,active) VALUES('BTEST','2026-08-01',1)")
                .execute(&pool)
                .await?
                .last_insert_rowid();
        let truie = sqlx::query(
            "INSERT INTO truie(num_travail,bande_code,statut,reformee,salle_id) VALUES('T1','BTEST','active',0,?)",
        )
        .bind(salle)
        .execute(&pool)
        .await?
        .last_insert_rowid();
        // Aucun transfert de bande créé : uniquement l'événement mise-bas,
        // daté il y a trois jours.
        sqlx::query("INSERT INTO evenement(type,date,truie_id,bande_id,nes_vifs) VALUES('mise_bas',date('now','-3 days'),?,?,12)")
            .bind(truie)
            .bind(bande)
            .execute(&pool)
            .await?;

        assert!(bande_en_place(&pool, salle).await?.is_none());
        let (trouve, jours) = bande_via_mise_bas(&pool, salle)
            .await?
            .expect("la mise-bas doit être trouvée");
        assert_eq!(trouve, bande);
        assert_eq!(jours, 3);
        Ok(())
    }

    /// Une truie réformée (sortie du cheptel) ne doit plus servir de
    /// référence : la salle est censée en accueillir une autre.
    #[tokio::test]
    async fn une_truie_reformee_nest_plus_prise_en_compte() -> Result<()> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::raw_sql(include_str!("../../migrations/0001_schema.sql"))
            .execute(&pool)
            .await?;
        let site = sqlx::query("INSERT INTO site(code,nom) VALUES('S1','Site test')")
            .execute(&pool)
            .await?
            .last_insert_rowid();
        let salle = sqlx::query(
            "INSERT INTO salle(site_id,nom,type,nb_cases,ordre) VALUES(?,'Mater 1','Maternité',1,1)",
        )
        .bind(site)
        .execute(&pool)
        .await?
        .last_insert_rowid();
        let bande =
            sqlx::query("INSERT INTO bande(code,date_mb,active) VALUES('BTEST','2026-08-01',1)")
                .execute(&pool)
                .await?
                .last_insert_rowid();
        let truie = sqlx::query(
            "INSERT INTO truie(num_travail,bande_code,statut,reformee,salle_id) VALUES('T1','BTEST','reformee',1,?)",
        )
        .bind(salle)
        .execute(&pool)
        .await?
        .last_insert_rowid();
        sqlx::query("INSERT INTO evenement(type,date,truie_id,bande_id,nes_vifs) VALUES('mise_bas',date('now','-3 days'),?,?,12)")
            .bind(truie)
            .bind(bande)
            .execute(&pool)
            .await?;

        assert!(bande_via_mise_bas(&pool, salle).await?.is_none());
        Ok(())
    }
}

#[cfg(test)]
mod preconditionnement_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool_avec_salle_et_bande() -> Result<(SqlitePool, i64, i64)> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::raw_sql(include_str!("../../migrations/0001_schema.sql"))
            .execute(&pool)
            .await?;
        sqlx::raw_sql(include_str!("../../migrations/0013_preconditionnement.sql"))
            .execute(&pool)
            .await?;
        let site = sqlx::query("INSERT INTO site(code,nom) VALUES('S1','Site test')")
            .execute(&pool)
            .await?
            .last_insert_rowid();
        let salle = sqlx::query(
            "INSERT INTO salle(site_id,nom,type,nb_cases,ordre) VALUES(?,'PS 1','Post-sevrage',1,1)",
        )
        .bind(site)
        .execute(&pool)
        .await?
        .last_insert_rowid();
        let bande =
            sqlx::query("INSERT INTO bande(code,date_mb,active) VALUES('BTEST','2026-08-01',1)")
                .execute(&pool)
                .await?
                .last_insert_rowid();
        Ok((pool, salle, bande))
    }

    async fn preconditionner(pool: &SqlitePool, salle: i64, bande: i64, decalage_h: f64) {
        sqlx::query(
            "INSERT INTO ambiance_preconditionnement(salle_id,bande_id,arrivee_prevue,heures_avant) \
             VALUES(?,?,datetime('now',? || ' hours'),8)",
        )
        .bind(salle)
        .bind(bande)
        .bind(decalage_h.to_string())
        .execute(pool)
        .await
        .unwrap();
    }

    /// Une arrivée prévue dans 4h, avec 8h de préchauffage : on est déjà dans
    /// la fenêtre, jour 0 doit s'appliquer.
    #[tokio::test]
    async fn preconditionnement_actif_dans_la_fenetre_de_prechauffage() -> Result<()> {
        let (pool, salle, bande) = pool_avec_salle_et_bande().await?;
        preconditionner(&pool, salle, bande, 4.0).await;
        let resultat = preconditionnement_en_attente(&pool, salle).await?;
        assert_eq!(resultat, Some((bande, 0)));
        Ok(())
    }

    /// Une arrivée prévue dans 20h, avec seulement 8h de préchauffage
    /// annoncées : trop tôt, aucun effet encore.
    #[tokio::test]
    async fn preconditionnement_inactif_trop_tot() -> Result<()> {
        let (pool, salle, bande) = pool_avec_salle_et_bande().await?;
        preconditionner(&pool, salle, bande, 20.0).await;
        let resultat = preconditionnement_en_attente(&pool, salle).await?;
        assert_eq!(resultat, None);
        Ok(())
    }

    /// Une arrivée prévue il y a 3 jours, jamais suivie d'un vrai transfert :
    /// passé la marge de grâce, le préconditionnement ne chauffe plus une
    /// salle vide indéfiniment.
    #[tokio::test]
    async fn preconditionnement_expire_sans_arrivee_reelle() -> Result<()> {
        let (pool, salle, bande) = pool_avec_salle_et_bande().await?;
        preconditionner(&pool, salle, bande, -72.0).await;
        let resultat = preconditionnement_en_attente(&pool, salle).await?;
        assert_eq!(resultat, None);
        Ok(())
    }

    /// Un vrai transfert déjà enregistré doit toujours l'emporter sur une
    /// simple hypothèse de préconditionnement.
    #[tokio::test]
    async fn le_vrai_transfert_reste_prioritaire_sur_le_preconditionnement() -> Result<()> {
        let (pool, salle, bande) = pool_avec_salle_et_bande().await?;
        preconditionner(&pool, salle, bande, 4.0).await;
        sqlx::query(
            "INSERT INTO transfert(date,espece,bande_id,salle_dest_id,nombre) \
             VALUES(date('now'),'porc',?,?,10)",
        )
        .bind(bande)
        .bind(salle)
        .execute(&pool)
        .await?;
        assert!(bande_en_place(&pool, salle).await?.is_some());
        Ok(())
    }
}
