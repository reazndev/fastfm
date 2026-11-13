//! The sync crate's error type.

/// Everything that can stop a sync job.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// Last.fm could not be reached or answered with an error.
    #[error(transparent)]
    Upstream(#[from] lfm_upstream::Error),
    /// The store failed.
    #[error(transparent)]
    Store(#[from] lfm_store::StoreError),
    /// A persisted job cursor could not be read or written.
    #[error("invalid job cursor: {0}")]
    Cursor(#[from] serde_json::Error),
    /// Last.fm's answer contradicts itself, such as an empty page while `total` says more.
    #[error("inconsistent Last.fm answer: {0}")]
    Inconsistent(String),
    /// The user is not in the store.
    #[error("unknown user {0}")]
    UnknownUser(String),
    /// The job was stopped by a shutdown or `stop_user`; it resumes on the next start.
    #[error("stopped")]
    Stopped,
}

/// Last.fm's code for a profile that hides its recent tracks: "Login: User required to be
/// logged in".
const PRIVATE_PROFILE: u32 = 17;

impl SyncError {
    /// Whether Last.fm refused because the profile hides its listening history. Retrying soon
    /// cannot help: only the owner can make it public again.
    #[must_use]
    pub fn is_private_profile(&self) -> bool {
        matches!(
            self,
            Self::Upstream(lfm_upstream::Error::LastFm { code, .. }) if *code == PRIVATE_PROFILE
        )
    }
}

/// Result type of this crate.
pub type Result<T, E = SyncError> = std::result::Result<T, E>;
