//! Write and auth methods without a signature, replayed against the recorded Last.fm answers.
#![allow(clippy::unwrap_used, reason = "tests")]

mod support;

use std::sync::Arc;

use async_trait::async_trait;
use lfm_api::{AccountServices, Dispatcher};
use lfm_core::ApiError;
use lfm_store::AccountId;
use lfm_upstream::{ScrobbleItem, ScrobbleOutcome, TrackItem};
use support::{assert_replays, dispatcher, same, store};

/// Accounts that every unsigned request is refused before reaching.
#[derive(Debug)]
struct Unreached;

#[async_trait]
impl AccountServices for Unreached {
    async fn check_password(&self, _: &str, _: &str) -> Result<Option<AccountId>, ApiError> {
        unreachable!("an unsigned request never checks a password")
    }

    async fn scrobble(
        &self,
        _: AccountId,
        _: Vec<ScrobbleItem>,
    ) -> Result<Vec<ScrobbleOutcome>, ApiError> {
        unreachable!("an unsigned request never scrobbles")
    }

    async fn update_now_playing(&self, _: AccountId, _: TrackItem) -> Result<(), ApiError> {
        unreachable!("an unsigned request never updates now playing")
    }

    async fn set_love(&self, _: AccountId, _: &str, _: &str, _: bool) -> Result<(), ApiError> {
        unreachable!("an unsigned request never loves")
    }
}

async fn with_writes() -> Dispatcher {
    let accounts: Arc<dyn AccountServices> = Arc::new(Unreached);
    let d = dispatcher(store().await.0);
    let registry = lfm_api::methods::with_accounts(lfm_api::methods::registry(), &accounts);
    Dispatcher::new(registry, d.services().clone())
}

#[tokio::test]
async fn unsigned_writes_answer_as_recorded() {
    let d = with_writes().await;
    for method in [
        "track.scrobble",
        "track.love",
        "track.unlove",
        "track.updateNowPlaying",
        "auth.getSession",
    ] {
        assert_replays(&d, method, "error-unsigned", same).await;
    }
}
