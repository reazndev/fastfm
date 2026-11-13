//! Reconciliation: finding and repairing ranges where the store and Last.fm disagree.
//!
//! Users delete and edit scrobbles on Last.fm, and scrobbles sent late (offline scrobbling)
//! arrive below the sync cursor, so live sync never sees them. A full pass compares Last.fm's
//! count (`@attr.total` of a one-item page) with the store's for slices of the history on a
//! fixed grid, and live sync checks the last days the same way at a low frequency. A range that
//! differs is cut in half until a half fits in one page, which is then replaced. A split costs
//! one request when Last.fm's total for the left half matches the store and the right half's
//! total, the range's minus the left's, does too, and two otherwise: the right half is asked
//! for directly before the pass descends into it. Edits that keep the count the same are not
//! detected.
//!
//! Last.fm's totals are not always additive: a range reported 20,894 scrobbles while its halves
//! reported 14,361 and 6,526, which is what it lists and what the store holds. A range whose
//! halves both match, or whose listing does not reach its total, is recorded as an upstream
//! mismatch and left alone for a week, as is any range whose difference the recorded mismatches
//! inside it explain.

use lfm_store::{TimeRange, UpstreamMismatch, UserId};
use lfm_upstream::Priority;
use tracing::info;

use crate::clock;
use crate::context::Context;
use crate::error::{Result, SyncError};

/// Most slices of the history compared on a full pass.
const SLICES: i64 = 32;

/// How long a recorded upstream mismatch keeps its range from being checked again.
const MISMATCH_TTL: i64 = 7 * 86_400;

/// What a reconcile pass found and did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileReport {
    /// The range that was checked.
    pub range: TimeRange,
    /// Ranges whose counts were compared.
    pub compared: u64,
    /// Ranges that were replaced with Last.fm's contents.
    pub repaired: Vec<TimeRange>,
    /// Ranges where Last.fm's own counts disagree, recorded and left alone.
    pub inconsistent: Vec<TimeRange>,
    /// Scrobbles written into the repaired ranges.
    pub written: u64,
    /// Last.fm requests the pass made.
    pub requests: u64,
}

/// Checks `[0, until)` of `user`, repairs what differs, and records when it ran.
pub(crate) async fn reconcile(ctx: &Context, user: UserId, until: i64) -> Result<ReconcileReport> {
    let stored = ctx
        .store
        .user(user)
        .await?
        .ok_or_else(|| SyncError::UnknownUser(format!("id {}", user.0)))?;
    let started = clock::now();
    let mut pass = Pass::start(ctx, (user, &stored.name), TimeRange::new(0, until)).await?;
    pass.check(slices(stored.profile.registered, until)).await?;
    ctx.store.set_reconciled_at(user, started).await?;
    Ok(pass.report())
}

/// Checks the `seconds` below the user's sync cursor, where late scrobbles land, with one
/// request when nothing differs.
pub(crate) async fn trailing(
    ctx: &Context,
    (user, name): (UserId, &str),
    seconds: i64,
) -> Result<ReconcileReport> {
    let cursor = crate::tasks::sync_cursor(ctx, user).await?;
    let range = TimeRange::new(cursor.saturating_sub(seconds).max(0), cursor);
    let mut pass = Pass::start(ctx, (user, name), range).await?;
    pass.check(vec![range]).await?;
    Ok(pass.report())
}

/// One pass over some ranges of one user.
struct Pass<'a> {
    ctx: &'a Context,
    user: UserId,
    name: &'a str,
    /// Mismatches recorded within the last week, this pass's included.
    known: Vec<UpstreamMismatch>,
    first_request: u64,
    report: ReconcileReport,
}

impl<'a> Pass<'a> {
    async fn start(
        ctx: &'a Context,
        (user, name): (UserId, &'a str),
        range: TimeRange,
    ) -> Result<Self> {
        let known = ctx
            .store
            .upstream_mismatches(user, clock::now() - MISMATCH_TTL)
            .await?;
        Ok(Self {
            ctx,
            user,
            name,
            known,
            first_request: ctx.upstream.requests(),
            report: ReconcileReport {
                range,
                compared: 0,
                repaired: Vec::new(),
                inconsistent: Vec::new(),
                written: 0,
                requests: 0,
            },
        })
    }

    fn report(mut self) -> ReconcileReport {
        self.report.requests = self.ctx.upstream.requests() - self.first_request;
        self.report
    }

    /// Compares each range and repairs what differs, half by half.
    async fn check(&mut self, ranges: Vec<TimeRange>) -> Result<()> {
        let mut pending: Vec<(TimeRange, Option<u64>)> = ranges
            .into_iter()
            .rev()
            .map(|range| (range, None))
            .collect();
        while let Some((range, remote)) = pending.pop() {
            let remote = match remote {
                Some(remote) => remote,
                None => self.total(range).await?,
            };
            let local = self.local(range).await?;
            self.report.compared += 1;
            if remote == local || self.explained(range, remote, local) {
                continue;
            }
            if remote <= u64::from(self.ctx.config.page_limit) || range.to - range.from <= 1 {
                self.repair(range, remote, local).await?;
                continue;
            }
            let halves = self.differing_halves(range, remote).await?;
            if halves.is_empty() {
                self.record(range, remote, local).await?;
            }
            // The left half goes on top, so the pass moves forward in time.
            pending.extend(halves.into_iter().rev());
        }
        Ok(())
    }

    /// The halves of `range`, whose total is `parent`, that differ from the store, with their
    /// totals.
    async fn differing_halves(
        &self,
        range: TimeRange,
        parent: u64,
    ) -> Result<Vec<(TimeRange, Option<u64>)>> {
        let mid = range.from + (range.to - range.from) / 2;
        let (left, right) = (
            TimeRange::new(range.from, mid),
            TimeRange::new(mid, range.to),
        );
        let left_remote = self.total(left).await?;
        let mut out = Vec::new();
        if left_remote != self.local(left).await? {
            out.push((left, Some(left_remote)));
        }
        let right_local = self.local(right).await?;
        if parent.saturating_sub(left_remote) != right_local {
            let right_remote = self.total(right).await?;
            if right_remote != right_local {
                out.push((right, Some(right_remote)));
            }
        }
        Ok(out)
    }

    async fn repair(&mut self, range: TimeRange, remote: u64, local: u64) -> Result<()> {
        info!(user = %self.name, from = range.from, to = range.to, remote, local, "repairing");
        let walker = self.ctx.walker(self.name, Priority::Prewarm);
        self.report.written += walker.copy(&self.ctx.store, self.user, range).await?;
        self.report.repaired.push(range);
        let now = self.local(range).await?;
        if now != remote {
            // Last.fm lists fewer or more scrobbles than its total says.
            self.record(range, remote, now).await?;
        }
        Ok(())
    }

    /// Whether the mismatches recorded inside `range` add up to its difference.
    fn explained(&self, range: TimeRange, remote: u64, local: u64) -> bool {
        let extra: i128 = self
            .known
            .iter()
            .filter(|m| m.range.from >= range.from && m.range.to <= range.to)
            .map(|m| i128::from(m.remote) - i128::from(m.local))
            .sum();
        extra != 0 && i128::from(remote) - i128::from(local) == extra
    }

    async fn record(&mut self, range: TimeRange, remote: u64, local: u64) -> Result<()> {
        info!(
            user = %self.name, from = range.from, to = range.to, remote, local,
            "Last.fm's counts disagree; leaving the range alone for a week"
        );
        let now = clock::now();
        let mismatch = UpstreamMismatch {
            range,
            remote,
            local,
            seen_at: now,
        };
        let store = &self.ctx.store;
        store
            .record_upstream_mismatch(self.user, mismatch, now - MISMATCH_TTL)
            .await?;
        self.known.retain(|m| m.range != range);
        self.known.push(mismatch);
        self.report.inconsistent.push(range);
        Ok(())
    }

    async fn total(&self, range: TimeRange) -> Result<u64> {
        let up = &self.ctx.upstream;
        up.total(self.name, range, Priority::Prewarm).await
    }

    async fn local(&self, range: TimeRange) -> Result<u64> {
        Ok(self.ctx.store.count_range(self.user, range).await?)
    }
}

/// `[0, registered)`, then `[registered, until)` cut on a grid that stays put as `until`
/// grows: the step is a power of two, so the same slices, and the same halves, come back on
/// every pass until the history doubles.
fn slices(registered: i64, until: i64) -> Vec<TimeRange> {
    let start = registered.clamp(0, until);
    let step = ((until - start) / SLICES)
        .max(1)
        .unsigned_abs()
        .next_power_of_two();
    let step = i64::try_from(step).unwrap_or(i64::MAX);
    let before = (start > 0).then(|| TimeRange::new(0, start));
    let after = (0..)
        .map(|i| start.saturating_add(step.saturating_mul(i)))
        .take_while(|from| *from < until)
        .map(move |from| TimeRange::new(from, from.saturating_add(step).min(until)));
    before.into_iter().chain(after).collect()
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
            assert!(all.len() <= 33, "{}", all.len());
        }
    }

    #[test]
    fn slices_stay_put_as_the_history_grows() {
        let first = slices(1000, 1_000_000);
        let later = slices(1000, 1_020_000);
        let whole = |all: &[lfm_store::TimeRange]| all.len() - 1;
        assert_eq!(first[..whole(&first)], later[..whole(&first)]);
    }
}
