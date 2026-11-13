//! Forgets what has expired: web sessions, Last.fm auth tokens, and started web auth flows.

use std::time::Duration;

use crate::state::{AppState, unix_now};

/// How often expired rows are deleted.
const EVERY: Duration = Duration::from_secs(3600);

/// Sweeps once.
pub(crate) async fn once(state: &AppState) {
    let now = unix_now();
    if let Err(error) = state.store.purge_expired_sessions(now).await {
        tracing::warn!(%error, "purging expired sessions failed");
    }
    state.auth_states.purge(now);
}

/// Sweeps every hour, for as long as the runtime runs.
pub(crate) async fn run(state: AppState) {
    loop {
        once(&state).await;
        tokio::time::sleep(EVERY).await;
    }
}
