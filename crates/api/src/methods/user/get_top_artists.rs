//! `user.getTopArtists`: the user's artists by plays in a period.

use async_trait::async_trait;
use lfm_core::{ApiError, ImageSize, Node, Params, Response, images};
use lfm_store::{Kind, Ranked};

use crate::method::{Context, Method};
use crate::methods::shared::{ListShape, PLACEHOLDER, artist_url, cache_user, mbid, top_list};

/// `user.getTopArtists`.
#[derive(Debug, Clone, Copy, Default)]
pub struct GetTopArtists;

#[async_trait]
impl Method for GetTopArtists {
    fn name(&self) -> &'static str {
        "user.getTopArtists"
    }

    fn cache_user<'p>(&self, params: &'p Params) -> Option<&'p str> {
        cache_user(params)
    }

    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError> {
        let shape = ListShape {
            kind: Kind::Artist,
            root: "topartists",
            item: "artist",
            build: top_artist,
        };
        top_list(ctx, shape).await
    }
}

fn top_artist(ranked: &Ranked) -> Node {
    let artist = &ranked.entity;
    Node::new("artist")
        .json_order(&[
            "streamable",
            "image",
            "mbid",
            "url",
            "playcount",
            "@attr",
            "name",
        ])
        .attr("rank", ranked.rank)
        .child(Node::leaf("name", artist.name.clone()))
        .child(Node::leaf("playcount", ranked.plays))
        .child(Node::leaf("mbid", mbid(artist.mbid.as_ref())))
        .child(Node::leaf(
            "url",
            artist_url(&artist.name, artist.url.as_ref()),
        ))
        .child(Node::leaf("streamable", "0"))
        .children(images(Some(PLACEHOLDER), ImageSize::WITH_MEGA))
}
