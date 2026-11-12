//! `user.getWeeklyChartList`: the week ranges weekly charts are available for.

use async_trait::async_trait;
use lfm_core::{ApiError, Node, Params, Response, Style};

use crate::method::{Context, Method};
use crate::methods::shared::{cache_user, mirrored_user};

/// Sun 2005-02-13 12:00 UTC, where Last.fm's list starts for every user.
const FIRST_WEEK: u64 = 1_108_296_000;
const WEEK: u64 = 604_800;

/// `user.getWeeklyChartList`. One of the methods on Last.fm's modern backend.
#[derive(Debug, Clone, Copy, Default)]
pub struct GetWeeklyChartList;

#[async_trait]
impl Method for GetWeeklyChartList {
    fn name(&self) -> &'static str {
        "user.getWeeklyChartList"
    }

    fn cache_user<'p>(&self, params: &'p Params) -> Option<&'p str> {
        cache_user(params)
    }

    fn style(&self) -> Style {
        Style::Modern
    }

    fn utf8_json(&self) -> bool {
        true
    }

    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError> {
        let user = mirrored_user(ctx)?;
        let charts = weeks(ctx.now).map(|from| {
            Node::new("chart")
                .json_order(&["#text"])
                .attr("from", from)
                .attr("to", from + WEEK)
        });
        let root = Node::new("weeklychartlist")
            .array("chart")
            .attr("user", user.name.clone())
            .children(charts);
        Ok(Response::new(root))
    }
}

/// The start of every whole week, Sunday noon to Sunday noon UTC, that ended by `now`. The list
/// does not depend on the user: it starts in 2005 even for users who registered later.
fn weeks(now: u64) -> impl Iterator<Item = u64> {
    (FIRST_WEEK..)
        .step_by(usize::try_from(WEEK).unwrap_or(usize::MAX))
        .take_while(move |from| from + WEEK <= now)
}
