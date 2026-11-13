//! The loops the service runs in the background.

use std::time::Duration;

use lfm_store::UserId;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::clock;
use crate::context::Context;
use crate::error::{Result, SyncError};
use crate::{enrich, import, live, reconcile};

/// Follows one user until `stop` fires: import first, then live polling with a reconcile pass
/// every `reconcile_interval` and an enrichment pass every `enrich_interval`.
pub(crate) async fn follow(ctx: &Context, user: UserId, stop: &CancellationToken) {
    let Ok(name) = ctx.user_name(user).await else {
        warn!(user = user.0, "cannot follow a user missing from the store");
        return;
    };
    loop {
        match import::ensure(ctx, &name, stop).await {
            Ok(()) => break,
            Err(SyncError::Stopped) => return,
            Err(err) => warn!(user = %name, error = %err, "import failed; retrying later"),
        }
        if !pause(ctx.config.idle_interval, stop).await {
            return;
        }
    }
    let mut interval = ctx.config.live_interval;
    let mut next_reconcile = Instant::now() + ctx.config.reconcile_interval;
    let mut next_enrich = Instant::now() + ctx.config.enrich_interval;
    loop {
        interval = match live::poll(ctx, user, &name).await {
            Ok(true) => ctx.config.live_interval,
            Ok(false) => (interval * 2).min(ctx.config.idle_interval),
            Err(err) => {
                warn!(user = %name, error = %err, "live poll failed");
                ctx.config.idle_interval
            }
        };
        if Instant::now() >= next_reconcile {
            next_reconcile = Instant::now() + ctx.config.reconcile_interval;
            run_reconcile(ctx, user, &name).await;
        }
        if Instant::now() >= next_enrich {
            next_enrich = Instant::now() + ctx.config.enrich_interval;
            if let Err(err) = enrich::enrich(ctx, user).await {
                warn!(user = %name, error = %err, "enrichment failed");
            }
        }
        if !pause(interval, stop).await {
            ctx.now_playing.set(user, None, 0);
            return;
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
        if !pause(ctx.config.periods_interval, stop).await {
            return;
        }
    }
}

/// The user's sync cursor: everything before it is stored.
pub(crate) async fn sync_cursor(ctx: &Context, user: UserId) -> Result<i64> {
    ctx.store
        .user(user)
        .await?
        .map(|u| u.sync_cursor)
        .ok_or_else(|| SyncError::UnknownUser(format!("id {}", user.0)))
}

/// Sleeps for `duration`. Returns `false` if `stop` fired first.
async fn pause(duration: Duration, stop: &CancellationToken) -> bool {
    tokio::select! {
        () = stop.cancelled() => false,
        () = tokio::time::sleep(duration) => true,
    }
}
