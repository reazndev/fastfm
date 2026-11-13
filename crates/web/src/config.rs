//! Settings of the web API, built by chaining setters on [`WebConfig::new`].

use std::path::PathBuf;

use lfm_upstream::AppCredentials;

/// When the session cookie gets the `Secure` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CookiePolicy {
    /// When the request came over HTTPS: the URL's scheme, or `X-Forwarded-Proto: https` from a
    /// trusted proxy.
    #[default]
    Auto,
    /// Always, for instances that are only reached over HTTPS.
    Always,
    /// Never, for plain-HTTP development.
    Never,
}

/// Settings for [`crate::Web`].
#[derive(Debug, Clone)]
pub struct WebConfig {
    pub(crate) data_dir: PathBuf,
    pub(crate) cookies: CookiePolicy,
    pub(crate) trust_proxy: bool,
    pub(crate) public_url: Option<String>,
    pub(crate) lastfm_app: Option<AppCredentials>,
    pub(crate) key_pool_size: usize,
}

impl WebConfig {
    /// Defaults, with the secrets key file kept in `data_dir`.
    #[must_use]
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            cookies: CookiePolicy::Auto,
            trust_proxy: false,
            public_url: None,
            lastfm_app: None,
            key_pool_size: 0,
        }
    }

    /// When the session cookie is `Secure`. Defaults to [`CookiePolicy::Auto`].
    #[must_use]
    pub fn cookies(self, cookies: CookiePolicy) -> Self {
        Self { cookies, ..self }
    }

    /// Trusts `X-Forwarded-For` and `X-Forwarded-Proto`, for instances behind a reverse proxy.
    /// Off by default: a client could otherwise pick its own IP for the login rate limit.
    #[must_use]
    pub fn trust_proxy(self, trust_proxy: bool) -> Self {
        Self {
            trust_proxy,
            ..self
        }
    }

    /// The instance's public base URL, such as `https://fm.example.org`, for the Last.fm web
    /// auth callback. Without it the request's `Host` header is used.
    #[must_use]
    pub fn public_url(self, public_url: impl Into<String>) -> Self {
        Self {
            public_url: Some(public_url.into().trim_end_matches('/').to_owned()),
            ..self
        }
    }

    /// The instance's own Last.fm application: the key and shared secret that sign web auth and
    /// forwarded writes for accounts without their own key.
    #[must_use]
    pub fn lastfm_app(self, lastfm_app: AppCredentials) -> Self {
        Self {
            lastfm_app: Some(lastfm_app),
            ..self
        }
    }

    /// How many keys the instance's Last.fm key pool holds, shown to admins.
    #[must_use]
    pub fn key_pool_size(self, key_pool_size: usize) -> Self {
        Self {
            key_pool_size,
            ..self
        }
    }
}
