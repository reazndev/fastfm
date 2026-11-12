//! `lfm-mirror`: serves the Last.fm API from a local copy.

use anyhow::Context as _;
use lfm_mirror::{App, Config, router};
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

    // SEAM: background jobs (import, live sync, reconcile, pre-warm) start here, sharing the
    // store with the dispatcher, and stop on the same shutdown signal.
    let maintenance = tokio::spawn(app.clone().maintain());
    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("binding {}", config.bind))?;
    axum::serve(listener, router(app.dispatcher))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serving")?;
    maintenance.abort();
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
