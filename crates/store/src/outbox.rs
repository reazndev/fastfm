//! Client writes on their way to Last.fm: retried after transient failures, or held locally.

use rusqlite::{Connection, Row, params};

use crate::error::{Result, changed};
use crate::pool::Store;
use crate::text::text_enum;
use crate::types::AccountId;

/// Rows kept per account; older ones are dropped, since Last.fm refuses old scrobbles anyway.
const MAX_PER_ACCOUNT: i64 = 1000;

text_enum!(
    /// Where a queued write stands.
    OutboxStatus {
        /// Waiting for its next attempt.
        Queued => "queued",
        /// Gave up: a permanent error, or too many attempts.
        Failed => "failed",
        /// Applied locally only, because the account had no Last.fm session key.
        Held => "held",
    }
);

/// A write to queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewOutboxEntry {
    /// The account that made the write.
    pub account: AccountId,
    /// The Last.fm method, such as `track.scrobble`.
    pub method: String,
    /// What to send, as JSON defined by the caller.
    pub payload: String,
    /// Queued, failed or held.
    pub status: OutboxStatus,
    /// Unix time of the next attempt.
    pub next_at: i64,
    /// Why it is not done.
    pub error: Option<String>,
}

/// A queued write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboxEntry {
    /// Row id.
    pub id: i64,
    /// The account that made the write.
    pub account: AccountId,
    /// The Last.fm method.
    pub method: String,
    /// The caller's JSON.
    pub payload: String,
    /// Attempts made after the first one.
    pub attempts: u32,
}

/// One account's queue at a glance.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutboxSummary {
    /// Writes waiting for a retry.
    pub queued: u64,
    /// Writes that gave up.
    pub failed: u64,
    /// Writes kept local only.
    pub held: u64,
    /// The newest error message.
    pub last_error: Option<String>,
}

impl Store {
    /// Adds a write, dropping the account's oldest rows beyond the limit.
    pub async fn enqueue_write(&self, entry: NewOutboxEntry, now: i64) -> Result<()> {
        self.write(move |tx| {
            let sql = "INSERT INTO outbox (account_id, method, payload, status, next_at, error, \
                       created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";
            let e = &entry;
            let values = params![
                e.account, e.method, e.payload, e.status, e.next_at, e.error, now
            ];
            tx.prepare_cached(sql)?.execute(values)?;
            let trim = "DELETE FROM outbox WHERE account_id = ?1 AND id <= (SELECT id FROM outbox \
                        WHERE account_id = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?2)";
            tx.prepare_cached(trim)?
                .execute(params![e.account, MAX_PER_ACCOUNT])?;
            Ok(())
        })
        .await
    }

    /// Queued writes whose next attempt is due at `now`, oldest first.
    pub async fn due_writes(&self, now: i64, limit: u32) -> Result<Vec<OutboxEntry>> {
        self.read(move |conn| {
            let sql = "SELECT id, account_id, method, payload, attempts FROM outbox \
                       WHERE status = 'queued' AND next_at <= ?1 ORDER BY id LIMIT ?2";
            let mut stmt = conn.prepare_cached(sql)?;
            let rows = stmt.query_map(params![now, limit], entry_from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
        .await
    }

    /// Removes a write that reached Last.fm.
    pub async fn finish_write(&self, id: i64) -> Result<()> {
        self.write(move |tx| {
            changed(
                tx.prepare_cached("DELETE FROM outbox WHERE id = ?1")?
                    .execute([id])?,
            )
        })
        .await
    }

    /// Counts a failed attempt and schedules the next one.
    pub async fn retry_write(&self, id: i64, next_at: i64, error: &str) -> Result<()> {
        let error = error.to_owned();
        self.write(move |tx| {
            let sql = "UPDATE outbox SET attempts = attempts + 1, next_at = ?2, error = ?3 \
                       WHERE id = ?1";
            changed(
                tx.prepare_cached(sql)?
                    .execute(params![id, next_at, error])?,
            )
        })
        .await
    }

    /// Gives up on a write.
    pub async fn fail_write(&self, id: i64, error: &str) -> Result<()> {
        let error = error.to_owned();
        self.write(move |tx| {
            let sql = "UPDATE outbox SET status = 'failed', error = ?2 WHERE id = ?1";
            changed(tx.prepare_cached(sql)?.execute(params![id, error])?)
        })
        .await
    }

    /// Counts of the account's queued, failed and held writes, and the newest error.
    pub async fn outbox_summary(&self, account: AccountId) -> Result<OutboxSummary> {
        self.read(move |conn| summary(conn, account)).await
    }
}

fn summary(conn: &Connection, account: AccountId) -> Result<OutboxSummary> {
    let sql = "SELECT status, count(*) FROM outbox WHERE account_id = ?1 GROUP BY status";
    let mut stmt = conn.prepare_cached(sql)?;
    let counts = stmt.query_map([account], |row| {
        Ok((row.get::<_, OutboxStatus>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut out = OutboxSummary::default();
    for row in counts {
        let (status, n) = row?;
        let n = u64::try_from(n).unwrap_or(0);
        match status {
            OutboxStatus::Queued => out.queued = n,
            OutboxStatus::Failed => out.failed = n,
            OutboxStatus::Held => out.held = n,
        }
    }
    let error = "SELECT error FROM outbox WHERE account_id = ?1 AND error IS NOT NULL \
                 ORDER BY id DESC LIMIT 1";
    out.last_error = conn
        .prepare_cached(error)?
        .query_map([account], |row| row.get(0))?
        .next()
        .transpose()?;
    Ok(out)
}

fn entry_from_row(row: &Row<'_>) -> rusqlite::Result<OutboxEntry> {
    Ok(OutboxEntry {
        id: row.get(0)?,
        account: row.get(1)?,
        method: row.get(2)?,
        payload: row.get(3)?,
        attempts: row.get(4)?,
    })
}
