//! Reconciliation: finding and repairing ranges where the store and Last.fm disagree.
//!
//! Users delete and edit scrobbles on Last.fm, and none of that shows up in live sync. A pass
//! compares Last.fm's count (`@attr.total` of a one-item page) with the store's for a fixed set
//! of slices of the history. A slice that differs is cut in half until a half fits in one page,
//! which is then replaced. Each split costs one request: the right half's count is the slice's
//! count minus the left half's. Edits that keep the count the same are not detected.

use lfm_store::{TimeRange, UserId};
use lfm_upstream::Priority;
use tracing::info;

use crate::context::Context;
use crate::error::{Result, SyncError};

/// Slices of the history, after registration, compared on every pass.
const SLICES: i64 = 32;

/// What a reconcile pass found and did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileReport {
    /// The range that was checked.
    pub range: TimeRange,
    /// Ranges whose counts were compared.
    pub compared: u64,
    /// Ranges that were replaced with Last.fm's contents.
    pub repaired: Vec<TimeRange>,
    /// Scrobbles written into the repaired ranges.
    pub written: u64,
    /// Last.fm requests the pass made.
    pub requests: u64,
}

/// Checks `[0, until)` of `user` and repairs what differs.
pub(crate) async fn reconcile(ctx: &Context, user: UserId, until: i64) -> Result<ReconcileReport> {
    let stored = ctx
        .store
        .user(user)
        .await?
        .ok_or_else(|| SyncError::UnknownUser(format!("id {}", user.0)))?;
    let name = stored.name;
    let first_request = ctx.upstream.requests();
    let mut report = ReconcileReport {
        range: TimeRange::new(0, until),
        compared: 0,
        repaired: Vec::new(),
        written: 0,
        requests: 0,
    };
    let mut pending: Vec<(TimeRange, Option<u64>)> = slices(stored.profile.registered, until)
        .into_iter()
        .map(|range| (range, None))
        .collect();
    while let Some((range, remote)) = pending.pop() {
        let remote = match remote {
            Some(remote) => remote,
            None => ctx.upstream.total(&name, range, Priority::Import).await?,
        };
        let local = ctx.store.count_range(user, range).await?;
        report.compared += 1;
        if remote == local {
            continue;
        }
        if remote <= u64::from(ctx.config.page_limit) || range.to - range.from <= 1 {
            info!(user = %name, from = range.from, to = range.to, remote, local, "repairing");
            let walker = ctx.walker(&name, Priority::Import);
            report.written += walker.copy(&ctx.store, user, range).await?;
            report.repaired.push(range);
            continue;
        }
        let mid = range.from + (range.to - range.from) / 2;
        let left = TimeRange::new(range.from, mid);
        let left_remote = ctx.upstream.total(&name, left, Priority::Import).await?;
        pending.push((
            TimeRange::new(mid, range.to),
            Some(remote.saturating_sub(left_remote)),
        ));
        pending.push((left, Some(left_remote)));
    }
    report.requests = ctx.upstream.requests() - first_request;
    Ok(report)
}

/// `[0, registered)`, then `[registered, until)` cut into equal slices.
fn slices(registered: i64, until: i64) -> Vec<TimeRange> {
    let start = registered.clamp(0, until);
    let step = ((until - start) / SLICES).max(1);
    let before = (start > 0).then(|| TimeRange::new(0, start));
    let after = (0..SLICES)
        .map(|i| start + i * step)
        .take_while(|from| *from < until)
        .map(move |from| TimeRange::new(from, (from + step).min(until)));
    let mut all: Vec<TimeRange> = before.into_iter().chain(after).collect();
    if let Some(last) = all.last_mut() {
        last.to = until;
    }
    all
}

#[cfg(test)]
mod tests {
    use super::slices;

    #[test]
    fn slices_cover_the_range_without_gaps() {
        for (registered, until) in [(1000, 1_000_000), (0, 33), (500, 510), (0, 1), (5, 5)] {
            let all = slices(registered, until);
            let mut expected_from = 0;
            for range in &all {
                assert_eq!(range.from, expected_from);
                assert!(range.to > range.from);
                expected_from = range.to;
            }
            assert_eq!(expected_from, if all.is_empty() { 0 } else { until });
        }
    }
}
