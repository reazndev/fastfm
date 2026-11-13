//! Counting failed attempts per key (an IP or a username) in a fixed window.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

/// Keys tracked before expired ones are swept, to bound memory under a flood of names.
const SWEEP_AT: usize = 10_000;

/// Allows `limit` failures per key within `window` seconds.
#[derive(Debug)]
pub(crate) struct Throttle {
    limit: u32,
    window: i64,
    failures: Mutex<HashMap<String, (i64, u32)>>,
}

impl Throttle {
    pub(crate) fn new(limit: u32, window: i64) -> Self {
        Self {
            limit,
            window,
            failures: Mutex::default(),
        }
    }

    /// Whether `key` may try again at `now`.
    pub(crate) fn allows(&self, key: &str, now: i64) -> bool {
        let failures = self.failures.lock().unwrap_or_else(PoisonError::into_inner);
        failures
            .get(key)
            .is_none_or(|&(start, count)| now - start >= self.window || count < self.limit)
    }

    /// Counts one failure of `key` at `now`.
    pub(crate) fn fail(&self, key: &str, now: i64) {
        let mut failures = self.failures.lock().unwrap_or_else(PoisonError::into_inner);
        if failures.len() >= SWEEP_AT {
            failures.retain(|_, &mut (start, _)| now - start < self.window);
        }
        let entry = failures.entry(key.to_owned()).or_insert((now, 0));
        if now - entry.0 >= self.window {
            *entry = (now, 0);
        }
        entry.1 = entry.1.saturating_add(1);
    }

    /// Forgets `key`'s failures, after a success.
    pub(crate) fn clear(&self, key: &str) {
        self.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_the_limit_until_the_window_ends() {
        let throttle = Throttle::new(2, 60);
        assert!(throttle.allows("a", 0));
        throttle.fail("a", 0);
        throttle.fail("a", 1);
        assert!(!throttle.allows("a", 2));
        assert!(throttle.allows("b", 2));
        assert!(throttle.allows("a", 60));
        throttle.fail("a", 61);
        assert!(throttle.allows("a", 61));
        throttle.clear("a");
        assert!(throttle.allows("a", 62));
    }
}
