//! A fake Last.fm for the sync tests, served by `wiremock`, plus helpers.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test helpers fail the test the same way the tests do"
)]
#![allow(dead_code, reason = "each test binary uses a different subset")]
#![allow(
    unreachable_pub,
    reason = "shared by several test binaries through `mod support`"
)]

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lfm_store::{Page, Store, TimeRange, UserId};
use lfm_sync::{ImportProgress, SyncConfig, SyncService};
use lfm_upstream::{ApiKey, Client, ClientConfig, RetryPolicy};
use serde_json::{Value, json};
use wiremock::{MockServer, Request, Respond, ResponseTemplate};

pub const USER: &str = "Alice";
pub const REGISTERED: i64 = 1_000_000;

/// The fake's data. Scrobbles are kept in arrival order; later ones in a second are newer.
#[derive(Debug, Default)]
pub struct Data {
    pub scrobbles: Vec<(i64, String)>,
    pub now_playing: Option<String>,
    pub loved: Vec<(i64, String)>,
    /// After this many more `user.getRecentTracks` calls, answer with error 8 forever.
    pub fail_after: Option<usize>,
    pub calls: HashMap<String, usize>,
}

/// A fake Last.fm whose data tests change while it runs.
#[derive(Debug, Clone, Default)]
pub struct Fake(pub Arc<Mutex<Data>>);

impl Fake {
    pub fn with<T>(&self, f: impl FnOnce(&mut Data) -> T) -> T {
        f(&mut self.0.lock().unwrap())
    }

    pub fn add(&self, items: &[(i64, &str)]) {
        self.with(|d| {
            d.scrobbles
                .extend(items.iter().map(|(t, n)| (*t, (*n).to_owned())));
        });
    }

    pub fn calls(&self, method: &str) -> usize {
        self.with(|d| d.calls.get(method).copied().unwrap_or_default())
    }

    /// What Last.fm would list, newest first: `(uts, name)`.
    pub fn listing(&self, range: TimeRange) -> Vec<(i64, String)> {
        self.with(|d| sorted(&d.scrobbles, range))
    }
}

fn sorted(all: &[(i64, String)], range: TimeRange) -> Vec<(i64, String)> {
    let mut rows: Vec<(usize, &(i64, String))> = all
        .iter()
        .enumerate()
        .filter(|(_, (t, _))| *t >= range.from && *t < range.to)
        .collect();
    rows.sort_by(|(ia, (ta, _)), (ib, (tb, _))| tb.cmp(ta).then(ib.cmp(ia)));
    rows.into_iter().map(|(_, row)| row.clone()).collect()
}

impl Respond for Fake {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let query: HashMap<String, String> = request.url.query_pairs().into_owned().collect();
        let method = query.get("method").cloned().unwrap_or_default();
        let mut data = self.0.lock().unwrap();
        *data.calls.entry(method.clone()).or_default() += 1;
        let body = match method.as_str() {
            "user.getRecentTracks" if data.fail_after == Some(0) => {
                json!({"error": 8, "message": "Operation failed"})
            }
            "user.getRecentTracks" => {
                data.fail_after = data.fail_after.map(|n| n - 1);
                recent(&data, &query)
            }
            "user.getInfo" => json!({"user": {"name": USER, "playcount": "0",
                "registered": {"unixtime": REGISTERED.to_string(), "#text": REGISTERED},
                "type": "user", "image": []}}),
            "user.getLovedTracks" => loved(&data),
            "user.getWeeklyChartList" => json!({"weeklychartlist": {"chart": [
                {"#text": "", "from": "1000", "to": "605800"},
                {"#text": "", "from": "605800", "to": "1210600"}], "@attr": {"user": USER}}}),
            _ => json!({"error": 3, "message": "Invalid Method"}),
        };
        ResponseTemplate::new(200).set_body_json(body)
    }
}

fn number(query: &HashMap<String, String>, name: &str, default: i64) -> i64 {
    query
        .get(name)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn recent(data: &Data, query: &HashMap<String, String>) -> Value {
    let range = TimeRange::new(number(query, "from", 0), number(query, "to", i64::MAX));
    let (limit, page) = (number(query, "limit", 50), number(query, "page", 1));
    let rows = sorted(&data.scrobbles, range);
    let total = i64::try_from(rows.len()).unwrap();
    let skip = usize::try_from((page - 1) * limit).unwrap();
    let mut tracks: Vec<Value> = rows
        .iter()
        .skip(skip)
        .take(usize::try_from(limit).unwrap())
        .map(|(uts, name)| track(name, Some(*uts)))
        .collect();
    if let (Some(name), 1) = (&data.now_playing, page) {
        tracks.insert(0, track(name, None));
    }
    let attr = json!({"user": USER, "page": page.to_string(), "perPage": limit.to_string(),
        "totalPages": ((total + limit - 1) / limit).to_string(), "total": total.to_string()});
    json!({"recenttracks": {"track": tracks, "@attr": attr}})
}

fn track(name: &str, uts: Option<i64>) -> Value {
    let mut track = json!({
        "artist": {"url": "https://www.last.fm/music/Cher", "name": "Cher", "mbid": "", "image": []},
        "name": name, "mbid": "", "url": format!("https://www.last.fm/music/Cher/_/{name}"),
        "album": {"mbid": "", "#text": "Believe"}, "image": [], "loved": "0", "streamable": "0"
    });
    match uts {
        Some(uts) => track["date"] = json!({"uts": uts.to_string(), "#text": ""}),
        None => track["@attr"] = json!({"nowplaying": "true"}),
    }
    track
}

fn loved(data: &Data) -> Value {
    let tracks: Vec<Value> = data
        .loved
        .iter()
        .map(|(uts, name)| {
            json!({"artist": {"url": "u", "name": "Cher", "mbid": ""}, "name": name,
                "mbid": "", "url": "t", "image": [],
                "date": {"uts": uts.to_string(), "#text": ""}})
        })
        .collect();
    let attr = json!({"user": USER, "page": "1", "perPage": "1000", "totalPages": "1",
        "total": tracks.len().to_string()});
    json!({"lovedtracks": {"track": tracks, "@attr": attr}})
}

/// A running fake and a service against it, with small pages.
pub struct Harness {
    pub server: MockServer,
    pub fake: Fake,
    pub store: Store,
    pub service: SyncService,
}

impl Harness {
    pub async fn new(page_limit: u32) -> Self {
        Self::with_config(SyncConfig::default().page_limit(page_limit)).await
    }

    pub async fn with_config(config: SyncConfig) -> Self {
        let server = MockServer::start().await;
        let fake = Fake::default();
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(fake.clone())
            .mount(&server)
            .await;
        let store = Store::open_in_memory().unwrap();
        let service = SyncService::new(store.clone(), Arc::new(client(&server)), config);
        Self {
            server,
            fake,
            store,
            service,
        }
    }

    pub async fn import(&self, until: Option<i64>) -> ImportProgress {
        self.service.import(USER, until).await.unwrap()
    }

    pub async fn user(&self) -> UserId {
        self.store.user_by_name(USER).await.unwrap().unwrap().id
    }

    /// What the store lists in `range`, newest first: `(uts, name)`.
    pub async fn stored(&self, range: TimeRange) -> Vec<(i64, String)> {
        let listing = self
            .store
            .recent_tracks(self.user().await, range, Page::new(1, 10_000))
            .await
            .unwrap();
        listing
            .items
            .into_iter()
            .map(|r| (r.uts, r.track.name))
            .collect()
    }

    /// Asserts that the store holds exactly what the fake lists in `range`.
    pub async fn assert_mirrors(&self, range: TimeRange) {
        assert_eq!(self.stored(range).await, self.fake.listing(range));
    }
}

/// A client against the mock server that never waits and does not retry.
pub fn client(server: &MockServer) -> Client {
    let config = ClientConfig::new([ApiKey::new("test-key")])
        .base_url(format!("{}/2.0/", server.uri()))
        .requests_per_second(NonZeroU32::new(10_000).unwrap())
        .retry(RetryPolicy {
            max_attempts: 1,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
        });
    Client::new(config).unwrap()
}

/// The current Unix time.
pub fn now() -> i64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    i64::try_from(secs).unwrap()
}

/// Polls `check` every 10 ms until it holds, failing after five seconds.
pub async fn eventually<F, Fut>(mut check: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    for _ in 0..500 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not reached in time");
}
