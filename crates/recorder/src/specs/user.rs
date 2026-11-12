//! `user.*` and `library.*`: the methods served from local data.

use super::paging;
use crate::discover::Discovered;
use crate::spec::Spec;

const PERIODS: [&str; 6] = ["overall", "7day", "1month", "3month", "6month", "12month"];

pub(super) fn specs(d: &Discovered) -> Vec<Spec> {
    [
        recent_tracks(d),
        top_lists(d),
        weekly_charts(d),
        profile(d),
        tags(d),
    ]
    .concat()
}

fn base(method: &str, user: &str) -> Spec {
    Spec::new(method, "default").param("user", user)
}

fn recent_tracks(d: &Discovered) -> Vec<Spec> {
    let base = base("user.getRecentTracks", &d.user);
    let ranged = |variant: &str, from: u64, to: u64| {
        base.with_variant(variant).num("from", from).num("to", to)
    };
    let mut specs = vec![
        base.clone(),
        base.with_variant("extended-0").num("extended", 0),
        base.with_variant("extended-1").num("extended", 1),
        base.with_variant("extended-1-limit-200")
            .num("extended", 1)
            .num("limit", 200),
        base.with_variant("limit-0").num("limit", 0),
        base.with_variant("limit-1000").num("limit", 1000),
        base.with_variant("page-0").num("page", 0),
        ranged("range-day", d.day.from, d.day.to),
        ranged("range-day-extended-1", d.day.from, d.day.to).num("extended", 1),
        ranged("range-one-scrobble", d.one.from, d.one.to),
        ranged("range-zero-scrobbles", d.zero.from, d.zero.to),
        ranged("range-from-equals-to", d.single_uts, d.single_uts),
        // These two show which end of the range is inclusive.
        ranged("range-from-at-scrobble", d.single_uts, d.single_uts + 1),
        ranged("range-to-at-scrobble", d.single_uts - 1, d.single_uts),
        base.with_variant("from-only").num("from", d.day.from),
        base.with_variant("to-only").num("to", d.day.to),
    ];
    specs.extend(paging(&base, d.last_pages.recent));
    specs
}

fn top_lists(d: &Discovered) -> Vec<Spec> {
    [
        "user.getTopArtists",
        "user.getTopAlbums",
        "user.getTopTracks",
    ]
    .into_iter()
    .flat_map(|method| {
        let base = base(method, &d.user);
        let periods = PERIODS.map(|p| base.with_variant(&format!("period-{p}")).param("period", p));
        let extra = [
            base.clone(),
            base.with_variant("period-invalid").param("period", "2week"),
            base.with_variant("period-7day-limit-1")
                .param("period", "7day")
                .num("limit", 1),
        ];
        let paged = paging(&base, d.last_pages.top_artists);
        // The last-page count is only known for artists; the others keep pages 1 and 2.
        let paged = paged
            .into_iter()
            .filter(move |s| method == "user.getTopArtists" || s.variant() != "page-last");
        extra.into_iter().chain(periods).chain(paged)
    })
    .collect()
}

fn weekly_charts(d: &Discovered) -> Vec<Spec> {
    let list = vec![base("user.getWeeklyChartList", &d.user)];
    let charts = [
        "user.getWeeklyArtistChart",
        "user.getWeeklyAlbumChart",
        "user.getWeeklyTrackChart",
    ]
    .into_iter()
    .flat_map(|method| {
        let base = base(method, &d.user);
        let ranged = |variant: &str, from: u64, to: u64| {
            base.with_variant(variant).num("from", from).num("to", to)
        };
        let latest = d
            .charts
            .last()
            .map(|c| ranged("range-latest", c.from, c.to));
        let oldest = d
            .charts
            .first()
            .map(|c| ranged("range-oldest", c.from, c.to));
        let middle = d
            .charts
            .get(d.charts.len().saturating_sub(10))
            .map(|c| ranged("range-ten-weeks-ago", c.from, c.to));
        let off_grid = ranged("range-not-on-week-grid", d.day.from, d.day.to);
        [Some(base.clone()), latest, oldest, middle, Some(off_grid)]
            .into_iter()
            .flatten()
            .chain([base.with_variant("limit-1").num("limit", 1)])
            .collect::<Vec<_>>()
    });
    list.into_iter().chain(charts).collect()
}

fn profile(d: &Discovered) -> Vec<Spec> {
    let loved = base("user.getLovedTracks", &d.user);
    let library = base("library.getArtists", &d.user);
    let friends = base("user.getFriends", &d.user);
    let friend_info = d
        .friend
        .as_ref()
        .map(|f| Spec::new("user.getInfo", "friend").param("user", f));
    [
        base("user.getInfo", &d.user),
        Spec::new("user.getInfo", "user-uppercase").param("user", &d.user.to_uppercase()),
        friends.clone(),
        friends
            .with_variant("recenttracks-1")
            .num("recenttracks", 1),
        friends.with_variant("limit-1").num("limit", 1),
        friends.with_variant("page-2").num("page", 2),
        loved.clone(),
        library.clone(),
    ]
    .into_iter()
    .chain(friend_info)
    .chain(paging(&loved, d.last_pages.loved))
    .chain(paging(&library, d.last_pages.library))
    .collect()
}

fn tags(d: &Discovered) -> Vec<Spec> {
    let top = base("user.getTopTags", &d.user);
    // Without a tag of the user's own, a global tag still shows the empty-result shape.
    let tag = d.user_tag.as_deref().unwrap_or(&d.global_tag);
    let personal = base("user.getPersonalTags", &d.user).param("tag", tag);
    let by_kind = ["artist", "album", "track"].map(|kind| {
        personal
            .with_variant(&format!("taggingtype-{kind}"))
            .param("taggingtype", kind)
    });
    [
        top.clone(),
        top.with_variant("limit-1").num("limit", 1),
        personal
            .with_variant("taggingtype-artist-limit-1")
            .param("taggingtype", "artist")
            .num("limit", 1),
        personal.with_variant("error-missing-taggingtype"),
    ]
    .into_iter()
    .chain(by_kind)
    .collect()
}
