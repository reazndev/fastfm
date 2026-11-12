//! `lfm-sync`: imports, reconciles, or live-syncs one Last.fm user into a store file.

mod args;
mod env;

use std::num::NonZeroU32;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use lfm_store::{Store, TimeRange};
use lfm_sync::{SyncConfig, SyncError, SyncService};
use lfm_upstream::{BucketMode, Client, ClientConfig};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::args::{Args, Command, USAGE};

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("{0}\n{USAGE}")]
    Usage(String),
    #[error(transparent)]
    Sync(#[from] SyncError),
    #[error(transparent)]
    Store(#[from] lfm_store::StoreError),
    #[error(transparent)]
    Upstream(#[from] lfm_upstream::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .init();
    let result = args::parse(std::env::args().skip(1))
        .map_err(CliError::Usage)
        .and_then(|args| {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(run(&args))
        });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error!("{err}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: &Args) -> Result<(), CliError> {
    if let Some(dir) = args.db.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let store = Store::open(&args.db)?;
    let rps = NonZeroU32::new(args.rps).unwrap_or(NonZeroU32::MIN);
    let config = ClientConfig::new(env::api_keys().map_err(CliError::Usage)?)
        .requests_per_second(rps)
        .bucket_mode(BucketMode::Shared);
    let client = Arc::new(Client::new(config)?);
    let sync_config = SyncConfig::default().import_concurrency(args.concurrency);
    let service = SyncService::new(store.clone(), client, sync_config);
    match args.command {
        Command::Import => import(&service, &store, args).await,
        Command::Reconcile => reconcile(&service, &store, args).await,
        Command::Live => live(&service, args).await,
    }
}

async fn import(service: &SyncService, store: &Store, args: &Args) -> Result<(), CliError> {
    let started = Instant::now();
    let progress = service.import(&args.user, args.until).await?;
    let user = store
        .user_by_name(&args.user)
        .await?
        .ok_or_else(|| SyncError::UnknownUser(args.user.clone()))?;
    let stored = store
        .count_range(user.id, TimeRange::new(0, progress.until))
        .await?;
    info!(
        user = %user.name,
        until = progress.until,
        lastfm_total = progress.estimated_total,
        written = progress.imported,
        stored,
        artists = user.stats.artists,
        albums = user.stats.albums,
        tracks = user.stats.tracks,
        seconds = started.elapsed().as_secs_f64(),
        requests = service.requests(),
        "import finished"
    );
    Ok(())
}

async fn reconcile(service: &SyncService, store: &Store, args: &Args) -> Result<(), CliError> {
    let started = Instant::now();
    let user = store
        .user_by_name(&args.user)
        .await?
        .ok_or_else(|| SyncError::UnknownUser(args.user.clone()))?;
    let report = service.reconcile(user.id, args.until).await?;
    info!(
        user = %user.name,
        until = report.range.to,
        compared = report.compared,
        repaired = ?report.repaired,
        written = report.written,
        requests = report.requests,
        seconds = started.elapsed().as_secs_f64(),
        "reconcile finished"
    );
    Ok(())
}

async fn live(service: &SyncService, args: &Args) -> Result<(), CliError> {
    service.start().await?;
    let user = service.follow(&args.user).await?;
    info!(user = user.0, "live sync running; press Ctrl-C to stop");
    tokio::signal::ctrl_c().await?;
    service.shutdown().await;
    Ok(())
}
