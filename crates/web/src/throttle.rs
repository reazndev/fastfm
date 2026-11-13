//! Counting failed attempts per key (an IP or a username) in a fixed window.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

/// Keys tracked before expired ones are swept, and the most kept after a sweep, to bound
/// memory under a flood of names or addresses.
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
        if failures.len() >= SWEEP_AT {
            // Still full within one window: forget the older half rather than grow.
            let mut starts: Vec<i64> = failures.values().map(|(start, _)| *start).collect();
            starts.sort_unstable();
            let cutoff = starts.get(starts.len() / 2).copied().unwrap_or(now);
            failures.retain(|_, &mut (start, _)| start > cutoff);
        }
        let entry = failures.entry(key.to_owned()).or_insert((now, 0));
        if now - entry.0 >= self.window {
            *entry = (now, 0);
        }
        entry.1 = entry.1.saturating_add(1);
    }

    /// Failures of `key` in its current window.
    pub(crate) fn count(&self, key: &str, now: i64) -> u32 {
        let failures = self.failures.lock().unwrap_or_else(PoisonError::into_inner);
        failures
            .get(key)
            .filter(|(start, _)| now - start < self.window)
            .map_or(0, |(_, count)| *count)
    }

    /// Counts one use of `key` at `now` if the limit allows it, for a plain request limit.
    /// Returns whether it did.
    pub(crate) fn take(&self, key: &str, now: i64) -> bool {
        let allowed = self.allows(key, now);
        if allowed {
            self.fail(key, now);
        }
        allowed
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
        assert!(throttle.take("c", 0) && throttle.take("c", 1));
        assert!(!throttle.take("c", 2));
        assert_eq!(throttle.count("c", 2), 2);
        assert_eq!(throttle.count("c", 60), 0);
    }

    #[test]
    fn a_flood_of_keys_stays_bounded() {
        let throttle = Throttle::new(2, 1_000_000);
        for n in 0..SWEEP_AT * 3 {
            throttle.fail(&n.to_string(), i64::try_from(n).unwrap());
        }
        let kept = throttle.failures.lock().unwrap().len();
        assert!(kept <= SWEEP_AT, "{kept}");
        assert_eq!(throttle.count(&(SWEEP_AT * 3 - 1).to_string(), 30_000), 1);
    }
}
