//! Every local method replayed against the recorded Last.fm fixtures, in XML and JSON.
//!
//! The store holds the newest 1000 scrobbles of the fixture user, so whatever only depends on
//! that window (ranges, 7-day lists, weekly charts, loved tracks) must match byte for byte. For
//! requests over the whole history, the edit closures swap the history's totals for ours.
#![allow(clippy::unwrap_used, reason = "tests")]

mod support;

use lfm_api::Dispatcher;
use support::{KEY, assert_replays, dispatcher, same, store};

/// Replaces each `(recorded, ours)` value in both JSON (`"name":"v"`) and XML (`name="v"`,
/// `<name>v</name>`) spellings.
fn swap(pairs: &[(&str, &str, &str)]) -> impl Fn(&str) -> String + use<> {
    let pairs: Vec<(String, String, String)> = pairs
        .iter()
        .map(|(name, from, to)| ((*name).to_owned(), (*from).to_owned(), (*to).to_owned()))
        .collect();
    move |body| {
        pairs
            .iter()
            .fold(body.to_owned(), |body, (name, from, to)| {
                body.replace(
                    &format!("\"{name}\":\"{from}\""),
                    &format!("\"{name}\":\"{to}\""),
                )
                .replace(&format!("{name}=\"{from}\""), &format!("{name}=\"{to}\""))
                .replace(&format!("<{name}>{from}<"), &format!("<{name}>{to}<"))
            })
    }
}

async fn fixture_store() -> Dispatcher {
    dispatcher(store().await.0)
}

const RECENT: &str = "user.getRecentTracks";

#[tokio::test]
async fn recent_tracks_in_ranges_match_exactly() {
    let d = fixture_store().await;
    for variant in [
        "range-day",
        "range-day-extended-1",
        "range-from-at-scrobble",
        "range-from-equals-to",
        "range-one-scrobble",
        "range-to-at-scrobble",
        "range-zero-scrobbles",
        "from-only",
    ] {
        assert_replays(&d, RECENT, variant, same).await;
    }
}

#[tokio::test]
async fn recent_tracks_pages_match_but_for_the_history_size() {
    let d = fixture_store().await;
    let total = ("total", "160643", "1000");
    let cases: [(&str, &str, &str); 12] = [
        ("default", "3213", "20"),
        ("limit-50", "3213", "20"),
        ("extended-0", "3213", "20"),
        ("extended-1", "3213", "20"),
        ("page-2", "3213", "20"),
        ("error-page-out-of-range", "3213", "20"),
        ("limit-1", "160643", "1000"),
        ("limit-200", "804", "5"),
        ("extended-1-limit-200", "804", "5"),
        ("limit-1000", "161", "1"),
        ("jsonp-callback", "80322", "500"),
        ("post-limit-2", "80322", "500"),
    ];
    for (variant, pages, ours) in cases {
        let edit = swap(&[("totalPages", pages, ours), total]);
        assert_replays(&d, RECENT, variant, edit).await;
    }
}

#[tokio::test]
async fn recent_tracks_errors() {
    let d = fixture_store().await;
    for variant in [
        "limit-0",
        "page-0",
        "error-missing-user",
        "error-unknown-user",
    ] {
        assert_replays(&d, RECENT, variant, same).await;
    }
}

#[tokio::test]
async fn seven_day_top_lists_match_exactly() {
    let d = fixture_store().await;
    for method in [
        "user.getTopArtists",
        "user.getTopAlbums",
        "user.getTopTracks",
    ] {
        for variant in ["period-7day", "period-7day-limit-1"] {
            assert_replays(&d, method, variant, same).await;
        }
    }
}

#[tokio::test]
async fn top_list_past_the_end_and_unknown_user() {
    let (store, user) = store().await;
    let artists = store.user(user).await.unwrap().unwrap().stats.artists;
    let d = dispatcher(store);
    let pages = artists.div_ceil(50).to_string();
    let artists = artists.to_string();
    let edit = swap(&[("totalPages", "81", &pages), ("total", "4046", &artists)]);
    let method = "user.getTopArtists";
    assert_replays(&d, method, "error-page-out-of-range", edit).await;
    assert_replays(&d, method, "error-unknown-user", same).await;
}

/// Album charts show the album's own artist MBID, which recent tracks do not carry; for this
/// album Last.fm has none although the artist has one. The store shows the artist's.
fn album_artist_mbid(body: &str) -> String {
    let cranberries = "The Cranberries";
    let mbid = "c98d40fd-f6cf-4b26-883e-eaa515ee2851";
    body.replace(
        &format!(r##"{{"mbid":"","#text":"{cranberries}"}},"mbid":"","url""##),
        &format!(r##"{{"mbid":"{mbid}","#text":"{cranberries}"}},"mbid":"","url""##),
    )
    .replace(
        &format!("<artist mbid=\"\">{cranberries}</artist>\n      <name>Everybody"),
        &format!("<artist mbid=\"{mbid}\">{cranberries}</artist>\n      <name>Everybody"),
    )
}

#[tokio::test]
async fn weekly_charts_match_exactly() {
    let d = fixture_store().await;
    for method in [
        "user.getWeeklyArtistChart",
        "user.getWeeklyAlbumChart",
        "user.getWeeklyTrackChart",
    ] {
        for variant in ["limit-1", "range-not-on-week-grid", "range-oldest"] {
            assert_replays(&d, method, variant, album_artist_mbid).await;
        }
    }
    // Last.fm's artist and album charts of these weeks count a few plays under a corrected artist
    // (`Berlioz` as `Hector Berlioz`) or an album artist (`Various Artists`) that recent tracks
    // do not show, so only the track charts can match whole.
    for variant in ["default", "range-latest"] {
        assert_replays(&d, "user.getWeeklyTrackChart", variant, same).await;
    }
    assert_replays(&d, "user.getWeeklyArtistChart", "error-unknown-user", same).await;
}

#[tokio::test]
async fn weekly_chart_list_matches_exactly() {
    let d = fixture_store().await;
    assert_replays(&d, "user.getWeeklyChartList", "default", same).await;
}

#[tokio::test]
async fn weekly_chart_list_sends_the_extra_cors_header_in_both_formats() {
    let d = fixture_store().await;
    for format in ["json", "xml"] {
        let params = support::recorded("user.getWeeklyChartList", "default", format).params;
        let reply = d.dispatch(&params).await;
        assert_eq!(reply.headers.len(), 1, "{format}");
    }
}

#[tokio::test]
async fn user_info_matches_but_for_the_counts() {
    let (store, user) = store().await;
    let stats = store.user(user).await.unwrap().unwrap().stats;
    let d = dispatcher(store);
    let counts = [
        stats.playcount.to_string(),
        stats.artists.to_string(),
        stats.albums.to_string(),
        stats.tracks.to_string(),
    ];
    let edit = swap(&[
        ("playcount", "160643", &counts[0]),
        ("artist_count", "4046", &counts[1]),
        ("album_count", "6198", &counts[2]),
        ("track_count", "15477", &counts[3]),
    ]);
    for variant in ["default", "user-uppercase", "jsonp-callback", "post"] {
        assert_replays(&d, "user.getInfo", variant, &edit).await;
    }
    for variant in ["error-missing-user", "error-unknown-user"] {
        assert_replays(&d, "user.getInfo", variant, same).await;
    }
}

#[tokio::test]
async fn loved_tracks_match_exactly() {
    let d = fixture_store().await;
    for variant in [
        "default",
        "limit-1",
        "limit-50",
        "limit-200",
        "page-2",
        "page-last",
        "error-page-out-of-range",
    ] {
        assert_replays(&d, "user.getLovedTracks", variant, same).await;
    }
}

async fn call(d: &Dispatcher, query: &[(&str, &str)]) -> String {
    let params = query
        .iter()
        .fold(lfm_core::Params::new().with("api_key", KEY), |p, (k, v)| {
            p.with(*k, *v)
        });
    String::from_utf8(d.dispatch(&params).await.body.to_vec()).unwrap()
}

#[tokio::test]
async fn library_echoes_the_requested_name_and_collapses_one_artist() {
    let d = fixture_store().await;
    let base = [("method", "library.getArtists"), ("user", "SYNTIIIX")];
    let one = call(&d, &[base[0], base[1], ("format", "json"), ("limit", "1")]).await;
    assert!(one.starts_with(r#"{"artists":{"artist":{"tagcount":"0","image":[{"size":"small""#));
    assert!(
        one.contains(r#""@attr":{"user":"SYNTIIIX","totalPages":"#),
        "{one}"
    );
    let two = call(&d, &[base[0], base[1], ("format", "json"), ("limit", "2")]).await;
    assert!(
        two.starts_with(r#"{"artists":{"artist":[{"tagcount""#),
        "{two}"
    );
    let xml = call(&d, &[base[0], base[1], ("limit", "1")]).await;
    assert!(
        xml.contains("  <artists user=\"SYNTIIIX\" page=\"1\" perPage=\"1\""),
        "{xml}"
    );
    assert!(xml.contains("      <tagcount>0</tagcount>\n"), "{xml}");
    let past = call(&d, &[base[0], base[1], ("page", "9999")]).await;
    assert!(past.contains("perPage=\"50\" totalPages="), "{past}");
    assert!(past.ends_with(" />\n</lfm>\n"), "{past}");
}

#[tokio::test]
async fn library_lists_the_same_artists_as_overall_top_artists() {
    let d = fixture_store().await;
    let top = call(
        &d,
        &[("method", "user.getTopArtists"), ("user", "syntiiix")],
    )
    .await;
    let library = call(
        &d,
        &[("method", "library.getArtists"), ("user", "syntiiix")],
    )
    .await;
    let names = |body: &str| -> Vec<String> {
        body.lines()
            .filter(|line| line.starts_with("      <name>"))
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(names(&top).len(), 50);
    assert_eq!(names(&top), names(&library));
}

#[tokio::test]
async fn cached_reply_is_dropped_when_the_user_changes() {
    let (store, user) = store().await;
    let d = dispatcher(store.clone());
    let query = [("method", "user.getLovedTracks"), ("user", "syntiiix")];
    let before = call(&d, &query).await;
    assert_eq!(call(&d, &query).await, before);
    store
        .unlove(user, "iluvmsj", "Love is suicidal ft. d4vd & SSJ Twiin")
        .await
        .unwrap();
    let after = call(&d, &query).await;
    assert!(
        before.contains("total=\"34\"") && after.contains("total=\"33\""),
        "{after}"
    );
}
