//! Instance settings, stored as text by key.

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Result;
use crate::pool::Store;
use crate::text::text_enum;

text_enum!(
    /// Who may create an account.
    SignupMode {
        /// New accounts wait for an admin's approval. The default.
        Approval => "approval",
        /// New accounts are active at once.
        Open => "open",
        /// Nobody can sign up, except the very first account.
        Closed => "closed",
    }
);

const SIGNUP_MODE: &str = "signup_mode";

impl Store {
    /// The value of setting `key`, if set.
    pub async fn setting(&self, key: &str) -> Result<Option<String>> {
        let key = key.to_owned();
        self.read(move |conn| get(conn, &key)).await
    }

    /// Sets setting `key` to `value`.
    pub async fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let (key, value) = (key.to_owned(), value.to_owned());
        self.write(move |tx| set(tx, &key, &value)).await
    }

    /// The sign-up mode; `approval` when unset.
    pub async fn signup_mode(&self) -> Result<SignupMode> {
        self.read(signup_mode).await
    }

    /// Sets the sign-up mode.
    pub async fn set_signup_mode(&self, mode: SignupMode) -> Result<()> {
        self.write(move |tx| set(tx, SIGNUP_MODE, mode.as_str()))
            .await
    }
}

pub(crate) fn signup_mode(conn: &Connection) -> Result<SignupMode> {
    get(conn, SIGNUP_MODE)?.map_or(Ok(SignupMode::Approval), |text| text.parse())
}

fn get(conn: &Connection, key: &str) -> Result<Option<String>> {
    let sql = "SELECT value FROM settings WHERE key = ?1";
    Ok(conn
        .prepare_cached(sql)?
        .query_row([key], |row| row.get(0))
        .optional()?)
}

fn set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    let sql = "INSERT INTO settings (key, value) VALUES (?1, ?2) \
               ON CONFLICT DO UPDATE SET value = excluded.value";
    conn.prepare_cached(sql)?.execute(params![key, value])?;
    Ok(())
}
