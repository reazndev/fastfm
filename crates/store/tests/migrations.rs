//! Migrations over a database that already holds data.

use lfm_store::{Kind, Page, Period, Store, TopQuery};
use rusqlite::Connection;

const BEFORE_SPELLINGS: [&str; 7] = [
    include_str!("../../../migrations/0001_init.sql"),
    include_str!("../../../migrations/0002_details.sql"),
    include_str!("../../../migrations/0003_track_artist_mbid.sql"),
    include_str!("../../../migrations/0004_upstream_cache_headers.sql"),
    include_str!("../../../migrations/0005_write_through.sql"),
    include_str!("../../../migrations/0006_library.sql"),
    include_str!("../../../migrations/0007_unique_lastfm_link.sql"),
];

/// Rebuilding `tracks` keeps loved tracks, which reference it, and every track's spelling.
#[tokio::test]
async fn tracks_are_rebuilt_with_loved_tracks_pointing_at_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    for sql in BEFORE_SPELLINGS {
        conn.execute_batch(sql).unwrap();
    }
    conn.pragma_update(None, "user_version", BEFORE_SPELLINGS.len())
        .unwrap();
    conn.execute_batch(
        "INSERT INTO artists (id, name, name_fold) VALUES (1, 'twenty one pilots', 'twenty one pilots');
         INSERT INTO tracks (id, artist_id, name, name_fold, artist_name)
             VALUES (5, 1, 'Heathens', 'heathens', 'Twenty One Pilots'),
                    (6, 1, 'Ride', 'ride', NULL);
         INSERT INTO users (id, name, name_fold) VALUES (1, 'alice', 'alice');
         INSERT INTO loved (user_id, track_id, uts) VALUES (1, 5, 100);
         INSERT INTO scrobbles (user_id, uts, seq, track_id, artist_id) VALUES (1, 100, 0, 5, 1);
         INSERT INTO user_track_total (user_id, period, track_id, plays) VALUES (1, 0, 5, 1);
         INSERT INTO user_track_hist (user_id, period, plays, n) VALUES (1, 0, 1, 1);",
    )
    .unwrap();
    drop(conn);

    let store = Store::open(&path).unwrap();
    let query = TopQuery {
        kind: Kind::Track,
        period: Period::Overall,
        now: 200,
    };
    let top = store
        .top(lfm_store::UserId(1), query, Page::new(1, 10))
        .await
        .unwrap();
    let track = &top.items[0].entity;
    assert_eq!(track.name, "Heathens");
    assert_eq!(track.artist.as_ref().unwrap().name, "Twenty One Pilots");
    drop(store);
    let conn = Connection::open(&path).unwrap();
    let spellings: Vec<(i64, String)> = conn
        .prepare("SELECT id, artist_spelling FROM tracks ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let want = [(5, "Twenty One Pilots"), (6, "twenty one pilots")];
    assert_eq!(spellings, want.map(|(id, s)| (id, s.to_owned())));
    let loved: i64 = conn
        .query_row("SELECT track_id FROM loved", [], |row| row.get(0))
        .unwrap();
    assert_eq!(loved, 5);
}
