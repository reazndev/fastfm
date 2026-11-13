//! Read-through cache in front of Last.fm for the methods the mirror does not compute.
//!
//! An answer is served from memory, then from the store's `upstream_cache` table, then from
//! Last.fm. A stale answer is served at once and refreshed in the background on the pre-warm
//! lane, once per key. Concurrent misses of the same key share one upstream call.

mod entry;
mod flight;
mod methods;
mod overlay;

use std::sync::Arc;

use lfm_core::{ApiError, ErrorCode, Format};
use lfm_store::Store;
use lfm_upstream::{Client, Priority, RawRequest};

use crate::clock::system_now;
use crate::dispatch::Source;
use flight::Flights;
use methods::EntryId;

pub use entry::Entry;
pub use methods::{Forward, Forwarded, forwarded};
pub(crate) use overlay::{Overlay, lookup as overlay_lookup, overlay_user};

/// Memory budget of the hot layer in front of the store.
const MEMORY_BYTES: u64 = 64 << 20;

/// Why Last.fm could not answer: an outage or a rate limit that outlasted the retries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Unavailable {
    /// Network failure, HTTP 5xx, or error 8, 11 or 16.
    #[error("Last.fm is unavailable")]
    Down,
    /// Error 29 or HTTP 429.
    #[error("Last.fm rate limit exceeded")]
    RateLimited,
}

impl From<Unavailable> for ApiError {
    fn from(failure: Unavailable) -> Self {
        match failure {
            Unavailable::Down => ErrorCode::TemporarilyUnavailable.into(),
            Unavailable::RateLimited => ErrorCode::RateLimitExceeded.into(),
        }
    }
}

type Outcome = Result<Arc<Entry>, Unavailable>;

/// The read-through cache. Cheap to clone; clones share the cache and the calls in flight.
#[derive(Clone, Debug)]
pub struct Passthrough {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    store: Store,
    client: Arc<Client>,
    memory: moka::sync::Cache<EntryId, Arc<Entry>>,
    flights: Flights<EntryId, Outcome>,
}

impl Passthrough {
    /// A cache over `store`'s `upstream_cache` table that fills from `client`'s key pool.
    #[must_use]
    pub fn new(store: Store, client: Arc<Client>) -> Self {
        let memory = moka::sync::Cache::builder()
            .max_capacity(MEMORY_BYTES)
            .weigher(|_, entry: &Arc<Entry>| entry.weight())
            .build();
        let inner = Inner {
            store,
            client,
            memory,
            flights: Flights::default(),
        };
        Self {
            inner: Arc::new(inner),
        }
    }

    /// Last.fm's answer to `request`, and where it came from.
    ///
    /// # Errors
    /// When nothing is cached and Last.fm does not answer.
    pub async fn get(&self, request: &Forward) -> Result<(Arc<Entry>, Source), Unavailable> {
        match self.cached(&request.id()).await {
            Some(entry) if entry.is_fresh(now()) => Ok((entry, Source::Cache)),
            Some(entry) => {
                self.refresh(request);
                Ok((entry, Source::Stale))
            }
            None => {
                let entry = self.fetch(request, Priority::Interactive).await?;
                Ok((entry, Source::Upstream))
            }
        }
    }

    /// Fetches `request` on the pre-warm lane unless a fresh answer is cached. Returns whether
    /// an upstream call was made.
    ///
    /// # Errors
    /// When Last.fm does not answer.
    pub async fn warm(&self, request: &Forward) -> Result<bool, Unavailable> {
        if self
            .cached(&request.id())
            .await
            .is_some_and(|entry| entry.is_fresh(now()))
        {
            return Ok(false);
        }
        self.fetch(request, Priority::Prewarm).await.map(|_| true)
    }

    async fn cached(&self, id: &EntryId) -> Option<Arc<Entry>> {
        if let Some(entry) = self.inner.memory.get(id) {
            return Some(entry);
        }
        let (key, format) = id;
        let found = self.inner.store.cache_get(key, format.as_str()).await;
        let entry = match found {
            Ok(found) => Arc::new(Entry::from(found?)),
            Err(error) => {
                tracing::warn!(%error, "reading the upstream cache failed");
                return None;
            }
        };
        self.inner.memory.insert(id.clone(), Arc::clone(&entry));
        Some(entry)
    }

    // A refresh already in flight covers this one too.
    fn refresh(&self, request: &Forward) {
        if self.inner.flights.is_running(&request.id()) {
            return;
        }
        let (this, request) = (self.clone(), request.clone());
        tokio::spawn(async move {
            if let Err(error) = this.fetch(&request, Priority::Prewarm).await {
                tracing::debug!(%error, key = request.key(), "background refresh failed");
            }
        });
    }

    async fn fetch(&self, request: &Forward, priority: Priority) -> Outcome {
        let id = request.id();
        let call = || self.fetch_and_store(request, priority);
        self.inner.flights.run(&id, call).await
    }

    async fn fetch_and_store(&self, request: &Forward, priority: Priority) -> Outcome {
        let format = match request.format {
            Format::Xml => lfm_upstream::Format::Xml,
            Format::Json => lfm_upstream::Format::Json,
        };
        let call = RawRequest::new(request.method.name)
            .params(request.params.iter().cloned())
            .format(format)
            .priority(priority);
        let response = self.inner.client.raw(&call).await.map_err(|error| {
            tracing::warn!(%error, method = request.method.name, "Last.fm call failed");
            Unavailable::Down
        })?;
        if response.is_transient() {
            let limited = response.status == 429
                || response
                    .lastfm_error()
                    .is_some_and(|error| error.code == 29);
            tracing::warn!(status = response.status, "Last.fm failed after retries");
            return Err(if limited {
                Unavailable::RateLimited
            } else {
                Unavailable::Down
            });
        }
        let ttl = if response.status == 200 && response.lastfm_error().is_none() {
            request.method.ttl
        } else {
            methods::ERRORS
        };
        let entry = Arc::new(Entry::from_response(response, now(), ttl));
        let stored = (&*entry).into();
        let (key, format) = request.id();
        if let Err(error) = self
            .inner
            .store
            .cache_put(&key, format.as_str(), stored)
            .await
        {
            tracing::warn!(%error, "writing the upstream cache failed");
        }
        self.inner.memory.insert((key, format), Arc::clone(&entry));
        Ok(entry)
    }
}

/// Cache times are wall-clock times, whatever the request clock says.
fn now() -> i64 {
    i64::try_from(system_now()).unwrap_or(i64::MAX)
}
