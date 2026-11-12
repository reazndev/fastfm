//! In-process latency of the dispatcher: 1,000 calls each of `user.getRecentTracks` with
//! `limit=200` and `user.getTopArtists`, without and with the response cache.
//!
//! `LFM_BENCH_DB=<copy of data/conformance.db> LFM_BENCH_USER=syntiiix \
//!  cargo run -p lfm-api --release --example bench`
//!
//! Use a copy: opening the database applies pending migrations.
#![allow(clippy::print_stdout, reason = "the benchmark reports to the terminal")]
#![allow(clippy::expect_used, reason = "a benchmark stops on the first failure")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lfm_api::{
    ApiKey, Clock, Dispatcher, InMemoryApiKeys, InMemorySessions, MemoryCache, Scope, Services,
};
use lfm_core::Params;
use lfm_store::Store;

const CALLS: usize = 1000;
const KEY: &str = "bench";

fn dispatcher(store: Store, cache_bytes: u64) -> Dispatcher {
    let key = ApiKey {
        account_id: 1,
        secret: String::new(),
        scope: Scope::Read,
        active: true,
    };
    let services = Services {
        store,
        clock: Clock::System,
        api_keys: Arc::new(InMemoryApiKeys::default().with(KEY, key)),
        sessions: Arc::new(InMemorySessions::default()),
        cache: Arc::new(MemoryCache::new(cache_bytes, Duration::from_secs(3600))),
    };
    Dispatcher::new(lfm_api::methods::registry(), services)
}

/// Median and 99th percentile of `CALLS` calls, after one warm-up call.
async fn measure(d: &Dispatcher, params: &Params) -> (Duration, Duration, usize) {
    let warm = d.dispatch(params).await;
    assert_eq!(warm.status, 200, "{}", String::from_utf8_lossy(&warm.body));
    let mut times = Vec::with_capacity(CALLS);
    for _ in 0..CALLS {
        let start = Instant::now();
        let reply = d.dispatch(params).await;
        times.push(start.elapsed());
        assert_eq!(reply.status, 200);
    }
    times.sort_unstable();
    let at = |q: usize| times.get(CALLS * q / 100).copied().unwrap_or_default();
    (at(50), at(99), warm.body.len())
}

#[tokio::main]
async fn main() {
    let path = PathBuf::from(std::env::var("LFM_BENCH_DB").expect("set LFM_BENCH_DB"));
    let user = std::env::var("LFM_BENCH_USER").unwrap_or_else(|_| "syntiiix".to_owned());
    let store = Store::open(&path).expect("open the database");
    let base = |method: &str| {
        Params::new()
            .with("method", method)
            .with("user", user.as_str())
            .with("api_key", KEY)
            .with("format", "json")
    };
    let cases = [
        (
            "user.getRecentTracks limit=200",
            base("user.getRecentTracks").with("limit", "200"),
        ),
        ("user.getTopArtists", base("user.getTopArtists")),
    ];
    for (label, cache) in [("uncached", 0), ("cached", 64 << 20)] {
        let d = dispatcher(store.clone(), cache);
        for (name, params) in &cases {
            let (p50, p99, bytes) = measure(&d, params).await;
            println!("{label:9} {name:32} p50 {p50:>10.1?}  p99 {p99:>10.1?}  {bytes} bytes");
        }
    }
}
