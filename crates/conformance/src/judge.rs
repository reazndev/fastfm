//! Turns one case's differences into a verdict, splitting off what the ignore rules cover.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::diff::{Category, Difference};
use crate::fixture::{Case, Format};
use crate::ignore::Rules;

/// At most this many differences of one category are listed per case.
const SHOWN_PER_CATEGORY: usize = 5;
/// At most this many differences are listed per case and list.
const SHOWN_PER_CASE: usize = 40;

/// How a case came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Status, content type, CORS headers and body bytes all match.
    Exact,
    /// Every difference is covered by an ignore rule.
    Ignored,
    /// At least one difference is not covered.
    Failed,
}

/// A difference an ignore rule covered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IgnoredDifference {
    /// The rule's id.
    pub rule: usize,
    /// The difference.
    #[serde(flatten)]
    pub difference: Difference,
}

/// The verdict for one case. Counts are complete; the lists are capped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaseResult {
    /// `method/variant.format`.
    pub id: String,
    /// Last.fm method.
    pub method: String,
    /// Fixture variant.
    pub variant: String,
    /// Response format.
    pub format: Format,
    /// The verdict.
    pub outcome: Outcome,
    /// Differences not covered by a rule, per category.
    pub failing: BTreeMap<Category, usize>,
    /// Differences covered by a rule, per category.
    pub ignored: BTreeMap<Category, usize>,
    /// The first uncovered differences, a few per category.
    pub differences: Vec<Difference>,
    /// The first covered differences, a few per category.
    pub ignored_differences: Vec<IgnoredDifference>,
    /// Differences counted but not listed, both lists together.
    pub not_listed: usize,
    /// Covered differences per rule id.
    #[serde(skip)]
    pub rule_hits: BTreeMap<usize, usize>,
}

/// Judges one case.
#[must_use]
pub fn judge(case: &Case, differences: Vec<Difference>, rules: &Rules) -> CaseResult {
    let (ignored, failing): (Vec<_>, Vec<_>) = differences
        .into_iter()
        .map(|d| (rules.matching(case, &d).map(|r| r.id), d))
        .partition(|(rule, _)| rule.is_some());
    let ignored: Vec<IgnoredDifference> = ignored
        .into_iter()
        .filter_map(|(rule, difference)| {
            Some(IgnoredDifference {
                rule: rule?,
                difference,
            })
        })
        .collect();
    let failing: Vec<Difference> = failing.into_iter().map(|(_, d)| d).collect();
    let outcome = match (failing.is_empty(), ignored.is_empty()) {
        (false, _) => Outcome::Failed,
        (true, false) => Outcome::Ignored,
        (true, true) => Outcome::Exact,
    };
    let shown_failing = cap(&failing, |d| d.category);
    let shown_ignored = cap(&ignored, |i| i.difference.category);
    let not_listed = failing.len() + ignored.len() - shown_failing.len() - shown_ignored.len();
    CaseResult {
        id: case.id(),
        method: case.method.clone(),
        variant: case.variant.clone(),
        format: case.format,
        outcome,
        failing: count(failing.iter().map(|d| d.category)),
        ignored: count(ignored.iter().map(|i| i.difference.category)),
        rule_hits: count(ignored.iter().map(|i| i.rule)),
        differences: shown_failing,
        ignored_differences: shown_ignored,
        not_listed,
    }
}

fn count<K: Ord>(keys: impl Iterator<Item = K>) -> BTreeMap<K, usize> {
    keys.fold(BTreeMap::new(), |mut counts, key| {
        *counts.entry(key).or_insert(0) += 1;
        counts
    })
}

/// The first few items of each category, in their original order, up to the case limit.
fn cap<T: Clone>(items: &[T], category: impl Fn(&T) -> Category) -> Vec<T> {
    let mut per_category: BTreeMap<Category, usize> = BTreeMap::new();
    items
        .iter()
        .filter(|item| {
            let seen = per_category.entry(category(item)).or_insert(0);
            *seen += 1;
            *seen <= SHOWN_PER_CATEGORY
        })
        .take(SHOWN_PER_CASE)
        .cloned()
        .collect()
}
