//! The harness against an in-process replay of the fixtures: verbatim must score 100% with no
//! ignores, and each kind of mutation must be caught under the right category and path.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "helpers here fail the test the same way the tests do"
)]

mod support;

use std::collections::BTreeSet;

use lfm_conformance::diff::Category;
use lfm_conformance::ignore::Rules;
use lfm_conformance::judge::{CaseResult, Outcome};
use support::{Reply, cases, on, replace_body, result, run_with, verbatim};

const INFO_JSON: &str = "user.getInfo/default.json";
const INFO_XML: &str = "user.getInfo/default.xml";

#[tokio::test]
async fn verbatim_replay_is_exact_for_every_fixture() {
    let all = support::all_cases();
    assert_eq!(all.len(), 420, "210 fixtures in two formats");
    let report = run_with(all, &Rules::default(), verbatim()).await;
    let failing: Vec<_> = report
        .cases
        .iter()
        .filter(|r| r.outcome != Outcome::Exact)
        .map(|r| (&r.id, &r.differences))
        .collect();
    assert!(failing.is_empty(), "{failing:#?}");
    assert_eq!(report.overall.exact, 420);
    assert!((report.overall.pass_percent - 100.0).abs() < f64::EPSILON);
    assert!(report.categories.is_empty());
    assert!(report.passed());
}

#[tokio::test]
async fn verbatim_replay_needs_none_of_the_committed_ignore_rules() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../conformance/ignore.toml");
    let rules = Rules::load(path.as_ref()).expect("committed ignore file is valid");
    let report = run_with(support::all_cases(), &rules, verbatim()).await;
    assert_eq!(report.overall.exact, report.overall.total);
    assert_eq!(report.rules_used, Vec::new());
}

/// Runs the method of `id` with one mutation on `id`, checks that every other case stays exact,
/// and returns the mutated case's result.
async fn mutated(
    id: &'static str,
    change: impl Fn(&mut Reply) + Send + Sync + 'static,
) -> CaseResult {
    let method = id.split('/').next().expect("method");
    let report = run_with(cases(method), &Rules::default(), on(id, change)).await;
    let others: Vec<_> = report
        .cases
        .iter()
        .filter(|r| r.id != id && r.outcome != Outcome::Exact)
        .map(|r| &r.id)
        .collect();
    assert!(others.is_empty(), "unmutated cases failed: {others:?}");
    assert!(!report.passed());
    let found = result(&report, id).clone();
    assert_eq!(found.outcome, Outcome::Failed);
    found
}

fn categories(result: &CaseResult) -> BTreeSet<Category> {
    result.failing.keys().copied().collect()
}

fn only(result: &CaseResult, category: Category, path: &str) {
    assert_eq!(
        categories(result),
        BTreeSet::from([category]),
        "{result:#?}"
    );
    assert!(
        result.differences.iter().any(|d| d.path == path),
        "no {category:?} at {path}: {:#?}",
        result.differences
    );
}

#[tokio::test]
async fn reordered_json_keys_are_an_order_difference() {
    let r = mutated(INFO_JSON, |reply| {
        replace_body(
            reply,
            r#""playcount":"160643","playlists":"0""#,
            r#""playlists":"0","playcount":"160643""#,
        );
    })
    .await;
    only(&r, Category::Order, "user");
}

#[tokio::test]
async fn string_turned_number_is_a_type_difference() {
    let r = mutated(INFO_JSON, |reply| {
        replace_body(reply, r#""playcount":"160643""#, r#""playcount":160643"#);
    })
    .await;
    only(&r, Category::Type, "user.playcount");
}

#[tokio::test]
async fn changed_json_value_is_a_value_difference() {
    let r = mutated(INFO_JSON, |reply| {
        replace_body(reply, r#""playcount":"160643""#, r#""playcount":"160644""#);
    })
    .await;
    only(&r, Category::Value, "user.playcount");
    let d = &r.differences[0];
    assert_eq!(
        (d.expected.as_str(), d.actual.as_str()),
        ("160643", "160644")
    );
}

#[tokio::test]
async fn dropped_json_key_is_missing() {
    let r = mutated(INFO_JSON, |reply| {
        replace_body(reply, r#""playlists":"0","#, "");
    })
    .await;
    only(&r, Category::Missing, "user.playlists");
}

#[tokio::test]
async fn array_turned_object_is_a_shape_difference() {
    let r = mutated("user.getRecentTracks/limit-1.json", |reply| {
        replace_body(reply, r#""track":[{"#, r#""track":{"#);
        replace_body(reply, r#"}],"@attr""#, r#"},"@attr""#);
    })
    .await;
    only(&r, Category::Shape, "recenttracks.track");
}

#[tokio::test]
async fn unescaped_slashes_are_an_escape_difference() {
    let r = mutated(INFO_JSON, |reply| {
        replace_body(
            reply,
            r"https:\/\/www.last.fm\/user\/",
            "https://www.last.fm/user/",
        );
    })
    .await;
    only(&r, Category::Escape, "user.url");
}

#[tokio::test]
async fn pretty_printed_json_is_a_whitespace_difference() {
    let r = mutated(INFO_JSON, |reply| {
        replace_body(reply, r#"{"user":{"#, "{\n  \"user\": {");
    })
    .await;
    only(&r, Category::Whitespace, "$");
}

#[tokio::test]
async fn changed_status_is_a_status_difference() {
    let r = mutated(INFO_XML, |reply| reply.status = 500).await;
    only(&r, Category::Status, "status");
}

#[tokio::test]
async fn dropped_cors_header_is_a_header_difference() {
    let r = mutated(INFO_JSON, |reply| {
        reply
            .headers
            .retain(|(k, _)| k != "access-control-allow-origin");
    })
    .await;
    only(&r, Category::Header, "header.access-control-allow-origin");
}

#[tokio::test]
async fn changed_content_type_is_a_content_type_difference() {
    let r = mutated(INFO_JSON, |reply| {
        for (name, value) in &mut reply.headers {
            if name == "content-type" {
                *value = "application/json; charset=UTF-8".to_owned();
            }
        }
    })
    .await;
    only(&r, Category::ContentType, "header.content-type");
}

#[tokio::test]
async fn changed_indent_is_a_whitespace_difference_only() {
    let r = mutated(INFO_XML, |reply| replace_body(reply, "\n  ", "\n    ")).await;
    assert_eq!(categories(&r), BTreeSet::from([Category::Whitespace]));
    assert!(r.differences.iter().any(|d| d.path == "user"));
}

#[tokio::test]
async fn changed_declaration_is_a_declaration_difference() {
    let r = mutated(INFO_XML, |reply| {
        replace_body(reply, r#"encoding="UTF-8"?>"#, r#"encoding="UTF-8" ?>"#);
    })
    .await;
    only(&r, Category::Declaration, "$");
}

#[tokio::test]
async fn swapped_xml_elements_are_an_order_difference() {
    let r = mutated(INFO_XML, |reply| {
        replace_body(
            reply,
            "<name>Syntiiix</name>\n    <realname>Reazn</realname>",
            "<realname>Reazn</realname>\n    <name>Syntiiix</name>",
        );
    })
    .await;
    only(&r, Category::Order, "user");
}

#[tokio::test]
async fn changed_xml_attribute_is_a_value_difference() {
    let r = mutated(INFO_XML, |reply| {
        replace_body(reply, r#"<image size="small">"#, r#"<image size="tiny">"#);
    })
    .await;
    only(&r, Category::Value, "user.image[0].@size");
}

#[tokio::test]
async fn self_closed_empty_element_is_a_whitespace_difference() {
    let r = mutated("user.getRecentTracks/limit-1.xml", |reply| {
        replace_body(reply, "<mbid></mbid>", "<mbid/>");
    })
    .await;
    only(&r, Category::Whitespace, "recenttracks.track.mbid");
}

#[tokio::test]
async fn renamed_jsonp_callback_is_a_wrapper_difference() {
    let r = mutated("user.getInfo/jsonp-callback.json", |reply| {
        replace_body(reply, "handleResponse(", "other(");
    })
    .await;
    only(&r, Category::Wrapper, "$");
}

#[tokio::test]
async fn deep_paths_name_the_exact_item_in_both_formats() {
    for id in [
        "user.getRecentTracks/default.json",
        "user.getRecentTracks/default.xml",
    ] {
        let r = mutated(id, |reply| {
            let text = String::from_utf8(reply.body.clone()).unwrap();
            reply.body = change_fourth_track_third_image(&text).into_bytes();
        })
        .await;
        only(&r, Category::Value, "recenttracks.track[3].image[2].#text");
    }
}

/// Appends `-changed` to the URL of image 2 of track 3 (from 0), in either format.
fn change_fourth_track_third_image(text: &str) -> String {
    let (track_marker, image_marker, end) = if text.starts_with('{') {
        (r#"{"artist":"#, r#""size":"#, r#""}"#)
    } else {
        ("<track", "<image size=", "</image>")
    };
    let track = text.match_indices(track_marker).nth(3).unwrap().0;
    let image = track + text[track..].match_indices(image_marker).nth(2).unwrap().0;
    let close = image + text[image..].find(end).unwrap();
    format!("{}-changed{}", &text[..close], &text[close..])
}
