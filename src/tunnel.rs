//! Accès à distance par tunnel Cloudflare, piloté par EO_Sevel lui-même
//! (intégré) : le jeton du tunnel, collé dans Mes équipements,
//! suffit. EO_Sevel lance `cloudflared`, le relance s'il s'arrête et garde
//! ses dernières lignes de journal pour la page.
//!
//! Réglages dans `parametre` : `cf_token`, `cf_actif` ("1"), `cf_adresse`
//! (adresse publique, pour l'affichage). Le jeton passe par la variable
//! d'environnement TUNNEL_TOKEN, jamais sur la ligne de commande (visible
//! dans la liste des processus).

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use sqlx::SqlitePool;
use tokio::io::{AsyncBufReadExt, BufReader};

pub const BINAIRE: &str = "/usr/local/bin/cloudflared";
const LIGNES_GARDEES: usize = 40;

#[derive(Default, Clone)]
pub struct Etat {
    pub en_marche: bool,
    pub connexions: i64,
    pub depuis: Option<i64>,
    pub redemarrages: i64,
    pub derniere_erreur: Option<String>,
    pub journal: VecDeque<String>,
}

fn etat() -> &'static Mutex<Etat> {
    static ETAT: OnceLock<Mutex<Etat>> = OnceLock::new();
    ETAT.get_or_init(|| Mutex::new(Etat::default()))
}

fn relance() -> &'static Mutex<bool> {
    static R: OnceLock<Mutex<bool>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(false))
}

pub fn instantane() -> Etat {
    etat().lock().map(|e| e.clone()).unwrap_or_default()
}

/// Demande un redémarrage du tunnel (après un changement de réglage).
pub fn redemarrer() {
    if let Ok(mut r) = relance().lock() {
        *r = true;
    }
}

fn noter(ligne: &str) {
    let Ok(mut e) = etat().lock() else { return };
    // Horodatage et niveau de cloudflared : on garde l'essentiel lisible.
    let ligne = ligne.trim();
    if ligne.contains("Registered tunnel connection") {
        e.connexions += 1;
    } else if ligne.contains("Unregistered tunnel connection") || ligne.contains("Connection terminated") {
        e.connexions = (e.connexions - 1).max(0);
    }
    if ligne.contains(" ERR ") {
        e.derniere_erreur = Some(ligne.to_string());
    }
    e.journal.push_back(ligne.to_string());
    while e.journal.len() > LIGNES_GARDEES {
        e.journal.pop_front();
    }
}

async fn parametre(pool: &SqlitePool, cle: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>("SELECT valeur FROM parametre WHERE cle=?")
        .bind(cle)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .flatten()
        .filter(|v| !v.trim().is_empty())
}

async fn voulu(pool: &SqlitePool) -> Option<String> {
    if parametre(pool, "cf_actif").await.as_deref() != Some("1") {
        return None;
    }
    parametre(pool, "cf_token").await.map(|t| t.trim().to_string())
}

/// Surveille les réglages et fait tourner cloudflared en conséquence.
pub async fn boucle(pool: SqlitePool) {
    let mut attente = 5u64;
    loop {
        let Some(jeton) = voulu(&pool).await else {
            tokio::time::sleep(Duration::from_secs(10)).await;
            continue;
        };
        if !std::path::Path::new(BINAIRE).exists() {
            if let Ok(mut e) = etat().lock() {
                e.derniere_erreur = Some(format!("{BINAIRE} absent : cloudflared n'est pas installé sur le serveur"));
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
            continue;
        }
        let enfant = tokio::process::Command::new(BINAIRE)
            .args(["tunnel", "--no-autoupdate", "run"])
            .env("TUNNEL_TOKEN", &jeton)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn();
        let mut enfant = match enfant {
            Ok(e) => e,
            Err(err) => {
                if let Ok(mut e) = etat().lock() {
                    e.derniere_erreur = Some(format!("lancement impossible : {err}"));
                }
                tokio::time::sleep(Duration::from_secs(60)).await;
                continue;
            }
        };
        if let Ok(mut e) = etat().lock() {
            e.en_marche = true;
            e.connexions = 0;
            e.depuis = Some(chrono::Utc::now().timestamp());
        }
        for flux in [enfant.stderr.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
                     enfant.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)]
            .into_iter()
            .flatten()
        {
            tokio::spawn(async move {
                let mut lignes = BufReader::new(flux).lines();
                while let Ok(Some(l)) = lignes.next_line().await {
                    noter(&l);
                }
            });
        }
        let debut = std::time::Instant::now();
        // Tourne tant que le réglage ne change pas et que le processus vit.
        loop {
            tokio::select! {
                statut = enfant.wait() => {
                    noter(&format!("cloudflared arrêté ({})", statut.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string())));
                    break;
                }
                _ = tokio::time::sleep(Duration::from_secs(10)) => {
                    let demande = relance().lock().map(|mut r| std::mem::replace(&mut *r, false)).unwrap_or(false);
                    if demande || voulu(&pool).await.as_deref() != Some(jeton.as_str()) {
                        let _ = enfant.kill().await;
                        noter("cloudflared arrêté (réglages modifiés)");
                        attente = 1;
                        break;
                    }
                }
            }
        }
        if let Ok(mut e) = etat().lock() {
            e.en_marche = false;
            e.connexions = 0;
            e.redemarrages += 1;
        }
        // Un tunnel qui tombe tout de suite (jeton faux) n'est pas relancé en boucle serrée.
        if debut.elapsed() > Duration::from_secs(300) {
            attente = 5;
        } else {
            attente = (attente * 2).min(300);
        }
        tokio::time::sleep(Duration::from_secs(attente)).await;
    }
}
