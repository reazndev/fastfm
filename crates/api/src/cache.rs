//! Cache of rendered responses, keyed by the user's data version.

use std::time::Duration;

use lfm_core::{Format, Output, Params};

use crate::body::Body;

/// Parameters that do not change a response's content. `format` and `callback` count through
/// the [`Output`] instead.
const IGNORED: [&str; 6] = ["api_key", "api_sig", "sk", "format", "callback", "method"];

/// Identifies one rendered response: user, method, canonical parameters, output and the
/// version of the user's data it was rendered from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    user: String,
    method: &'static str,
    params: String,
    format: Format,
    callback: Option<String>,
    version: u64,
}

impl CacheKey {
    /// Builds a key. The user is lowercased, because Last.fm user names ignore case, and the
    /// parameters are sorted, without the ones that do not change the content. A JSONP
    /// callback is part of the key, since the cache holds the final bytes.
    #[must_use]
    pub fn new(user: &str, method: &'static str, params: &Params, output: &Output) -> Self {
        let mut canonical = form_urlencoded::Serializer::new(String::new());
        for (name, value) in params.effective() {
            if !IGNORED.contains(&name) {
                canonical.append_pair(name, value);
            }
        }
        Self {
            user: user.to_lowercase(),
            method,
            params: canonical.finish(),
            format: output.format(),
            callback: output.callback().map(str::to_owned),
            version: 0,
        }
    }

    /// Ties the key to a version of the user's stored data, so the entry stops matching once
    /// the store bumps that version.
    #[must_use]
    pub fn at_version(mut self, version: u64) -> Self {
        self.version = version;
        self
    }

    /// The lowercased user the entry belongs to.
    #[must_use]
    pub fn user(&self) -> &str {
        &self.user
    }
}

/// A store of rendered response bodies, final bytes included: XML, JSON, or JSONP.
pub trait ResponseCache: Send + Sync {
    /// The body cached under this key.
    fn get(&self, key: &CacheKey) -> Option<Body>;
    /// Stores a body.
    fn insert(&self, key: CacheKey, body: Body);
    /// Drops every entry of a user. Data changes need no call: they bump the user's version,
    /// which is part of the key.
    fn invalidate_user(&self, user: &str);
}

/// An in-memory [`ResponseCache`] on `moka`, bounded by the bytes of the bodies and of the
/// compressed copies they are expected to grow.
#[derive(Debug)]
pub struct MemoryCache {
    entries: moka::sync::Cache<CacheKey, Body>,
}

impl MemoryCache {
    /// A cache holding at most `max_bytes`, each body for at most `ttl`. Zero bytes disables it.
    #[must_use]
    pub fn new(max_bytes: u64, ttl: Duration) -> Self {
        let entries = moka::sync::Cache::builder()
            .max_capacity(max_bytes)
            .weigher(|_, body: &Body| body.weight())
            .time_to_live(ttl)
            .support_invalidation_closures()
            .build();
        Self { entries }
    }
}

impl ResponseCache for MemoryCache {
    fn get(&self, key: &CacheKey) -> Option<Body> {
        self.entries.get(key)
    }

    fn insert(&self, key: CacheKey, body: Body) {
        self.entries.insert(key, body);
    }

    fn invalidate_user(&self, user: &str) {
        let user = user.to_lowercase();
        if let Err(error) = self
            .entries
            .invalidate_entries_if(move |key, _| key.user == user)
        {
            tracing::warn!(%error, "invalidating cached responses failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;

    fn key(user: &str, params: &Params) -> CacheKey {
        CacheKey::new(user, "user.getInfo", params, &Output::plain(Format::Json))
    }

    fn body(text: &'static str) -> Body {
        Body::hashed(Bytes::from_static(text.as_bytes()))
    }

    #[test]
    fn key_ignores_auth_params_and_user_case() {
        let a = Params::new().with("user", "x").with("api_key", "1");
        let b = Params::new().with("api_key", "2").with("user", "x");
        assert_eq!(key("Syntiiix", &a), key("syntiiix", &b));
        assert_ne!(
            key("syntiiix", &a),
            key("syntiiix", &a.clone().with("page", "2"))
        );
    }

    #[test]
    fn key_tells_jsonp_callbacks_apart() {
        let params = Params::new().with("format", "json");
        let jsonp = |callback: &str| {
            let output = Output::from_params(&params.clone().with("callback", callback)).unwrap();
            CacheKey::new("a", "user.getInfo", &params, &output)
        };
        assert_ne!(jsonp("cb"), key("a", &params));
        assert_ne!(jsonp("cb"), jsonp("other"));
    }

    #[test]
    fn invalidating_a_user_hides_only_their_entries() {
        let cache = MemoryCache::new(1 << 20, Duration::from_secs(60));
        let params = Params::new();
        cache.insert(key("a", &params), body("A"));
        cache.insert(key("b", &params), body("B"));
        cache.invalidate_user("A");
        assert_eq!(cache.get(&key("a", &params)), None);
        assert_eq!(cache.get(&key("b", &params)), Some(body("B")));
        cache.insert(key("a", &params), body("A2"));
        assert_eq!(cache.get(&key("a", &params)), Some(body("A2")));
    }

    #[test]
    fn a_new_version_misses() {
        let cache = MemoryCache::new(1 << 20, Duration::from_secs(60));
        let params = Params::new();
        cache.insert(key("a", &params).at_version(1), body("A"));
        assert_eq!(cache.get(&key("a", &params).at_version(2)), None);
    }
}
