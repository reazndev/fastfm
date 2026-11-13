//! Timing one scenario on both sides: Last.fm first, then this mirror uncached, then cached.

use std::sync::PoisonError;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use lfm_upstream::Timed;
use serde::Serialize;

use super::{Demo, Scenario};
use crate::error::WebError;

/// Least time between two requests to Last.fm: four per second at most.
const LASTFM_SPACING: Duration = Duration::from_millis(250);
/// After this long without a request, the connections are warmed up again before timing.
const WARM_FOR: Duration = Duration::from_secs(20);
/// The header that says where the mirror's answer came from.
const SOURCE: &str = "x-mirror-source";

/// The timings of one scenario.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Outcome {
    id: &'static str,
    requests: usize,
    lastfm: Side,
    /// `None` when the mirror did not answer from local data.
    mirror: Option<MirrorSide>,
    /// `local`, or where the mirror's answer came from instead.
    mirror_source: String,
    /// Whether every Last.fm body equals the mirror's first body byte for byte.
    identical: bool,
}

/// Last.fm's side: the total over every request.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Side {
    ms: f64,
    bytes: usize,
    /// Whether every request got HTTP 200.
    ok: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MirrorSide {
    first_ms: f64,
    cached_ms: f64,
    bytes: usize,
    ok: bool,
}

impl Demo {
    /// Times `scenario` for `user`. One scenario runs at a time across the instance, and
    /// requests to Last.fm are spaced by [`LASTFM_SPACING`]; the waits are not timed.
    pub(crate) async fn measure(
        &self,
        user: &str,
        scenario: &Scenario,
    ) -> Result<Outcome, WebError> {
        let _turn = self
            .turn
            .acquire()
            .await
            .map_err(|e| WebError::internal(&e))?;
        self.warm_up(user).await?;
        let mut lastfm = Vec::new();
        for params in &scenario.requests {
            lastfm.push(self.lastfm(params).await?);
        }
        self.config.cache.invalidate_user(user);
        let mut first = Vec::new();
        for params in &scenario.requests {
            let answer = self.mirror(params).await?;
            let local = is_local(&answer);
            first.push(answer);
            if !local {
                return Ok(outcome(scenario, &lastfm, &first, None));
            }
        }
        let mut cached = Vec::new();
        for params in &scenario.requests {
            cached.push(self.mirror(params).await?);
        }
        Ok(outcome(scenario, &lastfm, &first, Some(&cached)))
    }

    /// Opens the connections to both sides with an untimed request, unless they were used
    /// recently, so TLS and TCP setup are not counted.
    async fn warm_up(&self, user: &str) -> Result<(), WebError> {
        let recent = self
            .lastfm_sent()
            .is_some_and(|sent| sent.elapsed() < WARM_FOR);
        if recent {
            return Ok(());
        }
        let params = [
            ("method", "user.getWeeklyChartList".to_owned()),
            ("user", user.to_owned()),
        ];
        self.lastfm(&params).await?;
        self.mirror(&params).await?;
        Ok(())
    }

    pub(super) async fn lastfm(
        &self,
        params: &[(&'static str, String)],
    ) -> Result<Timed, WebError> {
        if let Some(wait) = self
            .lastfm_sent()
            .and_then(|sent| LASTFM_SPACING.checked_sub(sent.elapsed()))
        {
            tokio::time::sleep(wait).await;
        }
        *self
            .lastfm_sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
        let key = self.config.lastfm_key.expose();
        self.send(&self.config.lastfm_url, params, key).await
    }

    fn lastfm_sent(&self) -> Option<Instant> {
        *self
            .lastfm_sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) async fn mirror(
        &self,
        params: &[(&'static str, String)],
    ) -> Result<Timed, WebError> {
        let key = self.config.mirror_key.as_str();
        self.send(&self.config.mirror_url, params, key).await
    }

    async fn send(
        &self,
        url: &str,
        params: &[(&'static str, String)],
        key: &str,
    ) -> Result<Timed, WebError> {
        let query: Vec<(&str, &str)> = params
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .chain([("api_key", key), ("format", "json")])
            .collect();
        self.client.get(url, &query).await.map_err(|error| {
            tracing::warn!(%error, "timed demo request failed");
            WebError::new(
                StatusCode::BAD_GATEWAY,
                "demo_request_failed",
                "A timed request did not complete. Try again.",
            )
        })
    }
}

fn is_local(answer: &Timed) -> bool {
    answer.header(SOURCE) == Some("local")
}

fn outcome(
    scenario: &Scenario,
    lastfm: &[Timed],
    first: &[Timed],
    cached: Option<&[Timed]>,
) -> Outcome {
    let source = first
        .iter()
        .map(|answer| answer.header(SOURCE).unwrap_or("unknown"))
        .find(|source| *source != "local")
        .unwrap_or("local");
    let identical =
        lastfm.len() == first.len() && lastfm.iter().zip(first).all(|(a, b)| a.body == b.body);
    Outcome {
        id: scenario.id,
        requests: scenario.requests.len(),
        lastfm: Side {
            ms: millis(lastfm),
            bytes: bytes(lastfm),
            ok: all_ok(lastfm),
        },
        mirror: cached.map(|cached| MirrorSide {
            first_ms: millis(first),
            cached_ms: millis(cached),
            bytes: bytes(first),
            ok: all_ok(first),
        }),
        mirror_source: source.to_owned(),
        identical,
    }
}

fn millis(answers: &[Timed]) -> f64 {
    answers
        .iter()
        .map(|answer| answer.elapsed.as_secs_f64() * 1000.0)
        .sum()
}

fn bytes(answers: &[Timed]) -> usize {
    answers.iter().map(|answer| answer.body.len()).sum()
}

fn all_ok(answers: &[Timed]) -> bool {
    answers.iter().all(|answer| answer.status == 200)
}
