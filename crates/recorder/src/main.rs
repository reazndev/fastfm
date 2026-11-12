//! `lfm-record`: records conformance fixtures from the real Last.fm. Run by hand only.
//!
//! - `lfm-record [record] [--force] [--only <method-prefix>]` writes `fixtures/`, skipping
//!   fixtures that already exist unless `--force` is given.
//! - `lfm-record probe` measures whether the rate limit is per key or per IP.

mod discover;
mod env;
mod error;
mod probe;
mod spec;
mod specs;
mod writer;

use std::num::NonZeroU32;
use std::process::ExitCode;

use lfm_upstream::{ApiKey, BucketMode, Client, ClientConfig};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::discover::Discovered;
use crate::env::{Env, repo_root};
use crate::error::RecordError;
use crate::writer::Writer;

/// About two requests per second across all keys, to stay well clear of Last.fm's limit.
const RECORD_RPS: NonZeroU32 = NonZeroU32::MIN.saturating_add(1);

#[derive(Debug, Default)]
struct RecordArgs {
    force: bool,
    only: Option<String>,
}

#[derive(Debug)]
enum Command {
    Record(RecordArgs),
    Probe,
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .init();
    let result = parse_args(std::env::args().skip(1)).and_then(|command| {
        let env = Env::load()?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(run(command, &env))
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error!("{err}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Command, RecordError> {
    let mut record = RecordArgs::default();
    let mut args = args.peekable();
    if args.peek().map(String::as_str) == Some("probe") {
        return Ok(Command::Probe);
    }
    if args.peek().map(String::as_str) == Some("record") {
        args.next();
    }
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--force" => record.force = true,
            "--only" => record.only = args.next(),
            other => return Err(RecordError::Usage(format!("unknown argument {other}"))),
        }
    }
    Ok(Command::Record(record))
}

async fn run(command: Command, env: &Env) -> Result<(), RecordError> {
    match command {
        Command::Probe => probe::run(env).await,
        Command::Record(args) => record(args, env).await,
    }
}

async fn record(args: RecordArgs, env: &Env) -> Result<(), RecordError> {
    let config = ClientConfig::new(env.keys.clone())
        .requests_per_second(RECORD_RPS)
        .bucket_mode(BucketMode::Shared);
    let client = Client::new(config)?;
    let discovered = Discovered::load(&client, &env.user).await?;
    info!(
        artists = ?discovered.artists,
        albums = ?discovered.albums,
        tracks = ?discovered.tracks,
        weeks = discovered.charts.len(),
        last_pages = ?discovered.last_pages,
        day = ?discovered.day,
        one = ?discovered.one,
        zero = ?discovered.zero,
        "discovered the user's data"
    );
    let secrets = env
        .keys
        .iter()
        .map(ApiKey::expose)
        .map(str::to_owned)
        .collect();
    let writer = Writer::new(repo_root().join("fixtures"), secrets, args.force);
    let specs = specs::all(&discovered);
    let selected: Vec<_> = specs
        .iter()
        .filter(|s| {
            args.only
                .as_deref()
                .is_none_or(|p| s.method().starts_with(p))
        })
        .collect();
    info!(total = selected.len(), "recording fixtures");
    let mut written = 0usize;
    for spec in selected {
        if writer.record(&client, spec).await? {
            written += 1;
        }
    }
    info!(written, "done");
    Ok(())
}
