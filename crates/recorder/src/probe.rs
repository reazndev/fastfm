//! A gentle measurement of whether Last.fm rate-limits per key or per IP: about 20 seconds at
//! 8 requests per second on one key, then the same total rate spread over four keys. Each
//! phase stops at the first error 29 or HTTP 429.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lfm_upstream::{ApiKey, BucketMode, Client, ClientConfig, Format, RawRequest, RetryPolicy};
use tokio::task::JoinSet;
use tracing::{info, warn};

use crate::env::Env;
use crate::error::RecordError;

const TOTAL_RPS: u32 = 8;
const PHASE: Duration = Duration::from_secs(20);
const PAUSE: Duration = Duration::from_secs(60);
const SPREAD_KEYS: usize = 4;

/// What one phase saw.
#[derive(Debug, Default)]
struct Outcome {
    sent: u32,
    ok: u32,
    other_errors: u32,
    limited_after: Option<(u32, Duration)>,
}

/// Runs both phases and logs what each saw.
pub(crate) async fn run(env: &Env) -> Result<(), RecordError> {
    let one: Vec<ApiKey> = env.keys.iter().take(1).cloned().collect();
    let spread: Vec<ApiKey> = env.keys.iter().skip(1).take(SPREAD_KEYS).cloned().collect();
    let single = phase(one, TOTAL_RPS, &env.user).await?;
    info!(?single, "phase 1: one key at {TOTAL_RPS} rps");
    tokio::time::sleep(PAUSE).await;
    let per_key = TOTAL_RPS / u32::try_from(spread.len().max(1)).unwrap_or(1);
    let spread_count = spread.len();
    let multi = phase(spread, per_key, &env.user).await?;
    info!(?multi, "phase 2: {spread_count} keys at {per_key} rps each");
    Ok(())
}

async fn phase(keys: Vec<ApiKey>, rps_per_key: u32, user: &str) -> Result<Outcome, RecordError> {
    let key_count = u32::try_from(keys.len()).unwrap_or(1);
    let rps = NonZeroU32::new(rps_per_key).unwrap_or(NonZeroU32::MIN);
    let config = ClientConfig::new(keys)
        .requests_per_second(rps)
        .bucket_mode(BucketMode::PerKey)
        .retry(RetryPolicy {
            max_attempts: 1,
            ..RetryPolicy::default()
        });
    let client = Arc::new(Client::new(config)?);
    let total = rps_per_key * key_count * u32::try_from(PHASE.as_secs()).unwrap_or(20);
    let request = RawRequest::new("user.getInfo")
        .param("user", user)
        .format(Format::Json);
    let start = Instant::now();
    let mut tasks = JoinSet::new();
    for _ in 0..total {
        let client = Arc::clone(&client);
        let request = request.clone();
        tasks.spawn(async move { client.raw(&request).await });
    }
    let mut outcome = Outcome::default();
    while let Some(joined) = tasks.join_next().await {
        outcome.sent += 1;
        match joined {
            Ok(Ok(response)) => {
                let code = response.lastfm_error().map(|e| e.code);
                if code == Some(29) || response.status == 429 {
                    warn!(
                        status = response.status,
                        ?code,
                        "rate limit hit, stopping phase"
                    );
                    outcome.limited_after = Some((outcome.sent, start.elapsed()));
                    tasks.abort_all();
                    break;
                }
                if code.is_some() || !response.is_success() {
                    outcome.other_errors += 1;
                } else {
                    outcome.ok += 1;
                }
            }
            Ok(Err(_)) | Err(_) => outcome.other_errors += 1,
        }
    }
    info!(elapsed = ?start.elapsed(), "phase finished");
    Ok(outcome)
}
