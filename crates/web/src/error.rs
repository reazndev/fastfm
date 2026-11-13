//! The web API's errors: `{"error": {"code", "message"}}` with an HTTP status.

use std::borrow::Cow;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lfm_api::methods::auth::RandomUnavailable;
use lfm_store::StoreError;
use serde_json::json;

use crate::crypto::VaultError;

/// A failed request. Client errors carry a stable `code` for the UI; internal errors are
/// logged where they happen and shown only as `internal`.
#[derive(Debug, thiserror::Error)]
pub enum WebError {
    /// Something the caller can fix or must know.
    #[error("{code}: {message}")]
    Client {
        /// HTTP status.
        status: StatusCode,
        /// Stable machine-readable code.
        code: &'static str,
        /// Human-readable message.
        message: Cow<'static, str>,
    },
    /// A failure on this side, already logged.
    #[error("internal error")]
    Internal,
}

impl WebError {
    /// A client error.
    pub(crate) fn new(
        status: StatusCode,
        code: &'static str,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::Client {
            status,
            code,
            message: message.into(),
        }
    }

    /// 400 with this code.
    pub(crate) fn invalid(code: &'static str, message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    /// Logs `error` and returns [`WebError::Internal`].
    pub(crate) fn internal(error: &dyn std::fmt::Display) -> Self {
        tracing::error!(%error, "web request failed");
        Self::Internal
    }

    pub(crate) fn not_logged_in() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "not_logged_in", "Log in first.")
    }

    pub(crate) fn bad_credentials() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "Wrong username or password.",
        )
    }

    pub(crate) fn pending() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "account_pending",
            "Your account is waiting for an admin's approval.",
        )
    }

    pub(crate) fn disabled() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "account_disabled",
            "Your account is disabled.",
        )
    }

    pub(crate) fn admin_only() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "admin_only",
            "Only admins can do this.",
        )
    }

    pub(crate) fn rate_limited() -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many attempts. Try again in a few minutes.",
        )
    }

    pub(crate) fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", "Not found.")
    }

    pub(crate) fn conflict(code: &'static str, message: &'static str) -> Self {
        Self::new(StatusCode::CONFLICT, code, message)
    }

    pub(crate) fn lastfm(error: &lfm_upstream::Error) -> Self {
        tracing::warn!(%error, "Last.fm call from the web API failed");
        if error.is_transient() {
            return Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "lastfm_unavailable",
                "Last.fm did not answer. Try again later.",
            );
        }
        let message = match error {
            lfm_upstream::Error::LastFm { message, .. } => format!("Last.fm said: {message}"),
            other => format!("Last.fm call failed: {other}"),
        };
        Self::new(StatusCode::BAD_GATEWAY, "lastfm_error", message)
    }
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Client {
                status,
                code,
                message,
            } => (status, code, message),
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                Cow::Borrowed("Something went wrong on the server."),
            ),
        };
        let body = json!({ "error": { "code": code, "message": message } });
        (status, Json(body)).into_response()
    }
}

impl From<StoreError> for WebError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::NotFound => Self::not_found(),
            StoreError::SignupClosed => Self::new(
                StatusCode::FORBIDDEN,
                "signup_closed",
                "Sign-up is closed on this instance.",
            ),
            StoreError::NameTaken => Self::conflict("name_taken", "That username is taken."),
            StoreError::AccountNotActive => Self::pending(),
            StoreError::LastAdmin => Self::conflict(
                "last_admin",
                "The last active admin cannot be disabled or demoted.",
            ),
            other => Self::internal(&other),
        }
    }
}

impl From<RandomUnavailable> for WebError {
    fn from(error: RandomUnavailable) -> Self {
        Self::internal(&error)
    }
}

impl From<VaultError> for WebError {
    fn from(error: VaultError) -> Self {
        Self::internal(&error)
    }
}

impl From<lfm_sync::SyncError> for WebError {
    fn from(error: lfm_sync::SyncError) -> Self {
        match error {
            lfm_sync::SyncError::Upstream(error) => Self::lastfm(&error),
            lfm_sync::SyncError::Store(error) => error.into(),
            other => Self::internal(&other),
        }
    }
}
