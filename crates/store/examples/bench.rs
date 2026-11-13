//! Store benchmark: builds 20 users with 1.5M scrobbles each and times the hot queries.
//!
//! `cargo run -p lfm-store --release --example bench`
//!
//! Environment:
//! - `LFM_BENCH_USERS` (20) and `LFM_BENCH_SCROBBLES` (1500000) set the size.
//! - `LFM_BENCH_DIR` is where the temporary database goes (the system temp dir by default).
//! - `LFM_BENCH_DB` names a database file to keep and reuse between runs instead.
//!
//! After loading, every user gets a library as large as a real one (every album moved, one
//! artist in a hundred merged), so the timed queries read mapped aggregates.
#![allow(clippy::print_stdout, reason = "the benchmark reports to the terminal")]

use std::future::Future;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use lfm_store::{
    Kind, LibAlbum, LibMove, Library, Listing, Lookup, Named, Page, Period, Scrobble, Store,
    TimeRange, TopQuery, UserId,
};

const DAY: i64 = 86_400;
const START: i64 = 1_262_304_000; // 2010-01-01
const WINDOW: usize = 20_000;
const RUNS: usize = 2_000;

/// Deterministic pseudo-random numbers (`SplitMix64`).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn range(&mut self, from: i64, to: i64) -> i64 {
        from + i64::try_from(self.below(u64::try_from(to - from).unwrap_or(1))).unwrap_or(0)
    }
}

/// Zipf-like choice of an artist from a user's pool of 15,000: rank r has weight 1 / r.
fn pick_artist(rng: &mut Rng, cdf: &[f64]) -> usize {
    let total = cdf.last().copied().unwrap_or(1.0);
    #[expect(
        clippy::cast_precision_loss,
        reason = "a random fraction needs no precision"
    )]
    let target = (rng.next() >> 11) as f64 / (1u64 << 53) as f64 * total;
    cdf.partition_point(|&weight| weight < target)
}

/// A listening history: sessions of consecutive album tracks, about four minutes apart.
struct History {
    rng: Rng,
    cdf: Vec<f64>,
    user: u64,
    uts: i64,
    session: Vec<Scrobble>,
}

impl History {
    fn new(user: u64) -> Self {
        let mut sum = 0.0;
        let cdf = (1..=15_000)
            .map(|rank| {
                sum += 1.0 / f64::from(rank);
                sum
            })
            .collect();
        Self {
            rng: Rng(user + 1),
            cdf,
            user,
            uts: START,
            session: Vec::new(),
        }
    }

    fn refill(&mut self) {
        let rank = u64::try_from(pick_artist(&mut self.rng, &self.cdf)).unwrap_or(0);
        let artist = (rank * 7_919 + self.user * 1_013) % 60_000;
        let album = self.rng.below(1 + artist % 6);
        let first = self.rng.below(10);
        let length = 3 + self.rng.below(10);
        self.uts += self.rng.range(0, 1_200);
        for track in (first..first + length).map(|t| t % 10).rev() {
            self.uts += self.rng.range(170, 300);
            self.session.push(Scrobble {
                uts: self.uts,
                artist: Named::new(format!("Artist {artist}")),
                album: Some(Named::new(format!("Album {artist}-{album}"))),
                track: Named::new(format!("Track {artist}-{album}-{track}")),
                image: Some(format!("{:032x}.png", artist * 10 + album)),
            });
        }
        self.session.sort_by_key(|s| s.uts);
    }
}

impl Iterator for History {
    type Item = Scrobble;

    fn next(&mut self) -> Option<Scrobble> {
        if self.session.is_empty() {
            self.refill();
        }
        let next = self.session.first().cloned();
        if !self.session.is_empty() {
            self.session.remove(0);
        }
        next
    }
}

fn env_number(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// Loads `count` scrobbles for each user in windows of `WINDOW`, newest window first.
async fn load(store: &Store, users: usize, count: usize) -> Vec<(UserId, i64)> {
    let mut out = Vec::new();
    for index in 0..users {
        let name = format!("bench{index}");
        let id = store.ensure_user(&name).await.unwrap_or(UserId(0));
        let existing = store
            .user(id)
            .await
            .ok()
            .flatten()
            .map_or(0, |u| u.stats.playcount);
        let all: Vec<Scrobble> = History::new(index as u64).take(count).collect();
        let last = all.last().map_or(START, |s| s.uts);
        if existing < count as u64 {
            let started = Instant::now();
            for chunk in all.rchunks(WINDOW) {
                let from = chunk.first().map_or(START, |s| s.uts);
                let to = chunk.last().map_or(START, |s| s.uts) + 1;
                let window = TimeRange::new(from, to);
                if let Err(error) = store.replace_window(id, window, chunk.to_vec()).await {
                    println!("load failed: {error}");
                }
            }
            let rate = f64::from(u32::try_from(count).unwrap_or(u32::MAX))
                / started.elapsed().as_secs_f64();
            println!("loaded {name}: {count} scrobbles, {rate:.0} scrobbles/s");
        }
        out.push((id, last));
    }
    out
}

/// Runs `query` `RUNS` times and prints p50 and p99.
async fn time<F, Fut>(label: &str, mut query: F)
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = usize>,
{
    let mut rows = 0;
    let mut samples: Vec<Duration> = Vec::with_capacity(RUNS);
    for run in 0..RUNS {
        let started = Instant::now();
        rows += query(run).await;
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    let at = |q: usize| samples.get(RUNS * q / 100).copied().unwrap_or_default();
    let (p50, p99) = (at(50), at(99));
    let mean_rows = rows / RUNS;
    println!(
        "{label:<36} p50 {:>8.3} ms   p99 {:>8.3} ms   rows {mean_rows}",
        p50.as_secs_f64() * 1e3,
        p99.as_secs_f64() * 1e3
    );
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let users = env_number("LFM_BENCH_USERS", 20);
    let count = env_number("LFM_BENCH_SCROBBLES", 1_500_000);
    let dir = std::env::var_os("LFM_BENCH_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let temp = tempfile::tempdir_in(dir).ok();
    let path = std::env::var_os("LFM_BENCH_DB")
        .map(PathBuf::from)
        .or_else(|| temp.as_ref().map(|temp| temp.path().join("bench.db")));
    let Some(path) = path else {
        println!("no place for the database");
        return;
    };
    let store = match Store::open(&path) {
        Ok(store) => store,
        Err(error) => {
            println!("open failed: {error}");
            return;
        }
    };
    let started = Instant::now();
    let loaded = load(&store, users, count).await;
    println!(
        "data ready in {:.0} s at {}",
        started.elapsed().as_secs_f64(),
        path.display()
    );
    run_queries(&store, &loaded).await;
}

fn items<T>(result: lfm_store::Result<Listing<T>>) -> usize {
    result.map_or(0, |listing| listing.items.len())
}

async fn run_queries(store: &Store, users: &[(UserId, i64)]) {
    let mut rng = Rng(99);
    let pick = |run: usize| {
        users
            .get(run % users.len())
            .copied()
            .unwrap_or((UserId(0), START))
    };
    let ends: Vec<i64> = (0..RUNS)
        .map(|_| rng.range(START + 400 * DAY, START + 4_000 * DAY))
        .collect();
    let end = |run: usize, last: i64| ends.get(run).copied().unwrap_or(last).min(last);
    let now = users.iter().map(|(_, last)| *last).max().unwrap_or(START);
    let started = Instant::now();
    if let Err(error) = store.advance_periods(now).await {
        println!("advance failed: {error}");
    }
    println!(
        "built period totals in {:.1} s",
        started.elapsed().as_secs_f64()
    );
    enrich(store, users).await;

    time_history(store, &pick, &end).await;
    time_top_lists(store, &pick, &end, now).await;
}

/// Applies a library to every user as large as a real one needs: every scrobbled album counts
/// under a library album, and one artist in a hundred under another artist.
async fn enrich(store: &Store, users: &[(UserId, i64)]) {
    for (user, _) in users {
        let started = Instant::now();
        let Ok(inputs) = store.library_inputs(*user).await else {
            continue;
        };
        let albums = inputs.albums.iter().map(|a| LibAlbum {
            artist: Named::new(a.artist.clone()),
            album: Named::new(a.name.clone()),
            image: None,
        });
        let album_moves = inputs.albums.iter().map(|a| LibMove {
            raw: a.id,
            to: Lookup::Album {
                artist: a.artist.clone(),
                name: a.name.clone(),
            },
        });
        let first = inputs
            .artists
            .first()
            .map(|a| a.name.clone())
            .unwrap_or_default();
        let artist_moves = inputs.artists.iter().skip(1).step_by(100).map(|a| LibMove {
            raw: a.id,
            to: Lookup::Artist {
                name: first.clone(),
            },
        });
        let library = Library {
            albums: albums.collect(),
            moves: album_moves.chain(artist_moves).collect(),
            ..Library::default()
        };
        let moves = library.moves.len();
        if let Err(error) = store.apply_library(*user, library).await {
            println!("enrichment failed: {error}");
        }
        println!(
            "applied a library of {moves} moves in {:.1} s",
            started.elapsed().as_secs_f64()
        );
    }
}

async fn time_history(
    store: &Store,
    pick: &impl Fn(usize) -> (UserId, i64),
    end: &impl Fn(usize, i64) -> i64,
) {
    time("history, one day (limit 200)", |run| {
        let (user, last) = pick(run);
        let range = TimeRange::new(end(run, last) - DAY, end(run, last));
        async move { items(store.recent_tracks(user, range, Page::new(1, 200)).await) }
    })
    .await;
    time("history, deep page (limit 200)", |run| {
        let page = Page::new(2_000 + u32::try_from(run % 5_000).unwrap_or(0), 200);
        async move { items(store.recent_tracks(pick(run).0, TimeRange::ALL, page).await) }
    })
    .await;
}

async fn time_top_lists(
    store: &Store,
    pick: &impl Fn(usize) -> (UserId, i64),
    end: &impl Fn(usize, i64) -> i64,
    now: i64,
) {
    for (label, kind, period, page) in [
        ("top artists, 7day", Kind::Artist, Period::Days7, 1),
        ("top artists, 12month", Kind::Artist, Period::Month12, 1),
        (
            "top tracks, 12month, page 20",
            Kind::Track,
            Period::Month12,
            20,
        ),
        ("top artists, overall", Kind::Artist, Period::Overall, 1),
        (
            "top tracks, overall, page 1000",
            Kind::Track,
            Period::Overall,
            1_000,
        ),
    ] {
        let query = TopQuery { kind, period, now };
        time(label, |run| async move {
            items(store.top(pick(run).0, query, Page::new(page, 50)).await)
        })
        .await;
    }
    for (label, days) in [
        ("top artists, any 7 days", 7),
        ("top artists, any 365 days", 365),
    ] {
        time(label, |run| {
            let (user, last) = pick(run);
            let range = TimeRange::new(end(run, last) - days * DAY, end(run, last));
            async move {
                items(
                    store
                        .top_in_range(user, Kind::Artist, range, Page::new(1, 50))
                        .await,
                )
            }
        })
        .await;
    }
    time("count, 30-day range", |run| {
        let (user, last) = pick(run);
        let to = end(run, last) - 12_345;
        let range = TimeRange::new(to - 30 * DAY, to);
        async move {
            store
                .count_range(user, range)
                .await
                .map_or(0, |n| usize::try_from(n).unwrap_or(0))
        }
    })
    .await;
}
