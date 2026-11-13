//! The fixed list of scenarios, with concrete parameters picked from the user's mirrored data.

use lfm_store::{MirroredUser, Page, Store, StoreError, TimeRange};
use serde::Serialize;

/// Seconds in a day.
const DAY: i64 = 86_400;
/// Rows per page of the deep-history and paged scenarios.
const PAGE: u32 = 200;
/// The largest `limit` that `user.getRecentTracks` accepts, on Last.fm and here alike.
const MAX_RECENT_LIMIT: u32 = 1000;
/// Scrobbles the "newest 2,000" scenarios fetch.
const NEWEST: u32 = 2000;

/// What the scenarios are built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Facts {
    /// The user, as Last.fm spells the name.
    pub(crate) user: String,
    /// The middle page of the history in pages of [`PAGE`] scrobbles.
    pub(crate) middle_page: u32,
    /// The time of the first scrobble on the middle page, if there is one.
    pub(crate) middle_uts: Option<i64>,
    /// The weekly chart range that holds `middle_uts`, or the nearest one before it.
    pub(crate) week: Option<TimeRange>,
}

/// Reads what [`scenarios`] needs from the store.
pub(crate) async fn facts(store: &Store, user: &MirroredUser) -> Result<Facts, StoreError> {
    let total = store
        .recent_tracks(user.id, TimeRange::ALL, Page::new(1, 1))
        .await?
        .total;
    let pages = u32::try_from(total.div_ceil(u64::from(PAGE))).unwrap_or(u32::MAX);
    let middle_page = (pages / 2).max(1);
    let first_of_middle = (middle_page - 1).saturating_mul(PAGE).saturating_add(1);
    let middle_uts = store
        .recent_tracks(user.id, TimeRange::ALL, Page::new(first_of_middle, 1))
        .await?
        .items
        .first()
        .map(|track| track.uts);
    let charts = store.weekly_charts(user.id).await?;
    let week = middle_uts.and_then(|uts| {
        charts
            .iter()
            .filter(|range| range.from <= uts)
            .max_by_key(|range| range.from)
            .copied()
    });
    Ok(Facts {
        user: user.name.clone(),
        middle_page,
        middle_uts,
        week,
    })
}

/// One measured case: one or more requests sent in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Scenario {
    pub(crate) id: &'static str,
    pub(crate) title: &'static str,
    /// Each request's parameters, without `api_key` and `format`.
    pub(crate) requests: Vec<Vec<(&'static str, String)>>,
}

/// A scenario as the UI lists it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScenarioView {
    id: &'static str,
    title: &'static str,
    /// The method and parameters, such as `user.getRecentTracks limit=200 page=12`.
    detail: String,
    requests: usize,
}

impl From<&Scenario> for ScenarioView {
    fn from(scenario: &Scenario) -> Self {
        let detail = scenario
            .requests
            .first()
            .map_or_else(String::new, |params| {
                params
                    .iter()
                    .filter(|(name, _)| *name != "user")
                    .map(|(name, value)| match *name {
                        "method" => value.clone(),
                        _ => format!("{name}={value}"),
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            });
        Self {
            id: scenario.id,
            title: scenario.title,
            detail,
            requests: scenario.requests.len(),
        }
    }
}

/// The scenarios, in the order the UI runs them.
pub(crate) fn scenarios(facts: &Facts) -> Vec<Scenario> {
    let user = facts.user.as_str();
    let mut all = single_requests(facts);
    all.push(newest(
        user,
        "newest_by_200",
        "Newest 2,000 scrobbles, 10 pages of 200",
        PAGE,
    ));
    let title = "Newest 2,000 scrobbles, 2 pages of 1,000";
    all.push(newest(user, "newest_by_1000", title, MAX_RECENT_LIMIT));
    all
}

fn single_requests(facts: &Facts) -> Vec<Scenario> {
    let one = |id, title, method, extra: Vec<(&'static str, String)>| Scenario {
        id,
        title,
        requests: vec![request(method, &facts.user, extra)],
    };
    let day = facts.middle_uts.unwrap_or_default().div_euclid(DAY) * DAY;
    let week = facts.week.map_or_else(Vec::new, |range| {
        vec![pair("from", range.from), pair("to", range.to)]
    });
    let recent = "user.getRecentTracks";
    vec![
        one(
            "latest",
            "Latest 50 scrobbles",
            recent,
            vec![pair("limit", 50)],
        ),
        one(
            "day",
            "One day of history",
            recent,
            vec![
                pair("limit", PAGE),
                pair("from", day),
                pair("to", day + DAY),
            ],
        ),
        one(
            "deep_page",
            "A deep history page",
            recent,
            vec![pair("limit", PAGE), pair("page", facts.middle_page)],
        ),
        one(
            "top_artists",
            "Top artists, last 12 months",
            "user.getTopArtists",
            vec![pair("period", "12month")],
        ),
        one(
            "top_tracks",
            "Top tracks, all time",
            "user.getTopTracks",
            vec![pair("period", "overall")],
        ),
        one(
            "top_albums",
            "Top albums, last 7 days",
            "user.getTopAlbums",
            vec![pair("period", "7day")],
        ),
        one(
            "weekly_chart",
            "Weekly artist chart",
            "user.getWeeklyArtistChart",
            week,
        ),
        one("profile", "Profile", "user.getInfo", Vec::new()),
    ]
}

/// The newest [`NEWEST`] scrobbles in sequential pages of `limit`.
fn newest(user: &str, id: &'static str, title: &'static str, limit: u32) -> Scenario {
    let requests = (1..=NEWEST / limit)
        .map(|page| {
            request(
                "user.getRecentTracks",
                user,
                vec![pair("limit", limit), pair("page", page)],
            )
        })
        .collect();
    Scenario {
        id,
        title,
        requests,
    }
}

fn request(
    method: &'static str,
    user: &str,
    extra: Vec<(&'static str, String)>,
) -> Vec<(&'static str, String)> {
    [pair("method", method), pair("user", user)]
        .into_iter()
        .chain(extra)
        .collect()
}

fn pair(name: &'static str, value: impl std::fmt::Display) -> (&'static str, String) {
    (name, value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_middle_page_its_day_and_its_week() {
        let facts = Facts {
            user: "u".into(),
            middle_page: 5,
            middle_uts: Some(DAY * 3 + 5),
            week: Some(TimeRange::new(DAY * 2, DAY * 9)),
        };
        let all = scenarios(&facts);
        assert_eq!(all.len(), 10);
        let view = |id: &str| ScenarioView::from(all.iter().find(|s| s.id == id).unwrap());
        let day = format!(
            "user.getRecentTracks limit=200 from={} to={}",
            DAY * 3,
            DAY * 4
        );
        assert_eq!(view("day").detail, day);
        assert_eq!(
            view("deep_page").detail,
            "user.getRecentTracks limit=200 page=5"
        );
        let week = format!("user.getWeeklyArtistChart from={} to={}", DAY * 2, DAY * 9);
        assert_eq!(view("weekly_chart").detail, week);
        assert_eq!(view("newest_by_200").requests, 10);
        assert_eq!(view("newest_by_1000").requests, 2);
        assert_eq!(view("profile").detail, "user.getInfo");
    }
}
