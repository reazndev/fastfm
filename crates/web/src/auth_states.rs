//! The `state` values that tie a Last.fm web auth callback to the session that started it.
//!
//! Without one, anyone could send a logged-in user a callback link carrying the attacker's own
//! Last.fm token and link the victim's account to the attacker's profile.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use lfm_api::methods::auth::random_hex;

use crate::error::WebError;

/// How long a started flow may take, in seconds.
const LIFETIME: i64 = 600;

/// Started flows by state: the stored session id and the expiry time.
#[derive(Debug, Default)]
pub(crate) struct AuthStates {
    pending: Mutex<HashMap<String, (String, i64)>>,
}

impl AuthStates {
    /// A new state for `session`. It replaces any flow the session started before, so the map
    /// holds at most one entry per session.
    pub(crate) fn start(&self, session: &str, now: i64) -> Result<String, WebError> {
        let state = random_hex(32).map_err(|error| WebError::internal(&error))?;
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        pending.retain(|_, (owner, expires)| owner != session && *expires > now);
        pending.insert(state.clone(), (session.to_owned(), now + LIFETIME));
        Ok(state)
    }

    /// Whether `state` was started by `session` and has not expired. A state works once.
    pub(crate) fn finish(&self, state: &str, session: &str, now: i64) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(state)
            .is_some_and(|(owner, expires)| owner == session && expires > now)
    }

    /// Forgets expired flows.
    pub(crate) fn purge(&self, now: i64) {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|_, (_, expires)| *expires > now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_works_once_for_its_own_session_until_it_expires() {
        let states = AuthStates::default();
        let state = states.start("s1", 0).unwrap();
        assert!(!states.finish(&state, "s2", 1), "another session");
        let state = states.start("s1", 0).unwrap();
        assert!(states.finish(&state, "s1", 1));
        assert!(!states.finish(&state, "s1", 2), "reused");
        let late = states.start("s1", 0).unwrap();
        assert!(!states.finish(&late, "s1", LIFETIME));
        let replaced = states.start("s1", 0).unwrap();
        states.start("s1", 0).unwrap();
        assert!(
            !states.finish(&replaced, "s1", 1),
            "a newer flow replaced it"
        );
        states.purge(LIFETIME);
        assert!(states.pending.lock().unwrap().is_empty());
    }
}
