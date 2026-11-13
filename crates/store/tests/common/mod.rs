//! Helpers shared by the store's integration tests.
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test the same way the tests do"
)]
#![allow(dead_code, reason = "each test binary uses a different subset")]
#![allow(
    unreachable_pub,
    reason = "shared by several test binaries through `mod common`"
)]

use std::path::PathBuf;

use lfm_store::{Kind, Named, Scrobble, Store, TimeRange, UserId, collation_key};
use rusqlite::Connection;
use tempfile::TempDir;

pub const DAY: i64 = 86_400;
pub const T0: i64 = 1_650_000_000;

/// A store in a temporary directory, with a raw connection for checking results.
pub struct TestDb {
    pub store: Store,
    pub dir: TempDir,
}

impl TestDb {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_with_readers(&dir.path().join("test.db"), 2).unwrap();
        Self { store, dir }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.path().join("test.db")
    }

    /// A raw read connection for naive reference queries.
    pub fn raw(&self) -> Connection {
        Connection::open(self.path()).unwrap()
    }
}

/// Deterministic pseudo-random numbers (`SplitMix64`).
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A scrobble of `track` by `artist` on `album`.
pub fn scrobble(uts: i64, artist: &str, album: &str, track: &str) -> Scrobble {
    Scrobble {
        uts,
        artist: Named::new(artist),
        album: (!album.is_empty()).then(|| Named::new(album)),
        track: Named::new(track),
        image: None,
    }
}

/// Random scrobbles in `[from, to)` from a small catalogue, with frequent same-second plays.
pub fn random_scrobbles(rng: &mut Rng, from: i64, to: i64, count: usize) -> Vec<Scrobble> {
    let span = u64::try_from(to - from).unwrap();
    let mut out: Vec<Scrobble> = (0..count)
        .map(|_| {
            let uts = from + i64::try_from(rng.below(span)).unwrap();
            let artist = rng.below(12);
            let album = rng.below(3);
            let track = rng.below(5);
            let album = if album == 0 {
                String::new()
            } else {
                format!("Album {artist}-{album}")
            };
            scrobble(
                uts,
                &format!("Artist {artist}"),
                &album,
                &format!("Track {artist}-{track}"),
            )
        })
        .collect();
    // Same-second duplicates.
    let copies: Vec<Scrobble> = out.iter().step_by(7).cloned().collect();
    out.extend(copies);
    out.sort_by_key(|s| std::cmp::Reverse(s.uts));
    out
}

pub async fn user(store: &Store, name: &str) -> UserId {
    store.ensure_user(name).await.unwrap()
}

/// Asserts that every aggregate equals a fresh recount from the scrobbles table.
pub fn assert_aggregates_match(conn: &Connection) {
    for kind in ["artist", "album", "track"] {
        let id = lib_id(kind);
        let total = format!(
            "SELECT user_id, {id}, count(*) FROM scrobbles s WHERE {kind}_id IS NOT NULL \
             GROUP BY 1, 2"
        );
        assert_same(
            conn,
            &format!("SELECT user_id, {kind}_id, plays FROM user_{kind}_total WHERE period = 0"),
            &total,
        );
        let tie = format!(
            "SELECT user_id, period, plays, {kind}_id / 4096, count(*) FROM user_{kind}_total \
             GROUP BY 1, 2, 3, 4"
        );
        let stored = format!("SELECT user_id, period, plays, block, n FROM user_{kind}_tie");
        assert_same(conn, &stored, &tie);
        let hist = format!(
            "SELECT user_id, period, plays, count(*) FROM user_{kind}_total GROUP BY 1, 2, 3"
        );
        assert_same(
            conn,
            &format!("SELECT user_id, period, plays, n FROM user_{kind}_hist"),
            &hist,
        );
        for (level, days) in [(0, 1), (1, 8), (2, 64)] {
            let recount = format!(
                "SELECT user_id, {level}, uts / ({days} * 86400), {id}, count(*) \
                 FROM scrobbles s WHERE {kind}_id IS NOT NULL GROUP BY 1, 3, 4"
            );
            let stored = format!(
                "SELECT user_id, lvl, bucket, {kind}_id, plays FROM user_{kind}_rollup \
                 WHERE lvl = {level}"
            );
            assert_same(conn, &stored, &recount);
        }
        let distinct = format!(
            "SELECT id, (SELECT count(*) FROM user_{kind}_total t \
             WHERE t.user_id = users.id AND t.period = 0) \
             FROM users"
        );
        assert_same(
            conn,
            &format!("SELECT id, {kind}_count FROM users"),
            &distinct,
        );
    }
    for (level, days) in [(0, 1), (1, 8), (2, 64)] {
        let recount = format!(
            "SELECT user_id, {level}, uts / ({days} * 86400), count(*) FROM scrobbles GROUP BY 1, 3"
        );
        let stored = format!(
            "SELECT user_id, lvl, bucket, plays FROM user_scrobble_rollup WHERE lvl = {level}"
        );
        assert_same(conn, &stored, &recount);
    }
    let playcount =
        "SELECT id, (SELECT count(*) FROM scrobbles s WHERE s.user_id = users.id) FROM users";
    assert_same(conn, "SELECT id, playcount FROM users", playcount);
}

/// Asserts that every period total equals a recount of the scrobbles from the period's start.
pub fn assert_periods_match(conn: &Connection) {
    let periods = "(SELECT 1 AS period, 7 AS days UNION ALL SELECT 2, 30 UNION ALL SELECT 3, 90 \
                   UNION ALL SELECT 4, 180 UNION ALL SELECT 5, 365) p";
    for kind in ["artist", "album", "track"] {
        let id = lib_id(kind);
        let recount = format!(
            "SELECT s.user_id, p.period, {id}, count(*) FROM scrobbles s \
             JOIN users u ON u.id = s.user_id CROSS JOIN {periods} \
             WHERE u.period_day IS NOT NULL AND s.{kind}_id IS NOT NULL \
             AND s.uts >= (u.period_day - p.days + 1) * 86400 GROUP BY 1, 2, 3"
        );
        let stored = format!(
            "SELECT user_id, period, {kind}_id, plays FROM user_{kind}_total WHERE period > 0"
        );
        assert_same(conn, &stored, &recount);
    }
}

/// The library entity a scrobble `s` counts under for `kind`.
fn lib_id(kind: &str) -> String {
    let code = match kind {
        "artist" => 0,
        "album" => 1,
        _ => 2,
    };
    format!(
        "coalesce((SELECT m.lib_id FROM user_library_map m WHERE m.user_id = s.user_id \
         AND m.kind = {code} AND m.raw_id = s.{kind}_id), s.{kind}_id)"
    )
}

fn assert_same(conn: &Connection, stored: &str, recount: &str) {
    let diff = format!(
        "SELECT (SELECT count(*) FROM (SELECT * FROM ({stored}) EXCEPT SELECT * FROM ({recount}))) \
              + (SELECT count(*) FROM (SELECT * FROM ({recount}) EXCEPT SELECT * FROM ({stored})))"
    );
    let differing: i64 = conn.query_row(&diff, [], |row| row.get(0)).unwrap();
    assert_eq!(
        differing, 0,
        "aggregate differs from recount:\n  {stored}\n  {recount}"
    );
}

/// A user with about 9,000 scrobbles over two years, loaded in day-sized windows.
pub async fn loaded(db: &TestDb) -> UserId {
    let alice = user(&db.store, "alice").await;
    user(&db.store, "bob").await;
    let mut rng = Rng(42);
    for day in 0..730 {
        let window = TimeRange::new(T0 + day * DAY, T0 + (day + 1) * DAY);
        let count = usize::try_from(rng.below(20)).unwrap();
        let plays = random_scrobbles(&mut rng, window.from, window.to, count);
        db.store.replace_window(alice, window, plays).await.unwrap();
    }
    alice
}

const ARTIST_OF_ALBUM: &str = "(SELECT ar.name FROM albums x JOIN artists ar \
     ON ar.id = x.artist_id WHERE x.id = s.album_id)";
const ARTIST_OF_TRACK: &str = "(SELECT ar.name FROM tracks x JOIN artists ar \
     ON ar.id = x.artist_id WHERE x.id = s.track_id)";

/// The naive aggregation that top lists must agree with: (id, plays), by plays, ties by the
/// artist's name and then the entity's name, as Last.fm orders them.
pub fn naive_top(conn: &Connection, user: UserId, kind: Kind, range: TimeRange) -> Vec<(i64, u64)> {
    let (word, names) = match kind {
        Kind::Artist => ("artist", "''"),
        Kind::Album => ("album", ARTIST_OF_ALBUM),
        Kind::Track => ("track", ARTIST_OF_TRACK),
    };
    let sql = format!(
        "SELECT {word}_id, count(*) AS n, {names}, (SELECT name FROM {word}s WHERE id = {word}_id) \
         FROM scrobbles s WHERE user_id = ?1 AND uts >= ?2 \
         AND uts < ?3 AND {word}_id IS NOT NULL GROUP BY 1"
    );
    let mut stmt = conn.prepare(&sql).unwrap();
    let mut rows: Vec<(i64, u64, String, String)> = stmt
        .query_map(rusqlite::params![user.0, range.from, range.to], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    rows.sort_by_cached_key(|(_, plays, artist, name)| {
        (
            std::cmp::Reverse(*plays),
            collation_key(artist),
            collation_key(name),
        )
    });
    rows.into_iter()
        .map(|(id, plays, _, _)| (id, plays))
        .collect()
}
