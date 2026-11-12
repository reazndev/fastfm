//! Ignore rules, stale-rule detection and the baseline, against the replay server.
#![allow(
    clippy::unwrap_used,
    reason = "helpers here fail the test the same way the tests do"
)]

mod support;

use std::collections::BTreeSet;

use lfm_conformance::baseline;
use lfm_conformance::diff::{Category, Difference};
use lfm_conformance::ignore::{Rules, glob};
use lfm_conformance::judge::Outcome;
use support::{cases, on, replace_body, result, run_with, verbatim};

const INFO_JSON: &str = "user.getInfo/default.json";

fn bump_playcount(reply: &mut support::Reply) {
    replace_body(reply, r#""playcount":"160643""#, r#""playcount":"160644""#);
}

#[test]
fn glob_star_stops_at_dots_and_double_star_does_not() {
    assert!(glob("artist.*", "artist.getInfo"));
    assert!(glob("artist.stats.*", "artist.stats.listeners"));
    assert!(!glob("artist.*", "artist.stats.listeners"));
    assert!(glob("artist.**", "artist.stats.listeners"));
    assert!(glob(
        "toptracks.track[*].listeners",
        "toptracks.track[12].listeners"
    ));
    assert!(!glob(
        "toptracks.track[*].listeners",
        "toptracks.track[12].playcount"
    ));
    assert!(glob("**.listeners", "similarartists.artist[3].listeners"));
}

#[test]
fn every_rule_needs_a_reason() {
    let err = Rules::parse("[[rule]]\nmethod = \"x\"\npath = \"y\"\nreason = \"  \"\n");
    assert!(err.unwrap_err().contains("reason"));
    let err = Rules::parse("[[rule]]\nmethod = \"x\"\npath = \"y\"\n");
    assert!(err.is_err());
}

#[test]
fn unknown_categories_and_fields_are_rejected() {
    let bad_category =
        "[[rule]]\nmethod = \"x\"\npath = \"y\"\nreason = \"r\"\ncategories = [\"vibes\"]\n";
    assert!(Rules::parse(bad_category).is_err());
    let bad_field = "[[rule]]\nmethod = \"x\"\npath = \"y\"\nreason = \"r\"\npth = \"z\"\n";
    assert!(Rules::parse(bad_field).is_err());
}

#[tokio::test]
async fn a_covering_rule_turns_a_failure_into_an_ignored_pass() {
    let rules = Rules::parse(
        r#"
        [[rule]]
        method = "user.getInfo"
        path = "user.playcount"
        reason = "test: the play count moves"
        "#,
    )
    .unwrap();
    let report = run_with(cases("user.getInfo"), &rules, on(INFO_JSON, bump_playcount)).await;
    let r = result(&report, INFO_JSON);
    assert_eq!(r.outcome, Outcome::Ignored);
    assert!(r.failing.is_empty());
    assert_eq!(r.ignored.get(&Category::Value), Some(&1));
    assert_eq!(r.ignored_differences[0].rule, 1);
    assert_eq!(report.overall.ignored, 1);
    assert_eq!(report.rules_used[0].hits, 1);
    assert_eq!(report.stale_rules, Vec::new());
    assert!(report.passed());
}

#[tokio::test]
async fn a_rule_only_covers_its_categories() {
    let rules = Rules::parse(
        r#"
        [[rule]]
        method = "user.getInfo"
        path = "user.playcount"
        reason = "test: values only, so a type change still fails"
        "#,
    )
    .unwrap();
    let mutation = on(INFO_JSON, |reply| {
        replace_body(reply, r#""playcount":"160643""#, r#""playcount":160643"#);
    });
    let report = run_with(cases("user.getInfo"), &rules, mutation).await;
    let r = result(&report, INFO_JSON);
    assert_eq!(r.outcome, Outcome::Failed);
    assert_eq!(r.failing.get(&Category::Type), Some(&1));
}

#[tokio::test]
async fn rules_that_match_nothing_are_stale_or_not_exercised() {
    let rules = Rules::parse(
        r#"
        [[rule]]
        method = "user.getInfo"
        path = "user.nothing"
        reason = "test: stale"

        [[rule]]
        method = "artist.getInfo"
        path = "artist.stats.listeners"
        reason = "test: not in this run"
        "#,
    )
    .unwrap();
    let report = run_with(cases("user.getInfo"), &rules, verbatim()).await;
    let stale: Vec<(usize, usize, usize)> = report
        .stale_rules
        .iter()
        .map(|s| (s.rule.id, s.cases, s.failing_cases))
        .collect();
    assert_eq!(stale, vec![(1, 22, 0)]);
    let not_exercised: Vec<usize> = report.rules_not_exercised.iter().map(|r| r.id).collect();
    assert_eq!(not_exercised, vec![2]);
    assert!(report.summary().contains("stale ignore rule 1"));
}

#[tokio::test]
async fn the_baseline_fails_only_on_regressions() {
    let dir = std::env::temp_dir().join(format!("lfm-conformance-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("expected.txt");

    let report = run_with(cases("user.getInfo"), &Rules::default(), verbatim()).await;
    baseline::write(&file, &report.cases).unwrap();
    let listed = baseline::read(&file).unwrap();
    assert_eq!(listed.len(), report.cases.len());
    assert!(listed.contains(INFO_JSON));

    let mut broken = run_with(
        cases("user.getInfo"),
        &Rules::default(),
        on(INFO_JSON, bump_playcount),
    )
    .await;
    broken.baseline = Some(baseline::Check::new(&file, &listed, &broken.cases));
    assert!(!broken.passed());
    assert_eq!(
        broken.baseline.as_ref().unwrap().regressions,
        vec![INFO_JSON.to_owned()]
    );

    let partial: BTreeSet<String> = listed
        .iter()
        .filter(|id| *id != INFO_JSON)
        .cloned()
        .collect();
    let mut improved = run_with(cases("user.getInfo"), &Rules::default(), verbatim()).await;
    improved.baseline = Some(baseline::Check::new(&file, &partial, &improved.cases));
    assert!(improved.passed());
    assert_eq!(
        improved.baseline.as_ref().unwrap().new_passes,
        vec![INFO_JSON.to_owned()]
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

const ALBUM_A: &str = "e604dee6-0c2d-4f31-a8f8-291d13578d4f";
const ALBUM_B: &str = "d0874319-5fdd-4c32-ae82-9eab28ca95d5";
const TRACK: &str = "7235d173-cacd-4a9c-ba67-dbbf17d4753c";

/// Whether `conformance/ignore.toml` covers a value difference: (method, path, expected, actual).
fn committed_rules_cover((method, path, expected, actual): (&str, &str, &str, &str)) -> bool {
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../conformance/ignore.toml");
    let rules = Rules::load(file.as_ref()).unwrap();
    let mut case = support::cases("user.getInfo/default.json").remove(0);
    method.clone_into(&mut case.method);
    let difference = Difference {
        category: Category::Value,
        path: path.to_owned(),
        expected: expected.to_owned(),
        actual: actual.to_owned(),
    };
    rules.matching(&case, &difference).is_some()
}

#[test]
fn committed_rules_cover_what_drifted_live() {
    for drifted in [
        (
            "artist.getInfo",
            "artist.stats.listeners",
            "2645722",
            "2646393",
        ),
        (
            "chart.getTopArtists",
            "artists.artist[0].playcount",
            "1",
            "2",
        ),
        ("auth.getToken", "token", "abc", "def"),
        (
            "user.getRecentTracks",
            "recenttracks.track[502].album.mbid",
            ALBUM_A,
            ALBUM_B,
        ),
        (
            "user.getRecentTracks",
            "recenttracks.track.album.@mbid",
            ALBUM_B,
            ALBUM_A,
        ),
        (
            "user.getTopAlbums",
            "topalbums.album[40].mbid",
            ALBUM_B,
            ALBUM_A,
        ),
        ("user.getTopTracks", "toptracks.track[49].mbid", TRACK, ""),
        (
            "user.getWeeklyTrackChart",
            "weeklytrackchart.track[1].mbid",
            TRACK,
            "",
        ),
    ] {
        assert!(committed_rules_cover(drifted), "{drifted:?}");
    }
}

#[test]
fn committed_rules_cover_nothing_next_to_it() {
    let artist = "da8fb855-1a7f-4648-8d97-dd832328b92b";
    for real_bug in [
        ("artist.getInfo", "artist.stats.userplaycount", "1", "2"),
        (
            "user.getRecentTracks",
            "recenttracks.track[0].name",
            "a",
            "b",
        ),
        (
            "user.getRecentTracks",
            "recenttracks.track[0].artist.mbid",
            artist,
            "",
        ),
        ("user.getTopAlbums", "topalbums.album[40].mbid", ALBUM_A, ""),
        ("user.getInfo", "user.playcount", "1", "2"),
    ] {
        assert!(!committed_rules_cover(real_bug), "{real_bug:?}");
    }
}
