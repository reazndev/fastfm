//! Cache of rendered responses, invalidated per user by a version number.

use std::collections::HashMap;
use std::sync::{PoisonError, RwLock};
use std::time::Duration;

use bytes::Bytes;
use lfm_core::{Format, Params};

/// Parameters that do not change a response's content.
const IGNORED: [&str; 6] = ["api_key", "api_sig", "sk", "format", "callback", "method"];

/// Identifies one rendered response: user, method, canonical parameters and format.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    user: String,
    method: &'static str,
    params: String,
    format: Format,
    version: u64,
}

impl CacheKey {
    /// Builds a key. The user is lowercased, because Last.fm user names ignore case, and the
    /// parameters are sorted, without the ones that do not change the content.
    #[must_use]
    pub fn new(user: &str, method: &'static str, params: &Params, format: Format) -> Self {
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
            format,
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

/// A store of rendered response bodies. Bodies are XML or plain JSON; JSONP wrapping happens
/// when serving.
pub trait ResponseCache: Send + Sync {
    /// The body cached under this key at the user's current version.
    fn get(&self, key: &CacheKey) -> Option<Bytes>;
    /// Stores a body at the user's current version.
    fn insert(&self, key: CacheKey, body: Bytes);
    /// Drops every entry of a user by moving them to a new version. Call it after each ingest.
    fn invalidate_user(&self, user: &str);
}

/// An in-memory [`ResponseCache`] on `moka`, bounded by total body size.
#[derive(Debug)]
pub struct MemoryCache {
    entries: moka::sync::Cache<(CacheKey, u64), Bytes>,
    versions: RwLock<HashMap<String, u64>>,
}

impl MemoryCache {
    /// A cache holding at most `max_bytes` of bodies, each for at most `ttl`.
    #[must_use]
    pub fn new(max_bytes: u64, ttl: Duration) -> Self {
        let entries = moka::sync::Cache::builder()
            .max_capacity(max_bytes)
            .weigher(|_, body: &Bytes| u32::try_from(body.len()).unwrap_or(u32::MAX))
            .time_to_live(ttl)
            .build();
        Self {
            entries,
            versions: RwLock::new(HashMap::new()),
        }
    }

    fn version(&self, user: &str) -> u64 {
        self.versions
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(user)
            .copied()
            .unwrap_or(0)
    }
}

impl ResponseCache for MemoryCache {
    fn get(&self, key: &CacheKey) -> Option<Bytes> {
        let version = self.version(key.user());
        self.entries.get(&(key.clone(), version))
    }

    fn insert(&self, key: CacheKey, body: Bytes) {
        let version = self.version(key.user());
        self.entries.insert((key, version), body);
    }

    fn invalidate_user(&self, user: &str) {
        *self
            .versions
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(user.to_lowercase())
            .or_insert(0) += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(user: &str, params: &Params) -> CacheKey {
        CacheKey::new(user, "user.getInfo", params, Format::Json)
    }

    #[test]
    fn key_ignores_auth_and_output_params_and_user_case() {
        let a = Params::new()
            .with("user", "x")
            .with("api_key", "1")
            .with("callback", "cb");
        let b = Params::new().with("api_key", "2").with("user", "x");
        assert_eq!(key("Syntiiix", &a), key("syntiiix", &b));
        assert_ne!(
            key("syntiiix", &a),
            key("syntiiix", &a.clone().with("page", "2"))
        );
    }

    #[test]
    fn invalidating_a_user_hides_only_their_entries() {
        let cache = MemoryCache::new(1 << 20, Duration::from_secs(60));
        let params = Params::new();
        cache.insert(key("a", &params), Bytes::from_static(b"A"));
        cache.insert(key("b", &params), Bytes::from_static(b"B"));
        cache.invalidate_user("A");
        assert_eq!(cache.get(&key("a", &params)), None);
        assert_eq!(
            cache.get(&key("b", &params)),
            Some(Bytes::from_static(b"B"))
        );
        cache.insert(key("a", &params), Bytes::from_static(b"A2"));
        assert_eq!(
            cache.get(&key("a", &params)),
            Some(Bytes::from_static(b"A2"))
        );
    }
}
