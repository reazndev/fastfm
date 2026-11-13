//! What reconciliation remembers between passes: when it last ran for a user, and the ranges
//! where Last.fm's counts contradict each other.

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Result;
use crate::pool::Store;
use crate::types::{TimeRange, UserId, count};

/// A range whose Last.fm total the store cannot match, because Last.fm's own answers disagree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpstreamMismatch {
    /// The range.
    pub range: TimeRange,
    /// Last.fm's `@attr.total` for the range.
    pub remote: u64,
    /// The store's count, which matches Last.fm's list or the totals of the range's halves.
    pub local: u64,
    /// When it was found, in Unix seconds.
    pub seen_at: i64,
}

impl Store {
    /// When reconciliation last checked `user`, in Unix seconds.
    pub async fn reconciled_at(&self, user: UserId) -> Result<Option<i64>> {
        self.read(move |conn| {
            let sql = "SELECT reconciled_at FROM users WHERE id = ?1";
            let found = conn
                .prepare_cached(sql)?
                .query_row([user], |row| row.get(0))
                .optional()?;
            Ok(found.flatten())
        })
        .await
    }

    /// Records that reconciliation checked `user` at `at`.
    pub async fn set_reconciled_at(&self, user: UserId, at: i64) -> Result<()> {
        self.write(move |tx| {
            let sql = "UPDATE users SET reconciled_at = ?2 WHERE id = ?1";
            tx.prepare_cached(sql)?.execute(params![user, at])?;
            Ok(())
        })
        .await
    }

    /// Records a range where Last.fm's counts disagree, replacing an older record of it, and
    /// forgets the user's records seen before `forget_before`.
    pub async fn record_upstream_mismatch(
        &self,
        user: UserId,
        mismatch: UpstreamMismatch,
        forget_before: i64,
    ) -> Result<()> {
        self.write(move |tx| {
            let sql = "DELETE FROM upstream_mismatches WHERE user_id = ?1 AND seen_at < ?2";
            tx.prepare_cached(sql)?
                .execute(params![user, forget_before])?;
            let sql = "INSERT INTO upstream_mismatches \
                       (user_id, from_uts, to_uts, remote, local, seen_at) \
                       VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT DO UPDATE \
                       SET remote = excluded.remote, local = excluded.local, \
                       seen_at = excluded.seen_at";
            let UpstreamMismatch {
                range,
                remote,
                local,
                seen_at,
            } = mismatch;
            let values = params![user, range.from, range.to, remote, local, seen_at];
            tx.prepare_cached(sql)?.execute(values)?;
            Ok(())
        })
        .await
    }

    /// The user's recorded mismatches seen at `since` or later, oldest range first.
    pub async fn upstream_mismatches(
        &self,
        user: UserId,
        since: i64,
    ) -> Result<Vec<UpstreamMismatch>> {
        self.read(move |conn| mismatches(conn, user, since)).await
    }
}

fn mismatches(conn: &Connection, user: UserId, since: i64) -> Result<Vec<UpstreamMismatch>> {
    let sql = "SELECT from_uts, to_uts, remote, local, seen_at FROM upstream_mismatches \
               WHERE user_id = ?1 AND seen_at >= ?2 ORDER BY from_uts, to_uts";
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(params![user, since], |row| {
        Ok(UpstreamMismatch {
            range: TimeRange::new(row.get(0)?, row.get(1)?),
            remote: count(row.get(2)?),
            local: count(row.get(3)?),
            seen_at: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
