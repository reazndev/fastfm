//! The now-playing item of `user.getRecentTracks`: first on every page and in every range,
//! beyond `limit`, without a date, and never served from the response cache once it changes.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "tests fail on a broken store"
)]

#[allow(dead_code, reason = "this binary uses only part of the shared helpers")]
mod support;

#[path = "support/key_order.rs"]
mod key_order;

use std::sync::{Arc, Mutex};

use lfm_api::{NowPlayingSource, recent_track_of};
use lfm_core::Params;
use lfm_store::{RecentTrack, UserId};
use lfm_upstream::{AlbumRef, ArtistRef, Image, Track};
use serde_json::Value;

/// A now-playing map the test switches on and off.
#[derive(Debug, Default)]
struct Playing(Mutex<Option<RecentTrack>>);

impl NowPlayingSource for Playing {
    fn now_playing(&self, _user: UserId) -> Option<RecentTrack> {
        self.0.lock().unwrap().clone()
    }
}

fn track() -> Track {
    Track {
        artist: ArtistRef {
            name: "Cher".into(),
            mbid: None,
            url: None,
        },
        name: "Believe".into(),
        mbid: None,
        url: "https://www.last.fm/music/Cher/_/Believe".into(),
        album: AlbumRef {
            name: "Believe".into(),
            mbid: None,
        },
        images: vec![Image {
            size: "extralarge".into(),
            url: "https://lastfm.freetls.fastly.net/i/u/300x300/0123abcd.png".into(),
        }],
        loved: false,
    }
}

fn recent(page: &str, extended: &str) -> Params {
    Params::new()
        .with("method", "user.getRecentTracks")
        .with("user", "syntiiix")
        .with("api_key", support::KEY)
        .with("format", "json")
        .with("limit", "2")
        .with("page", page)
        .with("extended", extended)
}

fn tracks(body: &[u8]) -> Vec<Value> {
    let json: Value = serde_json::from_slice(body).unwrap();
    json["recenttracks"]["track"].as_array().unwrap().clone()
}

#[tokio::test]
async fn now_playing_comes_first_on_every_page_and_range() {
    let (store, _) = support::store().await;
    let playing = Arc::new(Playing::default());
    let d = support::dispatcher(store).with_now_playing(Arc::clone(&playing) as _);
    let before = tracks(&d.dispatch(&recent("1", "0")).await.body);
    assert_eq!(before.len(), 2);

    *playing.0.lock().unwrap() = Some(recent_track_of(&track()));
    for extended in ["0", "1"] {
        let items = tracks(&d.dispatch(&recent("1", extended)).await.body);
        assert_eq!(items.len(), 3, "extended={extended}");
        let first = &items[0];
        assert_eq!(first["@attr"]["nowplaying"], "true");
        assert_eq!(first["name"], "Believe");
        assert!(first.get("date").is_none());
        let image = first["image"][3]["#text"].as_str().unwrap();
        assert!(image.ends_with("/300x300/0123abcd.png"), "{image}");
        assert_eq!(items[1]["date"], before[0]["date"]);
    }
    let xml = d.dispatch(&recent("1", "0").with("format", "xml")).await;
    let xml = String::from_utf8_lossy(&xml.body).into_owned();
    assert!(xml.contains("<track nowplaying=\"true\">"), "{xml}");
    let page_2 = tracks(&d.dispatch(&recent("2", "0")).await.body);
    assert_eq!(page_2.len(), 3);
    assert_eq!(page_2[0]["@attr"]["nowplaying"], "true");
    let past = recent("1", "0").with("from", "1").with("to", "2");
    let past = tracks(&d.dispatch(&past).await.body);
    assert_eq!(past.len(), 1);
    assert_eq!(past[0]["@attr"]["nowplaying"], "true");

    *playing.0.lock().unwrap() = None;
    assert_eq!(tracks(&d.dispatch(&recent("1", "0")).await.body), before);
}

#[tokio::test]
async fn now_playing_keys_follow_last_fm_order() {
    let (store, _) = support::store().await;
    let playing = Arc::new(Playing(Mutex::new(Some(recent_track_of(&track())))));
    let d = support::dispatcher(store).with_now_playing(playing);
    let orders = [
        ("0", "artist,streamable,image,mbid,album,name,@attr,url"),
        (
            "1",
            "artist,mbid,name,image,streamable,album,url,@attr,loved",
        ),
    ];
    for (extended, order) in orders {
        let reply = d.dispatch(&recent("1", extended)).await;
        let body = String::from_utf8_lossy(&reply.body);
        let keys = key_order::key_order(&body, r#""track":["#);
        assert_eq!(keys, order, "extended={extended}");
    }
}
