//! The loops the service runs in the background.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use lfm_store::{Page, TimeRange, UserId};
use tokio::sync::Notify;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::context::Context;
use crate::error::{Result, SyncError};
use crate::{clock, enrich, import, live, reconcile, tiers};

/// How the service reaches a follow loop: a wake-up for an early poll, and when it last polled.
#[derive(Debug, Default)]
pub(crate) struct Signal {
    wake: Notify,
    last_poll: Mutex<Option<Instant>>,
}

impl Signal {
    /// Ends the loop's current pause, or the next one if it is busy.
    pub(crate) fn wake(&self) {
        self.wake.notify_one();
    }

    /// Whether the loop has not polled within `age`.
    pub(crate) fn stale(&self, age: Duration) -> bool {
        self.last_poll
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none_or(|at| at.elapsed() >= age)
    }

    fn polled(&self) {
        *self
            .last_poll
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
    }

    /// Sleeps for `duration` or until woken. Returns `false` if `stop` fired first.
    async fn pause(&self, duration: Duration, stop: &CancellationToken) -> bool {
        tokio::select! {
            () = stop.cancelled() => false,
            () = self.wake.notified() => true,
            () = tokio::time::sleep(duration) => true,
        }
    }
}

/// Follows one user until `stop` fires: import first, then live polling with a reconcile pass
/// every `reconcile_interval` and an enrichment pass every `enrich_interval`.
pub(crate) async fn follow(ctx: &Context, user: UserId, signal: &Signal, stop: &CancellationToken) {
    let Ok(name) = ctx.user_name(user).await else {
        warn!(user = user.0, "cannot follow a user missing from the store");
        return;
    };
    if import_first(ctx, (user, &name), signal, stop).await {
        poll_live(ctx, (user, &name), signal, stop).await;
    }
    ctx.now_playing.set(user, None, 0);
}

/// Runs the import until it finishes. A private profile is tried again once per
/// `private_interval`, counted from the last attempt even across restarts, or when woken.
/// Returns `false` if `stop` fired.
async fn import_first(
    ctx: &Context,
    (user, name): (UserId, &str),
    signal: &Signal,
    stop: &CancellationToken,
) -> bool {
    match import::private_wait(ctx, user).await {
        Ok(Some(wait)) => {
            ctx.set_private(user, true);
            if !signal.pause(wait, stop).await {
                return false;
            }
        }
        Ok(None) => {}
        Err(err) => warn!(user = %name, error = %err, "reading the last import failed"),
    }
    loop {
        let retry = match import::ensure(ctx, name, stop).await {
            Ok(()) => {
                ctx.set_private(user, false);
                return true;
            }
            Err(SyncError::Stopped) => return false,
            // The job shows why; asking again soon would only repeat the refusal.
            Err(err) if err.is_private_profile() => {
                ctx.set_private(user, true);
                ctx.config.private_interval
            }
            Err(err) => {
                warn!(user = %name, error = %err, "import failed; retrying later");
                ctx.config.import_retry
            }
        };
        if !signal.pause(retry, stop).await {
            return false;
        }
    }
}

/// Polls at the user's tier interval with jitter, until `stop` fires.
async fn poll_live(
    ctx: &Context,
    (user, name): (UserId, &str),
    signal: &Signal,
    stop: &CancellationToken,
) {
    let tiers = ctx.config.tiers;
    let mut last = newest_scrobble(ctx, user).await;
    let mut maintenance = Maintenance::new(ctx, user).await;
    // Users followed together at start-up begin at different times.
    if !signal
        .pause(tiers::jitter(tiers.playing / 2, 100), stop)
        .await
    {
        return;
    }
    loop {
        signal.polled();
        let wait = match live::poll(ctx, user, name).await {
            Ok(poll) => {
                ctx.set_private(user, false);
                let fresh = poll.newest.is_some_and(|n| last.is_none_or(|l| n > l));
                last = last.max(poll.newest);
                tiers::interval(&tiers, poll.playing || fresh, last, clock::now())
            }
            Err(err) if err.is_private_profile() => {
                if !ctx.is_private(user) {
                    info!(user = %name, "the Last.fm profile turned private; polling daily");
                }
                ctx.set_private(user, true);
                ctx.config.private_interval
            }
            Err(err) => {
                warn!(user = %name, error = %err, "live poll failed");
                tiers.recent
            }
        };
        if !ctx.is_private(user) {
            let active = last.is_some_and(|l| active_since(&tiers, l, clock::now()));
            maintenance.run_due(ctx, (user, name), active).await;
        }
        if !signal
            .pause(tiers::jitter(wait, tiers.jitter_percent), stop)
            .await
        {
            return;
        }
    }
}

/// The newest stored scrobble of `user`, in Unix seconds.
async fn newest_scrobble(ctx: &Context, user: UserId) -> Option<i64> {
    let all = TimeRange::new(0, i64::MAX);
    let listing = ctx.store.recent_tracks(user, all, Page::new(1, 1)).await;
    listing.ok()?.items.first().map(|t| t.uts)
}

/// Whether a user whose newest scrobble is at `last` counts as active for the trailing check.
fn active_since(tiers: &crate::config::PollTiers, last: i64, now: i64) -> bool {
    let age = u64::try_from(now.saturating_sub(last)).unwrap_or(0);
    Duration::from_secs(age) <= tiers.day_window
}

/// When the follow loop next reconciles, checks the trailing window, and enriches.
struct Maintenance {
    reconcile: Instant,
    trailing_check: Instant,
    enrichment: Instant,
}

impl Maintenance {
    /// Schedules the next reconcile pass from the last one stored, so restarts do not postpone
    /// it. A user never reconciled counts from now.
    async fn new(ctx: &Context, user: UserId) -> Self {
        let now = clock::now();
        let last = match ctx.store.reconciled_at(user).await {
            Ok(Some(at)) => at,
            Ok(None) => {
                if let Err(err) = ctx.store.set_reconciled_at(user, now).await {
                    warn!(user = user.0, error = %err, "storing the reconcile time failed");
                }
                now
            }
            Err(err) => {
                warn!(user = user.0, error = %err, "reading the reconcile time failed");
                now
            }
        };
        let since = Duration::from_secs(u64::try_from(now.saturating_sub(last)).unwrap_or(0));
        let start = Instant::now();
        Self {
            reconcile: start + ctx.config.reconcile_interval.saturating_sub(since),
            trailing_check: start + ctx.config.trailing.active,
            enrichment: start + ctx.config.enrich_interval,
        }
    }

    async fn run_due(&mut self, ctx: &Context, (user, name): (UserId, &str), active: bool) {
        let trailing = &ctx.config.trailing;
        if Instant::now() >= self.reconcile {
            self.reconcile = Instant::now() + ctx.config.reconcile_interval;
            // A full pass covers the trailing window too.
            self.trailing_check = Instant::now() + trailing.active;
            run_reconcile(ctx, user, name).await;
        }
        if Instant::now() >= self.trailing_check {
            let every = if active {
                trailing.active
            } else {
                trailing.idle
            };
            self.trailing_check = Instant::now() + every;
            let window = i64::try_from(trailing.window.as_secs()).unwrap_or(i64::MAX);
            match reconcile::trailing(ctx, (user, name), window).await {
                Ok(report) if !report.repaired.is_empty() => {
                    info!(user = %name, repaired = report.repaired.len(), "late scrobbles");
                }
                Ok(_) => {}
                Err(err) => warn!(user = %name, error = %err, "trailing check failed"),
            }
        }
        if Instant::now() >= self.enrichment {
            self.enrichment = Instant::now() + ctx.config.enrich_interval;
            if let Err(err) = enrich::enrich(ctx, user).await {
                warn!(user = %name, error = %err, "enrichment failed");
            }
        }
    }
}

async fn run_reconcile(ctx: &Context, user: UserId, name: &str) {
    let outcome = match sync_cursor(ctx, user).await {
        Ok(until) => reconcile::reconcile(ctx, user, until).await,
        Err(err) => Err(err),
    };
    match outcome {
        Ok(report) => info!(
            user = %name,
            repaired = report.repaired.len(),
            inconsistent = report.inconsistent.len(),
            requests = report.requests,
            "reconciled"
        ),
        Err(err) => warn!(user = %name, error = %err, "reconcile failed"),
    }
}

/// Moves the store's period totals forward every `periods_interval` until `stop` fires.
pub(crate) async fn advance_periods(ctx: &Context, stop: &CancellationToken) {
    loop {
        if let Err(err) = ctx.store.advance_periods(clock::now()).await {
            warn!(error = %err, "advancing period totals failed");
        }
        tokio::select! {
            () = stop.cancelled() => return,
            () = tokio::time::sleep(ctx.config.periods_interval) => {}
        }
    }
}

/// The user's sync cursor: everything before it is stored.
pub(crate) async fn sync_cursor(ctx: &Context, user: UserId) -> Result<i64> {
    ctx.store
        .user(user)
        .await?
        .ok_or_else(|| SyncError::UnknownUser(format!("id {}", user.0)))
        .map(|u| u.sync_cursor)
}
