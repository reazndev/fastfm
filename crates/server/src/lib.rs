//! The mirror's HTTP server: configuration, routes and service wiring.

pub mod config;
pub mod routes;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use lfm_api::{ApiKey, Clock, Dispatcher, MemoryCache, Scope, Services, StoreKeys, StoreSessions};
use lfm_store::Store;

pub use config::{Config, DevKey};
pub use routes::router;

/// Response cache budget: 256 MiB of rendered bodies, each kept for a day at most.
const CACHE_BYTES: u64 = 256 << 20;
const CACHE_TTL: Duration = Duration::from_secs(86_400);

/// How often key use is written, keys are reloaded and period totals move on.
const MAINTENANCE_EVERY: Duration = Duration::from_secs(60);

/// The running service: the dispatcher and what background maintenance needs.
#[derive(Debug, Clone)]
pub struct App {
    /// Routes API calls.
    pub dispatcher: Arc<Dispatcher>,
    /// The in-memory key map, reloaded by [`App::maintain`].
    pub keys: Arc<StoreKeys>,
}

impl App {
    /// Wires the dispatcher to the store, with every registered method.
    ///
    /// # Errors
    /// When the API keys cannot be loaded.
    pub async fn new(store: Store, config: &Config) -> anyhow::Result<Self> {
        let fixed: HashMap<String, ApiKey> = config
            .dev_key
            .iter()
            .map(|dev| (dev.key.clone(), dev_key(dev)))
            .collect();
        let keys = Arc::new(
            StoreKeys::load(store.clone(), fixed)
                .await
                .context("loading API keys")?,
        );
        let services = Services {
            store: store.clone(),
            clock: config.now.map_or(Clock::System, Clock::Fixed),
            api_keys: Arc::clone(&keys) as Arc<dyn lfm_api::ApiKeys>,
            sessions: Arc::new(StoreSessions::new(store)),
            cache: Arc::new(MemoryCache::new(CACHE_BYTES, CACHE_TTL)),
        };
        let dispatcher = Arc::new(Dispatcher::new(lfm_api::methods::registry(), services));
        Ok(Self { dispatcher, keys })
    }

    /// Runs forever: every minute, writes key use, reloads keys and moves period totals to the
    /// current day. Failures are logged and retried on the next round.
    pub async fn maintain(self) {
        let services = self.dispatcher.services().clone();
        let mut ticks = tokio::time::interval(MAINTENANCE_EVERY);
        loop {
            ticks.tick().await;
            let now = i64::try_from(services.clock.now()).unwrap_or(i64::MAX);
            if let Err(error) = services.store.advance_periods(now).await {
                tracing::warn!(%error, "moving period totals failed");
            }
            if let Err(error) = services.store.flush_key_usage().await {
                tracing::warn!(%error, "writing key use failed");
            }
            if let Err(error) = self.keys.refresh().await {
                tracing::warn!(%error, "reloading API keys failed");
            }
        }
    }
}

fn dev_key(dev: &DevKey) -> ApiKey {
    ApiKey {
        account_id: 0,
        secret: dev.secret.clone(),
        scope: Scope::ReadWrite,
        active: true,
    }
}
