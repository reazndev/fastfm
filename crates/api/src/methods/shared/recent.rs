//! The items of `user.getRecentTracks`: scrobbles, and the track playing now.

use lfm_core::{ImageSize, Node, images};
use lfm_store::{MirroredUser, RecentTrack};

use super::{PLACEHOLDER, artist_mbid_attr, artist_of, artist_url, mbid, track_url};
use crate::method::Context;

/// JSON key orders of a recent-tracks item. The now-playing item has its own, taken from live
/// responses on 2026-11-13.
const PLAIN: &[&str] = &[
    "artist",
    "streamable",
    "image",
    "mbid",
    "album",
    "name",
    "url",
    "date",
];
const PLAIN_PLAYING: &[&str] = &[
    "artist",
    "streamable",
    "image",
    "mbid",
    "album",
    "name",
    "@attr",
    "url",
];
const EXTENDED: &[&str] = &[
    "artist",
    "date",
    "mbid",
    "name",
    "image",
    "url",
    "streamable",
    "album",
    "loved",
];
const EXTENDED_PLAYING: &[&str] = &[
    "artist",
    "mbid",
    "name",
    "image",
    "streamable",
    "album",
    "url",
    "@attr",
    "loved",
];

/// The now-playing item for `user`. Last.fm puts it first on every page and in every
/// `from`/`to` range, even one that ended in the past (seen on 2026-11-13 on pages 1, 402 and
/// 804 and on a day-long range a day earlier).
pub(crate) fn now_playing(ctx: &Context<'_>, user: &MirroredUser) -> Option<Node> {
    let playing = ctx.now_playing?.now_playing(user.id)?;
    let extended = ctx.params.flag("extended");
    let order = if extended {
        EXTENDED_PLAYING
    } else {
        PLAIN_PLAYING
    };
    Some(recent_item(&playing, extended, order).attr("nowplaying", "true"))
}

/// A scrobble with its date.
pub(crate) fn scrobble(recent: &RecentTrack, extended: bool, date: Node) -> Node {
    let order = if extended { EXTENDED } else { PLAIN };
    recent_item(recent, extended, order).child(date)
}

/// Images follow the album: its cover, the placeholder for an album without one, and empty
/// strings when the scrobble has no album (128 of 128 such tracks on 2026-11-13).
fn recent_item(recent: &RecentTrack, extended: bool, order: &'static [&'static str]) -> Node {
    let track = &recent.track;
    let album = recent.album.as_ref();
    let cover = album.map(|album| album.image.as_deref().unwrap_or(PLACEHOLDER));
    let album_node = Node::leaf("album", album.map(|a| a.name.clone()).unwrap_or_default())
        .attr("mbid", mbid(album.and_then(|a| a.mbid.as_ref())));
    let node = Node::new("track").json_order(order);
    let node = if extended {
        node.child(extended_artist(recent))
            .child(Node::leaf("loved", if recent.loved { "1" } else { "0" }))
    } else {
        node.child(artist_mbid_attr(artist_of(track)))
    };
    node.child(Node::leaf("name", track.name.clone()))
        .child(Node::leaf("streamable", "0"))
        .child(Node::leaf("mbid", mbid(track.mbid.as_ref())))
        .child(album_node)
        .child(Node::leaf("url", track_url(track)))
        .children(images(cover, ImageSize::STANDARD))
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
