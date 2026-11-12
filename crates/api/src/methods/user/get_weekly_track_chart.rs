//! `user.getWeeklyTrackChart`: the user's tracks by plays in one week or any range.

use async_trait::async_trait;
use lfm_core::{ApiError, ImageSize, Node, Params, Response, images};
use lfm_store::{Kind, Ranked};

use crate::method::{Context, Method};
use crate::methods::shared::{
    ListShape, PLACEHOLDER, artist_mbid_attr, artist_of, cache_user, chart_list, mbid, track_url,
};

/// `user.getWeeklyTrackChart`.
#[derive(Debug, Clone, Copy, Default)]
pub struct GetWeeklyTrackChart;

#[async_trait]
impl Method for GetWeeklyTrackChart {
    fn name(&self) -> &'static str {
        "user.getWeeklyTrackChart"
    }

    fn cache_user<'p>(&self, params: &'p Params) -> Option<&'p str> {
        cache_user(params)
    }

    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError> {
        let shape = ListShape {
            kind: Kind::Track,
            root: "weeklytrackchart",
            item: "track",
            build: chart_track,
        };
        chart_list(ctx, shape).await
    }
}

/// Weekly track charts list three image sizes, not four.
const CHART_IMAGES: &[ImageSize] = &[ImageSize::Small, ImageSize::Medium, ImageSize::Large];

fn chart_track(ranked: &Ranked) -> Node {
    let track = &ranked.entity;
    Node::new("track")
        .json_order(&[
            "artist",
            "image",
            "mbid",
            "url",
            "name",
            "@attr",
            "playcount",
        ])
        .attr("rank", ranked.rank)
        .child(artist_mbid_attr(artist_of(track)))
        .child(Node::leaf("name", track.name.clone()))
        .child(Node::leaf("mbid", mbid(track.mbid.as_ref())))
        .child(Node::leaf("playcount", ranked.plays))
        .children(images(Some(PLACEHOLDER), CHART_IMAGES))
        .child(Node::leaf("url", track_url(track)))
}
