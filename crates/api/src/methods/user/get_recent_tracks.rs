//! `user.getRecentTracks`: the user's scrobbles, newest first.

use async_trait::async_trait;
use lfm_core::{ApiError, ErrorCode, Params, Response, date_node};
use lfm_store::Page;

use crate::method::{Context, Method};
use crate::methods::shared::{
    ListAttrs, OrFailed, cache_user, history_range, list_root, mirrored_user, now_playing, scrobble,
};

const DEFAULT_LIMIT: u64 = 50;
const MAX_LIMIT: u64 = 1000;
const MAX_PAGE: u64 = 1_000_000;

/// Extension: `bulk=1` raises the `limit` bound to the store's maximum, so a full history
/// takes a tenth of the requests. Last.fm has no such parameter, so no Last.fm client sends it,
/// and without it every bound and error stays Last.fm's.
const BULK_PARAM: &str = "bulk";
const BULK_MAX_LIMIT: u64 = lfm_store::MAX_LIMIT as u64;

/// `user.getRecentTracks`.
#[derive(Debug, Clone, Copy, Default)]
pub struct GetRecentTracks;

#[async_trait]
impl Method for GetRecentTracks {
    fn name(&self) -> &'static str {
        "user.getRecentTracks"
    }

    fn cache_user<'p>(&self, params: &'p Params) -> Option<&'p str> {
        cache_user(params)
    }

    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError> {
        let user = mirrored_user(ctx)?;
        let page = page(ctx.params)?;
        let range = history_range(ctx.params);
        let listing = ctx
            .services
            .store
            .recent_tracks(user.id, range, page)
            .await
            .or_failed()?;
        let extended = ctx.params.flag("extended");
        let items = listing.items.iter().map(|recent| {
            let date = date_node("date", u64::try_from(recent.uts).unwrap_or(0));
            scrobble(recent, extended, date)
        });
        let items = now_playing(ctx, user).into_iter().chain(items);
        let attrs = ListAttrs::of(&user.name, &listing);
        let root = list_root("recenttracks", "track", attrs, items);
        Ok(Response::new(root).escape_slashes())
    }
}

/// Unlike the other lists, recent tracks reject `page` and `limit` out of bounds with error 6.
/// The documented maximum `limit` of 200 is not enforced; 1000 is, or 10,000 with `bulk=1`.
fn page(params: &Params) -> Result<Page, ApiError> {
    let page = params.number("page").unwrap_or(1);
    if !(1..=MAX_PAGE).contains(&page) {
        return Err(out_of_bounds("page param out of bounds (1-1000000)".into()));
    }
    let limit = params.number("limit").unwrap_or(DEFAULT_LIMIT);
    let max = if params.flag(BULK_PARAM) {
        BULK_MAX_LIMIT
    } else {
        MAX_LIMIT
    };
    if !(1..=max).contains(&limit) {
        return Err(out_of_bounds(format!(
            "limit param out of bounds (1-{max})"
        )));
    }
    let narrow = |n: u64| u32::try_from(n).unwrap_or(u32::MAX);
    Ok(Page::new(narrow(page), narrow(limit)))
}

fn out_of_bounds(message: String) -> ApiError {
    ApiError::new(ErrorCode::InvalidParameters).with_message(message)
}
