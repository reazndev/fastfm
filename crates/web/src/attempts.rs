//! Limits on password guessing, for the web login and `auth.getMobileSession` alike, and on
//! `auth.getToken`.
//!
//! A lockout counts failures per username and origin (the client IP, or the API as a whole),
//! so a stranger guessing at a name cannot lock its owner out of their own browser. Per
//! username alone there is only a short delay that grows with the failures, which slows a
//! spread-out guesser without refusing anyone. The API has no client IP, so its password
//! checks also share one ceiling, which keeps a spray from taking every argon2 permit, and each
//! API key has a lower ceiling of its own, so one key cannot use up the shared one.

use std::time::Duration;

use crate::throttle::Throttle;

/// Failed logins and sign-ups per client IP.
const PER_IP: u32 = 30;
/// Failed logins per username and origin.
const PER_NAME_AND_ORIGIN: u32 = 10;
/// The lockout window in seconds.
const WINDOW: i64 = 900;
/// Password checks through the API per minute, all callers together.
const API_CHECKS_PER_MINUTE: u32 = 60;
/// Password checks through the API per minute and API key.
const API_CHECKS_PER_KEY: u32 = 10;
/// `auth.getToken` calls per minute and API key. Each one writes a token row.
const TOKENS_PER_KEY: u32 = 30;
/// Delay per recent failure of a username, and the most failures counted.
const DELAY_STEP: Duration = Duration::from_millis(250);
const DELAY_STEPS: u32 = 8;

/// Where a password check comes from.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Origin<'a> {
    /// The web login, from this client IP.
    Web(&'a str),
    /// `auth.getMobileSession` with this API key; it has no client IP.
    Api(&'a str),
}

impl Origin<'_> {
    fn key(self, name_key: &str) -> String {
        match self {
            Origin::Web(ip) => format!("{name_key}\n{ip}"),
            Origin::Api(_) => format!("{name_key}\napi"),
        }
    }
}

/// The counters behind the limits.
#[derive(Debug)]
pub(crate) struct Attempts {
    per_ip: Throttle,
    per_name_and_origin: Throttle,
    per_name: Throttle,
    api_checks: Throttle,
    api_checks_per_key: Throttle,
    tokens_per_key: Throttle,
}

impl Attempts {
    pub(crate) fn new() -> Self {
        Self {
            per_ip: Throttle::new(PER_IP, WINDOW),
            per_name_and_origin: Throttle::new(PER_NAME_AND_ORIGIN, WINDOW),
            per_name: Throttle::new(u32::MAX, WINDOW),
            api_checks: Throttle::new(API_CHECKS_PER_MINUTE, 60),
            api_checks_per_key: Throttle::new(API_CHECKS_PER_KEY, 60),
            tokens_per_key: Throttle::new(TOKENS_PER_KEY, 60),
        }
    }

    /// Whether `ip` may try a sign-up or login.
    pub(crate) fn allows_ip(&self, ip: &str, now: i64) -> bool {
        self.per_ip.allows(ip, now)
    }

    /// Counts a sign-up against `ip`: each one costs a password hash.
    pub(crate) fn count_ip(&self, ip: &str, now: i64) {
        self.per_ip.fail(ip, now);
    }

    /// Whether a password check for `name_key` from `origin` may run. When it may, first waits
    /// the username's delay.
    pub(crate) async fn admit(&self, name_key: &str, origin: Origin<'_>, now: i64) -> bool {
        let admitted = match origin {
            Origin::Web(ip) => self.per_ip.allows(ip, now),
            Origin::Api(api_key) => {
                self.api_checks_per_key.take(api_key, now) && self.api_checks.take("api", now)
            }
        } && self.per_name_and_origin.allows(&origin.key(name_key), now);
        if admitted {
            let steps = self.per_name.count(name_key, now).min(DELAY_STEPS);
            tokio::time::sleep(DELAY_STEP * steps).await;
        }
        admitted
    }

    /// Counts one `auth.getToken` call of `api_key`; whether it is within the limit.
    pub(crate) fn admit_token(&self, api_key: &str, now: i64) -> bool {
        self.tokens_per_key.take(api_key, now)
    }

    /// Records a wrong password.
    pub(crate) fn failed(&self, name_key: &str, origin: Origin<'_>, now: i64) {
        if let Origin::Web(ip) = origin {
            self.per_ip.fail(ip, now);
        }
        self.per_name_and_origin.fail(&origin.key(name_key), now);
        self.per_name.fail(name_key, now);
    }

    /// Forgets the failures of `name_key`, after its owner logged in.
    pub(crate) fn succeeded(&self, name_key: &str, origin: Origin<'_>) {
        self.per_name_and_origin.clear(&origin.key(name_key));
        self.per_name.clear(name_key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_guesser_elsewhere_does_not_lock_the_owner_out() {
        let attempts = Attempts::new();
        for _ in 0..PER_NAME_AND_ORIGIN {
            assert!(attempts.admit("alice", Origin::Web("6.6.6.6"), 0).await);
            attempts.failed("alice", Origin::Web("6.6.6.6"), 0);
        }
        assert!(!attempts.admit("alice", Origin::Web("6.6.6.6"), 1).await);
        let start = tokio::time::Instant::now();
        assert!(attempts.admit("alice", Origin::Web("192.0.2.1"), 1).await);
        assert_eq!(
            start.elapsed(),
            DELAY_STEP * DELAY_STEPS,
            "only slowed down"
        );
        attempts.succeeded("alice", Origin::Web("192.0.2.1"));
        let start = tokio::time::Instant::now();
        assert!(attempts.admit("alice", Origin::Api("k"), 1).await);
        assert_eq!(start.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn api_checks_share_one_ceiling() {
        let attempts = Attempts::new();
        for n in 0..API_CHECKS_PER_MINUTE {
            let key = format!("key{}", n % 6);
            assert!(
                attempts
                    .admit(&format!("user{n}"), Origin::Api(&key), 0)
                    .await
            );
        }
        assert!(!attempts.admit("another", Origin::Api("new key"), 1).await);
        assert!(attempts.admit("another", Origin::Api("new key"), 61).await);
    }

    #[tokio::test(start_paused = true)]
    async fn one_api_key_cannot_use_up_the_shared_ceiling() {
        let attempts = Attempts::new();
        for n in 0..API_CHECKS_PER_KEY {
            assert!(
                attempts
                    .admit(&format!("user{n}"), Origin::Api("spray"), 0)
                    .await
            );
        }
        assert!(!attempts.admit("another", Origin::Api("spray"), 1).await);
        assert!(attempts.admit("another", Origin::Api("other"), 1).await);
        assert!(attempts.admit("another", Origin::Api("spray"), 61).await);
    }

    #[test]
    fn tokens_are_limited_per_api_key() {
        let attempts = Attempts::new();
        for _ in 0..TOKENS_PER_KEY {
            assert!(attempts.admit_token("k", 0));
        }
        assert!(!attempts.admit_token("k", 1));
        assert!(attempts.admit_token("other", 1));
        assert!(attempts.admit_token("k", 60));
    }
}
