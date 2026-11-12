//! Finding the user a request names, and mapping store failures.

use lfm_core::{ApiError, ErrorCode, Params};
use lfm_store::{MirroredUser, StoreError};

use crate::method::Context;

/// The `user` parameter, which every local method reads and caches by.
pub(crate) fn cache_user(params: &Params) -> Option<&str> {
    params.optional("user")
}

/// The mirrored user the request names, looked up case-insensitively by the dispatcher.
///
/// A missing `user` and a user this instance does not mirror both get Last.fm's
/// "User not found" (error 6, HTTP 404). Pass-through to Last.fm for users we do not mirror
/// replaces the second case here.
pub(crate) fn mirrored_user<'c>(ctx: &'c Context<'_>) -> Result<&'c MirroredUser, ApiError> {
    ctx.user
        .ok_or_else(|| ApiError::not_found("User not found"))
}

/// Turns a store failure into error 8, logging it rather than showing it.
pub(crate) trait OrFailed<T> {
    /// The value, or error 8.
    fn or_failed(self) -> Result<T, ApiError>;
}

impl<T> OrFailed<T> for Result<T, StoreError> {
    fn or_failed(self) -> Result<T, ApiError> {
        self.map_err(|error| {
            tracing::error!(%error, "store query failed");
            ErrorCode::OperationFailed.into()
        })
    }
}
