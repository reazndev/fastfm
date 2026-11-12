//! The run's machine-readable report and its short human summary.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

use crate::baseline::Check;
use crate::diff::Category;
use crate::fixture::Case;
use crate::ignore::{Rule, Rules};
use crate::judge::{CaseResult, Outcome};

/// Passing and failing counts for a group of cases.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Score {
    /// Cases run.
    pub total: usize,
    /// Cases matching byte for byte.
    pub exact: usize,
    /// Cases passing once ignore rules apply.
    pub ignored: usize,
    /// Cases with uncovered differences.
    pub failed: usize,
    /// `(exact + ignored) / total`, in percent.
    pub pass_percent: f64,
    /// `exact / total`, in percent.
    pub exact_percent: f64,
}

impl Score {
    fn of<'a>(results: impl IntoIterator<Item = &'a CaseResult>) -> Self {
        let tally = results.into_iter().fold(Self::default(), |mut s, r| {
            s.total += 1;
            match r.outcome {
                Outcome::Exact => s.exact += 1,
                Outcome::Ignored => s.ignored += 1,
                Outcome::Failed => s.failed += 1,
            }
            s
        });
        Self {
            pass_percent: percent(tally.exact + tally.ignored, tally.total),
            exact_percent: percent(tally.exact, tally.total),
            ..tally
        }
    }
}

/// The score of one method.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MethodScore {
    /// Last.fm method.
    pub method: String,
    /// Its score.
    #[serde(flatten)]
    pub score: Score,
}

/// Differences of one category over the whole run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct CategoryCount {
    /// Not covered by a rule.
    pub failing: usize,
    /// Covered by a rule.
    pub ignored: usize,
}

/// A rule and how many differences it covered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleUse {
    /// The rule.
    #[serde(flatten)]
    pub rule: Rule,
    /// Differences it covered in this run.
    pub hits: usize,
}

/// A rule that applies to cases in this run but covered nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StaleRule {
    /// The rule.
    #[serde(flatten)]
    pub rule: Rule,
    /// Cases the rule applies to.
    pub cases: usize,
    /// How many of them failed for other reasons, which may hide what the rule is for.
    pub failing_cases: usize,
}

/// Everything a run found.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// The server under test.
    pub base_url: String,
    /// All cases.
    pub overall: Score,
    /// Per method, by name.
    pub methods: Vec<MethodScore>,
    /// Per category.
    pub categories: BTreeMap<Category, CategoryCount>,
    /// Rules that covered something.
    pub rules_used: Vec<RuleUse>,
    /// Rules that apply to cases in this run but covered nothing.
    pub stale_rules: Vec<StaleRule>,
    /// Rules for cases this run did not include (`--only`, `--format`).
    pub rules_not_exercised: Vec<Rule>,
    /// Regressions against the `--expect` baseline.
    pub baseline: Option<Check>,
    /// Every case, by id.
    pub cases: Vec<CaseResult>,
}

impl Report {
    /// Builds the report from the cases run and their verdicts.
    #[must_use]
    pub fn build(base_url: &str, cases: &[Case], results: Vec<CaseResult>, rules: &Rules) -> Self {
        let by_method = results.iter().fold(BTreeMap::new(), |mut map, r| {
            map.entry(r.method.clone()).or_insert_with(Vec::new).push(r);
            map
        });
        let methods = by_method
            .into_iter()
            .map(|(method, rs)| MethodScore {
                method,
                score: Score::of(rs),
            })
            .collect();
        let usage = RuleUsage::of(rules, cases, &results);
        Self {
            base_url: base_url.to_owned(),
            overall: Score::of(&results),
            methods,
            categories: category_counts(&results),
            rules_used: usage.used,
            stale_rules: usage.stale,
            rules_not_exercised: usage.not_exercised,
            baseline: None,
            cases: results,
        }
    }

    /// Whether the run succeeds: no regressions against the baseline when one is given,
    /// otherwise no failing case.
    #[must_use]
    pub fn passed(&self) -> bool {
        match &self.baseline {
            Some(check) => check.regressions.is_empty(),
            None => self.overall.failed == 0,
        }
    }

    /// A short summary for a terminal.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut out = String::new();
        let o = &self.overall;
        let _ = writeln!(out, "conformance against {}", self.base_url);
        let _ = writeln!(
            out,
            "  {} cases: {} exact, {} pass with ignores, {} failed ({:.1}% pass, {:.1}% exact)",
            o.total, o.exact, o.ignored, o.failed, o.pass_percent, o.exact_percent
        );
        let _ = writeln!(
            out,
            "\nmethod                         exact ignored failed   pass"
        );
        for m in &self.methods {
            let s = &m.score;
            let _ = writeln!(
                out,
                "  {:<28} {:>5} {:>7} {:>6} {:>5.1}%",
                m.method, s.exact, s.ignored, s.failed, s.pass_percent
            );
        }
        let categories: Vec<String> = self
            .categories
            .iter()
            .map(|(c, n)| format!("{} {}/{}", c.as_str(), n.failing, n.ignored))
            .collect();
        let _ = writeln!(
            out,
            "\ndifferences (failing/ignored): {}",
            categories.join(", ")
        );
        for stale in &self.stale_rules {
            let (rule, cases) = (&stale.rule, stale.cases);
            let verdict = match stale.failing_cases {
                0 => format!("all {cases} of its cases pass; remove or narrow it"),
                n => format!("{n} of its {cases} cases fail for other reasons"),
            };
            let _ = writeln!(
                out,
                "stale ignore rule {} ({} {}): matched nothing; {verdict}",
                rule.id, rule.method, rule.path
            );
        }
        if let Some(check) = &self.baseline {
            out.push_str(&check.summary());
        }
        out
    }
}

/// Rules split by what they did in a run.
struct RuleUsage {
    used: Vec<RuleUse>,
    stale: Vec<StaleRule>,
    not_exercised: Vec<Rule>,
}

impl RuleUsage {
    fn of(rules: &Rules, cases: &[Case], results: &[CaseResult]) -> Self {
        let hits = results.iter().flat_map(|r| &r.rule_hits).fold(
            BTreeMap::new(),
            |mut map, (rule, n)| {
                *map.entry(*rule).or_insert(0) += n;
                map
            },
        );
        let failed: BTreeMap<&str, bool> = results
            .iter()
            .map(|r| (r.id.as_str(), r.outcome == Outcome::Failed))
            .collect();
        let mut usage = Self {
            used: Vec::new(),
            stale: Vec::new(),
            not_exercised: Vec::new(),
        };
        for rule in rules.rules() {
            let selected: Vec<String> = cases
                .iter()
                .filter(|c| rule.selects(c))
                .map(Case::id)
                .collect();
            if let Some(&hits) = hits.get(&rule.id) {
                usage.used.push(RuleUse {
                    rule: rule.clone(),
                    hits,
                });
            } else if selected.is_empty() {
                usage.not_exercised.push(rule.clone());
            } else {
                usage.stale.push(StaleRule {
                    rule: rule.clone(),
                    cases: selected.len(),
                    failing_cases: selected
                        .iter()
                        .filter(|id| failed.get(id.as_str()).copied().unwrap_or(false))
                        .count(),
                });
            }
        }
        usage
    }
}

fn category_counts(results: &[CaseResult]) -> BTreeMap<Category, CategoryCount> {
    let mut counts: BTreeMap<Category, CategoryCount> = BTreeMap::new();
    for result in results {
        for (category, n) in &result.failing {
            counts.entry(*category).or_default().failing += n;
        }
        for (category, n) in &result.ignored {
            counts.entry(*category).or_default().ignored += n;
        }
    }
    counts
}

fn percent(part: usize, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let as_f64 = |n: usize| f64::from(u32::try_from(n).unwrap_or(u32::MAX));
    (as_f64(part) * 1000.0 / as_f64(total)).round() / 10.0
}
