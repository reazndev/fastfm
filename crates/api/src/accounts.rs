//! The hook through which methods that act for a local account reach the account side.

use async_trait::async_trait;
use lfm_core::ApiError;
use lfm_store::AccountId;
use lfm_upstream::{ScrobbleItem, ScrobbleOutcome, TrackItem};

/// What `auth.getMobileSession` and the track writes need from accounts. `lfm-web` implements
/// it: password checks with its rate limit, and local writes forwarded to Last.fm.
#[async_trait]
pub trait AccountServices: Send + Sync + std::fmt::Debug {
    /// The active account with this name and password, or `None` for any mismatch.
    async fn check_password(
        &self,
        name: &str,
        password: &str,
    ) -> Result<Option<AccountId>, ApiError>;

    /// Stores the scrobbles and forwards them. One outcome per item, in the order given.
    async fn scrobble(
        &self,
        account: AccountId,
        items: Vec<ScrobbleItem>,
    ) -> Result<Vec<ScrobbleOutcome>, ApiError>;

    /// Shows the track as playing now and forwards it.
    async fn update_now_playing(
        &self,
        account: AccountId,
        track: TrackItem,
    ) -> Result<(), ApiError>;

    /// Loves the track, or unloves it when `loved` is false, and forwards that.
    async fn set_love(
        &self,
        account: AccountId,
        artist: &str,
        track: &str,
        loved: bool,
    ) -> Result<(), ApiError>;
}
