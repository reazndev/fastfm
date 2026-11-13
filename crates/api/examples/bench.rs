//! In-process server time of the dispatcher: every local method, uncached and cached, plus
//! `artist.getInfo` served from the pass-through cache. A run is `CALLS` calls after one
//! warm-up call; the database answers from the page cache by then. The benchmark never calls
//! Last.fm: the pass-through client points nowhere, so `artist.getInfo` is reported only when
//! the database holds that answer (as after one conformance run against it).
//!
//! `LFM_BENCH_DB=<copy of data/lfm.db> LFM_BENCH_USER=syntiiix LFM_BENCH_NOW=1794518863 \
//!  cargo run -p lfm-api --release --example bench`
//!
//! Use a copy: opening the database applies pending migrations.
#![allow(clippy::print_stdout, reason = "the benchmark reports to the terminal")]
#![allow(clippy::expect_used, reason = "a benchmark stops on the first failure")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lfm_api::{
    ApiKey, Clock, Dispatcher, InMemoryApiKeys, InMemorySessions, MemoryCache, Passthrough, Scope,
    Services,
};
use lfm_core::Params;
use lfm_store::Store;

// The server's allocator, so allocation costs match what it pays.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const CALLS: usize = 1000;
const KEY: &str = "bench";

/// Every local method with the parameters clients use most.
/// A method and its extra parameters.
type Case = (&'static str, &'static [(&'static str, &'static str)]);

const CASES: [Case; 17] = [
    ("user.getRecentTracks", &[("limit", "50")]),
    ("user.getRecentTracks", &[("limit", "200")]),
    ("user.getRecentTracks", &[("limit", "200"), ("page", "500")]),
    (
        "user.getRecentTracks",
        &[("limit", "200"), ("extended", "1")],
    ),
    ("user.getRecentTracks", &[("limit", "1000")]),
    ("user.getTopArtists", &[]),
    ("user.getTopArtists", &[("period", "7day")]),
    ("user.getTopArtists", &[("period", "12month")]),
    ("user.getTopAlbums", &[("period", "1month")]),
    ("user.getTopTracks", &[("limit", "1000"), ("page", "10")]),
    ("user.getWeeklyChartList", &[]),
    ("user.getWeeklyArtistChart", &[]),
    ("user.getWeeklyAlbumChart", &[]),
    ("user.getWeeklyTrackChart", &[]),
    ("user.getInfo", &[]),
    ("user.getLovedTracks", &[]),
    ("library.getArtists", &[]),
];

fn dispatcher(store: Store, clock: Clock, cache_bytes: u64) -> Dispatcher {
    let key = ApiKey {
        account_id: 1,
        secret: String::new(),
        scope: Scope::Read,
        active: true,
    };
    let services = Services {
        store,
        clock,
        api_keys: Arc::new(InMemoryApiKeys::default().with(KEY, key)),
        sessions: Arc::new(InMemorySessions::default()),
        cache: Arc::new(MemoryCache::new(cache_bytes, Duration::from_secs(3600))),
    };
    Dispatcher::new(lfm_api::methods::registry(), services)
}

/// A pass-through cache whose client points nowhere, so a miss fails instead of calling out.
fn offline(store: Store) -> Passthrough {
    let config = lfm_upstream::ClientConfig::new([]).base_url("http://127.0.0.1:9/2.0/");
    let client = lfm_upstream::Client::new(config).expect("build the client");
    Passthrough::new(store, Arc::new(client))
}

/// Median and 99th percentile of `CALLS` calls after one warm-up call, and the body size.
/// `None` when the warm-up call fails.
async fn measure(d: &Dispatcher, params: &Params) -> Option<(Duration, Duration, usize)> {
    let warm = d.dispatch(params).await;
    if warm.status != 200 {
        return None;
    }
    let mut times = Vec::with_capacity(CALLS);
    for _ in 0..CALLS {
        let start = Instant::now();
        let reply = d.dispatch(params).await;
        times.push(start.elapsed());
        assert_eq!(reply.status, 200);
    }
    times.sort_unstable();
    let at = |q: usize| times.get(CALLS * q / 100).copied().unwrap_or_default();
    Some((at(50), at(99), warm.body.len()))
}

fn report(label: &str, name: &str, result: Option<(Duration, Duration, usize)>) {
    match result {
        Some((p50, p99, bytes)) => println!(
            "{label:9} {name:58} p50 {:>8.3} ms  p99 {:>8.3} ms  {bytes:>7} B",
            p50.as_secs_f64() * 1e3,
            p99.as_secs_f64() * 1e3
        ),
        None => println!("{label:9} {name:58} not answered"),
    }
}

#[tokio::main]
async fn main() {
    let path = PathBuf::from(std::env::var("LFM_BENCH_DB").expect("set LFM_BENCH_DB"));
    let user = std::env::var("LFM_BENCH_USER").unwrap_or_else(|_| "syntiiix".to_owned());
    let clock = std::env::var("LFM_BENCH_NOW")
        .ok()
        .and_then(|now| now.parse().ok())
        .map_or(Clock::System, Clock::Fixed);
    let store = Store::open(&path).expect("open the database");
    for (label, cache) in [("uncached", 0), ("cached", 64 << 20)] {
        let d = dispatcher(store.clone(), clock, cache);
        for (method, extra) in CASES {
            let params = extra.iter().fold(
                Params::new()
                    .with("method", method)
                    .with("user", user.as_str())
                    .with("api_key", KEY)
                    .with("format", "json"),
                |params, (name, value)| params.with(*name, *value),
            );
            let args: Vec<String> = extra.iter().map(|(n, v)| format!("{n}={v}")).collect();
            let name = format!("{method} {}", args.join("&"));
            report(label, &name, measure(&d, &params).await);
        }
    }
    let info = Params::new()
        .with("method", "artist.getInfo")
        .with("artist", "Juice WRLD")
        .with("api_key", KEY)
        .with("format", "json");
    let hot = dispatcher(store.clone(), clock, 0).with_passthrough(offline(store.clone()));
    report(
        "memory",
        "artist.getInfo pass-through",
        measure(&hot, &info).await,
    );
}
