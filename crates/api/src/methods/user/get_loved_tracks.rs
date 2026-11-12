//! `user.getLovedTracks`: the tracks the user loved, newest first.

use async_trait::async_trait;
use lfm_core::{ApiError, ImageSize, Node, Params, Response, date_node, images};
use lfm_store::LovedTrack;

use crate::method::{Context, Method};
use crate::methods::shared::{
    ListAttrs, OrFailed, PLACEHOLDER, artist_of, cache_user, list_root, mbid, mirrored_user,
    named_artist, paging, streamable_full, track_url,
};

/// `user.getLovedTracks`.
#[derive(Debug, Clone, Copy, Default)]
pub struct GetLovedTracks;

#[async_trait]
impl Method for GetLovedTracks {
    fn name(&self) -> &'static str {
        "user.getLovedTracks"
    }

    fn cache_user<'p>(&self, params: &'p Params) -> Option<&'p str> {
        cache_user(params)
    }

    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError> {
        let user = mirrored_user(ctx)?;
        let listing = ctx
            .services
            .store
            .loved_tracks(user.id, paging(ctx.params))
            .await
            .or_failed()?;
        let attrs = ListAttrs::of(&user.name, &listing).loved();
        let items = listing.items.iter().map(loved_track);
        let root = list_root("lovedtracks", "track", attrs, items);
        Ok(Response::new(root).escape_slashes())
    }
}

fn loved_track(loved: &LovedTrack) -> Node {
    let track = &loved.track;
    Node::new("track")
        .json_order(&[
            "artist",
            "date",
            "mbid",
            "url",
            "name",
            "image",
            "streamable",
        ])
        .child(Node::leaf("name", track.name.clone()))
        .child(Node::leaf("mbid", mbid(track.mbid.as_ref())))
        .child(Node::leaf("url", track_url(track)))
        .child(date_node("date", u64::try_from(loved.uts).unwrap_or(0)))
        .child(named_artist(artist_of(track)))
        .children(images(Some(PLACEHOLDER), ImageSize::STANDARD))
        .child(streamable_full())
}
