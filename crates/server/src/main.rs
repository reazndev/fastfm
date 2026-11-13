//! `lfm-mirror`: serves the Last.fm API from a local copy.

use anyhow::Context as _;
use std::net::SocketAddr;

use lfm_mirror::{App, Config, Jobs, app_router};
use lfm_store::Store;
use tracing_subscriber::EnvFilter;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let config = Config::from_env()?;
    tracing::info!(bind = %config.bind, db = %config.db.display(), "starting");
    if config.now.is_some() || config.dev_key.is_some() {
        tracing::warn!("LFM_NOW or LFM_DEV_API_KEY is set; these are for development only");
    }
    if let Some(dir) = config.db.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let store =
        Store::open(&config.db).with_context(|| format!("opening {}", config.db.display()))?;
    let app = App::new(store, &config).await?;
    if app.passthrough.is_none() {
        tracing::warn!(
            "LASTFM_API_KEYS is not set; catalogue methods answer error 3, and only accounts \
             with their own Last.fm key can sync"
        );
    }
    if config.lastfm_app().is_none() {
        tracing::info!(
            "LASTFM_API_SECRET is not set; Last.fm web auth and forwarded writes need an \
             account's own key"
        );
    }

    // Background jobs share the store with the dispatcher and stop on the same signal.
    let jobs = Jobs::start(&app, &config).await?;
    let maintenance = tokio::spawn(app.clone().maintain());
    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("binding {}", config.bind))?;
    let service = app_router(&app).into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, service)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serving")?;
    maintenance.abort();
    if let Some(jobs) = jobs {
        jobs.shutdown().await;
    }
    tracing::info!("stopped");
    Ok(())
}
async fn shutdown_signal() {
    let interrupt = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "cannot listen for Ctrl-C");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::error!(%error, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = interrupt => {},
        () = terminate => {},
    }
    tracing::info!("shutting down");
}
