//! `user.getTopTracks`: the user's tracks by plays in a period.

use async_trait::async_trait;
use lfm_core::{ApiError, ImageSize, Node, Params, Response, images};
use lfm_store::{Kind, Ranked};

use crate::method::{Context, Method};
use crate::methods::shared::{
    ListShape, PLACEHOLDER, artist_of, cache_user, mbid, named_artist, streamable_full, top_list,
    track_url,
};

/// `user.getTopTracks`.
#[derive(Debug, Clone, Copy, Default)]
pub struct GetTopTracks;

#[async_trait]
impl Method for GetTopTracks {
    fn name(&self) -> &'static str {
        "user.getTopTracks"
    }

    fn cache_user<'p>(&self, params: &'p Params) -> Option<&'p str> {
        cache_user(params)
    }

    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError> {
        let shape = ListShape {
            kind: Kind::Track,
            root: "toptracks",
            item: "track",
            build: top_track,
        };
        top_list(ctx, shape).await
    }
}

fn top_track(ranked: &Ranked) -> Node {
    let track = &ranked.entity;
    Node::new("track")
        .json_order(&[
            "streamable",
            "mbid",
            "name",
            "image",
            "artist",
            "url",
            "duration",
            "@attr",
            "playcount",
        ])
        .attr("rank", ranked.rank)
        .child(Node::leaf("name", track.name.clone()))
        .child(Node::leaf("duration", track.duration.unwrap_or(0)))
        .child(Node::leaf("playcount", ranked.plays))
        .child(Node::leaf("mbid", mbid(track.mbid.as_ref())))
        .child(Node::leaf("url", track_url(track)))
        .child(streamable_full())
        .child(named_artist(artist_of(track)))
        .children(images(Some(PLACEHOLDER), ImageSize::STANDARD))
}
