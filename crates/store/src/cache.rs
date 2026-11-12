//! The upstream response cache: Last.fm's response bodies, kept until they expire.

use rusqlite::{OptionalExtension, params};

use crate::error::Result;
use crate::pool::Store;

/// A cached upstream response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedResponse {
    /// Response body as received.
    pub body: Vec<u8>,
    /// HTTP status.
    pub status: u16,
    /// When it was fetched, in unix seconds.
    pub fetched_at: i64,
    /// When it goes stale, in unix seconds.
    pub expires_at: i64,
}

impl CachedResponse {
    /// Whether the entry is still fresh at `now`.
    #[must_use]
    pub fn is_fresh(&self, now: i64) -> bool {
        now < self.expires_at
    }
}

impl Store {
    /// The cached response for `key` in `format`, fresh or stale. Callers serve a stale entry and
    /// refresh it in the background.
    pub async fn cache_get(&self, key: &str, format: &str) -> Result<Option<CachedResponse>> {
        let (key, format) = (key.to_owned(), format.to_owned());
        self.read(move |conn| {
            let sql = "SELECT body, status, fetched_at, expires_at FROM upstream_cache \
                       WHERE key = ?1 AND format = ?2";
            let entry = conn
                .prepare_cached(sql)?
                .query_row(params![key, format], |row| {
                    Ok(CachedResponse {
                        body: row.get(0)?,
                        status: row.get(1)?,
                        fetched_at: row.get(2)?,
                        expires_at: row.get(3)?,
                    })
                });
            Ok(entry.optional()?)
        })
        .await
    }

    /// Stores or replaces the response for `key` in `format`.
    pub async fn cache_put(&self, key: &str, format: &str, entry: CachedResponse) -> Result<()> {
        let (key, format) = (key.to_owned(), format.to_owned());
        self.write(move |tx| {
            let sql = "INSERT INTO upstream_cache (key, format, body, status, fetched_at, expires_at) \
                       VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT (key, format) DO UPDATE SET \
                       body = excluded.body, status = excluded.status, \
                       fetched_at = excluded.fetched_at, expires_at = excluded.expires_at";
            let e = &entry;
            tx.prepare_cached(sql)?.execute(params![
                key,
                format,
                e.body,
                e.status,
                e.fetched_at,
                e.expires_at
            ])?;
            Ok(())
        })
        .await
    }

    /// Deletes entries that expired before `before`. Returns how many were deleted.
    pub async fn cache_purge(&self, before: i64) -> Result<u64> {
        self.write(move |tx| {
            let sql = "DELETE FROM upstream_cache WHERE expires_at < ?1";
            let deleted = tx.prepare_cached(sql)?.execute([before])?;
            Ok(u64::try_from(deleted).unwrap_or(u64::MAX))
        })
        .await
    }
}
