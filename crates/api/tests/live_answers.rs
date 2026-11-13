//! Differences from Last.fm that a comparison of live answers for ten users found
//! (2026-11-13), which the single-user fixtures did not show.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "tests fail on a broken store"
)]

#[allow(dead_code, reason = "this binary uses only part of the shared helpers")]
mod support;

#[path = "support/key_order.rs"]
mod key_order;

use lfm_api::Dispatcher;
use lfm_core::Params;
use lfm_store::{Named, ProfileExtras, Scrobble, Store, TimeRange, UserId};
use serde_json::Value;

const CHER: &str = "https://www.last.fm/music/Cher";

/// The fixture store plus one newer scrobble of Cher without an album, whose artist row holds
/// a `+noredirect` URL as top-artist lists can give it.
async fn with_albumless_scrobble() -> (Store, UserId) {
    let (store, user) = support::store().await;
    let recorded = i64::try_from(support::RECORDED).unwrap();
    let cher = |url: &str| Named {
        name: "Cher".into(),
        mbid: None,
        url: Some(url.into()),
    };
    let play = Scrobble {
        uts: recorded - 1,
        artist: cher(CHER),
        album: None,
        track: Named {
            name: "Believe".into(),
            mbid: None,
            url: Some(format!("{CHER}/_/Believe")),
        },
        image: None,
    };
    let window = TimeRange::new(recorded - 10, recorded);
    store
        .replace_window(user, window, vec![play])
        .await
        .unwrap();
    let noredirect = "https://www.last.fm/music/+noredirect/Cher";
    store.enrich_artists(vec![cher(noredirect)]).await.unwrap();
    (store, user)
}

async fn text(d: &Dispatcher, pairs: &[(&str, &str)]) -> String {
    let params = pairs.iter().fold(
        Params::new()
            .with("user", "syntiiix")
            .with("api_key", support::KEY)
            .with("format", "json"),
        |params, (name, value)| params.with(*name, *value),
    );
    String::from_utf8_lossy(&d.dispatch(&params).await.body).into_owned()
}

async fn json(d: &Dispatcher, pairs: &[(&str, &str)]) -> Value {
    serde_json::from_str(&text(d, pairs).await).unwrap()
}

#[tokio::test]
async fn a_scrobble_without_an_album_has_empty_images() {
    let d = support::dispatcher(with_albumless_scrobble().await.0);
    for extended in ["0", "1"] {
        let query = [
            ("method", "user.getRecentTracks"),
            ("limit", "2"),
            ("extended", extended),
        ];
        let body = json(&d, &query).await;
        let tracks = &body["recenttracks"]["track"];
        assert_eq!(tracks[0]["name"], "Believe");
        for image in tracks[0]["image"].as_array().unwrap() {
            assert_eq!(image["#text"], "", "extended={extended}");
        }
        let with_album = tracks[1]["image"][0]["#text"].as_str().unwrap();
        assert_ne!(with_album, "");
    }
}

#[tokio::test]
async fn artist_url_is_the_track_urls_artist_part() {
    let d = support::dispatcher(with_albumless_scrobble().await.0);
    let query = [
        ("method", "user.getRecentTracks"),
        ("limit", "1"),
        ("extended", "1"),
    ];
    let body = json(&d, &query).await;
    assert_eq!(body["recenttracks"]["track"][0]["artist"]["url"], CHER);
    let query = [
        ("method", "user.getTopTracks"),
        ("period", "7day"),
        ("limit", "1000"),
    ];
    let body = json(&d, &query).await;
    let cher = body["toptracks"]["track"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["artist"]["name"] == "Cher")
        .cloned()
        .unwrap();
    assert_eq!(cher["artist"]["url"], CHER);
}

#[tokio::test]
async fn user_info_without_a_spotify_estimate_leaves_it_out() {
    let (store, user) = support::store().await;
    let extras = ProfileExtras {
        corrections: 1,
        spotify_expiry: 0,
    };
    store.update_profile_extras(user, extras).await.unwrap();
    let d = support::dispatcher(store);
    let body = text(&d, &[("method", "user.getInfo")]).await;
    let order = "playcount,playlists,album_count,bootstrap,url,country,name,artist_count,age,\
                 track_count,subscriber,image,registered,realname,gender,corrections,type";
    assert_eq!(key_order::key_order(&body, r#"{"user":"#), order);
    let xml = Params::new()
        .with("method", "user.getInfo")
        .with("user", "syntiiix")
        .with("api_key", support::KEY);
    let xml = d.dispatch(&xml).await;
    assert!(!String::from_utf8_lossy(&xml.body).contains("spotify"));
}

#[tokio::test]
async fn attr_order_follows_the_number_of_items() {
    let d = support::dispatcher(support::store().await.0);
    let orders = [
        ("3", "user,totalPages,page,total,perPage"),
        ("10", "perPage,totalPages,page,total,user"),
        ("50", "user,totalPages,page,perPage,total"),
        ("200", "perPage,totalPages,page,user,total"),
    ];
    for (limit, order) in orders {
        let query = [("method", "user.getRecentTracks"), ("limit", limit)];
        let keys = key_order::key_order(&text(&d, &query).await, r#""@attr":"#);
        assert_eq!(keys, order, "limit={limit}");
    }
}
