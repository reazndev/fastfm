//! History, top lists, charts, lookups, loved tracks, and the upstream cache.
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test the same way the tests do"
)]

mod common;

use common::{DAY, T0, TestDb, loaded, naive_top, scrobble, user};
use lfm_store::{
    CachedResponse, Kind, Listing, Lookup, Love, Named, Page, Period, Ranked, TimeRange, TopQuery,
    UserId, UserPlays, UserStats,
};
use rusqlite::Connection;

/// The naive OFFSET query that history paging must agree with.
fn naive_history(
    conn: &Connection,
    user: UserId,
    range: TimeRange,
    page: Page,
) -> Vec<(i64, String)> {
    let sql = "SELECT s.uts, t.name FROM scrobbles s JOIN tracks t ON t.id = s.track_id \
               WHERE s.user_id = ?1 AND s.uts >= ?2 AND s.uts < ?3 \
               ORDER BY s.uts DESC, s.seq DESC LIMIT ?4 OFFSET ?5";
    let offset = i64::from(page.page - 1) * i64::from(page.limit);
    let mut stmt = conn.prepare(sql).unwrap();
    let values = rusqlite::params![user.0, range.from, range.to, page.limit, offset];
    stmt.query_map(values, |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn overall(kind: Kind) -> TopQuery {
    TopQuery {
        kind,
        period: Period::Overall,
        now: 0,
    }
}

fn ids(listing: &Listing<Ranked>) -> Vec<(i64, u64)> {
    listing
        .items
        .iter()
        .map(|r| (r.entity.id, r.plays))
        .collect()
}

#[tokio::test]
async fn deep_pages_equal_the_naive_query() {
    let db = TestDb::new();
    let alice = loaded(&db).await;
    let conn = db.raw();
    let ranges = [
        TimeRange::ALL,
        TimeRange::new(T0 + 3 * DAY + 77, T0 + 400 * DAY + 5),
        TimeRange::new(T0 + 10 * DAY, T0 + 10 * DAY + 3600),
    ];
    for range in ranges {
        let total = db.store.count_range(alice, range).await.unwrap();
        for limit in [1, 7, 50, 200, 1000] {
            let pages = total.div_ceil(u64::from(limit));
            let last = u32::try_from(pages).unwrap();
            for page in [
                1,
                2,
                last / 3,
                last / 2,
                last.saturating_sub(1),
                last,
                last + 1,
            ] {
                let page = Page::new(page, limit);
                let got = db.store.recent_tracks(alice, range, page).await.unwrap();
                assert_eq!(got.total, total);
                let got: Vec<_> = got
                    .items
                    .iter()
                    .map(|r| (r.uts, r.track.name.clone()))
                    .collect();
                assert_eq!(
                    got,
                    naive_history(&conn, alice, range, page),
                    "{range:?} {page:?}"
                );
            }
        }
    }
}

#[tokio::test]
async fn top_lists_equal_the_naive_aggregation() {
    let db = TestDb::new();
    let alice = loaded(&db).await;
    let conn = db.raw();
    for kind in Kind::ALL {
        let all = naive_top(&conn, alice, kind, TimeRange::ALL);
        let total = u64::try_from(all.len()).unwrap();
        for (page, limit) in [(1, 5), (2, 5), (3, 4), (1, 1000), (99, 50)] {
            let page = Page::new(page, limit);
            let got = db.store.top(alice, overall(kind), page).await.unwrap();
            let skip = usize::try_from((page.page - 1) * page.limit).unwrap();
            let want: Vec<_> = all
                .iter()
                .copied()
                .skip(skip)
                .take(limit as usize)
                .collect();
            assert_eq!(
                (ids(&got), got.total),
                (want, total),
                "{kind:?} overall {page:?}"
            );
            assert!(
                got.items
                    .iter()
                    .zip(skip as u64 + 1..)
                    .all(|(r, rank)| r.rank == rank)
            );
        }
        for range in [
            TimeRange::new(T0 + 100 * DAY + 5000, T0 + 107 * DAY + 5000),
            TimeRange::new(T0 + 33, T0 + 365 * DAY + 33),
            TimeRange::new(T0 + 3 * DAY, T0 + 3 * DAY + 600),
        ] {
            let all = naive_top(&conn, alice, kind, range);
            for page in [Page::new(1, 10), Page::new(2, 10), Page::new(1, 10_000)] {
                let got = db
                    .store
                    .top_in_range(alice, kind, range, page)
                    .await
                    .unwrap();
                let skip = usize::try_from((page.page - 1) * page.limit).unwrap();
                let want: Vec<_> = all
                    .iter()
                    .copied()
                    .skip(skip)
                    .take(page.limit as usize)
                    .collect();
                assert_eq!(ids(&got), want, "{kind:?} {range:?} {page:?}");
                assert_eq!(got.total, u64::try_from(all.len()).unwrap());
            }
            let chart = db.store.chart(alice, kind, range).await.unwrap();
            let chart: Vec<_> = chart.iter().map(|r| (r.entity.id, r.plays)).collect();
            assert_eq!(chart, all);
        }
    }
}

#[tokio::test]
async fn top_entities_carry_names() {
    let db = TestDb::new();
    let alice = user(&db.store, "alice").await;
    let plays = vec![
        scrobble(T0, "Björk", "Homogenic", "Jóga"),
        scrobble(T0 + 1, "björk", "HOMOGENIC", "JÓGA"),
        scrobble(T0 + 2, "Björk", "Post", "Army of Me"),
    ];
    db.store
        .replace_window(alice, TimeRange::new(T0, T0 + 3), plays)
        .await
        .unwrap();
    let albums = db
        .store
        .top(alice, overall(Kind::Album), Page::new(1, 10))
        .await
        .unwrap();
    let first = albums.items.first().unwrap();
    assert_eq!((first.entity.name.as_str(), first.plays), ("Homogenic", 2));
    assert_eq!(first.entity.artist.as_ref().unwrap().name, "Björk");
    assert_eq!(albums.total, 2);
}

#[tokio::test]
async fn lookups_ignore_case_and_accept_mbids() {
    let db = TestDb::new();
    let alice = user(&db.store, "alice").await;
    let mut play = scrobble(T0, "Mötley Crüe", "Dr. Feelgood", "Kickstart My Heart");
    play.track.mbid = Some("track-mbid".into());
    let plays = vec![
        play.clone(),
        play,
        scrobble(T0 + 9, "STRASSE", "", "Straße"),
    ];
    db.store
        .replace_window(alice, TimeRange::new(T0, T0 + 10), plays)
        .await
        .unwrap();
    let love = Love {
        uts: T0,
        artist: Named::new("MÖTLEY CRÜE"),
        track: Named::new("kickstart my heart"),
    };
    db.store.love(alice, love).await.unwrap();

    let artist = Lookup::Artist {
        name: "mötley crüe".into(),
    };
    assert_eq!(db.store.user_plays(alice, artist).await.unwrap().plays, 2);
    let album = Lookup::Album {
        artist: "MÖTLEY CRÜE".into(),
        name: "dr. feelgood".into(),
    };
    assert_eq!(db.store.user_plays(alice, album).await.unwrap().plays, 2);
    let by_mbid = Lookup::Mbid {
        kind: Kind::Track,
        mbid: "track-mbid".into(),
    };
    let track = db.store.user_plays(alice, by_mbid).await.unwrap();
    assert_eq!((track.plays, track.loved), (2, true));
    let folded = Lookup::Track {
        artist: "straße".into(),
        name: "STRASSE".into(),
    };
    assert_eq!(db.store.user_plays(alice, folded).await.unwrap().plays, 1);
    let unknown = Lookup::Artist {
        name: "nobody".into(),
    };
    assert_eq!(
        db.store.user_plays(alice, unknown).await.unwrap(),
        UserPlays::default()
    );
}

#[tokio::test]
async fn stats_and_counts() {
    let db = TestDb::new();
    let alice = loaded(&db).await;
    let conn = db.raw();
    let stats = db.store.user(alice).await.unwrap().unwrap().stats;
    let count = |sql: &str| -> u64 { conn.query_row(sql, [alice.0], |row| row.get(0)).unwrap() };
    assert_eq!(
        stats.playcount,
        count("SELECT count(*) FROM scrobbles WHERE user_id = ?1")
    );
    let distinct = "SELECT count(DISTINCT album_id) FROM scrobbles WHERE user_id = ?1";
    assert_eq!(stats.albums, count(distinct));
    let range = TimeRange::new(T0 + 12_345, T0 + 30 * DAY + 999);
    let sql = format!(
        "SELECT count(*) FROM scrobbles WHERE user_id = ?1 AND uts >= {} AND uts < {}",
        range.from, range.to
    );
    assert_eq!(
        db.store.count_range(alice, range).await.unwrap(),
        count(&sql)
    );
    let library = db
        .store
        .top(alice, overall(Kind::Artist), Page::new(1, 50))
        .await
        .unwrap();
    assert_eq!(library.total, stats.artists);
    let bob = db.store.user_by_name("BOB").await.unwrap().unwrap();
    assert_eq!(bob.stats, UserStats::default());
}

#[tokio::test]
async fn loved_tracks_replace_add_remove() {
    let db = TestDb::new();
    let alice = user(&db.store, "alice").await;
    let love = |uts, track: &str| Love {
        uts,
        artist: Named::new("A"),
        track: Named::new(track),
    };
    let all = vec![love(T0, "One"), love(T0 + 5, "Two"), love(T0 + 2, "Three")];
    db.store.replace_loved(alice, all).await.unwrap();
    db.store.love(alice, love(T0 + 9, "Four")).await.unwrap();
    assert!(db.store.unlove(alice, "a", "TWO").await.unwrap());
    assert!(!db.store.unlove(alice, "a", "missing").await.unwrap());
    let page = db.store.loved_tracks(alice, Page::new(1, 2)).await.unwrap();
    let names: Vec<_> = page.items.iter().map(|l| l.track.name.as_str()).collect();
    assert_eq!((names, page.total), (vec!["Four", "Three"], 3));
    let page = db.store.loved_tracks(alice, Page::new(2, 2)).await.unwrap();
    assert_eq!(page.items.first().unwrap().track.name, "One");
    db.store
        .replace_loved(alice, vec![love(T0, "Five")])
        .await
        .unwrap();
    assert_eq!(
        db.store
            .loved_tracks(alice, Page::new(1, 50))
            .await
            .unwrap()
            .total,
        1
    );
}

#[tokio::test]
async fn weekly_charts_and_profile() {
    let db = TestDb::new();
    let alice = user(&db.store, "Alice").await;
    assert_eq!(user(&db.store, "ALICE").await, alice);
    let weeks = vec![
        TimeRange::new(T0 + 7 * DAY, T0 + 14 * DAY),
        TimeRange::new(T0, T0 + 7 * DAY),
    ];
    db.store
        .replace_weekly_charts(alice, weeks.clone())
        .await
        .unwrap();
    let stored = db.store.weekly_charts(alice).await.unwrap();
    assert_eq!(stored, vec![weeks[1], weeks[0]]);

    let profile = lfm_store::Profile {
        realname: "Alice A.".into(),
        country: "Iceland".into(),
        registered: T0,
        image: Some("https://lastfm.freetls.fastly.net/i/u/300x300/abcdef.png".into()),
        ..Default::default()
    };
    db.store.update_profile(alice, profile).await.unwrap();
    db.store.set_sync_cursor(alice, T0 + 99).await.unwrap();
    let user = db.store.user_by_name("alice").await.unwrap().unwrap();
    assert_eq!((user.name.as_str(), user.sync_cursor), ("Alice", T0 + 99));
    assert_eq!(user.profile.image.as_deref(), Some("abcdef.png"));
    assert_eq!(user.version, 2);
}

#[tokio::test]
async fn upstream_cache_round_trip() {
    let db = TestDb::new();
    let entry = CachedResponse {
        body: b"<lfm/>".to_vec(),
        status: 200,
        content_type: "text/xml; charset=UTF-8".to_owned(),
        headers: "access-control-allow-origin: *".to_owned(),
        fetched_at: 10,
        expires_at: 20,
    };
    assert_eq!(
        db.store
            .cache_get("artist.getInfo&artist=x", "xml")
            .await
            .unwrap(),
        None
    );
    db.store
        .cache_put("artist.getInfo&artist=x", "xml", entry.clone())
        .await
        .unwrap();
    let got = db
        .store
        .cache_get("artist.getInfo&artist=x", "xml")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, entry);
    assert!(got.is_fresh(19) && !got.is_fresh(20));
    assert_eq!(
        db.store
            .cache_get("artist.getInfo&artist=x", "json")
            .await
            .unwrap(),
        None
    );
    assert_eq!(db.store.cache_purge(15).await.unwrap(), 0);
    assert_eq!(db.store.cache_purge(21).await.unwrap(), 1);
}
