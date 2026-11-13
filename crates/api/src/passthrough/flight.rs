//! Single flight: concurrent fetches of the same entry share one upstream call.

use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::OnceCell;

/// Calls in flight, by key. The first caller of a key runs the call; callers that arrive while
/// it runs wait for its result. If the first caller is cancelled, a waiter takes over.
#[derive(Debug)]
pub(crate) struct Flights<K, V> {
    calls: Mutex<HashMap<K, Arc<OnceCell<V>>>>,
}

impl<K, V> Default for Flights<K, V> {
    fn default() -> Self {
        Self {
            calls: Mutex::default(),
        }
    }
}

impl<K: Hash + Eq + Clone, V: Clone> Flights<K, V> {
    /// Runs `fetch` for `key`, or waits for the run already in flight.
    pub(crate) async fn run<F, Fut>(&self, key: &K, fetch: F) -> V
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = V>,
    {
        let slot = Arc::clone(self.lock().entry(key.clone()).or_default());
        let value = slot.get_or_init(fetch).await.clone();
        let mut calls = self.lock();
        if calls.get(key).is_some_and(|c| Arc::ptr_eq(c, &slot)) {
            calls.remove(key);
        }
        value
    }

    /// Whether a call for `key` is running.
    pub(crate) fn is_running(&self, key: &K) -> bool {
        self.lock().contains_key(key)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<K, Arc<OnceCell<V>>>> {
        self.calls.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    use super::*;

    async fn slow_count(calls: Arc<AtomicU32>) -> u32 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        calls.fetch_add(1, Ordering::SeqCst) + 1
    }

    #[tokio::test]
    async fn concurrent_callers_share_one_call() {
        let flights: Arc<Flights<&str, u32>> = Arc::default();
        let calls = Arc::new(AtomicU32::new(0));
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let (flights, calls) = (Arc::clone(&flights), Arc::clone(&calls));
                tokio::spawn(async move { flights.run(&"k", || slow_count(calls)).await })
            })
            .collect();
        for task in tasks {
            assert_eq!(task.await.unwrap(), 1);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!flights.is_running(&"k"));
    }
}
