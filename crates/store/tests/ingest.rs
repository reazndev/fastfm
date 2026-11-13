//! Window replace, append, and aggregate consistency.

mod common;

use common::{Rng, TestDb, assert_aggregates_match, random_scrobbles, scrobble, user};
use lfm_store::{Page, StoreError, TimeRange};

const DAY: i64 = 86_400;
const T0: i64 = 1_700_000_000;

#[tokio::test]
async fn replace_is_idempotent() {
    let db = TestDb::new();
    let alice = user(&db.store, "alice").await;
    let window = TimeRange::new(T0, T0 + 10 * DAY);
    let plays = random_scrobbles(&mut Rng(1), window.from, window.to, 300);
    db.store
        .replace_window(alice, window, plays.clone())
        .await
        .unwrap();
    let first = db
        .store
        .recent_tracks(alice, window, Page::new(1, 1000))
        .await
        .unwrap();
    let version = db.store.user(alice).await.unwrap().unwrap().version;

    db.store.replace_window(alice, window, plays).await.unwrap();
    let second = db
        .store
        .recent_tracks(alice, window, Page::new(1, 1000))
        .await
        .unwrap();
    assert_eq!(first, second);
    let after = db.store.user(alice).await.unwrap().unwrap();
    assert_eq!(
        after.version, version,
        "an unchanged window keeps the version"
    );
    assert_eq!(after.stats.playcount, first.total);
    assert_aggregates_match(&db.raw());
}

#[tokio::test]
async fn same_second_duplicates_survive() {
    let db = TestDb::new();
    let alice = user(&db.store, "alice").await;
    let window = TimeRange::new(T0, T0 + 100);
    let plays = vec![
        scrobble(T0 + 5, "A", "X", "One"),
        scrobble(T0 + 5, "A", "X", "One"),
        scrobble(T0 + 5, "B", "", "Two"),
    ];
    db.store
        .replace_window(alice, window, plays.clone())
        .await
        .unwrap();
    db.store.replace_window(alice, window, plays).await.unwrap();
    let page = db
        .store
        .recent_tracks(alice, window, Page::new(1, 50))
        .await
        .unwrap();
    assert_eq!(page.total, 3);
    let names: Vec<_> = page.items.iter().map(|r| r.track.name.as_str()).collect();
    assert_eq!(
        names,
        ["Two", "One", "One"],
        "newest first: later seq first"
    );
    assert!(page.items.iter().all(|r| r.uts == T0 + 5));
    assert!(page.items.first().unwrap().album.is_none());
}

#[tokio::test]
async fn replace_rejects_scrobbles_outside_the_window() {
    let db = TestDb::new();
    let alice = user(&db.store, "alice").await;
    let window = TimeRange::new(T0, T0 + 10);
    let error = db
        .store
        .replace_window(alice, window, vec![scrobble(T0 + 10, "A", "", "T")])
        .await
        .unwrap_err();
    assert!(matches!(error, StoreError::OutsideWindow { uts, .. } if uts == T0 + 10));
    let stats = db.store.user(alice).await.unwrap().unwrap().stats;
    assert_eq!(
        stats.playcount, 0,
        "the failed transaction left nothing behind"
    );
}

#[tokio::test]
async fn unknown_user_is_not_found() {
    let db = TestDb::new();
    let window = TimeRange::new(T0, T0 + 10);
    let result = db
        .store
        .replace_window(
            lfm_store::UserId(99),
            window,
            vec![scrobble(T0, "A", "", "T")],
        )
        .await;
    assert!(matches!(result, Err(StoreError::NotFound)));
}

#[tokio::test]
async fn aggregates_match_recount_after_random_replaces() {
    let db = TestDb::new();
    let alice = user(&db.store, "alice").await;
    let bob = user(&db.store, "bob").await;
    let mut rng = Rng(7);
    let span = 400 * DAY;
    for round in 0..40 {
        let who = if round % 3 == 0 { bob } else { alice };
        let from = T0 + i64::try_from(rng.below(u64::try_from(span).unwrap())).unwrap();
        let len = [3600, DAY, 9 * DAY, 70 * DAY][usize::try_from(rng.below(4)).unwrap()];
        let window = TimeRange::new(from, from + len);
        let count = usize::try_from(rng.below(120)).unwrap();
        let plays = random_scrobbles(&mut rng, window.from, window.to, count);
        db.store.replace_window(who, window, plays).await.unwrap();
    }
    assert_aggregates_match(&db.raw());
}

#[tokio::test]
async fn append_numbers_after_existing_rows() {
    let db = TestDb::new();
    let alice = user(&db.store, "alice").await;
    let window = TimeRange::new(T0, T0 + 100);
    db.store
        .replace_window(alice, window, vec![scrobble(T0, "A", "X", "One")])
        .await
        .unwrap();
    let before = db.store.user(alice).await.unwrap().unwrap().version;
    let live = vec![
        scrobble(T0, "A", "X", "One"),
        // Titles match ignoring case; artist spellings that differ in case would not.
        scrobble(T0 + 50, "A", "x", "ONE"),
    ];
    db.store.append_scrobbles(alice, live).await.unwrap();

    let after = db.store.user(alice).await.unwrap().unwrap();
    assert!(after.version > before);
    assert_eq!(after.stats.playcount, 3);
    assert_eq!(
        (after.stats.artists, after.stats.albums, after.stats.tracks),
        (1, 1, 1)
    );
    let page = db
        .store
        .recent_tracks(alice, window, Page::new(1, 10))
        .await
        .unwrap();
    assert_eq!(
        page.items.first().unwrap().track.name,
        "One",
        "first spelling is kept"
    );
    assert_aggregates_match(&db.raw());
}

#[tokio::test]
async fn names_mbids_and_images_are_kept() {
    let db = TestDb::new();
    let alice = user(&db.store, "alice").await;
    let mut play = scrobble(T0, "Sigur Rós", "Ágætis byrjun", "Svefn-g-englar");
    play.artist.mbid = Some("f6f2326f-6b25-4170-b89d-e235b25508e8".into());
    play.image = Some(
        "https://lastfm.freetls.fastly.net/i/u/300x300/2a96cbd8b46e442fc41c2b86b821562f.jpg".into(),
    );
    play.track.mbid = Some(String::new());
    db.store
        .replace_window(alice, TimeRange::new(T0, T0 + 1), vec![play])
        .await
        .unwrap();
    let page = db
        .store
        .recent_tracks(alice, TimeRange::ALL, Page::new(1, 1))
        .await
        .unwrap();
    let row = page.items.first().unwrap();
    let artist = row.track.artist.as_ref().unwrap();
    assert_eq!(artist.name, "Sigur Rós");
    assert_eq!(
        artist.mbid.as_deref(),
        Some("f6f2326f-6b25-4170-b89d-e235b25508e8")
    );
    assert_eq!(row.track.mbid, None, "an empty mbid is stored as none");
    let album = row.album.as_ref().unwrap();
    assert_eq!(
        album.image.as_deref(),
        Some("2a96cbd8b46e442fc41c2b86b821562f.jpg")
    );
}

#[tokio::test]
async fn in_memory_store_works() {
    let store = lfm_store::Store::open_in_memory().unwrap();
    let alice = user(&store, "alice").await;
    let plays = vec![scrobble(T0, "A", "", "T"), scrobble(T0 + 1, "B", "", "U")];
    store
        .replace_window(alice, TimeRange::new(T0, T0 + 2), plays)
        .await
        .unwrap();
    assert_eq!(store.count_range(alice, TimeRange::ALL).await.unwrap(), 2);
}

#[tokio::test]
async fn a_remembered_user_is_read_again_after_a_write() {
    let db = TestDb::new();
    let alice = user(&db.store, "alice").await;
    let by_name = || async { db.store.user_by_name("Alice").await.unwrap().unwrap() };
    let before = by_name().await;
    assert_eq!(by_name().await, before, "a repeat is answered from memory");
    let window = TimeRange::new(T0, T0 + DAY);
    let plays = random_scrobbles(&mut Rng(2), window.from, window.to, 10);
    db.store.replace_window(alice, window, plays).await.unwrap();
    let after = by_name().await;
    assert!(after.version > before.version);
    assert!(after.stats.playcount > before.stats.playcount);
    assert_eq!(db.store.user_by_name("nobody").await.unwrap(), None);
    db.store.ensure_user("nobody").await.unwrap();
    assert!(db.store.user_by_name("nobody").await.unwrap().is_some());
}

/// Last.fm lists `João Bosco & Vinícius / Chora, me liga - Ao vivo` (2 plays) and
/// `JOÃO BOSCO & VINÍCIUS / Chora, me liga - Ao vivo` (1 play) as two tracks of one artist.
#[tokio::test]
async fn tracks_keep_artist_spellings_that_differ_in_case_apart() {
    let db = TestDb::new();
    let alice = user(&db.store, "alice").await;
    let title = "Chora, me liga - Ao vivo";
    let plays = vec![
        scrobble(T0 + 3, "JOÃO BOSCO & VINÍCIUS", "Curtição (Ao Vivo)", title),
        scrobble(T0 + 2, "João Bosco & Vinícius", "Curtição (Ao Vivo)", title),
        scrobble(
            T0 + 1,
            "João Bosco & Vinícius",
            "Curtição (Ao Vivo)",
            "CHORA, ME LIGA - AO VIVO",
        ),
    ];
    let window = TimeRange::new(T0, T0 + 10);
    db.store.replace_window(alice, window, plays).await.unwrap();
    let stats = db.store.user(alice).await.unwrap().unwrap().stats;
    assert_eq!((stats.artists, stats.albums, stats.tracks), (1, 1, 2));
    let query = lfm_store::TopQuery {
        kind: lfm_store::Kind::Track,
        period: lfm_store::Period::Overall,
        now: T0 + 10,
    };
    let top = db.store.top(alice, query, Page::new(1, 10)).await.unwrap();
    let rows: Vec<(String, u64)> = top
        .items
        .iter()
        .map(|r| (r.entity.artist.clone().unwrap().name, r.plays))
        .collect();
    assert_eq!(
        rows,
        [
            ("João Bosco & Vinícius".to_owned(), 2),
            ("JOÃO BOSCO & VINÍCIUS".to_owned(), 1)
        ]
    );
    assert_aggregates_match(&db.raw());
}
