//! One cached Last.fm answer, as kept in memory, in the store and on the wire.

use std::time::Duration;

use bytes::Bytes;
use lfm_store::CachedResponse;
use lfm_upstream::RawResponse;

/// Last.fm's answer to one request: everything needed to replay it byte for byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The HTTP status.
    pub status: u16,
    /// The `Content-Type` header.
    pub content_type: String,
    /// Last.fm's `access-control-*` headers, in the order received.
    pub headers: Vec<(String, String)>,
    /// The body, without a JSONP wrapper.
    pub body: Bytes,
    /// When the answer was fetched, in Unix seconds.
    pub fetched_at: i64,
    /// When it goes stale, in Unix seconds.
    pub expires_at: i64,
}

impl Entry {
    /// Keeps `response`, fresh for `ttl` from `now`.
    pub(crate) fn from_response(response: RawResponse, now: i64, ttl: Duration) -> Self {
        let headers = response
            .headers
            .iter()
            .filter(|(name, _)| name.as_str().starts_with("access-control-"))
            .filter_map(|(name, value)| Some((name.as_str().to_owned(), value.to_str().ok()?)))
            .map(|(name, value)| (name, value.to_owned()))
            .collect();
        let ttl = i64::try_from(ttl.as_secs()).unwrap_or(i64::MAX);
        Self {
            status: response.status,
            content_type: response.content_type.unwrap_or_default(),
            headers,
            body: response.body,
            fetched_at: now,
            expires_at: now.saturating_add(ttl),
        }
    }

    /// Whether the entry is still fresh at `now`.
    #[must_use]
    pub fn is_fresh(&self, now: i64) -> bool {
        now < self.expires_at
    }

    /// The bytes the entry takes in memory, for the cache's size bound.
    pub(crate) fn weight(&self) -> u32 {
        u32::try_from(self.body.len().saturating_add(256)).unwrap_or(u32::MAX)
    }
}

impl From<CachedResponse> for Entry {
    fn from(cached: CachedResponse) -> Self {
        let headers = cached
            .headers
            .lines()
            .filter_map(|line| line.split_once(": "))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        Self {
            status: cached.status,
            content_type: cached.content_type,
            headers,
            body: Bytes::from(cached.body),
            fetched_at: cached.fetched_at,
            expires_at: cached.expires_at,
        }
    }
}

impl From<&Entry> for CachedResponse {
    fn from(entry: &Entry) -> Self {
        let headers: Vec<String> = entry
            .headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect();
        Self {
            body: entry.body.to_vec(),
            status: entry.status,
            content_type: entry.content_type.clone(),
            headers: headers.join("\n"),
            fetched_at: entry.fetched_at,
            expires_at: entry.expires_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn survives_the_store_round_trip() {
        let entry = Entry {
            status: 200,
            content_type: "application/json; charset=UTF-8".into(),
            headers: vec![
                ("access-control-allow-origin".into(), "*".into()),
                (
                    "access-control-allow-methods".into(),
                    "POST, GET, OPTIONS".into(),
                ),
            ],
            body: Bytes::from_static(b"{}"),
            fetched_at: 1,
            expires_at: 2,
        };
        assert_eq!(Entry::from(CachedResponse::from(&entry)), entry);
    }
}
