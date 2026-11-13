//! API key and session key lookup, as traits the store implements, with in-memory versions.

use std::collections::HashMap;

use async_trait::async_trait;

/// What an API key may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Read methods only.
    Read,
    /// Read and write methods.
    ReadWrite,
}

/// An API key as the dispatcher needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKey {
    /// The account that owns the key.
    pub account_id: i64,
    /// The shared secret used to verify `api_sig`.
    pub secret: String,
    /// What the key may do.
    pub scope: Scope,
    /// False when the key or its account is disabled.
    pub active: bool,
}

/// A session key as the dispatcher needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// The account the session acts for.
    pub account_id: i64,
    /// The Last.fm user name of that account.
    pub user: String,
    /// The API key the session was issued to. Only that key may use it, as on Last.fm, so a
    /// leaked session key is no use with another key.
    pub api_key: String,
}

/// A failure of the backing store, as opposed to "not found".
#[derive(Debug, thiserror::Error)]
#[error("auth backend failed: {0}")]
pub struct AuthError(pub String);

impl From<AuthError> for lfm_core::ApiError {
    fn from(error: AuthError) -> Self {
        tracing::error!(%error, "auth lookup failed");
        lfm_core::ErrorCode::TemporarilyUnavailable.into()
    }
}

/// Looks up API keys.
#[async_trait]
pub trait ApiKeys: Send + Sync {
    /// The key record, or `None` when the key does not exist.
    async fn api_key(&self, key: &str) -> Result<Option<ApiKey>, AuthError>;
}

/// Looks up session keys (`sk`).
#[async_trait]
pub trait Sessions: Send + Sync {
    /// The session, or `None` when the session key does not exist.
    async fn session(&self, session_key: &str) -> Result<Option<Session>, AuthError>;
}

/// API keys held in a map, for tests and for running without a store.
#[derive(Debug, Clone, Default)]
pub struct InMemoryApiKeys {
    keys: HashMap<String, ApiKey>,
}

impl InMemoryApiKeys {
    /// Adds a key.
    #[must_use]
    pub fn with(mut self, key: impl Into<String>, record: ApiKey) -> Self {
        self.keys.insert(key.into(), record);
        self
    }
}

#[async_trait]
impl ApiKeys for InMemoryApiKeys {
    async fn api_key(&self, key: &str) -> Result<Option<ApiKey>, AuthError> {
        Ok(self.keys.get(key).cloned())
    }
}

/// Session keys held in a map, for tests and for running without a store.
#[derive(Debug, Clone, Default)]
pub struct InMemorySessions {
    sessions: HashMap<String, Session>,
}

impl InMemorySessions {
    /// Adds a session.
    #[must_use]
    pub fn with(mut self, session_key: impl Into<String>, session: Session) -> Self {
        self.sessions.insert(session_key.into(), session);
        self
    }
}

#[async_trait]
impl Sessions for InMemorySessions {
    async fn session(&self, session_key: &str) -> Result<Option<Session>, AuthError> {
        Ok(self.sessions.get(session_key).cloned())
    }
}
