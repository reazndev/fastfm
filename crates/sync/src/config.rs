//! Tunables of the sync jobs.

use std::num::NonZeroU32;
use std::time::Duration;

/// Settings for [`crate::SyncService`], built by chaining setters on [`SyncConfig::default`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncConfig {
    pub(crate) import_concurrency: usize,
    pub(crate) page_limit: u32,
    pub(crate) tiers: PollTiers,
    pub(crate) nudge_after: Duration,
    pub(crate) periods_interval: Duration,
    pub(crate) reconcile_interval: Duration,
    pub(crate) enrich_interval: Duration,
    pub(crate) private_interval: Duration,
    pub(crate) import_retry: Duration,
    pub(crate) pool_budget: NonZeroU32,
    pub(crate) key_budget: NonZeroU32,
}

/// How often live sync polls a user, by how recently they scrobbled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PollTiers {
    /// While a track is playing, and right after a new scrobble.
    pub(crate) playing: Duration,
    /// Last scrobble within `recent_window`.
    pub(crate) recent: Duration,
    /// Last scrobble within `day_window`.
    pub(crate) day: Duration,
    /// Last scrobble longer ago, or none at all.
    pub(crate) idle: Duration,
    pub(crate) recent_window: Duration,
    pub(crate) day_window: Duration,
    /// Each pause is moved by up to this percentage either way, so users drift apart.
    pub(crate) jitter_percent: u32,
}

/// Background requests per second each key may take: Last.fm allows about 5 per key, the
/// upstream client's buckets default to 4, and the rest stays free for pass-through reads.
const DEFAULT_BUDGET: NonZeroU32 = NonZeroU32::MIN.saturating_add(2);

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            import_concurrency: 8,
            page_limit: 1000,
            tiers: PollTiers {
                playing: Duration::from_secs(10),
                recent: Duration::from_secs(60),
                day: Duration::from_secs(300),
                idle: Duration::from_secs(900),
                recent_window: Duration::from_secs(1800),
                day_window: Duration::from_secs(86_400),
                jitter_percent: 10,
            },
            nudge_after: Duration::from_secs(60),
            periods_interval: Duration::from_secs(300),
            reconcile_interval: Duration::from_secs(86_400),
            enrich_interval: Duration::from_secs(86_400),
            // A private profile is retried daily, in case its owner makes it public.
            private_interval: Duration::from_secs(86_400),
            import_retry: Duration::from_secs(60),
            pool_budget: DEFAULT_BUDGET,
            key_budget: DEFAULT_BUDGET,
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

    /// Live poll intervals: while a track plays or right after a new scrobble (default 10 s),
    /// after a scrobble in the recent window (60 s), in the day window (5 minutes), and
    /// otherwise (15 minutes). Each is at least the one before it.
    #[must_use]
    pub fn poll_intervals(self, playing: Duration, recent: Duration, day: Duration) -> Self {
        let recent = recent.max(playing);
        let day = day.max(recent);
        let tiers = PollTiers {
            playing,
            recent,
            day,
            idle: self.tiers.idle.max(day),
            ..self.tiers
        };
        Self { tiers, ..self }
    }

    /// The poll interval of users whose last scrobble is older than the day window. Defaults
    /// to 15 minutes.
    #[must_use]
    pub fn idle_interval(self, idle: Duration) -> Self {
        let tiers = PollTiers {
            idle: idle.max(self.tiers.day),
            ..self.tiers
        };
        Self { tiers, ..self }
    }

    /// How recent a last scrobble keeps a user in the recent tier (default 30 minutes) and in
    /// the day tier (default 24 hours).
    #[must_use]
    pub fn poll_windows(self, recent: Duration, day: Duration) -> Self {
        let tiers = PollTiers {
            recent_window: recent,
            day_window: day.max(recent),
            ..self.tiers
        };
        Self { tiers, ..self }
    }

    /// How far each poll pause may move either way, in percent, so users do not poll in
    /// lockstep. Defaults to 10; capped at 50.
    #[must_use]
    pub fn poll_jitter(self, percent: u32) -> Self {
        let tiers = PollTiers {
            jitter_percent: percent.min(50),
            ..self.tiers
        };
        Self { tiers, ..self }
    }

    /// How old a user's last poll must be before [`crate::SyncService::nudge`] polls again.
    /// Defaults to 60 seconds.
    #[must_use]
    pub fn nudge_after(self, nudge_after: Duration) -> Self {
        Self {
            nudge_after,
            ..self
        }
    }

    /// How long a failed import waits before the next attempt. Defaults to 60 seconds; a
    /// private profile waits `private_interval` instead.
    #[must_use]
    pub fn import_retry(self, import_retry: Duration) -> Self {
        Self {
            import_retry,
            ..self
        }
    }

    /// How long a private profile waits before the next automatic attempt. Defaults to one
    /// day.
    #[must_use]
    pub fn private_interval(self, private_interval: Duration) -> Self {
        Self {
            private_interval,
            ..self
        }
    }

    /// Background requests per second for all users that share the instance key pool
    /// (default 3), and for all users of one account's own key (default 3). Live polls go
    /// first, then imports, then enrichment and reconciliation.
    #[must_use]
    pub fn request_budgets(self, pool: NonZeroU32, own_key: NonZeroU32) -> Self {
        Self {
            pool_budget: pool,
            key_budget: own_key,
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
