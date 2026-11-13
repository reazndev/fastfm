//! Counting scrobbles in a range and finding the n-th newest one, from the bucket counts.
//!
//! Instead of a page-anchor list that would need rebuilding whenever an old window changes, the
//! store keeps scrobble counts per 1, 8, and 64-day bucket, updated by the same delta as the
//! other rollups. Finding the n-th newest scrobble walks those counts from the newest bucket
//! down, steps into the bucket that holds it, and skips at most one day of rows. Any page costs
//! about the same, whatever its depth, and no ingest ever rebuilds anything.

use rusqlite::{Connection, params};

use crate::buckets::{Segment, segments, split};
use crate::error::Result;
use crate::pool::Store;
use crate::types::{TimeRange, UserId, count};

impl Store {
    /// Counts the user's scrobbles in `range`.
    pub async fn count_range(&self, user: UserId, range: TimeRange) -> Result<u64> {
        self.read(move |conn| count_in(conn, user, range)).await
    }
}

/// Counts the user's scrobbles in `range`.
pub(crate) fn count_in(conn: &Connection, user: UserId, range: TimeRange) -> Result<u64> {
    segments(range.from, range.to)
        .into_iter()
        .map(|segment| count_segment(conn, user, segment))
        .sum::<Result<i64>>()
        .map(count)
}

fn count_segment(conn: &Connection, user: UserId, segment: Segment) -> Result<i64> {
    let get = |row: &rusqlite::Row<'_>| row.get(0);
    match segment {
        Segment::Raw { from, to } => {
            let sql =
                "SELECT count(*) FROM scrobbles WHERE user_id = ?1 AND uts >= ?2 AND uts < ?3";
            Ok(conn
                .prepare_cached(sql)?
                .query_row(params![user, from, to], get)?)
        }
        Segment::Buckets { level, first, end } => {
            let sql = "SELECT coalesce(sum(plays), 0) FROM user_scrobble_rollup \
                       WHERE user_id = ?1 AND lvl = ?2 AND bucket >= ?3 AND bucket < ?4";
            Ok(conn
                .prepare_cached(sql)?
                .query_row(params![user, level, first, end], get)?)
        }
    }
}

/// Where the `skip`-th newest scrobble of `range` lies: the page starts after skipping `skip`
/// rows of `[range.from, end)`, newest first. `skip` is at most one day of scrobbles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Seek {
    pub(crate) end: i64,
    pub(crate) skip: u64,
}

/// Finds the `index`-th newest scrobble (from 0) of `range`.
pub(crate) fn locate(
    conn: &Connection,
    user: UserId,
    range: TimeRange,
    index: u64,
) -> Result<Seek> {
    let mut pending = segments(range.from, range.to);
    let mut left = index;
    while let Some(segment) = pending.pop() {
        match segment {
            Segment::Raw { to, .. } => {
                let rows = count(count_segment(conn, user, segment)?);
                if left < rows {
                    return Ok(Seek {
                        end: to,
                        skip: left,
                    });
                }
                left -= rows;
            }
            Segment::Buckets { level, first, end } => {
                let (found, passed) =
                    newest_bucket_reaching(conn, user, (level, first, end), left)?;
                left -= passed;
                if let Some(bucket) = found {
                    pending.push(split(level, bucket));
                }
            }
        }
    }
    // Only reached when the counts disagree with the rows; an offset query stays correct.
    Ok(Seek {
        end: range.to,
        skip: index,
    })
}

/// Walks buckets newest first. Returns the bucket holding the `left`-th newest scrobble, if any,
/// and how many scrobbles lie in newer buckets of this run.
fn newest_bucket_reaching(
    conn: &Connection,
    user: UserId,
    (level, first, end): (usize, i64, i64),
    left: u64,
) -> Result<(Option<i64>, u64)> {
    // `+?N` keeps the bound values out of planning; with plain `?N` this ordered range scan
    // was re-planned on every use (see `recent::PAGE_SQL`).
    let sql = "SELECT bucket, plays FROM user_scrobble_rollup \
               WHERE user_id = +?1 AND lvl = +?2 AND bucket >= +?3 AND bucket < +?4 \
               ORDER BY bucket DESC";
    let mut stmt = conn.prepare_cached(sql)?;
    let mut rows = stmt.query(params![user, level, first, end])?;
    let mut passed = 0;
    while let Some(row) = rows.next()? {
        let plays = count(row.get(1)?);
        if left - passed < plays {
            return Ok((Some(row.get(0)?), passed));
        }
        passed += plays;
    }
    Ok((None, passed))
}
