//! Limits on password guessing, for the web login and `auth.getMobileSession` alike.
//!
//! A lockout counts failures per username and origin (the client IP, or the API as a whole),
//! so a stranger guessing at a name cannot lock its owner out of their own browser. Per
//! username alone there is only a short delay that grows with the failures, which slows a
//! spread-out guesser without refusing anyone. The API has no client IP, so its password
//! checks also share one ceiling, which keeps a spray from taking every argon2 permit.

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
/// Delay per recent failure of a username, and the most failures counted.
const DELAY_STEP: Duration = Duration::from_millis(250);
const DELAY_STEPS: u32 = 8;

/// Where a password check comes from.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Origin<'a> {
    /// The web login, from this client IP.
    Web(&'a str),
    /// `auth.getMobileSession`, which has no client IP.
    Api,
}

impl Origin<'_> {
    fn key(self, name_key: &str) -> String {
        match self {
            Origin::Web(ip) => format!("{name_key}\n{ip}"),
            Origin::Api => format!("{name_key}\napi"),
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
}

impl Attempts {
    pub(crate) fn new() -> Self {
        Self {
            per_ip: Throttle::new(PER_IP, WINDOW),
            per_name_and_origin: Throttle::new(PER_NAME_AND_ORIGIN, WINDOW),
            per_name: Throttle::new(u32::MAX, WINDOW),
            api_checks: Throttle::new(API_CHECKS_PER_MINUTE, 60),
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
            Origin::Api => self.api_checks.take("api", now),
        } && self.per_name_and_origin.allows(&origin.key(name_key), now);
        if admitted {
            let steps = self.per_name.count(name_key, now).min(DELAY_STEPS);
            tokio::time::sleep(DELAY_STEP * steps).await;
        }
        admitted
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
        assert!(attempts.admit("alice", Origin::Api, 1).await);
        assert_eq!(start.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn api_checks_share_one_ceiling() {
        let attempts = Attempts::new();
        for n in 0..API_CHECKS_PER_MINUTE {
            assert!(attempts.admit(&format!("user{n}"), Origin::Api, 0).await);
        }
        assert!(!attempts.admit("another", Origin::Api, 1).await);
        assert!(attempts.admit("another", Origin::Api, 61).await);
    }
}
