//! `user.getRecentTracks`: the user's scrobbles, newest first.

use async_trait::async_trait;
use lfm_core::{ApiError, ErrorCode, ImageSize, Node, Params, Response, date_node, images};
use lfm_store::{Page, RecentTrack, TimeRange};

use crate::method::{Context, Method};
use crate::methods::shared::{
    ListAttrs, OrFailed, PLACEHOLDER, artist_mbid_attr, artist_of, artist_url, cache_user,
    list_root, mbid, mirrored_user, track_url,
};

const DEFAULT_LIMIT: u64 = 50;
const MAX_LIMIT: u64 = 1000;
const MAX_PAGE: u64 = 1_000_000;

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
        let range = range(ctx.params);
        let listing = ctx
            .services
            .store
            .recent_tracks(user.id, range, page)
            .await
            .or_failed()?;
        let extended = ctx.params.flag("extended");
        let items = listing
            .items
            .iter()
            .map(|track| recent_track(track, extended));
        let attrs = ListAttrs::of(&user.name, &listing).recent();
        let root = list_root("recenttracks", "track", attrs, items);
        Ok(Response::new(root).escape_slashes())
    }
}

/// Unlike the other lists, recent tracks reject `page` and `limit` out of bounds with error 6.
/// The documented maximum `limit` of 200 is not enforced; 1000 is.
fn page(params: &Params) -> Result<Page, ApiError> {
    let page = params.number("page").unwrap_or(1);
    if !(1..=MAX_PAGE).contains(&page) {
        return Err(out_of_bounds("page param out of bounds (1-1000000)"));
    }
    let limit = params.number("limit").unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(out_of_bounds("limit param out of bounds (1-1000)"));
    }
    let narrow = |n: u64| u32::try_from(n).unwrap_or(u32::MAX);
    Ok(Page::new(narrow(page), narrow(limit)))
}

fn out_of_bounds(message: &'static str) -> ApiError {
    ApiError::new(ErrorCode::InvalidParameters).with_message(message)
}

/// `from` is inclusive and `to` exclusive, as observed.
fn range(params: &Params) -> TimeRange {
    let bound = |name, default| {
        params
            .number(name)
            .map_or(default, |n| i64::try_from(n).unwrap_or(i64::MAX))
    };
    TimeRange::new(
        bound("from", TimeRange::ALL.from),
        bound("to", TimeRange::ALL.to),
    )
}

fn recent_track(recent: &RecentTrack, extended: bool) -> Node {
    let track = &recent.track;
    let album = recent.album.as_ref();
    let cover = album.and_then(|album| album.image.as_deref());
    let album_node = Node::leaf("album", album.map(|a| a.name.clone()).unwrap_or_default())
        .attr("mbid", mbid(album.and_then(|a| a.mbid.as_ref())));
    let node = if extended {
        Node::new("track")
            .json_order(&[
                "artist",
                "date",
                "mbid",
                "name",
                "image",
                "url",
                "streamable",
                "album",
                "loved",
            ])
            .child(extended_artist(recent))
            .child(Node::leaf("loved", if recent.loved { "1" } else { "0" }))
    } else {
        Node::new("track")
            .json_order(&[
                "artist",
                "streamable",
                "image",
                "mbid",
                "album",
                "name",
                "url",
                "date",
            ])
            .child(artist_mbid_attr(artist_of(track)))
    };
    node.child(Node::leaf("name", track.name.clone()))
        .child(Node::leaf("streamable", "0"))
        .child(Node::leaf("mbid", mbid(track.mbid.as_ref())))
        .child(album_node)
        .child(Node::leaf("url", track_url(track)))
        .children(images(
            Some(cover.unwrap_or(PLACEHOLDER)),
            ImageSize::STANDARD,
        ))
        .child(date_node("date", u64::try_from(recent.uts).unwrap_or(0)))
}

/// The artist of `extended=1`: name, mbid, URL and the placeholder image, in JSON ordered
/// `url`, `name`, `image`, `mbid`. Last.fm leaves the mbid empty here, even for artists whose
/// mbid the plain list shows.
fn extended_artist(recent: &RecentTrack) -> Node {
    let artist = artist_of(&recent.track);
    Node::new("artist")
        .json_order(&["url", "name", "image", "mbid"])
        .child(Node::leaf("name", artist.name.clone()))
        .child(Node::leaf("mbid", ""))
        .child(Node::leaf(
            "url",
            artist_url(&artist.name, artist.url.as_ref()),
        ))
        .children(images(Some(PLACEHOLDER), ImageSize::STANDARD))
}
