//! `library.getArtists`: every artist the user has played, by plays.

use async_trait::async_trait;
use lfm_core::{ApiError, ImageSize, Node, Params, Period, Response, images};
use lfm_store::{Kind, Ranked};

use crate::method::{Context, Method};
use crate::methods::shared::{
    ListAttrs, PLACEHOLDER, artist_url, cache_user, list_root, list_root_collapsing, mbid, top_page,
};

/// `library.getArtists`.
#[derive(Debug, Clone, Copy, Default)]
pub struct GetArtists;

#[async_trait]
impl Method for GetArtists {
    fn name(&self) -> &'static str {
        "library.getArtists"
    }

    fn cache_user<'p>(&self, params: &'p Params) -> Option<&'p str> {
        cache_user(params)
    }

    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError> {
        let (_, listing) = top_page(ctx, Kind::Artist, Period::Overall).await?;
        // Unlike the other user lists, `@attr.user` echoes the request's spelling.
        let requested = cache_user(ctx.params).unwrap_or_default();
        let attrs = ListAttrs::of(requested, &listing);
        let items = listing.items.iter().map(library_artist);
        // Observed: one artist collapses to an object in JSON, where other lists keep an array.
        let root = if listing.items.len() == 1 {
            list_root_collapsing("artists", attrs, items)
        } else {
            list_root("artists", "artist", attrs, items)
        };
        Ok(Response::new(root).escape_slashes())
    }
}

fn library_artist(ranked: &Ranked) -> Node {
    let artist = &ranked.entity;
    Node::new("artist")
        .json_order(&[
            "tagcount",
            "image",
            "mbid",
            "url",
            "playcount",
            "name",
            "streamable",
        ])
        .child(Node::leaf("name", artist.name.clone()))
        .child(Node::leaf("playcount", ranked.plays))
        .child(Node::leaf("tagcount", "0"))
        .child(Node::leaf("mbid", mbid(artist.mbid.as_ref())))
        .child(Node::leaf(
            "url",
            artist_url(&artist.name, artist.url.as_ref()),
        ))
        .child(Node::leaf("streamable", "0"))
        .children(images(Some(PLACEHOLDER), ImageSize::WITH_MEGA))
}
