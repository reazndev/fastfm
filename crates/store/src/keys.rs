//! API keys of local accounts.
//!
//! The server keeps active keys in memory, so a request costs no query. Last-used times collect
//! in memory too and reach the database in one transaction per `flush_key_usage`.

use std::collections::HashMap;
use std::sync::PoisonError;

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::accounts::AccountStatus;
use crate::error::{Result, StoreError, changed};
use crate::pool::Store;
use crate::text::text_enum;
use crate::types::AccountId;

text_enum!(
    /// What a key may do.
    KeyScope {
        /// Read methods only.
        Read => "read",
        /// Read and write methods.
        Write => "write",
    }
);

/// A key to create. The caller generates the key and secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewApiKey {
    /// The API key, 32 hex characters.
    pub key: String,
    /// The shared secret, 32 hex characters.
    pub secret: String,
    /// Label shown in the UI.
    pub label: String,
    /// Read or read-write.
    pub scope: KeyScope,
}

/// A stored API key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiKey {
    /// The API key.
    pub key: String,
    /// The shared secret, readable because `api_sig` checks need it.
    pub secret: String,
    /// Owning account.
    pub account: AccountId,
    /// Label shown in the UI.
    pub label: String,
    /// Read or read-write.
    pub scope: KeyScope,
    /// Creation time in unix seconds.
    pub created_at: i64,
    /// Last use in unix seconds, as of the last flush.
    pub last_used: Option<i64>,
}

const COLUMNS: &str = "k.key, k.secret, k.account_id, k.label, k.scope, k.created_at, k.last_used";

impl Store {
    /// Creates a key for an active account.
    pub async fn create_api_key(
        &self,
        account: AccountId,
        key: NewApiKey,
        now: i64,
    ) -> Result<ApiKey> {
        self.write(move |tx| {
            let status_sql = "SELECT status FROM accounts WHERE id = ?1";
            let status: Option<AccountStatus> = tx
                .prepare_cached(status_sql)?
                .query_row([account], |row| row.get(0))
                .optional()?;
            match status {
                None => return Err(StoreError::NotFound),
                Some(AccountStatus::Active) => {}
                Some(_) => return Err(StoreError::AccountNotActive),
            }
            let sql = "INSERT INTO api_keys (key, secret, account_id, label, scope, created_at) \
                       VALUES (?1, ?2, ?3, ?4, ?5, ?6)";
            let values = params![key.key, key.secret, account, key.label, key.scope, now];
            tx.prepare_cached(sql)?.execute(values)?;
            let NewApiKey {
                key,
                secret,
                label,
                scope,
            } = key;
            Ok(ApiKey {
                key,
                secret,
                account,
                label,
                scope,
                created_at: now,
                last_used: None,
            })
        })
        .await
    }

    /// The account's keys, oldest first.
    pub async fn api_keys(&self, account: AccountId) -> Result<Vec<ApiKey>> {
        self.read(move |conn| {
            let filter = "k.account_id = +?1 ORDER BY k.created_at, k.key";
            select(conn, filter, &[&account])
        })
        .await
    }

    /// Deletes one of the account's keys.
    pub async fn revoke_api_key(&self, account: AccountId, key: &str) -> Result<()> {
        let key = key.to_owned();
        self.write(move |tx| {
            let sql = "DELETE FROM api_keys WHERE account_id = ?1 AND key = ?2";
            changed(tx.prepare_cached(sql)?.execute(params![account, key])?)
        })
        .await
    }

    /// Every key whose account is active, for the server's in-memory key map.
    pub async fn active_api_keys(&self) -> Result<Vec<ApiKey>> {
        self.read(|conn| select(conn, "a.status = 'active' ORDER BY k.key", &[]))
            .await
    }

    /// Notes that `key` was used at `now`. Costs no query; see `flush_key_usage`.
    pub fn mark_key_used(&self, key: &str, now: i64) {
        let mut usage = self
            .key_usage()
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        usage.insert(key.to_owned(), now);
    }

    /// Writes all noted key uses in one transaction.
    pub async fn flush_key_usage(&self) -> Result<()> {
        let pending: HashMap<String, i64> = {
            let mut usage = self
                .key_usage()
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            std::mem::take(&mut *usage)
        };
        if pending.is_empty() {
            return Ok(());
        }
        self.write(move |tx| {
            let sql =
                "UPDATE api_keys SET last_used = max(coalesce(last_used, 0), ?2) WHERE key = ?1";
            let mut stmt = tx.prepare_cached(sql)?;
            for (key, used) in &pending {
                stmt.execute(params![key, used])?;
            }
            Ok(())
        })
        .await
    }
}

fn select(conn: &Connection, filter: &str, values: &[&dyn rusqlite::ToSql]) -> Result<Vec<ApiKey>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM api_keys k JOIN accounts a ON a.id = k.account_id WHERE {filter}"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(values, key_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn key_from_row(row: &Row<'_>) -> rusqlite::Result<ApiKey> {
    Ok(ApiKey {
        key: row.get(0)?,
        secret: row.get(1)?,
        account: row.get(2)?,
        label: row.get(3)?,
        scope: row.get(4)?,
        created_at: row.get(5)?,
        last_used: row.get(6)?,
    })
}
