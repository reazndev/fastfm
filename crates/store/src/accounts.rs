//! Local accounts: sign-up, approval, roles, and the Last.fm link.

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::error::{Result, StoreError, changed};
use crate::names::fold;
use crate::pool::Store;
use crate::settings::{SignupMode, signup_mode};
use crate::text::text_enum;
use crate::types::AccountId;

text_enum!(
    /// What an account may do.
    Role {
        /// Manages accounts and settings.
        Admin => "admin",
        /// A regular user.
        User => "user",
    }
);

text_enum!(
    /// Where an account stands.
    AccountStatus {
        /// Waiting for an admin's approval.
        Pending => "pending",
        /// Can use everything.
        Active => "active",
        /// Turned off by an admin; its keys and jobs stop.
        Disabled => "disabled",
    }
);

/// An account's own Last.fm credentials, encrypted by the caller.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LastfmCredentials {
    /// Own Last.fm API key.
    pub api_key: Option<Vec<u8>>,
    /// Own Last.fm shared secret.
    pub secret: Option<Vec<u8>>,
    /// Last.fm session key from web auth.
    pub session_key: Option<Vec<u8>>,
}

/// A local account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    /// Store id.
    pub id: AccountId,
    /// Username as given at sign-up.
    pub name: String,
    /// Password hash, made by the caller.
    pub password_hash: String,
    /// Admin or user.
    pub role: Role,
    /// Pending, active, or disabled.
    pub status: AccountStatus,
    /// Linked Last.fm username.
    pub lastfm_user: Option<String>,
    /// Own Last.fm credentials.
    pub credentials: LastfmCredentials,
    /// Creation time in unix seconds.
    pub created_at: i64,
}

const COLUMNS: &str = "id, name, password_hash, role, status, lastfm_user, own_key, own_secret, \
                       lastfm_sk, created_at";

impl Store {
    /// Creates an account. The first account ever becomes an active admin; later ones follow the
    /// sign-up mode.
    pub async fn create_account(
        &self,
        name: &str,
        password_hash: &str,
        now: i64,
    ) -> Result<Account> {
        let (name, hash) = (name.to_owned(), password_hash.to_owned());
        self.write(move |tx| create(tx, &name, &hash, now)).await
    }

    /// Looks up an account by id.
    pub async fn account(&self, id: AccountId) -> Result<Option<Account>> {
        self.read(move |conn| one(conn, "id = ?1", id)).await
    }

    /// Looks up an account by name, case-insensitively.
    pub async fn account_by_name(&self, name: &str) -> Result<Option<Account>> {
        let name_fold = fold(name);
        self.read(move |conn| one(conn, "name_fold = ?1", name_fold))
            .await
    }

    /// Lists accounts, oldest first, optionally only those with `status`.
    pub async fn list_accounts(&self, status: Option<AccountStatus>) -> Result<Vec<Account>> {
        self.read(move |conn| {
            let sql = format!(
                "SELECT {COLUMNS} FROM accounts WHERE ?1 IS NULL OR status = ?1 ORDER BY id"
            );
            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map([status], account_from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
        .await
    }

    /// Activates a pending account.
    pub async fn approve_account(&self, id: AccountId) -> Result<()> {
        self.write(move |tx| set_status(tx, id, &[AccountStatus::Pending], AccountStatus::Active))
            .await
    }

    /// Deletes a pending account.
    pub async fn reject_account(&self, id: AccountId) -> Result<()> {
        self.write(move |tx| {
            let sql = "DELETE FROM accounts WHERE id = ?1 AND status = 'pending'";
            changed(tx.prepare_cached(sql)?.execute([id])?)
        })
        .await
    }

    /// Disables an account. The last active admin cannot be disabled.
    pub async fn disable_account(&self, id: AccountId) -> Result<()> {
        self.write(move |tx| {
            guard_last_admin(tx, id)?;
            let from = [AccountStatus::Pending, AccountStatus::Active];
            set_status(tx, id, &from, AccountStatus::Disabled)
        })
        .await
    }

    /// Re-activates a disabled account.
    pub async fn enable_account(&self, id: AccountId) -> Result<()> {
        self.write(move |tx| set_status(tx, id, &[AccountStatus::Disabled], AccountStatus::Active))
            .await
    }

    /// Changes an account's role. The last active admin cannot lose the role.
    pub async fn set_role(&self, id: AccountId, role: Role) -> Result<()> {
        self.write(move |tx| {
            if role != Role::Admin {
                guard_last_admin(tx, id)?;
            }
            let sql = "UPDATE accounts SET role = ?2 WHERE id = ?1";
            changed(tx.prepare_cached(sql)?.execute(params![id, role])?)
        })
        .await
    }

    /// Replaces an account's password hash.
    pub async fn set_password_hash(&self, id: AccountId, password_hash: &str) -> Result<()> {
        let hash = password_hash.to_owned();
        self.write(move |tx| {
            let sql = "UPDATE accounts SET password_hash = ?2 WHERE id = ?1";
            changed(tx.prepare_cached(sql)?.execute(params![id, hash])?)
        })
        .await
    }

    /// Links or unlinks a Last.fm username.
    pub async fn link_lastfm(&self, id: AccountId, lastfm_user: Option<String>) -> Result<()> {
        self.write(move |tx| {
            let sql = "UPDATE accounts SET lastfm_user = ?2 WHERE id = ?1";
            changed(tx.prepare_cached(sql)?.execute(params![id, lastfm_user])?)
        })
        .await
    }

    /// Replaces an account's own Last.fm credentials.
    pub async fn set_lastfm_credentials(
        &self,
        id: AccountId,
        creds: LastfmCredentials,
    ) -> Result<()> {
        self.write(move |tx| {
            let sql =
                "UPDATE accounts SET own_key = ?2, own_secret = ?3, lastfm_sk = ?4 WHERE id = ?1";
            let values = params![id, creds.api_key, creds.secret, creds.session_key];
            changed(tx.prepare_cached(sql)?.execute(values)?)
        })
        .await
    }
}

fn create(conn: &Connection, name: &str, hash: &str, now: i64) -> Result<Account> {
    let name_fold = fold(name);
    let exists = "SELECT 1 FROM accounts WHERE name_fold = ?1";
    if conn.prepare_cached(exists)?.exists([&name_fold])? {
        return Err(StoreError::NameTaken);
    }
    let first = !conn
        .prepare_cached("SELECT 1 FROM accounts LIMIT 1")?
        .exists([])?;
    let (role, status) = match (first, signup_mode(conn)?) {
        (true, _) => (Role::Admin, AccountStatus::Active),
        (false, SignupMode::Closed) => return Err(StoreError::SignupClosed),
        (false, SignupMode::Open) => (Role::User, AccountStatus::Active),
        (false, SignupMode::Approval) => (Role::User, AccountStatus::Pending),
    };
    let sql = "INSERT INTO accounts (name, name_fold, password_hash, role, status, created_at) \
               VALUES (?1, ?2, ?3, ?4, ?5, ?6) RETURNING id";
    let values = params![name, name_fold, hash, role, status, now];
    let id: AccountId = conn
        .prepare_cached(sql)?
        .query_row(values, |row| row.get(0))?;
    one(conn, "id = ?1", id)?.ok_or(StoreError::NotFound)
}

fn one(conn: &Connection, filter: &str, value: impl rusqlite::ToSql) -> Result<Option<Account>> {
    let sql = format!("SELECT {COLUMNS} FROM accounts WHERE {filter}");
    Ok(conn
        .prepare_cached(&sql)?
        .query_row([value], account_from_row)
        .optional()?)
}

fn set_status(
    conn: &Connection,
    id: AccountId,
    from: &[AccountStatus],
    to: AccountStatus,
) -> Result<()> {
    let sql = "UPDATE accounts SET status = ?2 WHERE id = ?1 AND status IN (?3, ?4)";
    let first = from.first().copied().ok_or(StoreError::NotFound)?;
    let second = from.get(1).copied().unwrap_or(first);
    changed(
        conn.prepare_cached(sql)?
            .execute(params![id, to, first, second])?,
    )
}

/// Fails when `id` is the only active admin.
fn guard_last_admin(conn: &Connection, id: AccountId) -> Result<()> {
    let sql = "SELECT count(*) FROM accounts WHERE role = 'admin' AND status = 'active' \
               AND id != ?1";
    let others: i64 = conn
        .prepare_cached(sql)?
        .query_row([id], |row| row.get(0))?;
    let target = one(conn, "id = ?1", id)?.ok_or(StoreError::NotFound)?;
    let is_active_admin = target.role == Role::Admin && target.status == AccountStatus::Active;
    if is_active_admin && others == 0 {
        return Err(StoreError::LastAdmin);
    }
    Ok(())
}

fn account_from_row(row: &Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: row.get(0)?,
        name: row.get(1)?,
        password_hash: row.get(2)?,
        role: row.get(3)?,
        status: row.get(4)?,
        lastfm_user: row.get(5)?,
        credentials: LastfmCredentials {
            api_key: row.get(6)?,
            secret: row.get(7)?,
            session_key: row.get(8)?,
        },
        created_at: row.get(9)?,
    })
}
