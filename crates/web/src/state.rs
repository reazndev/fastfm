//! What every handler shares.

use std::ops::Deref;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use lfm_api::StoreKeys;
use lfm_store::Store;
use lfm_sync::SyncService;
use lfm_upstream::Client;

use crate::config::WebConfig;
use crate::crypto::Vault;
use crate::password::Passwords;
use crate::throttle::Throttle;

/// Failed logins allowed per IP, and per username, within [`LOGIN_WINDOW`].
const LOGINS_PER_IP: u32 = 30;
const LOGINS_PER_NAME: u32 = 10;
/// The rate-limit window in seconds.
const LOGIN_WINDOW: i64 = 900;

/// The handlers' shared state. Cheap to clone.
#[derive(Debug, Clone)]
pub(crate) struct AppState(Arc<Inner>);

#[derive(Debug)]
pub(crate) struct Inner {
    pub(crate) store: Store,
    pub(crate) sync: Arc<SyncService>,
    pub(crate) upstream: Arc<Client>,
    pub(crate) keys: Arc<StoreKeys>,
    pub(crate) vault: Vault,
    pub(crate) passwords: Passwords,
    pub(crate) config: WebConfig,
    /// Failed logins and sign-ups per client IP.
    pub(crate) ip_attempts: Throttle,
    /// Failed logins per username, the web form and `auth.getMobileSession` together.
    pub(crate) name_attempts: Throttle,
}

impl AppState {
    pub(crate) fn new(parts: Parts) -> Self {
        Self(Arc::new(Inner {
            store: parts.store,
            sync: parts.sync,
            upstream: parts.upstream,
            keys: parts.keys,
            vault: parts.vault,
            passwords: parts.passwords,
            config: parts.config,
            ip_attempts: Throttle::new(LOGINS_PER_IP, LOGIN_WINDOW),
            name_attempts: Throttle::new(LOGINS_PER_NAME, LOGIN_WINDOW),
        }))
    }

    /// Reloads the dispatcher's key map, so a new, revoked or disabled key takes effect now
    /// rather than at the next maintenance round.
    pub(crate) async fn refresh_keys(&self) {
        if let Err(error) = self.keys.refresh().await {
            tracing::warn!(%error, "reloading API keys failed");
        }
    }
}

/// The parts [`AppState::new`] puts together.
#[derive(Debug)]
pub(crate) struct Parts {
    pub(crate) store: Store,
    pub(crate) sync: Arc<SyncService>,
    pub(crate) upstream: Arc<Client>,
    pub(crate) keys: Arc<StoreKeys>,
    pub(crate) vault: Vault,
    pub(crate) passwords: Passwords,
    pub(crate) config: WebConfig,
}

impl Deref for AppState {
    type Target = Inner;

    fn deref(&self) -> &Inner {
        &self.0
    }
}

/// The current Unix time in seconds.
pub(crate) fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}
