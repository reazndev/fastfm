//! `user.getTopAlbums`: the user's albums by plays in a period.

use async_trait::async_trait;
use lfm_core::{ApiError, ImageSize, Node, Params, Response, images};
use lfm_store::{Kind, Ranked};

use crate::method::{Context, Method};
use crate::methods::shared::{
    ListShape, PLACEHOLDER, album_url, artist_of, cache_user, mbid, named_artist, top_list,
};

/// `user.getTopAlbums`.
#[derive(Debug, Clone, Copy, Default)]
pub struct GetTopAlbums;

#[async_trait]
impl Method for GetTopAlbums {
    fn name(&self) -> &'static str {
        "user.getTopAlbums"
    }

    fn cache_user<'p>(&self, params: &'p Params) -> Option<&'p str> {
        cache_user(params)
    }

    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError> {
        let shape = ListShape {
            kind: Kind::Album,
            root: "topalbums",
            item: "album",
            build: top_album,
        };
        top_list(ctx, shape).await
    }
}

fn top_album(ranked: &Ranked) -> Node {
    let album = &ranked.entity;
    // Albums without cover art show empty image URLs here, but the placeholder in recent tracks.
    // The library cover is stored as "" for none.
    let image = album
        .image
        .as_deref()
        .filter(|image| !image.is_empty() && *image != PLACEHOLDER);
    Node::new("album")
        .json_order(&[
            "artist",
            "image",
            "mbid",
            "url",
            "playcount",
            "@attr",
            "name",
        ])
        .attr("rank", ranked.rank)
        .child(Node::leaf("name", album.name.clone()))
        .child(Node::leaf("playcount", ranked.plays))
        .child(Node::leaf("mbid", mbid(album.mbid.as_ref())))
        .child(Node::leaf("url", album_url(album)))
        .child(named_artist(artist_of(album)))
        .children(images(image, ImageSize::STANDARD))
}
