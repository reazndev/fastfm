//! HTTP load generator for a running `lfm-mirror`: keeps `concurrency` keep-alive connections
//! busy with one request for `seconds`, then prints requests per second, latency percentiles
//! and the mean body size. A stand-in for `oha` where it is not installed.
//!
//! ```sh
//! cargo run -p lfm-mirror --release --example load -- \
//!   'http://127.0.0.1:3055/2.0/?method=user.getInfo&user=x&api_key=KEY' \
//!   [--concurrency 32] [--seconds 5] [--header 'Accept-Encoding: gzip'] [--unique]
//! ```
//!
//! `--unique` appends `&_n=<counter>` to every request, so a response cache keyed on the
//! parameters never hits: the cold-cache case.
#![allow(
    clippy::print_stdout,
    reason = "the load generator reports to the terminal"
)]
#![allow(
    clippy::expect_used,
    reason = "a load generator stops on bad arguments"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

/// What to send and for how long.
#[derive(Debug, Clone)]
struct Plan {
    url: String,
    concurrency: usize,
    seconds: u64,
    headers: HeaderMap,
    unique: bool,
}

fn plan() -> Plan {
    let mut args = std::env::args().skip(1);
    let mut plan = Plan {
        url: String::new(),
        concurrency: 32,
        seconds: 5,
        headers: HeaderMap::new(),
        unique: false,
    };
    while let Some(arg) = args.next() {
        let mut value = || args.next().expect("a value after the flag");
        match arg.as_str() {
            "--concurrency" => plan.concurrency = value().parse().expect("a number"),
            "--seconds" => plan.seconds = value().parse().expect("a number"),
            "--unique" => plan.unique = true,
            "--header" => {
                let line = value();
                let (name, text) = line.split_once(':').expect("Name: value");
                plan.headers.insert(
                    HeaderName::from_bytes(name.trim().as_bytes()).expect("a header name"),
                    HeaderValue::from_str(text.trim()).expect("a header value"),
                );
            }
            _ => plan.url = arg,
        }
    }
    assert!(!plan.url.is_empty(), "give the URL to load");
    plan
}

/// Latencies in microseconds and bytes received by one connection.
#[derive(Default)]
struct Tally {
    micros: Vec<u64>,
    bytes: u64,
    errors: u64,
}

async fn worker(plan: Arc<Plan>, counter: Arc<AtomicU64>, until: Instant) -> Tally {
    let client = reqwest::Client::builder()
        .default_headers(plan.headers.clone())
        .pool_max_idle_per_host(1)
        .build()
        .expect("build the client");
    let mut tally = Tally::default();
    while Instant::now() < until {
        let url = if plan.unique {
            let n = counter.fetch_add(1, Ordering::Relaxed);
            format!("{}&_n={n}", plan.url)
        } else {
            plan.url.clone()
        };
        let start = Instant::now();
        let outcome = match client.get(&url).send().await {
            Ok(response) if response.status().is_success() || response.status() == 304 => {
                response.bytes().await.ok()
            }
            _ => None,
        };
        let micros = u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX);
        match outcome {
            Some(body) => {
                tally.micros.push(micros);
                tally.bytes += body.len() as u64;
            }
            None => tally.errors += 1,
        }
    }
    tally
}

async fn run(plan: Arc<Plan>, duration: Duration) -> (Tally, Duration) {
    let counter = Arc::new(AtomicU64::new(0));
    let start = Instant::now();
    let until = start + duration;
    let tasks: Vec<_> = (0..plan.concurrency)
        .map(|_| tokio::spawn(worker(Arc::clone(&plan), Arc::clone(&counter), until)))
        .collect();
    let mut all = Tally::default();
    for task in tasks {
        let tally = task.await.expect("a worker finished");
        all.micros.extend(tally.micros);
        all.bytes += tally.bytes;
        all.errors += tally.errors;
    }
    (all, start.elapsed())
}

#[tokio::main]
async fn main() {
    let plan = Arc::new(plan());
    // A short warm-up fills connection pools and, unless --unique, the server's caches.
    run(Arc::clone(&plan), Duration::from_millis(500)).await;
    let (mut tally, elapsed) = run(Arc::clone(&plan), Duration::from_secs(plan.seconds)).await;
    tally.micros.sort_unstable();
    let count = tally.micros.len();
    let at = |q: usize| tally.micros.get(count * q / 1000).copied().unwrap_or(0);
    #[expect(clippy::cast_precision_loss, reason = "a rate needs no precision")]
    let rps = count as f64 / elapsed.as_secs_f64();
    let mean = tally.bytes / (count as u64).max(1);
    println!(
        "{rps:>9.0} req/s  p50 {:>6} us  p99 {:>6} us  p99.9 {:>6} us  {mean} B/resp  {} errors",
        at(500),
        at(990),
        at(999),
        tally.errors
    );
}
