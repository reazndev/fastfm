//! `user.getWeeklyAlbumChart`: the user's albums by plays in one week or any range.

use async_trait::async_trait;
use lfm_core::{ApiError, Node, Params, Response};
use lfm_store::{Kind, Ranked};

use crate::method::{Context, Method};
use crate::methods::shared::{
    ListShape, album_url, artist_mbid_attr, artist_of, cache_user, chart_list, mbid,
};

/// `user.getWeeklyAlbumChart`.
#[derive(Debug, Clone, Copy, Default)]
pub struct GetWeeklyAlbumChart;

#[async_trait]
impl Method for GetWeeklyAlbumChart {
    fn name(&self) -> &'static str {
        "user.getWeeklyAlbumChart"
    }

    fn cache_user<'p>(&self, params: &'p Params) -> Option<&'p str> {
        cache_user(params)
    }

    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError> {
        let shape = ListShape {
            kind: Kind::Album,
            root: "weeklyalbumchart",
            item: "album",
            build: chart_album,
        };
        chart_list(ctx, shape).await
    }
}

fn chart_album(ranked: &Ranked) -> Node {
    let album = &ranked.entity;
    Node::new("album")
        .json_order(&["artist", "mbid", "url", "name", "@attr", "playcount"])
        .attr("rank", ranked.rank)
        .child(artist_mbid_attr(artist_of(album)))
        .child(Node::leaf("name", album.name.clone()))
        .child(Node::leaf("mbid", mbid(album.mbid.as_ref())))
        .child(Node::leaf("playcount", ranked.plays))
        .child(Node::leaf("url", album_url(album)))
}
