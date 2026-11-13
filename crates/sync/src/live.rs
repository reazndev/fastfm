//! Live sync: one poll of a user's newest scrobbles and now-playing track.
//!
//! The user's sync cursor is a time below which everything is stored. A poll asks for every
//! scrobble from the cursor on and replaces the window from the cursor to the newest scrobble it
//! saw. The cursor then moves to that newest second, not past it, so the next poll reads that
//! second again: a scrobble that lands in it later is not lost, and since the window is replaced
//! rather than appended to, the ones already stored are not doubled.

use lfm_store::{TimeRange, UserId};
use lfm_upstream::Priority;

use crate::clock;
use crate::context::Context;
use crate::convert::{self, param, uts};
use crate::error::{Result, SyncError};
use crate::upstream::recent_query;

/// What one poll saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Poll {
    /// Whether a track is playing.
    pub(crate) playing: bool,
    /// The newest scrobble the poll returned, in Unix seconds.
    pub(crate) newest: Option<i64>,
}

/// Polls once.
pub(crate) async fn poll(ctx: &Context, user: UserId, name: &str) -> Result<Poll> {
    let cursor = ctx
        .store
        .user(user)
        .await?
        .ok_or_else(|| SyncError::UnknownUser(name.to_owned()))?
        .sync_cursor;
    let query = recent_query(name)
        .limit(ctx.config.page_limit)
        .from(param(cursor));
    let page = ctx.upstream.recent(name, &query, Priority::Live).await?;
    let playing = page.now_playing.is_some();
    ctx.now_playing.set(user, page.now_playing, clock::now());
    let fresh: Vec<_> = page
        .scrobbles
        .iter()
        .filter(|s| uts(s.uts) >= cursor)
        .collect();
    let Some(newest) = fresh.iter().map(|s| uts(s.uts)).max() else {
        return Ok(Poll {
            playing,
            newest: None,
        });
    };
    let range = TimeRange::new(cursor, newest + 1);
    if u64::try_from(fresh.len()).unwrap_or(u64::MAX) >= page.page.total {
        let scrobbles = convert::scrobbles(fresh.into_iter());
        ctx.store.replace_window(user, range, scrobbles).await?;
    } else {
        // More than one page arrived since the last poll, after downtime for example.
        ctx.walker(name, Priority::Live)
            .copy(&ctx.store, user, range)
            .await?;
    }
    if newest > cursor {
        ctx.store.set_sync_cursor(user, newest).await?;
    }
    Ok(Poll {
        playing,
        newest: Some(newest),
    })
}
