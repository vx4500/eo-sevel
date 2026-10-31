#![recursion_limit = "256"]
mod affectation;
mod ambiance;
mod auth;
mod cleanup;
mod config;
mod cours;
mod db;
mod demo;
mod eclairage_core;
mod demo_portal;
mod economic_import;
mod error;
mod formule;
mod gs1;
mod mdns;
mod models;
mod regul_core;
mod routes;
mod proxmox;
mod assistant;
mod sms_free;
mod soupe;
mod tunnel;
mod templates;

use axum::middleware;
use config::Config;
use routes::AppState;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use std::str::FromStr;
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "eo_sevel=info,tower_http=info".into()),
        )
        .init();

    let config = Config::from_env()?;
    if demo_portal::enabled() && !demo_portal::public() && !config.db_path.exists() {
        anyhow::ensure!(
            std::env::var("EO_DEMO_ADMIN_PASSWORD")
                .unwrap_or_default()
                .len()
                >= 16,
            "Mot de passe administrateur de démonstration requis (16 caractères minimum)."
        );
    }
    let db_url = format!("sqlite://{}", config.db_path.display());
    let options = SqliteConnectOptions::from_str(&db_url)?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await?;

    if demo_portal::enabled() {
        demo_portal::verify_database(&pool).await?;
    }
    db::init(&pool).await?;
    // Bandes actives sans date de mise-bas dont des IA sont déjà enregistrées
    // (import historique, ou date jamais renseignée manuellement) : la date
    // théorique se déduit de la moyenne des IA de la bande, plus la durée de
    // gestation réglée dans Paramètres. Exécuté une fois au démarrage — les
    // nouvelles IA la calculent elles-mêmes au moment de la saisie (voir
    // inseminations_enregistrer). Ne touche jamais une date déjà renseignée.
    if let Ok(schedule) = routes::load_band_schedule(&pool).await {
        if let Err(e) = sqlx::query(
            "UPDATE bande SET date_mb=date((SELECT date(AVG(julianday(date))) FROM evenement WHERE bande_id=bande.id AND type='ia'),printf('%+d day',?)),updated_at=CURRENT_TIMESTAMP \
             WHERE active=1 AND date_mb IS NULL AND EXISTS(SELECT 1 FROM evenement WHERE bande_id=bande.id AND type='ia')",
        )
        .bind(schedule.gestation)
        .execute(&pool)
        .await
        {
            tracing::warn!(error=%e, "Complétion des dates de mise-bas manquantes impossible");
        }
    }
    if demo_portal::enabled() {
        demo_portal::init(&pool).await?;
        let cleanup_pool = pool.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                if let Err(e) = demo_portal::prune(&cleanup_pool).await {
                    tracing::error!(error=%e,"Nettoyage démo impossible");
                }
            }
        });
    }
    // Annonce sur le réseau local, pour que l'application Android trouve le
    // serveur sans qu'on lui saisisse une adresse. Le démon doit rester vivant
    // aussi longtemps que le serveur : le lier ici, et non dans une fonction
    // dont il sortirait, sinon l'annonce disparaît aussitôt publiée.
    let nom_elevage: Option<String> =
        sqlx::query_scalar("SELECT valeur FROM parametre WHERE cle='nom_elevage'")
            .fetch_optional(&pool)
            .await?
            .flatten();
    let _annonce_reseau = mdns::annoncer(
        config.bind.port(),
        nom_elevage.as_deref(),
        env!("CARGO_PKG_VERSION"),
    );

    // Repliement et purge des séries d'ambiance. Sans lui, le détail à la
    // minute s'accumulerait indéfiniment ; avec lui, l'historique long terme
    // tient dans quelques centaines de mégaoctets.
    let ambiance_pool = pool.clone();
    tokio::spawn(async move {
        crate::ambiance::boucle_entretien(ambiance_pool).await;
    });

    // Régulation de ventilation : alerte critique si le démon se tait.
    let regulation_pool = pool.clone();
    tokio::spawn(async move {
        crate::ambiance::boucle_regulation(regulation_pool).await;
    });

    // Alertes SMS (Free Mobile) et SMS au retour du courant.
    let sms_pool = pool.clone();
    tokio::spawn(async move {
        crate::sms_free::boucle(sms_pool).await;
    });

    // Serveur Proxmox : alertes SMS (disque, mémoire, machines, sauvegardes).
    let proxmox_pool = pool.clone();
    tokio::spawn(async move {
        crate::proxmox::boucle(proxmox_pool).await;
    });

    // Suivi des cours (porc, céréales) : un relevé par jour.
    let cours_pool = pool.clone();
    tokio::spawn(async move {
        crate::cours::boucle(cours_pool).await;
    });

    // Machine à soupe : alerte quand le service d'écoute ne donne plus de nouvelles.
    let soupe_pool = pool.clone();
    tokio::spawn(async move {
        crate::soupe::boucle(soupe_pool).await;
    });

    // Tunnel Cloudflare (accès à distance), piloté depuis Mes équipements.
    let tunnel_pool = pool.clone();
    tokio::spawn(async move {
        crate::tunnel::boucle(tunnel_pool).await;
    });

    // Module Automatique : règles si… alors… évaluées toutes les 10 s.
    let automatique_pool = pool.clone();
    tokio::spawn(async move {
        routes::automatique::boucle(automatique_pool).await;
    });

    // Consommation électrique : journées finies gardées dans elec_jour.
    let elec_pool = pool.clone();
    tokio::spawn(async move {
        routes::electricite::boucle(elec_pool).await;
    });

    let templates = templates::build()?;
    routes::charger_equip_masques(&pool).await;
    let state = AppState::new(config.clone(), pool, templates);
    routes::recharger_nom_elevage(&state.pool).await;

    let app = routes::router(state.clone())
        .nest_service("/static", ServeDir::new(dossier_static()))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn_with_state(state, auth::guard));

    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!(address = %config.bind, database = %config.db_path.display(), "EO_Sevel démarré");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::warn!(%error, "écoute du signal Ctrl+C impossible");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => tracing::warn!(%error, "écoute du signal SIGTERM impossible"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

/// Dossier des fichiers statiques (styles, logo) : `static` du dossier
/// courant, sinon celui posé à côté de l'exécutable — sous Windows, un
/// double-clic ne lance pas toujours le programme depuis son propre dossier.
fn dossier_static() -> std::path::PathBuf {
    let local = std::path::PathBuf::from("static");
    if local.is_dir() {
        return local;
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dossier| dossier.join("static")))
        .filter(|dossier| dossier.is_dir())
        .unwrap_or(local)
}
