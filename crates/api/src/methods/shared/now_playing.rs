//! The now-playing item that `user.getRecentTracks` shows first on page 1.
//!
//! Last.fm documents it as an extra track beyond `limit`, with `nowplaying="true"` and no date.
//! No fixture holds one (nothing was playing while recording), so the JSON position of `@attr`
//! follows Last.fm's documentation and commonly captured responses.

use lfm_core::{ImageSize, Node, Params, images};
use lfm_store::{MirroredUser, RecentTrack};

use super::{PLACEHOLDER, artist_mbid_attr, artist_of, artist_url, mbid, track_url};
use crate::method::Context;

/// The now-playing item for `user`, when the request shows the newest page: page 1 (or no
/// `page`) and no `to` bound.
pub(crate) fn now_playing(ctx: &Context<'_>, user: &MirroredUser) -> Option<Node> {
    if !shows_now_playing(ctx.params) {
        return None;
    }
    let playing = ctx.now_playing?.now_playing(user.id)?;
    Some(now_playing_track(&playing, ctx.params.flag("extended")))
}

fn shows_now_playing(params: &Params) -> bool {
    params.number("page").unwrap_or(1) == 1 && params.optional("to").is_none()
}

fn now_playing_track(playing: &RecentTrack, extended: bool) -> Node {
    let track = &playing.track;
    let album = playing.album.as_ref();
    let cover = album.and_then(|album| album.image.as_deref());
    let album_node = Node::leaf("album", album.map(|a| a.name.clone()).unwrap_or_default())
        .attr("mbid", mbid(album.and_then(|a| a.mbid.as_ref())));
    let node = if extended {
        Node::new("track")
            .json_order(&[
                "artist",
                "mbid",
                "name",
                "image",
                "url",
                "streamable",
                "album",
                "@attr",
                "loved",
            ])
            .child(extended_artist(playing))
            .child(Node::leaf("loved", if playing.loved { "1" } else { "0" }))
    } else {
        Node::new("track")
            .json_order(&[
                "artist",
                "streamable",
                "image",
                "mbid",
                "album",
                "name",
                "@attr",
                "url",
            ])
            .child(artist_mbid_attr(artist_of(track)))
    };
    node.attr("nowplaying", "true")
        .child(Node::leaf("name", track.name.clone()))
        .child(Node::leaf("streamable", "0"))
        .child(Node::leaf("mbid", mbid(track.mbid.as_ref())))
        .child(album_node)
        .child(Node::leaf("url", track_url(track)))
        .children(images(
            Some(cover.unwrap_or(PLACEHOLDER)),
            ImageSize::STANDARD,
        ))
}

fn extended_artist(playing: &RecentTrack) -> Node {
    let artist = artist_of(&playing.track);
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
