//! The speed comparison against a temporary store, with Last.fm mocked and the mirror's
//! `/2.0/` served over loopback.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "tests")]

mod support;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lfm_core::Params;
use lfm_store::{Named, Scrobble, TimeRange};
use serde_json::{Value, json};
use support::mock::{mount_get, ok};
use support::{DEMO_MAX_SCROBBLES, Harness, INTERNAL_KEY, LASTFM_KEY, Reply};
use wiremock::matchers::{any, query_param};
use wiremock::{Mock, ResponseTemplate};

const DAY: i64 = 86_400;

fn now() -> i64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    i64::try_from(secs).unwrap()
}

/// Mirrors `name` with 450 scrobbles, one every ten minutes, and a weekly chart list.
async fn seed(h: &Harness, name: &str) {
    let user = h.store.ensure_user(name).await.unwrap();
    let now = now();
    let scrobbles = (0..450)
        .map(|i| Scrobble {
            uts: now - 3600 - i * 600,
            artist: Named::new("Cher"),
            album: Some(Named::new("Believe")),
            track: Named::new(format!("Song {}", i % 7)),
            image: None,
        })
        .collect();
    let store = &h.store;
    store
        .replace_window(user, TimeRange::new(0, now), scrobbles)
        .await
        .unwrap();
    let weeks = (1..5)
        .map(|w| TimeRange::new(now - (w + 1) * 7 * DAY, now - w * 7 * DAY))
        .collect();
    store.replace_weekly_charts(user, weeks).await.unwrap();
    store.set_sync_cursor(user, now).await.unwrap();
}

/// Answers every Last.fm request that no other mock matches.
async fn mount_fallback(h: &Harness) {
    Mock::given(any())
        .respond_with(ok(r#"{"other":"answer"}"#))
        .with_priority(10)
        .mount(&h.lastfm)
        .await;
}

/// A profile with this name and play count, and an empty history.
async fn mount_lastfm_user(h: &Harness, name: &str, playcount: u64) {
    let info = json!({"user": {"name": name, "playcount": playcount.to_string(),
        "registered": {"unixtime": "1628951940", "#text": 1_628_951_940},
        "type": "user", "image": []}});
    mount_get(h, "user.getInfo", &info.to_string()).await;
    let attr = json!({"user": name, "page": "1", "perPage": "50", "totalPages": "0", "total": "0"});
    let lists = [
        ("user.getRecentTracks", "recenttracks", "track"),
        ("user.getLovedTracks", "lovedtracks", "track"),
        ("user.getTopArtists", "topartists", "artist"),
        ("user.getTopAlbums", "topalbums", "album"),
        ("user.getTopTracks", "toptracks", "track"),
    ];
    for (method, root, item) in lists {
        let body = json!({ root: { item: [], "@attr": attr } });
        mount_get(h, method, &body.to_string()).await;
    }
    let charts = json!({ "weeklychartlist": { "chart": [], "@attr": { "user": name } } });
    mount_get(h, "user.getWeeklyChartList", &charts.to_string()).await;
}

async fn post(h: &Harness, cookie: &str, path: &str, body: Value) -> Reply {
    h.call("POST", path).cookie(cookie).json(body).send().await
}

fn assert_no_keys(replies: &[&Reply]) {
    for reply in replies {
        let text = format!("{:?} {}", reply.headers, reply.json);
        assert!(
            !text.contains(LASTFM_KEY) && !text.contains(INTERNAL_KEY),
            "{text}"
        );
    }
}

#[tokio::test]
async fn a_mirrored_user_runs_every_scenario_in_turn() {
    let h = Harness::with_demo().await;
    let cookie = h.signup("alice").await;
    seed(&h, "LfmAlice").await;
    let params = Params::new()
        .with("method", "user.getInfo")
        .with("user", "LfmAlice")
        .with("api_key", INTERNAL_KEY)
        .with("format", "json");
    let profile = h.dispatcher.dispatch(&params).await.body;
    Mock::given(query_param("method", "user.getInfo"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(profile.to_vec(), "application/json"))
        .mount(&h.lastfm)
        .await;
    mount_fallback(&h).await;

    let prepared = post(
        &h,
        &cookie,
        "/app/api/demo/prepare",
        json!({"username": "lfmalice"}),
    )
    .await;
    assert_eq!(prepared.status, 200, "{}", prepared.json);
    assert_eq!(prepared.json["state"], "mirrored");
    assert_eq!(prepared.json["username"], "LfmAlice");
    let scenarios = prepared.json["scenarios"].as_array().unwrap().clone();
    assert_eq!(scenarios.len(), 10);

    let mut replies = vec![prepared];
    for scenario in &scenarios {
        let body = json!({"username": "LfmAlice", "scenario": scenario["id"]});
        let run = post(&h, &cookie, "/app/api/demo/run", body).await;
        assert_eq!(run.status, 200, "{}", run.json);
        assert_eq!(run.json["mirrorSource"], "local");
        assert_eq!(run.json["requests"], scenario["requests"]);
        assert!(run.json["lastfm"]["ok"].as_bool().unwrap());
        assert!(run.json["mirror"]["firstMs"].as_f64().unwrap() > 0.0);
        assert!(run.json["mirror"]["cachedMs"].as_f64().unwrap() > 0.0);
        let identical = run.json["identical"].as_bool().unwrap();
        assert_eq!(identical, scenario["id"] == "profile", "{}", run.json);
        replies.push(run);
    }
    assert_no_keys(&replies.iter().collect::<Vec<_>>());

    // One warm-up, then every request once: 8 single ones, 10 pages and 2 pages.
    let calls = h.lastfm.received_requests().await.unwrap();
    assert_eq!(calls.len(), 21);
    for call in &calls {
        let pairs: Vec<(String, String)> = call.url.query_pairs().into_owned().collect();
        assert!(
            pairs.contains(&("api_key".into(), LASTFM_KEY.into())),
            "{pairs:?}"
        );
        assert!(pairs.contains(&("format".into(), "json".into())));
    }
}

#[tokio::test]
async fn an_unknown_last_fm_user_gets_a_clear_error() {
    let h = Harness::with_demo().await;
    let cookie = h.signup("alice").await;
    let missing = json!({"error": 6, "message": "User not found", "links": []});
    Mock::given(query_param("method", "user.getInfo"))
        .respond_with(ResponseTemplate::new(404).set_body_json(missing))
        .mount(&h.lastfm)
        .await;
    let reply = post(
        &h,
        &cookie,
        "/app/api/demo/prepare",
        json!({"username": "nobody"}),
    )
    .await;
    assert_eq!((reply.status, reply.code()), (404, "lastfm_user_not_found"));
    let reply = post(
        &h,
        &cookie,
        "/app/api/demo/import",
        json!({"username": "nobody"}),
    )
    .await;
    assert_eq!((reply.status, reply.code()), (404, "lastfm_user_not_found"));
}

#[tokio::test]
async fn a_user_that_is_not_mirrored_is_imported_then_compared() {
    let h = Harness::with_demo().await;
    let cookie = h.signup("alice").await;
    mount_lastfm_user(&h, "LfmBob", 0).await;
    mount_fallback(&h).await;
    let user = json!({"username": "lfmbob"});

    let prepared = post(&h, &cookie, "/app/api/demo/prepare", user.clone()).await;
    assert_eq!(prepared.status, 200, "{}", prepared.json);
    assert_eq!(prepared.json["state"], "not_mirrored");
    assert_eq!(prepared.json["username"], "LfmBob");
    let run = json!({"username": "lfmbob", "scenario": "latest"});
    let early = post(&h, &cookie, "/app/api/demo/run", run.clone()).await;
    assert_eq!((early.status, early.code()), (409, "not_mirrored"));

    let started = post(&h, &cookie, "/app/api/demo/import", user.clone()).await;
    assert_eq!(started.status, 200, "{}", started.json);
    let mut progress = started;
    for _ in 0..100 {
        progress = h
            .call("GET", "/app/api/demo/import?username=lfmbob")
            .cookie(&cookie)
            .send()
            .await;
        if progress.json["status"] == "done" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(progress.json["status"], "done", "{}", progress.json);

    let prepared = post(&h, &cookie, "/app/api/demo/prepare", user).await;
    assert_eq!(prepared.json["state"], "mirrored");
    let run = post(&h, &cookie, "/app/api/demo/run", run).await;
    assert_eq!(run.status, 200, "{}", run.json);
    assert_eq!(run.json["mirrorSource"], "local");
    assert_no_keys(&[&prepared, &run, &progress]);
}

#[tokio::test]
async fn histories_over_the_cap_are_not_imported() {
    let h = Harness::with_demo().await;
    let cookie = h.signup("alice").await;
    mount_lastfm_user(&h, "LfmBig", DEMO_MAX_SCROBBLES + 1).await;
    let reply = post(
        &h,
        &cookie,
        "/app/api/demo/import",
        json!({"username": "lfmbig"}),
    )
    .await;
    assert_eq!((reply.status, reply.code()), (422, "history_too_large"));
    assert!(h.store.user_by_name("LfmBig").await.unwrap().is_none());
}

#[tokio::test]
async fn only_active_accounts_on_a_configured_instance_may_compare() {
    let h = Harness::with_demo().await;
    let user = json!({"username": "lfmalice"});
    let anonymous = h
        .call("POST", "/app/api/demo/prepare")
        .json(user.clone())
        .send()
        .await;
    assert_eq!((anonymous.status, anonymous.code()), (401, "not_logged_in"));
    h.signup("alice").await;
    let pending = h.signup("bob").await;
    for path in [
        "/app/api/demo/prepare",
        "/app/api/demo/import",
        "/app/api/demo/run",
    ] {
        let body = json!({"username": "lfmalice", "scenario": "latest"});
        let reply = post(&h, &pending, path, body).await;
        assert_eq!(
            (reply.status, reply.code()),
            (403, "account_pending"),
            "{path}"
        );
    }
    let progress = h
        .call("GET", "/app/api/demo/import?username=lfmalice")
        .cookie(&pending)
        .send()
        .await;
    assert_eq!(progress.code(), "account_pending");

    let plain = Harness::new().await;
    let cookie = plain.signup("carol").await;
    let reply = post(&plain, &cookie, "/app/api/demo/prepare", user).await;
    assert_eq!((reply.status, reply.code()), (503, "demo_unavailable"));
}

#[tokio::test]
async fn race_pages_come_from_either_side_trimmed_to_the_tail() {
    let h = Harness::with_demo().await;
    let cookie = h.signup("alice").await;
    seed(&h, "LfmAlice").await;
    let tracks: Vec<Value> = (0..5)
        .map(|i| {
            json!({"name": format!("Song {i}"), "artist": {"#text": "Cher"}, "image": [],
            "date": {"uts": (1000 - i).to_string(), "#text": ""}})
        })
        .collect();
    let attr = json!({"user": "LfmAlice", "page": "2", "perPage": "200",
        "totalPages": "9", "total": "1700"});
    let body = json!({"recenttracks": {"track": tracks, "@attr": attr}});
    mount_get(&h, "user.getRecentTracks", &body.to_string()).await;
    let race = |side: &str| json!({"username": "lfmalice", "side": side, "page": 2, "tail": 3});

    let lastfm = post(&h, &cookie, "/app/api/demo/race", race("lastfm")).await;
    assert_eq!(lastfm.status, 200, "{}", lastfm.json);
    assert_eq!(
        (&lastfm.json["count"], &lastfm.json["total"]),
        (&json!(5), &json!(1700))
    );
    assert_eq!(lastfm.json["tracks"][0]["name"], "Song 2");
    assert_eq!(lastfm.json["tracks"].as_array().unwrap().len(), 3);

    let mirror = post(&h, &cookie, "/app/api/demo/race", race("mirror")).await;
    assert_eq!(mirror.status, 200, "{}", mirror.json);
    assert_eq!(
        (&mirror.json["count"], &mirror.json["total"]),
        (&json!(200), &json!(450))
    );
    assert_eq!(mirror.json["totalPages"], 3);
    assert_eq!(mirror.json["tracks"].as_array().unwrap().len(), 3);
    assert_no_keys(&[&lastfm, &mirror]);

    let missing = json!({"username": "nobody", "side": "mirror", "page": 1, "tail": 3});
    let reply = post(&h, &cookie, "/app/api/demo/race", missing).await;
    assert_eq!((reply.status, reply.code()), (409, "not_mirrored"));
}
