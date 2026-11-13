//! Settings of the speed comparison, built by chaining setters on [`DemoConfig::new`].

use std::fmt;
use std::sync::Arc;

use lfm_api::ResponseCache;
use lfm_upstream::{ApiKey, DEFAULT_BASE_URL};

/// Largest Last.fm history a demo import copies, in scrobbles.
const MAX_SCROBBLES: u64 = 2_000_000;

/// Where the comparison sends its requests, and with which keys.
#[derive(Clone)]
pub struct DemoConfig {
    pub(crate) lastfm_url: String,
    pub(crate) lastfm_key: ApiKey,
    pub(crate) mirror_url: String,
    pub(crate) mirror_key: String,
    pub(crate) cache: Arc<dyn ResponseCache>,
    pub(crate) max_scrobbles: u64,
}

impl DemoConfig {
    /// Times Last.fm with `lastfm_key` from the instance pool, and this server at `mirror_url`
    /// (its own `/2.0/` over loopback) with `mirror_key`, an internal key the UI never sees.
    /// `cache` is the dispatcher's response cache, emptied for the user before each
    /// uncached measurement.
    #[must_use]
    pub fn new(
        lastfm_key: ApiKey,
        mirror_url: impl Into<String>,
        mirror_key: impl Into<String>,
        cache: Arc<dyn ResponseCache>,
    ) -> Self {
        Self {
            lastfm_url: DEFAULT_BASE_URL.to_owned(),
            lastfm_key,
            mirror_url: mirror_url.into(),
            mirror_key: mirror_key.into(),
            cache,
            max_scrobbles: MAX_SCROBBLES,
        }
    }

    /// Overrides Last.fm's endpoint, for tests against a mock server.
    #[must_use]
    pub fn lastfm_url(self, lastfm_url: impl Into<String>) -> Self {
        Self {
            lastfm_url: lastfm_url.into(),
            ..self
        }
    }

    /// The largest history a demo import copies. Defaults to 2,000,000 scrobbles.
    #[must_use]
    pub fn max_scrobbles(self, max_scrobbles: u64) -> Self {
        Self {
            max_scrobbles,
            ..self
        }
    }
}

impl fmt::Debug for DemoConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DemoConfig")
            .field("lastfm_url", &self.lastfm_url)
            .field("mirror_url", &self.mirror_url)
            .field("max_scrobbles", &self.max_scrobbles)
            .finish_non_exhaustive()
    }
}
