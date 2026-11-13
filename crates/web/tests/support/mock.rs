//! Last.fm as `wiremock` answers it for the write-through tests.

use std::collections::HashMap;

use lfm_core::{Params, api_signature};
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, query_param};
use wiremock::{Mock, ResponseTemplate};

use super::{Harness, LASTFM_KEY, LASTFM_SECRET};

pub fn ok(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.as_bytes().to_vec(), "application/json")
}

pub async fn mount_get(h: &Harness, method: &str, body: &str) {
    Mock::given(query_param("method", method))
        .respond_with(ok(body))
        .mount(&h.lastfm)
        .await;
}

pub async fn mount_post(h: &Harness, method: &str, response: ResponseTemplate, priority: u8) {
    Mock::given(body_string_contains(format!("method={method}&")))
        .respond_with(response)
        .with_priority(priority)
        .mount(&h.lastfm)
        .await;
}

/// The reads that following `LfmAlice` makes, answered with an empty profile.
pub async fn mount_profile(h: &Harness) {
    let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures");
    let fixtures = format!("{fixtures}/user.getInfo/default.json");
    let info = std::fs::read_to_string(fixtures)
        .unwrap()
        .replace("Syntiiix", "LfmAlice");
    mount_get(h, "user.getInfo", &info).await;
    let attr = json!({
        "user": "LfmAlice", "page": "1", "perPage": "50", "totalPages": "0", "total": "0"
    });
    let recent = json!({ "recenttracks": { "track": [], "@attr": attr } });
    mount_get(h, "user.getRecentTracks", &recent.to_string()).await;
    let loved = json!({ "lovedtracks": { "track": [], "@attr": attr } });
    mount_get(h, "user.getLovedTracks", &loved.to_string()).await;
    let charts = json!({ "weeklychartlist": { "chart": [], "@attr": { "user": "LfmAlice" } } });
    mount_get(h, "user.getWeeklyChartList", &charts.to_string()).await;
}

/// Asserts that a call Last.fm received carries a valid signature from the real secret.
pub fn assert_signed(call: &HashMap<String, String>) {
    let params = call
        .iter()
        .filter(|(name, _)| !["api_sig", "format"].contains(&name.as_str()))
        .fold(Params::new(), |p, (n, v)| p.with(n.as_str(), v.as_str()));
    assert_eq!(
        call["api_sig"],
        api_signature(&params, LASTFM_SECRET),
        "{call:?}"
    );
    assert_eq!(call["api_key"], LASTFM_KEY);
}

/// One scrobble as Last.fm reports it.
pub fn entry(artist: (&str, &str), track: (&str, &str), ignored: (&str, &str)) -> Value {
    json!({
        "artist": { "corrected": artist.1, "#text": artist.0 },
        "album": { "corrected": "0" },
        "track": { "corrected": track.1, "#text": track.0 },
        "ignoredMessage": { "code": ignored.0, "#text": ignored.1 },
        "albumArtist": { "corrected": "0", "#text": "" },
        "timestamp": "1"
    })
}

pub fn scrobbles(entries: &Value, accepted: u32, ignored: u32) -> ResponseTemplate {
    let body = json!({
        "scrobbles": { "scrobble": entries, "@attr": { "ignored": ignored, "accepted": accepted } }
    });
    ok(&body.to_string())
}

pub fn accepted() -> ResponseTemplate {
    scrobbles(&entry(("Cher", "0"), ("Believe", "0"), ("0", "")), 1, 0)
}
