//! A store loaded from the recorded fixtures of the Last.fm user `syntiiix`, and a way to replay
//! a fixture's request through the dispatcher and compare the reply with the recorded bytes.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::missing_panics_doc,
    reason = "test helpers fail the test the same way the tests do"
)]
#![allow(
    unreachable_pub,
    reason = "shared by test binaries through `mod support`"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use lfm_api::{
    ApiKey, Clock, Dispatcher, InMemoryApiKeys, InMemorySessions, MemoryCache, Scope, Services,
};
use lfm_core::Params;
use lfm_store::{
    Love, Named, Profile, ProfileExtras, Scrobble, Store, TimeRange, TrackDuration, UserId,
};
use serde_json::Value;

/// The key the tests use; fixtures store theirs as `REDACTED`.
pub const KEY: &str = "REDACTED";

/// When the fixtures were recorded: the `user.getInfo` default fixture's `Date` header.
pub const RECORDED: u64 = 1_794_518_863;

/// Reads `fixtures/<path>`.
pub fn read(path: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    std::fs::read_to_string(root.join(path)).unwrap()
}

fn json(path: &str) -> Value {
    serde_json::from_str(&read(path)).unwrap()
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

fn named(name: &Value, mbid: &Value, url: Option<String>) -> Named {
    Named {
        name: text(name),
        mbid: Some(text(mbid)).filter(|m| !m.is_empty()),
        url,
    }
}

/// The 1000 newest scrobbles, from `user.getRecentTracks/limit-1000`, with artist URLs from
/// the `extended=1` page where it has them.
fn scrobbles() -> Vec<Scrobble> {
    let extended = json("user.getRecentTracks/extended-1-limit-200.json");
    let artist_urls: std::collections::HashMap<String, String> = extended["recenttracks"]["track"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (text(&t["artist"]["name"]), text(&t["artist"]["url"])))
        .collect();
    let page = json("user.getRecentTracks/limit-1000.json");
    let mut plays: Vec<Scrobble> = page["recenttracks"]["track"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let artist = text(&t["artist"]["#text"]);
            Scrobble {
                uts: text(&t["date"]["uts"]).parse().unwrap(),
                artist: named(
                    &t["artist"]["#text"],
                    &t["artist"]["mbid"],
                    artist_urls.get(&artist).cloned(),
                ),
                album: Some(named(&t["album"]["#text"], &t["album"]["mbid"], None)),
                track: named(&t["name"], &t["mbid"], Some(text(&t["url"]))),
                image: Some(text(&t["image"][3]["#text"])),
            }
        })
        .collect();
    plays.reverse();
    plays
}

/// Last.fm shows this track's MBID in some recordings and not in others made the same minute
/// (the JSON and XML of `limit-1000` differ, and so do the two weekly track charts), so
/// comparisons ignore it.
const FLAPPING_MBID: &str = "7235d173-cacd-4a9c-ba67-dbbf17d4753c";

fn loves() -> Vec<Love> {
    let page = json("user.getLovedTracks/default.json");
    page["lovedtracks"]["track"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| Love {
            uts: text(&t["date"]["uts"]).parse().unwrap(),
            artist: named(
                &t["artist"]["name"],
                &t["artist"]["mbid"],
                Some(text(&t["artist"]["url"])),
            ),
            track: named(&t["name"], &t["mbid"], Some(text(&t["url"]))),
        })
        .collect()
}

fn durations() -> Vec<TrackDuration> {
    let page = json("user.getTopTracks/period-7day.json");
    page["toptracks"]["track"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| TrackDuration {
            artist: text(&t["artist"]["name"]),
            track: text(&t["name"]),
            seconds: text(&t["duration"]).parse().unwrap(),
        })
        .collect()
}

fn profile() -> (Profile, ProfileExtras) {
    let info = &json("user.getInfo/default.json")["user"];
    let number = |v: &Value| text(v).parse::<i64>().unwrap();
    let profile = Profile {
        realname: text(&info["realname"]),
        url: text(&info["url"]),
        country: text(&info["country"]),
        gender: text(&info["gender"]),
        age: number(&info["age"]),
        subscriber: text(&info["subscriber"]) == "1",
        playlists: number(&info["playlists"]),
        bootstrap: number(&info["bootstrap"]),
        registered: number(&info["registered"]["unixtime"]),
        image: None,
        user_type: text(&info["type"]),
    };
    let extras = ProfileExtras {
        corrections: number(&info["corrections"]),
        spotify_expiry: number(&info["spotify_expiry_estimate"]["unixtime"]),
    };
    (profile, extras)
}

/// Artist MBIDs and URLs as the weekly artist charts and 7-day top artists show them. Recent
/// tracks show an artist's MBID per track, and leave it empty on some, so a mirror needs these
/// lists to know every artist's own MBID.
fn artists() -> Vec<Named> {
    let lists = [
        (
            "user.getWeeklyArtistChart/default.json",
            "weeklyartistchart",
        ),
        (
            "user.getWeeklyArtistChart/range-latest.json",
            "weeklyartistchart",
        ),
        ("user.getTopArtists/period-7day.json", "topartists"),
    ];
    lists
        .iter()
        .flat_map(|(path, root)| json(path)[root]["artist"].as_array().unwrap().clone())
        .map(|a| named(&a["name"], &a["mbid"], Some(text(&a["url"]))))
        .collect()
}

/// An in-memory store holding `Syntiiix` with the fixtures' newest 1000 scrobbles, loved
/// tracks, profile, artist MBIDs and the track lengths of the 7-day top tracks.
pub async fn store() -> (Store, UserId) {
    let store = Store::open_in_memory().unwrap();
    let user = store.ensure_user("Syntiiix").await.unwrap();
    let plays = scrobbles();
    let window = TimeRange::new(plays.first().unwrap().uts, i64::try_from(RECORDED).unwrap());
    store.replace_window(user, window, plays).await.unwrap();
    store.replace_loved(user, loves()).await.unwrap();
    store.enrich_artists(artists()).await.unwrap();
    store.set_track_durations(durations()).await.unwrap();
    let (profile, extras) = profile();
    store.update_profile(user, profile).await.unwrap();
    store.update_profile_extras(user, extras).await.unwrap();
    (store, user)
}

/// A dispatcher over `store` with every method, the test key, and the clock at `RECORDED`.
pub fn dispatcher(store: Store) -> Dispatcher {
    let key = ApiKey {
        account_id: 1,
        secret: "secret".into(),
        scope: Scope::ReadWrite,
        active: true,
    };
    let services = Services {
        store,
        clock: Clock::Fixed(RECORDED),
        api_keys: Arc::new(InMemoryApiKeys::default().with(KEY, key)),
        sessions: Arc::new(InMemorySessions::default()),
        cache: Arc::new(MemoryCache::new(1 << 24, Duration::from_secs(60))),
    };
    Dispatcher::new(lfm_api::methods::registry(), services)
}

/// One recorded exchange.
#[derive(Debug)]
pub struct Recorded {
    /// The request parameters, `api_key` included.
    pub params: Params,
    /// The HTTP status.
    pub status: u16,
    /// The `Content-Type` header.
    pub content_type: String,
    /// The body.
    pub body: String,
}

/// The recorded exchange of `<method>/<variant>` in `format` (`json` or `xml`).
pub fn recorded(method: &str, variant: &str, format: &str) -> Recorded {
    let meta = json(&format!("{method}/{variant}.meta.json"));
    let response = &meta["responses"][format];
    let params = response["params"]
        .as_array()
        .unwrap()
        .iter()
        .fold(Params::new(), |params, pair| {
            params.with(text(&pair[0]), text(&pair[1]))
        });
    Recorded {
        params,
        status: u16::try_from(response["status"].as_u64().unwrap()).unwrap(),
        content_type: text(&response["content_type"]),
        body: read(&format!("{method}/{variant}.{format}")),
    }
}

/// Replays `<method>/<variant>` in both formats and asserts status, content type and body
/// equal the recording after `edit` (which adjusts values that depend on the full history).
pub async fn assert_replays(
    d: &Dispatcher,
    method: &str,
    variant: &str,
    edit: impl Fn(&str) -> String,
) {
    for format in ["json", "xml"] {
        let want = recorded(method, variant, format);
        let reply = d.dispatch(&want.params).await;
        let body = String::from_utf8_lossy(&reply.body);
        let label = format!("{method}/{variant}.{format}");
        let body = body.replace(FLAPPING_MBID, "");
        let expected = edit(&want.body).replace(FLAPPING_MBID, "");
        assert!(
            body == expected,
            "{label}: {}",
            difference(&body, &expected)
        );
        assert_eq!(reply.status, want.status, "{label}");
        assert_eq!(reply.content_type, want.content_type, "{label}");
    }
}

/// Leaves the recording as it is.
pub fn same(body: &str) -> String {
    body.to_owned()
}

/// Where two bodies first differ, with some context, since whole bodies are too long to read.
fn difference(got: &str, want: &str) -> String {
    let (got_chars, want_chars): (Vec<char>, Vec<char>) =
        (got.chars().collect(), want.chars().collect());
    let at = got_chars
        .iter()
        .zip(&want_chars)
        .position(|(a, b)| a != b)
        .unwrap_or(got_chars.len().min(want_chars.len()));
    let around = |chars: &[char]| -> String {
        chars
            .iter()
            .skip(at.saturating_sub(120))
            .take(240)
            .collect()
    };
    format!(
        "lengths {} vs {}\n got: {}\nwant: {}",
        got.len(),
        want.len(),
        around(&got_chars),
        around(&want_chars)
    )
}
