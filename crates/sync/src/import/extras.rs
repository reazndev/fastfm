//! The parts of an import besides scrobbles: profile, loved tracks, weekly chart list.

use lfm_store::{TimeRange, UserId};
use lfm_upstream::{PageRequest, Priority};

use crate::context::Context;
use crate::convert::{self, uts};
use crate::error::Result;

/// Loved tracks per page; Last.fm accepts the same maximum as for recent tracks.
const LOVED_LIMIT: u32 = 1000;

/// Fetches the profile and makes sure the user exists under Last.fm's spelling of the name.
pub(crate) async fn profile(ctx: &Context, name: &str) -> Result<UserId> {
    let user = ctx.upstream.user_info(name, Priority::Import).await?;
    let id = ctx.store.ensure_user(&user.name).await?;
    ctx.store
        .update_profile(id, convert::profile(&user))
        .await?;
    Ok(id)
}

/// Replaces the loved tracks and the weekly chart list with what existed before `t0`.
pub(crate) async fn loved_and_charts(
    ctx: &Context,
    user: UserId,
    name: &str,
    t0: i64,
) -> Result<()> {
    let mut loves = Vec::new();
    let mut page = 1;
    loop {
        let request = PageRequest {
            page: Some(page),
            limit: Some(LOVED_LIMIT),
        };
        let found = ctx.upstream.loved(name, request, Priority::Import).await?;
        let empty = found.tracks.is_empty();
        loves.extend(
            found
                .tracks
                .iter()
                .map(convert::love)
                .filter(|l| l.uts < t0),
        );
        if empty || page >= found.page.total_pages {
            break;
        }
        page += 1;
    }
    ctx.store.replace_loved(user, loves).await?;
    let charts = ctx
        .upstream
        .chart_list(name, Priority::Import)
        .await?
        .into_iter()
        .map(|c| TimeRange::new(uts(c.from), uts(c.to)))
        .filter(|c| c.to <= t0)
        .collect();
    ctx.store.replace_weekly_charts(user, charts).await?;
    Ok(())
}
