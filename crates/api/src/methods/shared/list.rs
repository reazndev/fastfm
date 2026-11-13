//! List envelopes: paged lists with their `@attr`, and weekly charts with their range.

use lfm_core::{Node, PageSpec, Paging, Params};
use lfm_store::{Listing, Page, TimeRange};

/// Paging for top, loved and library lists. Last.fm's default page size is 50; past 1000 is
/// clamped.
const LIST_PAGING: PageSpec = PageSpec::new(50, 1000);

const DAY: u64 = 86_400;
const WEEK: u64 = 7 * DAY;

/// The page a top, loved or library request asks for.
pub(crate) fn paging(params: &Params) -> Page {
    let paging = Paging::parse(params, LIST_PAGING);
    Page::new(paging.page, paging.limit)
}

/// JSON key orders of a list's `@attr`. Last.fm's order changes with the request, as below; the
/// cause is unknown, so these copy what the 2026-11-12 fixtures show.
const USUAL_ORDER: &[&str] = &["user", "totalPages", "page", "perPage", "total"];
/// Every list method with `limit=200`.
const LIMIT_200_ORDER: &[&str] = &["perPage", "totalPages", "page", "user", "total"];
/// Recent tracks above 200 per page; loved tracks and friends when page 1 holds the whole list.
const TOTAL_FIRST_ORDER: &[&str] = &["user", "totalPages", "page", "total", "perPage"];

/// The `@attr` of a paged list.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ListAttrs<'a> {
    user: &'a str,
    page: u32,
    per_page: u32,
    total_pages: u64,
    total: u64,
    json_order: &'static [&'static str],
}

impl<'a> ListAttrs<'a> {
    /// The attributes of one page of a list that belongs to `user`.
    pub(crate) fn of<T>(user: &'a str, listing: &Listing<T>) -> Self {
        let per_page = listing.page.limit;
        Self {
            user,
            page: listing.page.page,
            per_page,
            total_pages: listing.total_pages(),
            total: listing.total,
            json_order: if per_page == 200 {
                LIMIT_200_ORDER
            } else {
                USUAL_ORDER
            },
        }
    }

    /// The JSON key order recent tracks use.
    pub(crate) fn recent(mut self) -> Self {
        if self.per_page > 200 {
            self.json_order = TOTAL_FIRST_ORDER;
        }
        self
    }

    /// The JSON key order loved tracks use.
    pub(crate) fn loved(mut self) -> Self {
        if self.page == 1 && self.total <= u64::from(self.per_page) {
            self.json_order = TOTAL_FIRST_ORDER;
        }
        self
    }
}

/// A paged list such as `<topartists user="…" page="1" perPage="50" totalPages="…" total="…">`.
///
/// In JSON the items are an array even when there is one or none; see [`list_root_collapsing`]
/// for the exception.
pub(crate) fn list_root(
    name: &'static str,
    item: &'static str,
    attrs: ListAttrs<'_>,
    items: impl IntoIterator<Item = Node>,
) -> Node {
    list_root_collapsing(name, attrs, items).array(item)
}

/// A paged list whose single item collapses to an object in JSON, as `library.getArtists` does.
pub(crate) fn list_root_collapsing(
    name: &'static str,
    attrs: ListAttrs<'_>,
    items: impl IntoIterator<Item = Node>,
) -> Node {
    Node::new(name)
        .json_order(attrs.json_order)
        .attr("user", attrs.user.to_owned())
        .attr("page", attrs.page)
        .attr("perPage", attrs.per_page)
        .attr("totalPages", attrs.total_pages)
        .attr("total", attrs.total)
        .children(items)
}

/// The `from` and `to` of a history request: `from` inclusive and `to` exclusive, as observed
/// on recent tracks.
pub(crate) fn history_range(params: &Params) -> TimeRange {
    let bound = |name, default| {
        params
            .number(name)
            .map_or(default, |n| i64::try_from(n).unwrap_or(i64::MAX))
    };
    TimeRange::new(
        bound("from", TimeRange::ALL.from),
        bound("to", TimeRange::ALL.to),
    )
}

/// The range of a weekly chart and how `@attr` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WeeklyRange {
    /// `from` as the request gave it, or the default range's start.
    pub(crate) from: String,
    /// `to` as the request gave it, or the default range's end.
    pub(crate) to: String,
    /// The range charted, `to` exclusive.
    pub(crate) range: TimeRange,
}

/// The chart range: `from` and `to` as given (off-grid values are accepted), or by default the
/// seven days ending at noon UTC of the day before `now`.
///
/// Observed: a request on Thu 2026-11-12 21:30 UTC without a range got Wed 2026-11-04 12:00 to
/// Wed 2026-11-11 12:00, which is not the last week of `user.getWeeklyChartList`. One sample
/// only; the rule may depend on the weekday.
pub(crate) fn weekly_range(params: &Params, now: u64) -> WeeklyRange {
    let numeric = |name| {
        params
            .optional(name)
            .filter(|_| params.number(name).is_some())
    };
    let given = numeric("from").zip(numeric("to"));
    let given = given.map(|(from, to)| (from.to_owned(), to.to_owned()));
    let (from, to) = given.unwrap_or_else(|| {
        let to = (now / DAY * DAY).saturating_sub(DAY / 2);
        (to.saturating_sub(WEEK).to_string(), to.to_string())
    });
    let bound = |text: &str| text.trim().parse::<i64>().unwrap_or(0);
    let range = TimeRange::new(bound(&from), bound(&to));
    WeeklyRange { from, to, range }
}

/// A weekly chart such as `<weeklyartistchart user="…" from="…" to="…">`. JSON orders the
/// `@attr` keys `from`, `user`, `to`.
pub(crate) fn chart_root(
    name: &'static str,
    item: &'static str,
    user: &str,
    range: &WeeklyRange,
    items: impl IntoIterator<Item = Node>,
) -> Node {
    Node::new(name)
        .json_order(&["from", "user", "to"])
        .array(item)
        .attr("user", user.to_owned())
        .attr("from", range.from.clone())
        .attr("to", range.to.clone())
        .children(items)
}
