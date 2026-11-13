//! What every handler shares.

use std::ops::Deref;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use lfm_api::StoreKeys;
use lfm_store::Store;
use lfm_sync::SyncService;
use lfm_upstream::Client;

use crate::attempts::Attempts;
use crate::auth_states::AuthStates;
use crate::config::WebConfig;
use crate::crypto::Vault;
use crate::demo::Demo;
use crate::error::WebError;
use crate::password::Passwords;

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
    /// The speed comparison, when configured.
    demo: Option<Demo>,
    /// Limits on password guessing, the web form and `auth.getMobileSession` together.
    pub(crate) attempts: Attempts,
    /// Started Last.fm web auth flows.
    pub(crate) auth_states: AuthStates,
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
            demo: parts.demo,
            attempts: Attempts::new(),
            auth_states: AuthStates::default(),
        }))
    }

    /// The speed comparison, or an error when this instance has none.
    pub(crate) fn demo(&self) -> Result<&Demo, WebError> {
        self.demo.as_ref().ok_or_else(|| {
            WebError::new(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "demo_unavailable",
                "The speed comparison needs a Last.fm API key on this server.",
            )
        })
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
    pub(crate) demo: Option<Demo>,
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
