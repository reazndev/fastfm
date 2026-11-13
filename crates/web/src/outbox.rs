//! The retry side of write-through: what a queued write holds, and the background retries.

use std::time::Duration;

use lfm_store::{AccountId, AccountStatus, NewOutboxEntry, OutboxEntry, OutboxStatus};
use lfm_upstream::ScrobbleItem;
use serde::{Deserialize, Serialize};

use crate::forward::send;
use crate::link::Linked;
use crate::state::{AppState, unix_now};

/// How often due writes are retried.
pub(crate) const RETRY_EVERY: Duration = Duration::from_secs(30);
/// Attempts after the first before a write is given up, about a day and a half of backoff.
const MAX_ATTEMPTS: u32 = 12;
/// Writes retried per round.
const BATCH: u32 = 50;
/// The first retry waits this long; each further one doubles it, up to [`MAX_DELAY`].
const FIRST_DELAY: i64 = 60;
const MAX_DELAY: i64 = 6 * 3600;

/// A write waiting in the outbox, stored as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Pending {
    /// `track.scrobble` with these items.
    Scrobble {
        /// The batch.
        items: Vec<ScrobbleItem>,
    },
    /// `track.love` or `track.unlove`.
    Love {
        /// Artist name.
        artist: String,
        /// Track title.
        track: String,
        /// Love when true, unlove when false.
        loved: bool,
    },
}

impl Pending {
    /// The Last.fm method it calls.
    pub(crate) fn method(&self) -> &'static str {
        match self {
            Self::Scrobble { .. } => "track.scrobble",
            Self::Love { loved: true, .. } => "track.love",
            Self::Love { loved: false, .. } => "track.unlove",
        }
    }
}

/// What to do with a write that did not reach Last.fm.
#[derive(Debug)]
pub(crate) enum Disposition {
    /// Applied locally only: the account cannot forward.
    Held,
    /// Retry later; the error was transient.
    Queued(String),
    /// Give up; Last.fm refused it.
    Failed(String),
}

/// Puts a write in the outbox. Failures are logged; the client has its answer already.
pub(crate) async fn record(
    state: &AppState,
    account: AccountId,
    pending: &Pending,
    disposition: Disposition,
    now: i64,
) {
    let Ok(payload) = serde_json::to_string(pending) else {
        return;
    };
    let (status, error) = match disposition {
        Disposition::Held => (OutboxStatus::Held, None),
        Disposition::Queued(error) => (OutboxStatus::Queued, Some(error)),
        Disposition::Failed(error) => (OutboxStatus::Failed, Some(error)),
    };
    let entry = NewOutboxEntry {
        account,
        method: pending.method().to_owned(),
        payload,
        status,
        // The first retry comes with the next round, [`RETRY_EVERY`] at most.
        next_at: now,
        error,
    };
    if let Err(error) = state.store.enqueue_write(entry, now).await {
        tracing::error!(%error, account = account.0, "queueing a write failed");
    }
}

/// Retries every queued write that is due. Returns how many reached Last.fm.
pub(crate) async fn retry_due(state: &AppState) -> usize {
    let now = unix_now();
    let due = match state.store.due_writes(now, BATCH).await {
        Ok(due) => due,
        Err(error) => {
            tracing::warn!(%error, "reading the outbox failed");
            return 0;
        }
    };
    let mut delivered = 0;
    for entry in due {
        let result = match retry_one(state, &entry).await {
            Ok(()) => {
                delivered += 1;
                state.store.finish_write(entry.id).await
            }
            Err(Retry::Later(error)) if entry.attempts + 1 < MAX_ATTEMPTS => {
                let next = now + delay(entry.attempts + 1);
                state.store.retry_write(entry.id, next, &error).await
            }
            Err(Retry::Later(error) | Retry::Never(error)) => {
                state.store.fail_write(entry.id, &error).await
            }
        };
        if let Err(error) = result {
            tracing::warn!(%error, "updating the outbox failed");
        }
    }
    delivered
}

/// Retries forever, every [`RETRY_EVERY`].
pub(crate) async fn run(state: AppState) {
    let mut ticks = tokio::time::interval(RETRY_EVERY);
    loop {
        ticks.tick().await;
        let delivered = retry_due(&state).await;
        if delivered > 0 {
            tracing::info!(delivered, "forwarded queued writes to Last.fm");
        }
    }
}

enum Retry {
    Later(String),
    Never(String),
}

async fn retry_one(state: &AppState, entry: &OutboxEntry) -> Result<(), Retry> {
    let never = |message: &str| Retry::Never(message.to_owned());
    let account = state
        .store
        .account(entry.account)
        .await
        .map_err(|e| Retry::Later(e.to_string()))?
        .filter(|account| account.status == AccountStatus::Active)
        .ok_or_else(|| never("the account is not active"))?;
    let name = account
        .lastfm_user
        .as_deref()
        .ok_or_else(|| never("no Last.fm user is linked"))?;
    let creds = Linked::open(&state.vault, &account)
        .map_err(|e| Retry::Never(e.to_string()))?
        .write_credentials(&state.config)
        .ok_or_else(|| never("no Last.fm session key"))?;
    let pending: Pending =
        serde_json::from_str(&entry.payload).map_err(|e| Retry::Never(e.to_string()))?;
    let user = state
        .store
        .ensure_user(name)
        .await
        .map_err(|e| Retry::Later(e.to_string()))?;
    match send(state, user, &pending, &creds).await {
        Ok(_) => Ok(()),
        Err(error) if error.is_transient() => Err(Retry::Later(error.to_string())),
        Err(error) => Err(Retry::Never(error.to_string())),
    }
}

/// Seconds to wait before attempt `attempt + 1`.
fn delay(attempt: u32) -> i64 {
    FIRST_DELAY
        .saturating_mul(1_i64 << attempt.min(20))
        .min(MAX_DELAY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_up_to_the_cap() {
        assert_eq!(delay(0), 60);
        assert_eq!(delay(1), 120);
        assert_eq!(delay(30), MAX_DELAY);
    }

    #[test]
    fn payload_round_trips() {
        let pending = Pending::Love {
            artist: "Cher".into(),
            track: "Believe".into(),
            loved: false,
        };
        let json = serde_json::to_string(&pending).unwrap();
        assert_eq!(serde_json::from_str::<Pending>(&json).unwrap(), pending);
        assert_eq!(pending.method(), "track.unlove");
    }
}
