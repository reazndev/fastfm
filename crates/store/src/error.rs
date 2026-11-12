//! The store's error type.

/// Everything that can go wrong in the store.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// SQLite returned an error.
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// A store thread could not be started.
    #[error("could not start a store thread: {0}")]
    Spawn(#[from] std::io::Error),
    /// The store's worker threads have stopped.
    #[error("the store has stopped")]
    Closed,
    /// The row to change does not exist or is not in the required state.
    #[error("not found")]
    NotFound,
    /// Sign-up mode is `closed`.
    #[error("sign-up is closed")]
    SignupClosed,
    /// Another account already uses this name.
    #[error("the name is already taken")]
    NameTaken,
    /// The account is pending or disabled.
    #[error("the account is not active")]
    AccountNotActive,
    /// The change would leave the instance without an active admin.
    #[error("the last active admin cannot be removed")]
    LastAdmin,
    /// A scrobble given to `replace_window` lies outside the window.
    #[error("scrobble at {uts} lies outside the window {from}..{to}")]
    OutsideWindow {
        /// The scrobble's timestamp.
        uts: i64,
        /// Window start, inclusive.
        from: i64,
        /// Window end, exclusive.
        to: i64,
    },
    /// A stored value could not be decoded.
    #[error("invalid stored value: {0}")]
    InvalidValue(String),
}

/// Result type of the store.
pub type Result<T, E = StoreError> = std::result::Result<T, E>;

/// Turns "no row was changed" into `NotFound`.
pub(crate) fn changed(rows: usize) -> Result<()> {
    if rows == 0 {
        Err(StoreError::NotFound)
    } else {
        Ok(())
    }
}
