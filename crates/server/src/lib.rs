//! The mirror's HTTP server: configuration, routes and service wiring.

pub mod config;
pub mod routes;

use std::sync::Arc;
use std::time::Duration;

use lfm_api::{Dispatcher, InMemoryApiKeys, InMemorySessions, MemoryCache, Services};

pub use config::Config;
pub use routes::router;

/// Response cache budget: 256 MiB of rendered bodies, each kept for a day at most.
const CACHE_BYTES: u64 = 256 << 20;
const CACHE_TTL: Duration = Duration::from_secs(86_400);

/// Builds the dispatcher with every registered method.
///
/// Until the store lands, keys and sessions are empty in-memory maps, so every call gets
/// error 10.
#[must_use]
pub fn dispatcher() -> Arc<Dispatcher> {
    let services = Services {
        api_keys: Arc::new(InMemoryApiKeys::default()),
        sessions: Arc::new(InMemorySessions::default()),
        cache: Arc::new(MemoryCache::new(CACHE_BYTES, CACHE_TTL)),
    };
    Arc::new(Dispatcher::new(lfm_api::methods::registry(), services))
}
