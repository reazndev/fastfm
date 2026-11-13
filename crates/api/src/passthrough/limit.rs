//! A token bucket per API key for requests that must go to Last.fm: answers already cached cost
//! nothing, but each miss spends the instance's Last.fm quota, so one key cannot spend it all.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

/// Keys tracked at most; past this the buckets start over, so made-up keys cannot grow the
/// map. Only valid keys reach the limiter, so this is generous.
const MAX_KEYS: usize = 10_000;

/// Misses allowed per second and key, in a bucket that holds two seconds' worth.
#[derive(Debug)]
pub(crate) struct MissLimiter {
    per_second: f64,
    buckets: Mutex<HashMap<String, (f64, Instant)>>,
}

impl MissLimiter {
    /// A limiter allowing `per_second` misses per key; 0 or less allows none.
    pub(crate) fn new(per_second: f64) -> Self {
        Self {
            per_second: per_second.max(0.0),
            buckets: Mutex::default(),
        }
    }

    /// Takes a token for `key`, or says no when its bucket is empty.
    pub(crate) fn allow(&self, key: &str) -> bool {
        let capacity = self.per_second * 2.0;
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        if buckets.len() >= MAX_KEYS && !buckets.contains_key(key) {
            buckets.clear();
        }
        let (tokens, at) = buckets.entry(key.to_owned()).or_insert((capacity, now));
        *tokens = (*tokens + now.duration_since(*at).as_secs_f64() * self.per_second).min(capacity);
        *at = now;
        if *tokens < 1.0 {
            return false;
        }
        *tokens -= 1.0;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_key_has_its_own_burst() {
        let limiter = MissLimiter::new(5.0);
        let allowed = (0..20).filter(|_| limiter.allow("a")).count();
        assert_eq!(allowed, 10);
        assert!(limiter.allow("b"));
        assert!(!MissLimiter::new(0.0).allow("c"));
    }
}
