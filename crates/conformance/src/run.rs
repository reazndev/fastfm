//! Sends every case with a small concurrency limit and an optional rate limit.

use std::time::Duration;

use futures::StreamExt as _;
use tokio::time::Instant;

use crate::client::Target;
use crate::diff::{Category, Difference, compare};
use crate::fixture::Case;
use crate::ignore::Rules;
use crate::judge::{CaseResult, judge};

/// How fast to send.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pace {
    /// Requests in flight at once.
    pub concurrency: usize,
    /// At most this many requests per second, or unlimited.
    pub requests_per_second: Option<f64>,
}

impl Default for Pace {
    fn default() -> Self {
        Self {
            concurrency: 8,
            requests_per_second: None,
        }
    }
}

impl Pace {
    /// When the request at `index` may start, counted from the start of the run.
    fn slot(&self, index: usize) -> Duration {
        let interval = self
            .requests_per_second
            .filter(|rps| *rps > 0.0)
            .map_or(Duration::ZERO, |rps| Duration::from_secs_f64(1.0 / rps));
        interval
            .checked_mul(u32::try_from(index).unwrap_or(u32::MAX))
            .unwrap_or(Duration::MAX)
    }
}

/// Runs every case against the target and returns the verdicts sorted by case id.
pub async fn run(target: &Target, cases: &[Case], rules: &Rules, pace: Pace) -> Vec<CaseResult> {
    let start = Instant::now();
    let mut results: Vec<CaseResult> = futures::stream::iter(cases.iter().enumerate())
        .map(|(index, case)| check(target, case, rules, start + pace.slot(index)))
        .buffer_unordered(pace.concurrency.max(1))
        .collect()
        .await;
    results.sort_by(|a, b| a.id.cmp(&b.id));
    results
}

async fn check(target: &Target, case: &Case, rules: &Rules, at: Instant) -> CaseResult {
    tokio::time::sleep_until(at).await;
    let differences = match target.send(case).await {
        Ok(received) => compare(case, &received),
        // Without the URL: it holds the key.
        Err(err) => vec![Difference::new(
            Category::Transport,
            "request",
            "a response",
            err.without_url().to_string(),
        )],
    };
    judge(case, differences, rules)
}
