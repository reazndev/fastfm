use bytes::Bytes;
use reqwest::header::HeaderMap;

use crate::config::ApiKey;
use crate::error::LastFmError;
use crate::retry::{detect_error, is_retryable};
use crate::signed::Secret;

/// Scheduling lane. Waiting requests in a higher lane are sent first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Priority {
    /// A client is waiting on the answer (pass-through).
    #[default]
    Interactive,
    /// Live sync polling.
    Live,
    /// History import.
    Import,
    /// Cache pre-warming.
    Prewarm,
}

impl Priority {
    /// Every lane, highest first.
    pub(crate) const ALL: [Self; 4] = [Self::Interactive, Self::Live, Self::Import, Self::Prewarm];
}

/// Response format requested from Last.fm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// Last.fm's default. No `format` parameter is sent.
    #[default]
    Xml,
    /// Sends `format=json`.
    Json,
}

/// HTTP verb of the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HttpMethod {
    /// Parameters in the query string.
    #[default]
    Get,
    /// Parameters as a form body.
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum KeyChoice {
    #[default]
    Pool,
    Own(ApiKey),
    Omit,
}

/// One Last.fm call, built by chaining setters on [`RawRequest::new`].
///
/// `params` must not contain `method`, `api_key` or `format`; the client adds those.
#[derive(Debug, Clone, Default)]
pub struct RawRequest {
    pub(crate) method: String,
    pub(crate) params: Vec<(String, String)>,
    pub(crate) format: Format,
    pub(crate) priority: Priority,
    pub(crate) http_method: HttpMethod,
    pub(crate) key: KeyChoice,
    /// Shared secret that signs the request with `api_sig`, for auth and write methods.
    pub(crate) secret: Option<Secret>,
    /// Sends once, without retries, for writes a client is waiting on.
    pub(crate) single_attempt: bool,
}

impl RawRequest {
    /// Starts a request for a Last.fm method such as `user.getInfo`.
    #[must_use]
    pub fn new(method: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            ..Self::default()
        }
    }

    /// Adds one parameter.
    #[must_use]
    pub fn param(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((name.into(), value.into()));
        self
    }

    /// Adds several parameters in order.
    #[must_use]
    pub fn params<K, V>(mut self, params: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.params
            .extend(params.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }

    /// Chooses XML or JSON.
    #[must_use]
    pub fn format(self, format: Format) -> Self {
        Self { format, ..self }
    }

    /// Chooses the scheduling lane.
    #[must_use]
    pub fn priority(self, priority: Priority) -> Self {
        Self { priority, ..self }
    }

    /// Chooses GET or POST.
    #[must_use]
    pub fn http_method(self, http_method: HttpMethod) -> Self {
        Self {
            http_method,
            ..self
        }
    }

    /// Uses this key instead of the pool. The key gets its own token bucket.
    #[must_use]
    pub fn api_key(self, key: ApiKey) -> Self {
        Self {
            key: KeyChoice::Own(key),
            ..self
        }
    }

    /// Sends no `api_key` at all, to observe Last.fm's error. Still takes a pool token.
    #[must_use]
    pub fn without_api_key(self) -> Self {
        Self {
            key: KeyChoice::Omit,
            ..self
        }
    }

    /// The Last.fm method name.
    #[must_use]
    pub fn method_name(&self) -> &str {
        &self.method
    }

    /// The parameters as sent, in order, with `key_text` in place of the key.
    ///
    /// The recorder passes `Some("REDACTED")` to describe a request without the real key.
    #[must_use]
    pub fn wire_params(&self, key_text: Option<&str>) -> Vec<(String, String)> {
        let method = ("method".to_owned(), self.method.clone());
        let key = key_text.map(|k| ("api_key".to_owned(), k.to_owned()));
        let format =
            (self.format == Format::Json).then(|| ("format".to_owned(), "json".to_owned()));
        std::iter::once(method)
            .chain(self.params.iter().cloned())
            .chain(key)
            .chain(format)
            .collect()
    }

    /// Whether the request sends an `api_key` parameter.
    #[must_use]
    pub fn sends_key(&self) -> bool {
        self.key != KeyChoice::Omit
    }
}

/// Last.fm's answer, untouched.
#[derive(Debug, Clone)]
pub struct RawResponse {
    /// HTTP status code.
    pub status: u16,
    /// The `Content-Type` header, if any.
    pub content_type: Option<String>,
    /// All response headers.
    pub headers: HeaderMap,
    /// The body bytes exactly as received.
    pub body: Bytes,
}

impl RawResponse {
    /// The Last.fm error in the body, for either format.
    #[must_use]
    pub fn lastfm_error(&self) -> Option<LastFmError> {
        detect_error(&self.body)
    }

    /// Whether this is a failure the client retries (codes 8, 11, 16, 29, HTTP 429 and 5xx),
    /// which a cache should not keep as Last.fm's answer.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        is_retryable(self.status, self.lastfm_error().as_ref())
    }

    /// Whether the status is 2xx.
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}
