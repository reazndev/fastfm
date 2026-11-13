//! A request budget per Last.fm key for background work.
//!
//! The upstream client already spaces requests per key and orders waiting requests by lane, but
//! its buckets are shared with pass-through reads and let background work take every token.
//! This budget caps what sync takes from each key below that, so reads keep headroom however
//! many users share the key. Within the budget, live polls go before imports, and imports
//! before enrichment and reconciliation.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use lfm_upstream::{ApiKey, Priority};
use tokio::time::Instant;

/// Ranks of the budget's lanes, highest first.
const LANES: usize = 3;

/// One budget for the instance pool and one per account key.
#[derive(Debug)]
pub(crate) struct Budgets {
    pool: Arc<Budget>,
    own: Mutex<HashMap<ApiKey, Arc<Budget>>>,
    own_rate: NonZeroU32,
}

impl Budgets {
    pub(crate) fn new(pool: NonZeroU32, own: NonZeroU32) -> Self {
        Self {
            pool: Arc::new(Budget::new(pool)),
            own: Mutex::default(),
            own_rate: own,
        }
    }

    /// The budget of `key`, or of the pool for `None`.
    pub(crate) fn of(&self, key: Option<&ApiKey>) -> Arc<Budget> {
        let Some(key) = key else {
            return Arc::clone(&self.pool);
        };
        let mut own = self.own.lock().unwrap_or_else(PoisonError::into_inner);
        let rate = self.own_rate;
        Arc::clone(
            own.entry(key.clone())
                .or_insert_with(|| Arc::new(Budget::new(rate))),
        )
    }
}

/// Evenly spaced requests at a fixed rate, granted by lane.
#[derive(Debug)]
pub(crate) struct Budget {
    spacing: Duration,
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    /// When the next request may leave.
    next: Instant,
    /// Requests waiting in each lane.
    waiting: [usize; LANES],
}

impl Budget {
    fn new(rate: NonZeroU32) -> Self {
        Self {
            spacing: Duration::from_secs(1) / rate.get(),
            state: Mutex::new(State {
                next: Instant::now(),
                waiting: [0; LANES],
            }),
        }
    }

    /// Waits until a request in `priority`'s lane may leave. A request waits while one in a
    /// higher lane does.
    pub(crate) async fn take(&self, priority: Priority) {
        let lane = lane(priority);
        let _waiting = Waiting::enter(self, lane);
        loop {
            let pause = {
                let mut state = self.lock();
                let now = Instant::now();
                let ahead = state.waiting.iter().take(lane).any(|n| *n > 0);
                if !ahead && now >= state.next {
                    state.next = now + self.spacing;
                    return;
                }
                if ahead {
                    self.spacing
                } else {
                    state.next - now
                }
            };
            tokio::time::sleep(pause).await;
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Counts a request as waiting in its lane until it is dropped, also when its future is.
struct Waiting<'a> {
    budget: &'a Budget,
    lane: usize,
}

impl<'a> Waiting<'a> {
    fn enter(budget: &'a Budget, lane: usize) -> Self {
        if let Some(n) = budget.lock().waiting.get_mut(lane) {
            *n += 1;
        }
        Self { budget, lane }
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        if let Some(n) = self.budget.lock().waiting.get_mut(self.lane) {
            *n = n.saturating_sub(1);
        }
    }
}

fn lane(priority: Priority) -> usize {
    match priority {
        Priority::Interactive | Priority::Live => 0,
        Priority::Import => 1,
        Priority::Prewarm => 2,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn rate(n: u32) -> NonZeroU32 {
        NonZeroU32::new(n).unwrap()
    }

    #[tokio::test]
    async fn requests_leave_no_faster_than_the_rate() {
        let budget = Budget::new(rate(50));
        let start = Instant::now();
        for _ in 0..6 {
            budget.take(Priority::Import).await;
        }
        assert!(
            start.elapsed() >= Duration::from_millis(100),
            "5 gaps of 20 ms"
        );
    }

    #[tokio::test]
    async fn live_requests_go_before_waiting_imports_and_imports_before_enrichment() {
        let budget = Arc::new(Budget::new(rate(20)));
        let order = Arc::new(Mutex::new(Vec::new()));
        budget.take(Priority::Import).await;
        let spawn = |priority: Priority, tag: &'static str| {
            let (budget, order) = (Arc::clone(&budget), Arc::clone(&order));
            tokio::spawn(async move {
                budget.take(priority).await;
                order.lock().unwrap().push(tag);
            })
        };
        let mut tasks = vec![
            spawn(Priority::Prewarm, "enrich"),
            spawn(Priority::Import, "import"),
        ];
        tokio::time::sleep(Duration::from_millis(5)).await;
        tasks.push(spawn(Priority::Live, "live"));
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(*order.lock().unwrap(), ["live", "import", "enrich"]);
    }

    #[test]
    fn the_pool_and_each_own_key_have_their_own_budget() {
        let budgets = Budgets::new(rate(1), rate(2));
        let key = ApiKey::new("k");
        assert!(Arc::ptr_eq(&budgets.of(None), &budgets.of(None)));
        assert!(Arc::ptr_eq(
            &budgets.of(Some(&key)),
            &budgets.of(Some(&key))
        ));
        assert!(!Arc::ptr_eq(&budgets.of(None), &budgets.of(Some(&key))));
        assert_eq!(budgets.of(Some(&key)).spacing, Duration::from_millis(500));
    }
}
