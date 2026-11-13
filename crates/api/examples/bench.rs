//! In-process latency of the dispatcher: 1,000 calls each of `user.getRecentTracks` with
//! `limit=200` and `user.getTopArtists`, without and with the response cache, and of
//! `artist.getInfo` served from the pass-through cache (from memory, and from the store as after
//! a restart). The database must hold that `artist.getInfo` answer, as it does after one
//! conformance run against it; the benchmark never calls Last.fm.
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
    ApiKey, Clock, Dispatcher, InMemoryApiKeys, InMemorySessions, MemoryCache, Passthrough, Scope,
    Services,
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

/// A pass-through cache whose client points nowhere, so a miss fails instead of calling out.
fn offline(store: Store) -> Passthrough {
    let config = lfm_upstream::ClientConfig::new([]).base_url("http://127.0.0.1:9/2.0/");
    let client = lfm_upstream::Client::new(config).expect("build the client");
    Passthrough::new(store, Arc::new(client))
}

/// Median and 99th percentile of `CALLS` calls, after one warm-up call. `make` gives the
/// dispatcher of each call.
async fn measure(make: impl Fn() -> Dispatcher, params: &Params) -> (Duration, Duration, usize) {
    let d = make();
    let warm = d.dispatch(params).await;
    assert_eq!(warm.status, 200, "{}", String::from_utf8_lossy(&warm.body));
    let mut times = Vec::with_capacity(CALLS);
    for _ in 0..CALLS {
        let d = make();
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
            let (p50, p99, bytes) = measure(|| d.clone(), params).await;
            println!("{label:9} {name:32} p50 {p50:>10.1?}  p99 {p99:>10.1?}  {bytes} bytes");
        }
    }
    let info = Params::new()
        .with("method", "artist.getInfo")
        .with("artist", "Juice WRLD")
        .with("api_key", KEY)
        .with("format", "json");
    let hot = dispatcher(store.clone(), 0).with_passthrough(offline(store.clone()));
    let restarted = || dispatcher(store.clone(), 0).with_passthrough(offline(store.clone()));
    for (label, make) in [
        (
            "memory",
            Box::new(|| hot.clone()) as Box<dyn Fn() -> Dispatcher>,
        ),
        ("store", Box::new(restarted)),
    ] {
        let (p50, p99, bytes) = measure(make, &info).await;
        let name = "artist.getInfo pass-through";
        println!("{label:9} {name:32} p50 {p50:>10.1?}  p99 {p99:>10.1?}  {bytes} bytes");
    }
}
