use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use governor::clock::{Clock, DefaultClock};
use governor::{DefaultDirectRateLimiter, Quota, RateLimiter};

use crate::config::{ApiKey, BucketMode};
use crate::error::{Error, Result};
use crate::gate::Gate;
use crate::request::{KeyChoice, Priority};

/// A set of keys behind one priority gate.
#[derive(Debug)]
struct Lane {
    gate: Arc<Gate>,
    slots: Vec<Slot>,
    cursor: AtomicUsize,
}

#[derive(Debug)]
struct Slot {
    key: ApiKey,
    limiter: Arc<DefaultDirectRateLimiter>,
}

/// The instance key pool plus one lane per user-supplied key.
#[derive(Debug)]
pub(crate) struct KeyPool {
    shared: Option<Arc<Lane>>,
    own: Mutex<HashMap<ApiKey, Arc<Lane>>>,
    quota: Quota,
}

impl KeyPool {
    pub(crate) fn new(keys: Vec<ApiKey>, rps: NonZeroU32, mode: BucketMode) -> Self {
        // Burst of one: requests leave evenly spaced rather than in clumps.
        let quota = Quota::per_second(rps).allow_burst(NonZeroU32::MIN);
        Self {
            shared: Lane::new(keys, quota, mode).map(Arc::new),
            own: Mutex::new(HashMap::new()),
            quota,
        }
    }

    /// Waits for this request's turn and a token, and returns the key to send, if any.
    pub(crate) async fn acquire(
        &self,
        choice: &KeyChoice,
        priority: Priority,
    ) -> Result<Option<ApiKey>> {
        let lane = match choice {
            KeyChoice::Pool | KeyChoice::Omit => self.shared.clone().ok_or(Error::NoApiKey)?,
            KeyChoice::Own(key) => self.own_lane(key),
        };
        let key = lane.acquire(priority).await;
        Ok((*choice != KeyChoice::Omit).then_some(key))
    }

    fn own_lane(&self, key: &ApiKey) -> Arc<Lane> {
        let mut own = self.own.lock().unwrap_or_else(PoisonError::into_inner);
        let quota = self.quota;
        Arc::clone(
            own.entry(key.clone())
                .or_insert_with(|| Arc::new(Lane::from_slots(vec![Slot::new(key.clone(), quota)]))),
        )
    }
}

impl Lane {
    fn new(keys: Vec<ApiKey>, quota: Quota, mode: BucketMode) -> Option<Self> {
        if keys.is_empty() {
            return None;
        }
        let shared = Arc::new(RateLimiter::direct(quota));
        let slots = keys
            .into_iter()
            .map(|key| match mode {
                BucketMode::PerKey => Slot::new(key, quota),
                BucketMode::Shared => Slot {
                    key,
                    limiter: Arc::clone(&shared),
                },
            })
            .collect();
        Some(Self::from_slots(slots))
    }

    fn from_slots(slots: Vec<Slot>) -> Self {
        Self {
            gate: Arc::default(),
            slots,
            cursor: AtomicUsize::new(0),
        }
    }

    async fn acquire(&self, priority: Priority) -> ApiKey {
        let _turn = self.gate.enter(priority).await;
        loop {
            match self.try_take() {
                Ok(key) => return key,
                Err(wait) => tokio::time::sleep(wait).await,
            }
        }
    }

    /// Takes a token from the first ready slot after the cursor, rotating through the keys.
    /// When none is ready, returns how long until the earliest one is.
    fn try_take(&self) -> Result<ApiKey, Duration> {
        let count = self.slots.len();
        let start = self.cursor.load(Ordering::Relaxed);
        let now = DefaultClock::default().now();
        let mut wait = Duration::from_secs(1);
        for offset in 0..count {
            let index = (start + offset) % count;
            let Some(slot) = self.slots.get(index) else {
                continue;
            };
            match slot.limiter.check() {
                Ok(()) => {
                    self.cursor.store(index + 1, Ordering::Relaxed);
                    return Ok(slot.key.clone());
                }
                Err(not_until) => wait = wait.min(not_until.wait_time_from(now)),
            }
        }
        Err(wait)
    }
}

impl Slot {
    fn new(key: ApiKey, quota: Quota) -> Self {
        Self {
            key,
            limiter: Arc::new(RateLimiter::direct(quota)),
        }
    }
}
