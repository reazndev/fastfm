//! Splitting an import into ranges of about equal scrobble counts, so workers finish together.
//!
//! With `limit=1`, page `n` of `user.getRecentTracks` is the `n`-th newest scrobble, so a few
//! requests give exact quantiles of the history without reading it.

use lfm_store::TimeRange;
use lfm_upstream::Priority;

use crate::context::Context;
use crate::convert::{param, uts};
use crate::error::Result;
use crate::import::state::{ImportState, Segment};
use crate::upstream::recent_query;

/// Last.fm's largest accepted page number.
const MAX_PAGE: u64 = 1_000_000;

/// Fills in `state.total` and `state.segments`.
pub(crate) async fn plan(ctx: &Context, user: &str, state: &mut ImportState) -> Result<()> {
    let all = TimeRange::new(0, state.t0);
    let total = ctx.upstream.total(user, all, Priority::Import).await?;
    let pages = total.div_ceil(u64::from(ctx.config.page_limit));
    let workers = pages.clamp(1, u64::try_from(ctx.config.import_concurrency).unwrap_or(1));
    let mut bounds = vec![state.t0];
    for index in (1..workers).map(|i| i * total / workers) {
        let bound = newest_at(ctx, user, state.t0, index).await?;
        if bounds.last().is_some_and(|last| bound > 0 && bound < *last) {
            bounds.push(bound);
        }
    }
    bounds.push(0);
    state.total = total;
    state.segments = bounds
        .windows(2)
        .filter_map(|pair| match pair {
            [to, from] => Some(Segment {
                from: *from,
                cursor: *to,
            }),
            _ => None,
        })
        .collect();
    Ok(())
}

/// The time of the `index`-th newest scrobble (from 0) before `t0`.
async fn newest_at(ctx: &Context, user: &str, t0: i64, index: u64) -> Result<i64> {
    let limit = if index < MAX_PAGE { 1 } else { 1000 };
    let page = u32::try_from(index / limit + 1).unwrap_or(u32::MAX);
    let query = recent_query(user)
        .limit(u32::try_from(limit).unwrap_or(1))
        .page(page)
        .to(param(t0));
    let found = ctx.upstream.recent(user, &query, Priority::Import).await?;
    Ok(found.scrobbles.first().map_or(0, |s| uts(s.uts)))
}
