//! [`ApiKeys`] and [`Sessions`] over the store.
//!
//! Active keys are held in memory, loaded at start and on [`StoreKeys::refresh`], so checking a
//! key costs no query. Key use is noted in the store's batch, which `flush_key_usage` writes.

use std::collections::HashMap;
use std::sync::{PoisonError, RwLock};

use async_trait::async_trait;
use lfm_store::{AccountStatus, KeyScope, Store, StoreError};

use crate::auth::{ApiKey, ApiKeys, AuthError, Scope, Session, Sessions};
use crate::clock::system_now;

/// API keys from the store, plus keys registered in code (such as the development key).
#[derive(Debug)]
pub struct StoreKeys {
    store: Store,
    fixed: HashMap<String, ApiKey>,
    active: RwLock<HashMap<String, ApiKey>>,
}

impl StoreKeys {
    /// Loads the active keys. `fixed` keys are always valid and never touch the store.
    pub async fn load(store: Store, fixed: HashMap<String, ApiKey>) -> Result<Self, StoreError> {
        let keys = Self {
            store,
            fixed,
            active: RwLock::default(),
        };
        keys.refresh().await?;
        Ok(keys)
    }

    /// Reloads the active keys, picking up new, revoked and disabled ones.
    pub async fn refresh(&self) -> Result<(), StoreError> {
        let loaded: HashMap<String, ApiKey> = self
            .store
            .active_api_keys()
            .await?
            .into_iter()
            .map(|key| {
                let record = ApiKey {
                    account_id: key.account.0,
                    secret: key.secret,
                    scope: match key.scope {
                        KeyScope::Read => Scope::Read,
                        KeyScope::Write => Scope::ReadWrite,
                    },
                    active: true,
                };
                (key.key, record)
            })
            .collect();
        *self.active.write().unwrap_or_else(PoisonError::into_inner) = loaded;
        Ok(())
    }
}

#[async_trait]
impl ApiKeys for StoreKeys {
    async fn api_key(&self, key: &str) -> Result<Option<ApiKey>, AuthError> {
        if let Some(record) = self.fixed.get(key) {
            return Ok(Some(record.clone()));
        }
        let record = self
            .active
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned();
        if record.is_some() {
            let now = i64::try_from(system_now()).unwrap_or(i64::MAX);
            self.store.mark_key_used(key, now);
        }
        Ok(record)
    }
}

/// Session keys from the store. Only write methods look them up, so each lookup may query.
#[derive(Debug, Clone)]
pub struct StoreSessions {
    store: Store,
}

impl StoreSessions {
    /// Sessions over `store`.
    #[must_use]
    pub fn new(store: Store) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Sessions for StoreSessions {
    async fn session(&self, session_key: &str) -> Result<Option<Session>, AuthError> {
        let failed = |error: StoreError| AuthError(error.to_string());
        let Some(session) = self.store.api_session(session_key).await.map_err(failed)? else {
            return Ok(None);
        };
        let account = self.store.account(session.account).await.map_err(failed)?;
        Ok(account
            .filter(|account| account.status == AccountStatus::Active)
            .map(|account| Session {
                account_id: account.id.0,
                user: account.lastfm_user.unwrap_or(account.name),
            }))
    }
}
