//! Enrichment against a fake Last.fm: corrected artists, albums filed under their album
//! artist, track details, and live scrobbles that follow the learned map.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "tests fail loudly on a broken expectation"
)]

mod support;

use std::time::Duration;

use lfm_store::{Kind, Page, Period, Ranked, TimeRange, TopQuery};
use lfm_sync::SyncConfig;
use serde_json::{Value, json};
use support::{Harness, REGISTERED, USER, eventually, now};

fn artist(name: &str, plays: u64, mbid: &str) -> Value {
    json!({"name": name, "playcount": plays.to_string(), "mbid": mbid,
        "url": format!("https://www.last.fm/music/{}", name.replace(' ', "+")),
        "streamable": "0", "image": []})
}

fn album(artist: &str, name: &str, plays: u64) -> Value {
    json!({"artist": {"name": artist, "mbid": "", "url": "u"}, "name": name, "mbid": "",
        "url": format!("https://www.last.fm/music/{artist}/{name}"),
        "playcount": plays.to_string(),
        "image": [{"size": "extralarge", "#text": "https://img/300x300/abc.jpg"}]})
}

fn track(artist: &str, name: &str, plays: u64, duration: u64) -> Value {
    json!({"artist": {"name": artist, "mbid": "m-track-artist", "url": "u"}, "name": name,
        "mbid": "", "url": "u", "duration": duration.to_string(),
        "playcount": plays.to_string()})
}

/// Tracks by artist and album: `zzz.` that Last.fm corrects to `ZZZ`, a soundtrack it files
/// under `Various Artists`, and a track it files under a longer artist name.
fn catalogue(h: &Harness) {
    let rows = [
        ("t1", "zzz.", "Unreleased"),
        ("t2", "zzz.", "Unreleased"),
        ("t3", "ZZZ", "Unreleased"),
        ("c1", "Juice WRLD", "Spider-Verse"),
        ("c2", "Lil Wayne", "Spider-Verse"),
        ("u1", "Udo Lindenberg", "Horizont"),
    ];
    h.fake.with(|d| {
        for (track, artist, album) in rows {
            let mbid = format!("m-{artist}");
            let entry = (artist.to_owned(), album.to_owned(), mbid);
            d.catalogue.insert(track.to_owned(), entry);
        }
        d.top = [
            vec![
                artist("ZZZ", 4, "m-ZZZ"),
                artist("Juice WRLD", 2, ""),
                artist("Lil' Wayne", 1, ""),
                artist("Udo Lindenberg", 1, ""),
            ],
            vec![
                album("ZZZ", "Unreleased", 4),
                album("Various Artists", "Spider-Verse", 3),
                album("Udo Lindenberg", "Horizont", 1),
            ],
            vec![
                track("zzz.", "t1", 2, 0),
                track("Juice WRLD", "c1", 2, 200),
                track("zzz.", "t2", 1, 0),
                track("ZZZ", "t3", 1, 0),
                track("Lil Wayne", "c2", 1, 0),
                track("Udo Lindenberg & das Panik-Orchester", "u1", 1, 0),
            ],
        ];
        d.corrections.insert("zzz.".into(), "ZZZ".into());
        d.corrections
            .insert("Lil Wayne".into(), "Lil' Wayne".into());
    });
    let plays = ["t1", "t1", "t2", "t3", "c1", "c1", "c2", "u1"];
    let rows: Vec<(i64, &str)> = (0..)
        .zip(plays)
        .map(|(i, t)| (REGISTERED + i * 60, t))
        .collect();
    h.fake.add(&rows);
}

async fn top(h: &Harness, kind: Kind) -> Vec<Ranked> {
    let query = TopQuery {
        kind,
        period: Period::Overall,
        now: now(),
    };
    let user = h.user().await;
    h.store
        .top(user, query, Page::new(1, 50))
        .await
        .unwrap()
        .items
}

fn names(items: &[Ranked]) -> Vec<(String, u64)> {
    items
        .iter()
        .map(|r| (r.entity.name.clone(), r.plays))
        .collect()
}

#[tokio::test]
async fn import_counts_plays_where_lastfm_does() {
    let h = Harness::new(1000).await;
    catalogue(&h);
    h.import(None).await;

    let artists = top(&h, Kind::Artist).await;
    let want = [
        ("ZZZ", 4),
        ("Juice WRLD", 2),
        ("Lil' Wayne", 1),
        ("Udo Lindenberg", 1),
    ];
    assert_eq!(names(&artists), want.map(|(n, p)| (n.to_owned(), p)));
    assert_eq!(artists[0].entity.mbid.as_deref(), Some("m-ZZZ"));
    let albums = top(&h, Kind::Album).await;
    let want = [("Unreleased", 4), ("Spider-Verse", 3), ("Horizont", 1)];
    assert_eq!(names(&albums), want.map(|(n, p)| (n.to_owned(), p)));
    assert_eq!(
        albums[1].entity.artist.as_ref().unwrap().name,
        "Various Artists"
    );
    assert_eq!(albums[1].entity.image.as_deref(), Some("abc.jpg"));
    let tracks = top(&h, Kind::Track).await;
    let c1 = tracks.iter().find(|t| t.entity.name == "c1").unwrap();
    assert_eq!(c1.entity.duration, Some(200));
    let u1 = tracks.iter().find(|t| t.entity.name == "u1").unwrap();
    let u1_artist = &u1.entity.artist.as_ref().unwrap().name;
    assert_eq!(u1_artist, "Udo Lindenberg & das Panik-Orchester");

    let user = h.store.user(h.user().await).await.unwrap().unwrap();
    assert_eq!((user.stats.artists, user.stats.albums), (4, 3));
    assert_eq!(
        (user.extras.corrections, user.extras.spotify_expiry),
        (1, 1_802_563_200)
    );
    assert_eq!(h.fake.calls("artist.getCorrection"), 2);
}

#[tokio::test]
async fn history_is_read_from_plain_pages_with_per_track_artist_mbids() {
    let h = Harness::new(1000).await;
    catalogue(&h);
    h.import(None).await;
    assert!(h.fake.with(|d| d.extended.iter().all(|e| e == "0")));
    let user = h.user().await;
    let recent = h
        .store
        .recent_tracks(user, TimeRange::ALL, Page::new(1, 1))
        .await
        .unwrap();
    let artist = recent.items[0].track.artist.clone().unwrap();
    assert_eq!(artist.name, "Udo Lindenberg");
    assert_eq!(artist.mbid.as_deref(), Some("m-Udo Lindenberg"));
    assert_eq!(
        artist.url.as_deref(),
        Some("https://www.last.fm/music/Udo+Lindenberg")
    );
}

#[tokio::test]
async fn live_scrobbles_follow_the_map_and_enrichment_reuses_corrections() {
    let config = SyncConfig::default()
        .live_intervals(Duration::from_millis(10), Duration::from_millis(30))
        .periods_interval(Duration::from_millis(50));
    let h = Harness::with_config(config).await;
    catalogue(&h);
    h.service.follow(USER).await.unwrap();
    eventually(|| async {
        h.store
            .user_by_name(USER)
            .await
            .unwrap()
            .unwrap()
            .stats
            .artists
            == 4
    })
    .await;

    // A new scrobble of a corrected name counts under the correction at once; an unknown
    // artist counts as itself.
    h.fake.with(|d| {
        d.catalogue.insert(
            "n1".into(),
            ("Brand New".into(), String::new(), String::new()),
        );
    });
    let base = now() + 100;
    h.fake.add(&[(base, "t1"), (base + 1, "n1")]);
    eventually(|| async { top(&h, Kind::Artist).await[0].plays == 5 }).await;
    let artists = names(&top(&h, Kind::Artist).await);
    assert!(
        artists.contains(&("Brand New".to_owned(), 1)),
        "{artists:?}"
    );

    // Stored corrections are not asked again; only the new name is.
    let user = h.user().await;
    let report = h.service.enrich(user).await.unwrap();
    assert_eq!(report.corrections, 1);
    assert_eq!(h.fake.calls("artist.getCorrection"), 3);
    assert_eq!(top(&h, Kind::Artist).await[0].plays, 5);
    h.service.shutdown().await;
}

#[tokio::test]
async fn an_accounts_own_key_is_used_for_its_user() {
    let h = Harness::new(1000).await;
    catalogue(&h);
    h.service
        .use_key("ALICE", Some(lfm_upstream::ApiKey::new("own-key")));
    h.import(None).await;
    let keys: Vec<String> = h
        .server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter_map(|r| {
            let pairs = r.url.query_pairs();
            pairs
                .into_iter()
                .find(|(k, _)| k == "api_key")
                .map(|(_, v)| v.into_owned())
        })
        .collect();
    assert_ne!(keys.len(), 0);
    assert!(keys.iter().all(|k| k == "own-key"), "{keys:?}");

    h.service.use_key(USER, None);
    let user = h.user().await;
    h.service.enrich(user).await.unwrap();
    let last = h.server.received_requests().await.unwrap();
    let pool = last
        .last()
        .unwrap()
        .url
        .query_pairs()
        .any(|(k, v)| k == "api_key" && v == "test-key");
    assert!(pool);
}
