//! [`AccountServices`] for the Last.fm API's account methods.

use async_trait::async_trait;
use lfm_api::AccountServices;
use lfm_core::{ApiError, ErrorCode};
use lfm_store::{AccountId, AccountStatus, fold};
use lfm_upstream::{ScrobbleItem, ScrobbleOutcome, TrackItem};

use crate::forward::{self, failed};
use crate::password::MAX_PASSWORD;
use crate::state::{AppState, unix_now};

/// Password checks and write-through for the dispatcher.
#[derive(Debug, Clone)]
pub(crate) struct Accounts(pub(crate) AppState);

#[async_trait]
impl AccountServices for Accounts {
    /// Shares the web login's per-username limit, so the API is no way around it.
    async fn check_password(
        &self,
        name: &str,
        password: &str,
    ) -> Result<Option<AccountId>, ApiError> {
        let state = &self.0;
        let (key, now) = (fold(name), unix_now());
        if !state.name_attempts.allows(&key, now) {
            return Err(ErrorCode::RateLimitExceeded.into());
        }
        let account = state.store.account_by_name(name).await.map_err(failed)?;
        let hash = account.as_ref().map(|a| a.password_hash.as_str());
        let verified =
            password.len() <= MAX_PASSWORD && state.passwords.verify(hash, password).await;
        let Some(account) = account.filter(|_| verified) else {
            state.name_attempts.fail(&key, now);
            return Ok(None);
        };
        state.name_attempts.clear(&key);
        Ok((account.status == AccountStatus::Active).then_some(account.id))
    }

    async fn scrobble(
        &self,
        account: AccountId,
        items: Vec<ScrobbleItem>,
    ) -> Result<Vec<ScrobbleOutcome>, ApiError> {
        forward::scrobble(&self.0, account, items).await
    }

    async fn update_now_playing(
        &self,
        account: AccountId,
        track: TrackItem,
    ) -> Result<(), ApiError> {
        forward::now_playing(&self.0, account, track).await
    }

    async fn set_love(
        &self,
        account: AccountId,
        artist: &str,
        track: &str,
        loved: bool,
    ) -> Result<(), ApiError> {
        forward::set_love(&self.0, account, artist, track, loved).await
    }
}
