//! `user.getWeeklyArtistChart`: the user's artists by plays in one week or any range.

use async_trait::async_trait;
use lfm_core::{ApiError, Node, Params, Response};
use lfm_store::{Kind, Ranked};

use crate::method::{Context, Method};
use crate::methods::shared::{ListShape, artist_url, cache_user, chart_list, mbid};

/// `user.getWeeklyArtistChart`.
#[derive(Debug, Clone, Copy, Default)]
pub struct GetWeeklyArtistChart;

#[async_trait]
impl Method for GetWeeklyArtistChart {
    fn name(&self) -> &'static str {
        "user.getWeeklyArtistChart"
    }

    fn cache_user<'p>(&self, params: &'p Params) -> Option<&'p str> {
        cache_user(params)
    }

    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError> {
        let shape = ListShape {
            kind: Kind::Artist,
            root: "weeklyartistchart",
            item: "artist",
            build: chart_artist,
        };
        chart_list(ctx, shape).await
    }
}

fn chart_artist(ranked: &Ranked) -> Node {
    let artist = &ranked.entity;
    Node::new("artist")
        .json_order(&["mbid", "url", "name", "@attr", "playcount"])
        .attr("rank", ranked.rank)
        .child(Node::leaf("name", artist.name.clone()))
        .child(Node::leaf("mbid", mbid(artist.mbid.as_ref())))
        .child(Node::leaf("playcount", ranked.plays))
        .child(Node::leaf(
            "url",
            artist_url(&artist.name, artist.url.as_ref()),
        ))
}
