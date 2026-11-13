//! Read-through cache in front of Last.fm for the methods the mirror does not compute.
//!
//! An answer is served from memory, then from the store's `upstream_cache` table, then from
//! Last.fm. A stale answer is served at once and refreshed in the background on the pre-warm
//! lane, once per key. Concurrent misses of the same key share one upstream call.

mod entry;
mod flight;
mod keys;
mod limit;
mod methods;
mod overlay;

use std::sync::Arc;

use lfm_core::{ApiError, ErrorCode, Format};
use lfm_store::Store;
use lfm_upstream::{Client, Priority, RawRequest};

use crate::clock::system_now;
use crate::dispatch::Source;
use flight::Flights;
use limit::MissLimiter;
use methods::EntryId;

pub use entry::Entry;
pub use keys::{Caller, CallerKeys};
pub use methods::{Forward, Forwarded, forwarded};
pub(crate) use overlay::{Overlay, lookup as overlay_lookup, overlay_user};

/// Upstream misses allowed per API key and second, unless set with [`Passthrough::miss_rate`].
const MISSES_PER_SECOND: f64 = 5.0;

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
    misses: Arc<MissLimiter>,
    /// Set on an instance without a key pool: misses use the caller's own Last.fm key.
    caller_keys: Option<Arc<dyn CallerKeys>>,
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
            misses: Arc::new(MissLimiter::new(MISSES_PER_SECOND)),
            caller_keys: None,
        }
    }

    /// For an instance without a Last.fm key pool: a miss calls Last.fm with the own key of
    /// the account behind the request's API key, from `keys`, and fails with error 16 when
    /// that account has none.
    #[must_use]
    pub fn without_pool(self, keys: Arc<dyn CallerKeys>) -> Self {
        Self {
            caller_keys: Some(keys),
            ..self
        }
    }

    /// Lets each API key cause at most `per_second` calls to Last.fm for answers not cached
    /// (default 5), with bursts of twice that. Past it the key gets error 29. Keys registered
    /// in code are exempt; see [`Caller::limited`].
    #[must_use]
    pub fn miss_rate(self, per_second: f64) -> Self {
        Self {
            misses: Arc::new(MissLimiter::new(per_second)),
            ..self
        }
    }

    /// Last.fm's answer to `request` made with the API key `caller`, and where it came from.
    ///
    /// # Errors
    /// When nothing is cached and Last.fm does not answer, or `caller` has used up its misses.
    pub async fn get(
        &self,
        request: &Forward,
        caller: Caller<'_>,
    ) -> Result<(Arc<Entry>, Source), Unavailable> {
        let cached = self.cached(&request.id()).await;
        if cached.as_ref().is_some_and(|entry| entry.is_fresh(now())) {
            return cached
                .map(|entry| (entry, Source::Cache))
                .ok_or(Unavailable::Down);
        }
        let request = &self.keyed(request, caller).await?;
        match cached {
            Some(entry) if entry.is_fresh(now()) => Ok((entry, Source::Cache)),
            Some(entry) => {
                self.refresh(request);
                Ok((entry, Source::Stale))
            }
            None if caller.limited() && !self.misses.allow(caller.api_key) => {
                Err(Unavailable::RateLimited)
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

    /// `request` with the Last.fm key to call it with: the pool's (none set), or the caller's.
    async fn keyed(&self, request: &Forward, caller: Caller<'_>) -> Result<Forward, Unavailable> {
        let Some(keys) = &self.caller_keys else {
            return Ok(request.clone());
        };
        let Some(key) = keys.lastfm_key(caller.account_id).await else {
            tracing::warn!(
                account = caller.account_id,
                method = request.method.name,
                "no Last.fm key for this request: the instance has no key pool and the \
                 account behind the API key has no own Last.fm key"
            );
            return Err(Unavailable::Down);
        };
        Ok(Forward {
            api_key: Some(key),
            ..request.clone()
        })
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
        let call = match &request.api_key {
            Some(key) => call.api_key(key.clone()),
            None => call,
        };
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
