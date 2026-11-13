//! The library map: plays counted under the artists, albums and tracks Last.fm's lists use.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "tests fail loudly on a broken expectation"
)]

mod common;

use common::{DAY, T0, TestDb, assert_aggregates_match, assert_periods_match, scrobble, user};
use lfm_store::{
    Kind, LibAlbum, LibMove, LibTrack, Library, Lookup, Love, Named, Page, Period, Ranked,
    Scrobble, TimeRange, TopQuery, UserId,
};

const NOW: i64 = T0 + 10 * DAY;

fn top(kind: Kind) -> TopQuery {
    TopQuery {
        kind,
        period: Period::Overall,
        now: NOW,
    }
}

fn names(items: &[Ranked]) -> Vec<(String, u64)> {
    items
        .iter()
        .map(|r| (r.entity.name.clone(), r.plays))
        .collect()
}

async fn list(db: &TestDb, user: UserId, query: TopQuery) -> Vec<(String, u64)> {
    let listing = db.store.top(user, query, Page::new(1, 50)).await.unwrap();
    names(&listing.items)
}

async fn raw_id(db: &TestDb, user: UserId, kind: Kind, name: &str) -> i64 {
    let inputs = db.store.library_inputs(user).await.unwrap();
    let all = match kind {
        Kind::Artist => inputs.artists,
        Kind::Album => inputs.albums,
        Kind::Track => inputs.tracks,
    };
    all.iter().find(|e| e.name == name).unwrap().id
}

fn named(name: &str, mbid: &str, url: &str) -> Named {
    Named {
        name: name.to_owned(),
        mbid: Some(mbid.to_owned()),
        url: Some(url.to_owned()),
    }
}

/// `zzz.` three times over three days, `ZZZ` once, `Other` twice; the last day is today.
async fn zzz_history(db: &TestDb) -> UserId {
    let alice = user(&db.store, "alice").await;
    db.store.advance_periods(NOW).await.unwrap();
    let plays = [
        (NOW - 2 * DAY, "zzz.", "t1"),
        (NOW - DAY, "zzz.", "t2"),
        (NOW - 30, "zzz.", "t1"),
        (NOW - 20, "ZZZ", "t3"),
        (T0, "Other", "o1"),
        (T0 + 60, "Other", "o1"),
    ];
    let scrobbles: Vec<Scrobble> = plays
        .iter()
        .map(|(uts, artist, track)| scrobble(*uts, artist, "", track))
        .collect();
    db.store.append_scrobbles(alice, scrobbles).await.unwrap();
    alice
}

async fn correct_zzz(db: &TestDb, alice: UserId) {
    let library = Library {
        artists: vec![named("ZZZ", "m-zzz", "https://www.last.fm/music/ZZZ")],
        moves: vec![LibMove {
            raw: raw_id(db, alice, Kind::Artist, "zzz.").await,
            to: Lookup::Artist { name: "ZZZ".into() },
        }],
        ..Library::default()
    };
    db.store.apply_library(alice, library).await.unwrap();
}

#[tokio::test]
async fn corrected_artists_merge_and_split_back() {
    let db = TestDb::new();
    let alice = zzz_history(&db).await;
    assert_eq!(
        db.store.user(alice).await.unwrap().unwrap().stats.artists,
        3
    );
    correct_zzz(&db, alice).await;

    let listing = db
        .store
        .top(alice, top(Kind::Artist), Page::new(1, 50))
        .await
        .unwrap();
    assert_eq!(
        names(&listing.items),
        [("ZZZ".into(), 4), ("Other".into(), 2)]
    );
    assert_eq!(listing.items[0].entity.mbid.as_deref(), Some("m-zzz"));
    assert_eq!(
        db.store.user(alice).await.unwrap().unwrap().stats.artists,
        2
    );
    let week = TopQuery {
        period: Period::Days7,
        ..top(Kind::Artist)
    };
    assert_eq!(list(&db, alice, week).await, [("ZZZ".into(), 4)]);
    // Raw edges of a range are mapped like the rollups.
    let range = TimeRange::new(NOW - DAY - 10, NOW);
    let chart = db.store.chart(alice, Kind::Artist, range).await.unwrap();
    assert_eq!(names(&chart), [("ZZZ".into(), 3)]);
    // Tracks keep the scrobbled artist.
    let tracks = db
        .store
        .top(alice, top(Kind::Track), Page::new(1, 50))
        .await
        .unwrap();
    let t1 = tracks.items.iter().find(|r| r.entity.name == "t1").unwrap();
    assert_eq!(t1.entity.artist.as_ref().unwrap().name, "zzz.");
    assert_aggregates_match(&db.raw());
    assert_periods_match(&db.raw());

    db.store
        .apply_library(alice, Library::default())
        .await
        .unwrap();
    let back = list(&db, alice, top(Kind::Artist)).await;
    assert_eq!(back[0], ("zzz.".into(), 3));
    assert_eq!(
        db.store.user(alice).await.unwrap().unwrap().stats.artists,
        3
    );
    assert_aggregates_match(&db.raw());
    assert_periods_match(&db.raw());
}

#[tokio::test]
async fn new_scrobbles_follow_the_stored_map() {
    let db = TestDb::new();
    let alice = zzz_history(&db).await;
    correct_zzz(&db, alice).await;
    let more = vec![
        scrobble(NOW - 5, "ZZZ.", "", "t9"),
        scrobble(NOW - 4, "New", "", "n1"),
    ];
    db.store.append_scrobbles(alice, more).await.unwrap();
    let artists = list(&db, alice, top(Kind::Artist)).await;
    assert_eq!(artists[0], ("ZZZ".into(), 5));
    assert!(artists.contains(&("New".into(), 1)));
    // Replacing a window takes the old plays out where they were counted.
    let window = TimeRange::new(NOW - DAY / 2, NOW);
    let again = vec![scrobble(NOW - 30, "zzz.", "", "t1")];
    db.store.replace_window(alice, window, again).await.unwrap();
    assert_eq!(
        list(&db, alice, top(Kind::Artist)).await[0],
        ("ZZZ".into(), 3)
    );
    assert_aggregates_match(&db.raw());
    assert_periods_match(&db.raw());
    let plays = db
        .store
        .user_plays(
            alice,
            Lookup::Artist {
                name: "ZZZ.".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(plays.plays, 3);
}

#[tokio::test]
async fn albums_count_under_their_album_artist() {
    let db = TestDb::new();
    let alice = user(&db.store, "alice").await;
    let plays = vec![
        scrobble(T0, "X", "Comp", "a"),
        scrobble(T0 + 1, "X", "Comp", "a"),
        scrobble(T0 + 2, "Y", "comp", "b"),
    ];
    db.store.append_scrobbles(alice, plays).await.unwrap();
    let cover = "https://lastfm.freetls.fastly.net/i/u/300x300/abc123.jpg";
    let library = Library {
        albums: vec![LibAlbum {
            artist: named(
                "Various Artists",
                "",
                "https://www.last.fm/music/Various+Artists",
            ),
            album: named(
                "Comp",
                "m-comp",
                "https://www.last.fm/music/Various+Artists/Comp",
            ),
            image: Some(cover.into()),
        }],
        moves: vec![
            album_move(raw_id(&db, alice, Kind::Album, "Comp").await),
            album_move(raw_id(&db, alice, Kind::Album, "comp").await),
        ],
        ..Library::default()
    };
    db.store.apply_library(alice, library).await.unwrap();
    let albums = db
        .store
        .top(alice, top(Kind::Album), Page::new(1, 50))
        .await
        .unwrap();
    assert_eq!(albums.total, 1);
    let album = &albums.items[0].entity;
    assert_eq!((album.name.as_str(), albums.items[0].plays), ("Comp", 3));
    assert_eq!(album.artist.as_ref().unwrap().name, "Various Artists");
    assert_eq!(album.artist.as_ref().unwrap().mbid.as_deref(), Some(""));
    assert_eq!(album.image.as_deref(), Some("abc123.jpg"));
    assert_eq!(album.mbid.as_deref(), Some("m-comp"));
    assert_eq!(db.store.user(alice).await.unwrap().unwrap().stats.albums, 1);
    let lookup = Lookup::Album {
        artist: "x".into(),
        name: "comp".into(),
    };
    assert_eq!(db.store.user_plays(alice, lookup).await.unwrap().plays, 3);
    // Recent tracks still show the album as scrobbled.
    let recent = db
        .store
        .recent_tracks(alice, TimeRange::ALL, Page::new(1, 1))
        .await
        .unwrap();
    assert_eq!(recent.items[0].album.as_ref().unwrap().name, "comp");
    assert_aggregates_match(&db.raw());
}

fn album_move(raw: i64) -> LibMove {
    LibMove {
        raw,
        to: Lookup::Album {
            artist: "Various Artists".into(),
            name: "Comp".into(),
        },
    }
}

#[tokio::test]
async fn tracks_show_their_own_artist_spelling_and_library_details() {
    let db = TestDb::new();
    let alice = user(&db.store, "alice").await;
    let mut odd = scrobble(T0 + 1, "twenty one pilots", "", "Drag Path");
    odd.track.url = Some("https://www.last.fm/music/twenty+one+pilots/_/Drag+Path".into());
    let plays = vec![scrobble(T0, "Twenty One Pilots", "", "Heathens"), odd];
    db.store.append_scrobbles(alice, plays).await.unwrap();
    let library = Library {
        tracks: vec![LibTrack {
            artist: named("Twenty One Pilots", "m-top", ""),
            track: Named::new("Heathens"),
            duration: 195,
        }],
        ..Library::default()
    };
    db.store.apply_library(alice, library).await.unwrap();
    let tracks = db
        .store
        .top(alice, top(Kind::Track), Page::new(1, 50))
        .await
        .unwrap();
    let drag = tracks
        .items
        .iter()
        .find(|r| r.entity.name == "Drag Path")
        .unwrap();
    let artist = drag.entity.artist.as_ref().unwrap();
    assert_eq!(artist.name, "twenty one pilots");
    assert_eq!(
        artist.url.as_deref(),
        Some("https://www.last.fm/music/twenty+one+pilots")
    );
    let heathens = tracks
        .items
        .iter()
        .find(|r| r.entity.name == "Heathens")
        .unwrap();
    assert_eq!(heathens.entity.duration, Some(195));
    let artist = heathens.entity.artist.as_ref().unwrap();
    assert_eq!(
        (artist.name.as_str(), artist.mbid.as_deref()),
        ("Twenty One Pilots", Some("m-top"))
    );
}

#[tokio::test]
async fn loved_tracks_in_one_second_keep_lastfms_order() {
    let db = TestDb::new();
    let alice = user(&db.store, "alice").await;
    let love = |uts, track: &str| Love {
        uts,
        artist: Named::new("A"),
        track: Named::new(track),
    };
    // Newest first, as Last.fm lists them; "Zed" and "Alpha" share a second.
    let loves = vec![love(T0 + 9, "Last"), love(T0, "Zed"), love(T0, "Alpha")];
    db.store.replace_loved(alice, loves).await.unwrap();
    db.store.love(alice, love(T0, "New")).await.unwrap();
    let page = db
        .store
        .loved_tracks(alice, Page::new(1, 10))
        .await
        .unwrap();
    let order: Vec<_> = page.items.iter().map(|l| l.track.name.as_str()).collect();
    assert_eq!(order, ["Last", "New", "Zed", "Alpha"]);
}

fn covered(uts: i64, album: &str, cover: &str) -> Scrobble {
    let mut scrobble = scrobble(
        uts,
        "S. Kiyotaka & Omega Tribe",
        album,
        "Glass no Palm Tree",
    );
    scrobble.image = Some(format!(
        "https://lastfm.freetls.fastly.net/i/u/300x300/{cover}.jpg"
    ));
    scrobble
}

fn finale(artist: &str, name: &str) -> LibAlbum {
    LibAlbum {
        artist: Named::new(artist),
        album: Named::new(name),
        image: None,
    }
}

/// Files the `aaa` cover under Kiyotaka Sugiyama and the `bbb` one under the band.
async fn split_finale(db: &TestDb, alice: UserId) -> Library {
    let inputs = db.store.library_inputs(alice).await.unwrap();
    let variant = |image: &str| {
        inputs
            .albums
            .iter()
            .find(|a| a.image.as_deref() == Some(image))
    };
    let to = |artist: &str, name: &str| Lookup::Album {
        artist: artist.into(),
        name: name.into(),
    };
    Library {
        albums: vec![
            finale("Kiyotaka Sugiyama", "First Finale"),
            finale("S. Kiyotaka & Omega Tribe", "FIRST FINALE"),
        ],
        moves: vec![
            LibMove {
                raw: variant("aaa.jpg").unwrap().id,
                to: to("Kiyotaka Sugiyama", "First Finale"),
            },
            LibMove {
                raw: variant("bbb.jpg").unwrap().id,
                to: to("S. Kiyotaka & Omega Tribe", "FIRST FINALE"),
            },
        ],
        ..Library::default()
    }
}

#[tokio::test]
async fn album_spellings_and_covers_can_belong_to_different_albums() {
    let db = TestDb::new();
    let alice = user(&db.store, "alice").await;
    let plays = vec![
        covered(T0, "FIRST FINALE", "bbb"),
        covered(T0 + 1, "First Finale", "aaa"),
        covered(T0 + 2, "First Finale", "aaa"),
    ];
    db.store.append_scrobbles(alice, plays).await.unwrap();
    // Before enrichment the variants count together, under the oldest.
    assert_eq!(
        list(&db, alice, top(Kind::Album)).await,
        [("FIRST FINALE".into(), 3)]
    );
    let recent = db
        .store
        .recent_tracks(alice, TimeRange::ALL, Page::new(1, 3))
        .await
        .unwrap();
    let covers: Vec<_> = recent
        .items
        .iter()
        .map(|r| r.album.as_ref().unwrap().image.clone().unwrap())
        .collect();
    assert_eq!(covers, ["aaa.jpg", "aaa.jpg", "bbb.jpg"]);

    let library = split_finale(&db, alice).await;
    db.store.apply_library(alice, library).await.unwrap();
    let albums = list(&db, alice, top(Kind::Album)).await;
    assert_eq!(
        albums,
        [("First Finale".into(), 2), ("FIRST FINALE".into(), 1)]
    );
    assert_eq!(db.store.user(alice).await.unwrap().unwrap().stats.albums, 2);

    // A cover seen for the first time counts where its spelling counts.
    let more = vec![covered(T0 + 3, "First Finale", "ccc")];
    db.store.append_scrobbles(alice, more).await.unwrap();
    let albums = list(&db, alice, top(Kind::Album)).await;
    assert_eq!(
        albums,
        [("First Finale".into(), 3), ("FIRST FINALE".into(), 1)]
    );
    assert_aggregates_match(&db.raw());
}
