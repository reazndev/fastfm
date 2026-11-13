//! Enrichment: learning which library entities Last.fm counts a user's scrobbles under.
//!
//! Last.fm's top lists, library and weekly charts do not count plays under the names in the
//! scrobbles. Artist names are corrected (`Berlioz` counts as `Hector Berlioz`, `zzz.` as
//! `ZZZ`), albums are filed under their album artist (`Various Artists`), and tracks keep their
//! scrobbled artist. The user's all-time top lists show the result for every entity with its
//! exact play count, along with MBIDs, URLs, covers and durations, at 1000 entries per request.
//!
//! A pass fetches the three lists, asks `artist.getCorrection` for each scrobbled artist name
//! the lists do not explain (and whose correction is not already stored), matches albums and
//! tracks by name and play count, and stores the result with `Store::apply_library`.

mod plan;
mod tracks;

use std::collections::HashMap;
use std::future::Future;

use lfm_store::{TimeRange, UserId, fold};
use lfm_upstream::{PageInfo, Priority, TopPage};
use tracing::info;

use crate::context::Context;
use crate::error::Result;

use plan::Lists;

/// What one enrichment pass did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EnrichReport {
    /// Artists, albums and tracks in Last.fm's lists.
    pub listed: [usize; 3],
    /// Scrobbled artists, albums and tracks that count under another entity.
    pub moved: [usize; 3],
    /// `artist.getCorrection` calls made.
    pub corrections: usize,
    /// Last.fm requests the pass made.
    pub requests: u64,
}

/// Reads `user`'s all-time top lists and stores the library they describe.
pub(crate) async fn enrich(ctx: &Context, user: UserId) -> Result<EnrichReport> {
    let first_request = ctx.upstream.requests();
    let name = ctx.user_name(user).await?;
    let up = &ctx.upstream;
    let lists = Lists {
        artists: all_pages(|page| up.top_artists(&name, page)).await?,
        albums: all_pages(|page| up.top_albums(&name, page)).await?,
        tracks: all_pages(|page| up.top_tracks(&name, page)).await?,
    };
    let mut inputs = ctx.store.library_inputs(user).await?;
    if reread_merged(ctx, (user, &name), &tracks::merged(&inputs, &lists)).await? > 0 {
        inputs = ctx.store.library_inputs(user).await?;
    }
    let asked = plan::needs_correction(&inputs, &lists);
    let mut corrections = HashMap::new();
    for artist in &asked {
        if let Some(corrected) = up.artist_correction(&name, artist).await? {
            corrections.insert(fold(artist), corrected.name);
        }
    }
    let library = plan::library(&inputs, &lists, &corrections);
    let report = EnrichReport {
        listed: [lists.artists.len(), lists.albums.len(), lists.tracks.len()],
        moved: plan::moved(&library),
        corrections: asked.len(),
        requests: ctx.upstream.requests() - first_request,
    };
    ctx.store.apply_library(user, library).await?;
    info!(user = %name, ?report, "library enriched");
    Ok(report)
}

/// Scrobbles one pass reads again at most, one request each.
const REREAD_CAP: usize = 100;

/// Reads the scrobbles of `tracks` again, one second each, so the store splits them by artist
/// spelling as Last.fm does. Returns how many seconds were read.
async fn reread_merged(
    ctx: &Context,
    (user, name): (UserId, &str),
    tracks: &[i64],
) -> Result<usize> {
    let mut seconds = Vec::new();
    for track in tracks {
        seconds.extend(ctx.store.track_scrobble_times(user, *track).await?);
        if seconds.len() >= REREAD_CAP {
            break;
        }
    }
    seconds.sort_unstable();
    seconds.dedup();
    seconds.truncate(REREAD_CAP);
    let walker = ctx.walker(name, Priority::Prewarm);
    for second in &seconds {
        let range = TimeRange::new(*second, second + 1);
        walker.copy(&ctx.store, user, range).await?;
    }
    if !seconds.is_empty() {
        info!(user = %name, scrobbles = seconds.len(), "read tracks merged by artist case again");
    }
    Ok(seconds.len())
}

/// Every page of a list, fetched one after another.
async fn all_pages<T, F, Fut>(fetch: F) -> Result<Vec<T>>
where
    F: Fn(u32) -> Fut,
    Fut: Future<Output = Result<TopPage<T>>>,
{
    let mut items = Vec::new();
    let mut page = 1;
    loop {
        let found = fetch(page).await?;
        let PageInfo { total_pages, .. } = found.page;
        let empty = found.items.is_empty();
        items.extend(found.items);
        if empty || page >= total_pages {
            return Ok(items);
        }
        page += 1;
    }
}
