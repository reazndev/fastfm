//! Reading top lists and weekly charts for the request's user.

use lfm_core::{ApiError, Node, Period, Response};
use lfm_store::{Kind, Listing, MirroredUser, Ranked, TopQuery};

use super::{
    ListAttrs, OrFailed, WeeklyRange, chart_root, list_root, mirrored_user, paging, weekly_range,
};
use crate::method::Context;

/// The page of the user's top `kind` that the request's `period`, `page` and `limit` ask for.
/// An unknown `period` means overall, as on Last.fm.
pub(crate) async fn top_page<'c>(
    ctx: &'c Context<'_>,
    kind: Kind,
    period: Period,
) -> Result<(&'c MirroredUser, Listing<Ranked>), ApiError> {
    let user = mirrored_user(ctx)?;
    let query = TopQuery {
        kind,
        period: store_period(period),
        now: i64::try_from(ctx.now).unwrap_or(i64::MAX),
    };
    let listing = ctx
        .services
        .store
        .top(user.id, query, paging(ctx.params))
        .await
        .or_failed()?;
    Ok((user, listing))
}

fn store_period(period: Period) -> lfm_store::Period {
    match period {
        Period::Overall => lfm_store::Period::Overall,
        Period::Week => lfm_store::Period::Days7,
        Period::Month => lfm_store::Period::Month1,
        Period::Quarter => lfm_store::Period::Month3,
        Period::HalfYear => lfm_store::Period::Month6,
        Period::Year => lfm_store::Period::Month12,
    }
}

/// How one kind of list is named and how its items look.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ListShape {
    /// Artists, albums or tracks.
    pub(crate) kind: Kind,
    /// The root element, such as `topartists`.
    pub(crate) root: &'static str,
    /// The item element, such as `artist`.
    pub(crate) item: &'static str,
    /// Builds one item.
    pub(crate) build: fn(&Ranked) -> Node,
}

/// A top list of the request's period, page and limit, with `@attr.user` the canonical name.
pub(crate) async fn top_list(ctx: &Context<'_>, shape: ListShape) -> Result<Response, ApiError> {
    let (user, listing) = top_page(ctx, shape.kind, Period::parse(ctx.params)).await?;
    let attrs = ListAttrs::of(&user.name, &listing);
    let items = listing.items.iter().map(shape.build);
    let root = list_root(shape.root, shape.item, attrs, items);
    Ok(Response::new(root).escape_slashes())
}

/// A weekly chart for the request's range.
pub(crate) async fn chart_list(ctx: &Context<'_>, shape: ListShape) -> Result<Response, ApiError> {
    let (user, range, chart) = weekly_chart(ctx, shape.kind).await?;
    let items = chart.iter().map(shape.build);
    let root = chart_root(shape.root, shape.item, &user.name, &range, items);
    Ok(Response::new(root).escape_slashes())
}

/// The user's whole chart of `kind` for the request's range, cut to `limit` when one is given.
/// A range before the user's first scrobble gives an empty chart, not an error.
async fn weekly_chart<'c>(
    ctx: &'c Context<'_>,
    kind: Kind,
) -> Result<(&'c MirroredUser, WeeklyRange, Vec<Ranked>), ApiError> {
    let user = mirrored_user(ctx)?;
    let range = weekly_range(ctx.params, ctx.now);
    let mut chart = ctx
        .services
        .store
        .chart(user.id, kind, range.range)
        .await
        .or_failed()?;
    // Undocumented, but Last.fm honours `limit` here.
    if let Some(limit) = ctx.params.number("limit").filter(|limit| *limit > 0) {
        chart.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    }
    Ok((user, range, chart))
}
