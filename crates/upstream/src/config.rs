use std::fmt;
use std::num::NonZeroU32;
use std::time::Duration;

/// Last.fm's public API endpoint.
pub const DEFAULT_BASE_URL: &str = "https://ws.audioscrobbler.com/2.0/";

const DEFAULT_RPS: NonZeroU32 = NonZeroU32::MIN.saturating_add(3);

/// A Last.fm API key. Its `Debug` output is redacted so it never reaches a log.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ApiKey(String);

impl ApiKey {
    /// Wraps a key string.
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// The key itself, for building a request.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(REDACTED)")
    }
}

/// How the key pool shares its rate limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BucketMode {
    /// One token bucket per key. Right when Last.fm limits per key.
    #[default]
    PerKey,
    /// One token bucket for the whole pool. Right when Last.fm limits per IP.
    Shared,
}

/// Retry settings for transient failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Total attempts, including the first one.
    pub max_attempts: u32,
    /// Delay before the second attempt; it doubles after each further failure.
    pub base_delay: Duration,
    /// Upper bound for one delay.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
        }
    }
}

/// Settings for [`crate::Client`], built by chaining setters on [`ClientConfig::new`].
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub(crate) api_keys: Vec<ApiKey>,
    pub(crate) base_url: String,
    pub(crate) requests_per_second: NonZeroU32,
    pub(crate) user_agent: String,
    pub(crate) bucket_mode: BucketMode,
    pub(crate) retry: RetryPolicy,
    pub(crate) timeout: Duration,
}

impl ClientConfig {
    /// Starts a config for the given key pool. The pool may be empty when every call brings
    /// its own key.
    #[must_use]
    pub fn new(api_keys: impl IntoIterator<Item = ApiKey>) -> Self {
        Self {
            api_keys: api_keys.into_iter().collect(),
            base_url: DEFAULT_BASE_URL.to_owned(),
            requests_per_second: DEFAULT_RPS,
            user_agent: concat!("lfm-mirror/", env!("CARGO_PKG_VERSION")).to_owned(),
            bucket_mode: BucketMode::default(),
            retry: RetryPolicy::default(),
            timeout: Duration::from_secs(30),
        }
    }

    /// Overrides the endpoint, for tests against a mock server.
    #[must_use]
    pub fn base_url(self, base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            ..self
        }
    }

    /// Sets the request rate of one bucket. Defaults to 4.
    #[must_use]
    pub fn requests_per_second(self, requests_per_second: NonZeroU32) -> Self {
        Self {
            requests_per_second,
            ..self
        }
    }

    /// Sets the `User-Agent` header.
    #[must_use]
    pub fn user_agent(self, user_agent: impl Into<String>) -> Self {
        Self {
            user_agent: user_agent.into(),
            ..self
        }
    }

    /// Chooses one bucket per key or one shared bucket.
    #[must_use]
    pub fn bucket_mode(self, bucket_mode: BucketMode) -> Self {
        Self {
            bucket_mode,
            ..self
        }
    }

    /// Sets the retry policy.
    #[must_use]
    pub fn retry(self, retry: RetryPolicy) -> Self {
        Self { retry, ..self }
    }

    /// Sets the timeout of one HTTP attempt.
    #[must_use]
    pub fn timeout(self, timeout: Duration) -> Self {
        Self { timeout, ..self }
    }
}
