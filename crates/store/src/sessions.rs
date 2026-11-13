//! Web login sessions and the Last.fm-style token to session key flow.

use rusqlite::{OptionalExtension, params};

use crate::error::{Result, changed};
use crate::pool::Store;
use crate::types::AccountId;

/// A web login session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebSession {
    /// Session id from the cookie.
    pub id: String,
    /// Logged-in account.
    pub account: AccountId,
    /// Creation time in unix seconds.
    pub created_at: i64,
    /// Expiry time in unix seconds.
    pub expires_at: i64,
}

/// A token from `auth.getToken`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthToken {
    /// The token.
    pub token: String,
    /// API key that asked for it.
    pub api_key: String,
    /// Account that approved it, once approved.
    pub account: Option<AccountId>,
    /// Expiry time in unix seconds.
    pub expires_at: i64,
}

/// A session key from `auth.getSession` or `auth.getMobileSession`. It does not expire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiSession {
    /// The session key.
    pub session_key: String,
    /// Account the key acts for.
    pub account: AccountId,
    /// API key it was issued to.
    pub api_key: String,
    /// Creation time in unix seconds.
    pub created_at: i64,
}

impl Store {
    /// Stores a new web session.
    pub async fn create_session(&self, session: WebSession) -> Result<()> {
        self.write(move |tx| {
            let sql = "INSERT INTO sessions (id, account_id, created_at, expires_at) \
                       VALUES (?1, ?2, ?3, ?4)";
            let s = &session;
            tx.prepare_cached(sql)?.execute(params![
                s.id,
                s.account,
                s.created_at,
                s.expires_at
            ])?;
            Ok(())
        })
        .await
    }

    /// The web session `id`, unless it has expired at `now`.
    pub async fn session(&self, id: &str, now: i64) -> Result<Option<WebSession>> {
        let id = id.to_owned();
        self.read(move |conn| {
            let sql = "SELECT id, account_id, created_at, expires_at FROM sessions \
                       WHERE id = ?1 AND expires_at > ?2";
            let session = conn
                .prepare_cached(sql)?
                .query_row(params![id, now], |row| {
                    Ok(WebSession {
                        id: row.get(0)?,
                        account: row.get(1)?,
                        created_at: row.get(2)?,
                        expires_at: row.get(3)?,
                    })
                });
            Ok(session.optional()?)
        })
        .await
    }

    /// Ends a web session.
    pub async fn delete_session(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.write(move |tx| {
            tx.prepare_cached("DELETE FROM sessions WHERE id = ?1")?
                .execute([id])?;
            Ok(())
        })
        .await
    }

    /// Deletes web sessions and auth tokens that expired before `now`.
    pub async fn purge_expired_sessions(&self, now: i64) -> Result<()> {
        self.write(move |tx| {
            tx.prepare_cached("DELETE FROM sessions WHERE expires_at <= ?1")?
                .execute([now])?;
            tx.prepare_cached("DELETE FROM auth_tokens WHERE expires_at <= ?1")?
                .execute([now])?;
            Ok(())
        })
        .await
    }

    /// Stores an unapproved token for `api_key`.
    pub async fn create_auth_token(&self, token: AuthToken, now: i64) -> Result<()> {
        self.write(move |tx| {
            let sql = "INSERT INTO auth_tokens (token, api_key, account_id, created_at, expires_at) \
                       VALUES (?1, ?2, ?3, ?4, ?5)";
            let t = &token;
            tx.prepare_cached(sql)?.execute(params![t.token, t.api_key, t.account, now, t.expires_at])?;
            Ok(())
        })
        .await
    }

    /// The token, unless it has expired at `now`.
    pub async fn auth_token(&self, token: &str, now: i64) -> Result<Option<AuthToken>> {
        let token = token.to_owned();
        self.read(move |conn| {
            let sql = "SELECT token, api_key, account_id, expires_at FROM auth_tokens \
                       WHERE token = ?1 AND expires_at > ?2";
            let found = conn
                .prepare_cached(sql)?
                .query_row(params![token, now], |row| {
                    Ok(AuthToken {
                        token: row.get(0)?,
                        api_key: row.get(1)?,
                        account: row.get(2)?,
                        expires_at: row.get(3)?,
                    })
                });
            Ok(found.optional()?)
        })
        .await
    }

    /// Records that `account` approved the token. Fails if it is unknown or expired.
    pub async fn authorize_auth_token(
        &self,
        token: &str,
        account: AccountId,
        now: i64,
    ) -> Result<()> {
        let token = token.to_owned();
        self.write(move |tx| {
            let sql = "UPDATE auth_tokens SET account_id = ?2 WHERE token = ?1 AND expires_at > ?3 \
                       AND (account_id IS NULL OR account_id = ?2)";
            changed(
                tx.prepare_cached(sql)?
                    .execute(params![token, account, now])?,
            )
        })
        .await
    }

    /// Trades an approved, unexpired token for a new session key, once. Returns `None` when the
    /// token is unknown, expired, or not yet approved.
    pub async fn exchange_auth_token(
        &self,
        token: &str,
        session_key: &str,
        now: i64,
    ) -> Result<Option<ApiSession>> {
        let (token, session_key) = (token.to_owned(), session_key.to_owned());
        self.write(move |tx| {
            let sql = "DELETE FROM auth_tokens WHERE token = ?1 AND expires_at > ?2 \
                       AND account_id IS NOT NULL RETURNING account_id, api_key";
            let found: Option<(AccountId, String)> = tx
                .prepare_cached(sql)?
                .query_row(params![token, now], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()?;
            let Some((account, api_key)) = found else {
                return Ok(None);
            };
            let session = ApiSession {
                session_key,
                account,
                api_key,
                created_at: now,
            };
            insert_api_session(tx, &session)?;
            Ok(Some(session))
        })
        .await
    }

    /// Stores a session key directly, for `auth.getMobileSession`.
    pub async fn create_api_session(&self, session: ApiSession) -> Result<()> {
        self.write(move |tx| insert_api_session(tx, &session)).await
    }

    /// Looks up a session key.
    pub async fn api_session(&self, session_key: &str) -> Result<Option<ApiSession>> {
        let session_key = session_key.to_owned();
        self.read(move |conn| {
            let sql = "SELECT session_key, account_id, api_key, created_at FROM api_sessions \
                       WHERE session_key = ?1";
            let found = conn.prepare_cached(sql)?.query_row([session_key], |row| {
                Ok(ApiSession {
                    session_key: row.get(0)?,
                    account: row.get(1)?,
                    api_key: row.get(2)?,
                    created_at: row.get(3)?,
                })
            });
            Ok(found.optional()?)
        })
        .await
    }
}

fn insert_api_session(conn: &rusqlite::Connection, session: &ApiSession) -> Result<()> {
    let sql = "INSERT INTO api_sessions (session_key, account_id, api_key, created_at) \
               VALUES (?1, ?2, ?3, ?4)";
    let s = session;
    conn.prepare_cached(sql)?.execute(params![
        s.session_key,
        s.account,
        s.api_key,
        s.created_at
    ])?;
    Ok(())
}
