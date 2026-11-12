//! Writing scrobbles.

use rusqlite::{Connection, params};

use crate::delta::Delta;
use crate::dims::{Ids, Resolver};
use crate::error::{Result, StoreError};
use crate::pool::Store;
use crate::types::{Scrobble, TimeRange, UserId};
use crate::users::{bump_version, period_day};

impl Store {
    /// Replaces the user's scrobbles in `range` with `scrobbles`, in one transaction.
    ///
    /// Scrobbles in the same second are numbered in the order given, so real duplicates survive
    /// and running the same replace twice leaves the same state. Bumps the user's version when
    /// the window's contents change.
    pub async fn replace_window(
        &self,
        user: UserId,
        range: TimeRange,
        scrobbles: Vec<Scrobble>,
    ) -> Result<()> {
        self.write(move |tx| replace_window(tx, user, range, &scrobbles))
            .await
    }

    /// Adds scrobbles without removing any, for live sync and locally sent scrobbles.
    pub async fn append_scrobbles(&self, user: UserId, scrobbles: Vec<Scrobble>) -> Result<()> {
        self.write(move |tx| append(tx, user, &scrobbles)).await
    }
}

/// One stored scrobble row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Stored {
    uts: i64,
    seq: i64,
    track: i64,
    album: Option<i64>,
    artist: i64,
}

impl Stored {
    fn ids(self) -> Ids {
        Ids {
            artist: self.artist,
            album: self.album,
            track: self.track,
        }
    }
}

fn replace_window(
    conn: &Connection,
    user: UserId,
    range: TimeRange,
    scrobbles: &[Scrobble],
) -> Result<()> {
    let outside = scrobbles
        .iter()
        .find(|s| s.uts < range.from || s.uts >= range.to);
    if let Some(scrobble) = outside {
        return Err(StoreError::OutsideWindow {
            uts: scrobble.uts,
            from: range.from,
            to: range.to,
        });
    }
    let old = window_rows(conn, user, range)?;
    let delete = "DELETE FROM scrobbles WHERE user_id = ?1 AND uts >= ?2 AND uts < ?3";
    conn.prepare_cached(delete)?
        .execute(params![user, range.from, range.to])?;
    let mut delta = Delta::new(period_day(conn, user)?);
    for row in &old {
        delta.add(row.uts, row.ids(), -1);
    }
    let mut new = insert_all(conn, user, scrobbles, &mut delta)?;
    new.sort_unstable();
    delta.apply(conn, user)?;
    if old != new {
        bump_version(conn, user)?;
    }
    Ok(())
}

fn append(conn: &Connection, user: UserId, scrobbles: &[Scrobble]) -> Result<()> {
    let mut delta = Delta::new(period_day(conn, user)?);
    let new = insert_all(conn, user, scrobbles, &mut delta)?;
    delta.apply(conn, user)?;
    if !new.is_empty() {
        bump_version(conn, user)?;
    }
    Ok(())
}

/// Inserts scrobbles after any already stored in the same second, in the order given.
fn insert_all(
    conn: &Connection,
    user: UserId,
    scrobbles: &[Scrobble],
    delta: &mut Delta,
) -> Result<Vec<Stored>> {
    let mut resolver = Resolver::new(conn);
    let next_seq =
        "SELECT coalesce(max(seq) + 1, 0) FROM scrobbles WHERE user_id = ?1 AND uts = ?2";
    let insert = "INSERT INTO scrobbles (user_id, uts, seq, track_id, album_id, artist_id) \
                  VALUES (?1, ?2, ?3, ?4, ?5, ?6)";
    scrobbles
        .iter()
        .map(|s| {
            let ids = resolver.ids(&s.artist, s.album.as_ref(), &s.track, s.image.as_deref())?;
            let seq: i64 = conn
                .prepare_cached(next_seq)?
                .query_row(params![user, s.uts], |row| row.get(0))?;
            let values = params![user, s.uts, seq, ids.track, ids.album, ids.artist];
            conn.prepare_cached(insert)?.execute(values)?;
            delta.add(s.uts, ids, 1);
            Ok(Stored {
                uts: s.uts,
                seq,
                track: ids.track,
                album: ids.album,
                artist: ids.artist,
            })
        })
        .collect()
}

fn window_rows(conn: &Connection, user: UserId, range: TimeRange) -> Result<Vec<Stored>> {
    let sql = "SELECT uts, seq, track_id, album_id, artist_id FROM scrobbles \
               WHERE user_id = ?1 AND uts >= ?2 AND uts < ?3 ORDER BY uts, seq";
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(params![user, range.from, range.to], |row| {
        Ok(Stored {
            uts: row.get(0)?,
            seq: row.get(1)?,
            track: row.get(2)?,
            album: row.get(3)?,
            artist: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
