//! The JSON API behind the web UI, mounted at `/app/api`: sign-up and login, admin, API keys,
//! the Last.fm link, and the approval page of the emulated Last.fm auth flow. It also implements
//! [`lfm_api::AccountServices`], through which `auth.getMobileSession` checks passwords and the
//! `track.*` writes are stored locally and forwarded to Last.fm.
//!
//! Every response is JSON; errors are `{"error": {"code", "message"}}`. Requests other than
//! `GET`, `HEAD` and `OPTIONS` must carry an `X-Requested-With` header (any value), as must
//! `GET /lastfm/auth/callback`.

mod bridge;
mod config;
pub mod crypto;
mod error;
mod forward;
mod link;
mod outbox;
mod password;
mod routes;
mod session;
mod state;
mod throttle;
mod views;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use lfm_api::{AccountServices, StoreKeys};
use lfm_store::Store;
use lfm_sync::SyncService;
use lfm_upstream::Client;

pub use config::{CookiePolicy, WebConfig};
pub use error::WebError;
pub use routes::app_auth::AUTHORIZE_ROUTE;
pub use routes::lastfm_auth::CALLBACK_ROUTE;

use crate::bridge::Accounts;
use crate::crypto::Vault;
use crate::password::Passwords;
use crate::state::{AppState, Parts};

/// The services the web API works with, shared with the rest of the server.
#[derive(Debug, Clone)]
pub struct WebDeps {
    /// The database.
    pub store: Store,
    /// Import and live sync, which `PUT /lastfm` starts.
    pub sync: Arc<SyncService>,
    /// The Last.fm client, for web auth and forwarded writes.
    pub upstream: Arc<Client>,
    /// The dispatcher's key map, reloaded when keys or accounts change.
    pub keys: Arc<StoreKeys>,
}

/// The running web API.
#[derive(Debug, Clone)]
pub struct Web {
    state: AppState,
}

impl Web {
    /// Opens or creates the secrets key file in the data directory and starts retrying queued
    /// writes in the background. Needs a tokio runtime.
    pub async fn start(config: WebConfig, deps: WebDeps) -> Result<Self, WebError> {
        let dir = config.data_dir.clone();
        let (vault, passwords) = tokio::task::spawn_blocking(move || {
            let vault = Vault::open_or_create(&dir)?;
            Ok::<_, WebError>((vault, Passwords::new()?))
        })
        .await
        .map_err(|error| WebError::internal(&error))??;
        let state = AppState::new(Parts {
            store: deps.store,
            sync: deps.sync,
            upstream: deps.upstream,
            keys: deps.keys,
            vault,
            passwords,
            config,
        });
        tokio::spawn(outbox::run(state.clone()));
        Ok(Self { state })
    }

    /// The routes to mount at `/app/api`.
    pub fn router<S>(&self) -> Router<S> {
        routes::routes().with_state(self.state.clone())
    }

    /// What the dispatcher's account methods need; pass it to
    /// [`lfm_api::methods::with_accounts`].
    #[must_use]
    pub fn accounts(&self) -> Arc<dyn AccountServices> {
        Arc::new(Accounts(self.state.clone()))
    }

    /// The Last.fm users that active accounts link, which the server keeps in sync. Accounts
    /// with their own Last.fm key get it registered with the sync service first, so the
    /// import and live sync of their user use it.
    pub async fn linked_users(&self) -> Result<Vec<String>, WebError> {
        link::linked_users(&self.state).await
    }

    /// Retries the queued writes that are due now, as the background task does every 30
    /// seconds. Returns how many reached Last.fm.
    pub async fn retry_writes(&self) -> usize {
        outbox::retry_due(&self.state).await
    }
}

/// `GET /api/auth` and `/api/auth/`, where Last.fm clients send users to approve a token:
/// redirects to the UI's [`AUTHORIZE_ROUTE`] with the same query. Merge it at the root.
pub fn auth_page_redirect<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let redirect = get(routes::app_auth::redirect);
    Router::new()
        .route("/api/auth", redirect.clone())
        .route("/api/auth/", redirect)
}
