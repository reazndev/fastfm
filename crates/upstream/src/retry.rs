use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::Duration;

use serde::Deserialize;

use crate::config::RetryPolicy;
use crate::error::LastFmError;

/// Last.fm codes that mean "try again later": operation failed, service offline,
/// temporary error, rate limit exceeded.
const RETRYABLE_CODES: [u32; 4] = [8, 11, 16, 29];

#[derive(Deserialize)]
struct JsonError {
    error: u32,
    #[serde(default)]
    message: String,
}

/// Finds a Last.fm error in a JSON, JSONP or XML body.
pub(crate) fn detect_error(body: &[u8]) -> Option<LastFmError> {
    let text = std::str::from_utf8(body).ok()?;
    json_error(text).or_else(|| xml_error(text))
}

fn json_error(text: &str) -> Option<LastFmError> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    let object = text.get(start..=end)?;
    let parsed: JsonError = serde_json::from_str(object).ok()?;
    Some(LastFmError {
        code: parsed.error,
        message: parsed.message,
    })
}

fn xml_error(text: &str) -> Option<LastFmError> {
    if !text.contains("status=\"failed\"") {
        return None;
    }
    let after_code = text.split_once("<error code=\"")?.1;
    let (code, rest) = after_code.split_once('"')?;
    let message = rest.split_once('>')?.1.split_once("</error>")?.0;
    Some(LastFmError {
        code: code.trim().parse().ok()?,
        message: unescape_xml(message.trim()),
    })
}

fn unescape_xml(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Whether a response should be retried.
pub(crate) fn is_retryable(status: u16, error: Option<&LastFmError>) -> bool {
    status >= 500 || status == 429 || error.is_some_and(|e| RETRYABLE_CODES.contains(&e.code))
}

/// Exponential backoff with full jitter: a random delay up to `base * 2^(attempt - 1)`,
/// capped at `max_delay`. `attempt` counts from 1.
pub(crate) fn backoff(policy: &RetryPolicy, attempt: u32) -> Duration {
    let factor = 2u32.saturating_pow(attempt.saturating_sub(1));
    let ceiling = policy
        .base_delay
        .saturating_mul(factor)
        .min(policy.max_delay);
    let fraction = f64::from(random_u32()) / f64::from(u32::MAX);
    ceiling.mul_f64(fraction).max(policy.base_delay / 4)
}

/// A cheap random number from the standard library's per-instance hash seed; jitter needs
/// no cryptographic quality.
fn random_u32() -> u32 {
    let bits = RandomState::new().build_hasher().finish();
    u32::try_from(bits >> 32).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_json_error() {
        let err = detect_error(br#"{"message":"Rate Limit Exceeded","error":29}"#).unwrap();
        assert_eq!(err.code, 29);
        assert_eq!(err.message, "Rate Limit Exceeded");
    }

    #[test]
    fn finds_jsonp_error() {
        let err = detect_error(br#"cb({"error":6,"message":"User not found"})"#).unwrap();
        assert_eq!(err.code, 6);
    }

    #[test]
    fn finds_xml_error() {
        let body = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<lfm status=\"failed\">\n\
            <error code=\"10\">Invalid API key - You must be granted a valid key by last.fm\
            </error>\n</lfm>";
        let err = detect_error(body).unwrap();
        assert_eq!(err.code, 10);
        assert!(err.message.starts_with("Invalid API key"));
    }

    #[test]
    fn ignores_success_bodies() {
        assert!(detect_error(br#"{"user":{"name":"x"}}"#).is_none());
        assert!(detect_error(b"<lfm status=\"ok\"><user/></lfm>").is_none());
    }

    #[test]
    fn backoff_stays_within_bounds() {
        let policy = RetryPolicy {
            max_attempts: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(300),
        };
        for attempt in 1..10 {
            let delay = backoff(&policy, attempt);
            assert!(delay >= Duration::from_millis(25));
            assert!(delay <= Duration::from_millis(300));
        }
    }
}
