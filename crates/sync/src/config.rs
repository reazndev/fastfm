//! Tunables of the sync jobs.

use std::time::Duration;

/// Settings for [`crate::SyncService`], built by chaining setters on [`SyncConfig::default`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncConfig {
    pub(crate) import_concurrency: usize,
    pub(crate) page_limit: u32,
    pub(crate) live_interval: Duration,
    pub(crate) idle_interval: Duration,
    pub(crate) periods_interval: Duration,
    pub(crate) reconcile_interval: Duration,
    pub(crate) enrich_interval: Duration,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            import_concurrency: 8,
            page_limit: 1000,
            live_interval: Duration::from_secs(10),
            idle_interval: Duration::from_secs(60),
            periods_interval: Duration::from_secs(300),
            reconcile_interval: Duration::from_secs(86_400),
            enrich_interval: Duration::from_secs(86_400),
        }
    }
}

impl SyncConfig {
    /// How many time ranges one import fetches in parallel. Defaults to 8.
    #[must_use]
    pub fn import_concurrency(self, import_concurrency: usize) -> Self {
        Self {
            import_concurrency: import_concurrency.max(1),
            ..self
        }
    }

    /// Scrobbles per `user.getRecentTracks` page, 1 to 1000. Defaults to Last.fm's real
    /// maximum of 1000; tests use small pages to cross many window boundaries.
    #[must_use]
    pub fn page_limit(self, page_limit: u32) -> Self {
        Self {
            page_limit: page_limit.clamp(1, 1000),
            ..self
        }
    }

    /// Poll interval while a track is playing (default 10 s) and the ceiling the interval
    /// backs off to while idle (default 60 s).
    #[must_use]
    pub fn live_intervals(self, playing: Duration, idle: Duration) -> Self {
        Self {
            live_interval: playing,
            idle_interval: idle.max(playing),
            ..self
        }
    }

    /// How often the store's period totals are moved forward. Defaults to 5 minutes.
    #[must_use]
    pub fn periods_interval(self, periods_interval: Duration) -> Self {
        Self {
            periods_interval,
            ..self
        }
    }

    /// How often each followed user's library is enriched again from their top lists.
    /// Defaults to one day.
    #[must_use]
    pub fn enrich_interval(self, enrich_interval: Duration) -> Self {
        Self {
            enrich_interval,
            ..self
        }
    }

    /// How often each followed user is reconciled. Defaults to one day.
    #[must_use]
    pub fn reconcile_interval(self, reconcile_interval: Duration) -> Self {
        Self {
            reconcile_interval,
            ..self
        }
    }
}
