//! Last.fm error codes, their default messages and HTTP statuses.

use std::borrow::Cow;

use crate::tree::Str;

/// Every error code Last.fm documents. Code 1 and 19 mean "this error does not exist" and 28
/// is not documented, so they are left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// 2: the service does not exist.
    InvalidService,
    /// 3: no method with that name.
    InvalidMethod,
    /// 4: the key may not use this service.
    AuthenticationFailed,
    /// 5: the `format` parameter is unknown.
    InvalidFormat,
    /// 6: a parameter is missing or invalid, or the resource is not found.
    InvalidParameters,
    /// 7: invalid resource specified.
    InvalidResource,
    /// 8: the backend failed.
    OperationFailed,
    /// 9: the session key is invalid.
    InvalidSessionKey,
    /// 10: the API key is invalid.
    InvalidApiKey,
    /// 11: the service is offline.
    ServiceOffline,
    /// 12: subscribers only.
    SubscribersOnly,
    /// 13: the `api_sig` does not match.
    InvalidSignature,
    /// 14: the token has not been authorised.
    UnauthorizedToken,
    /// 15: the item is not available for streaming.
    NotStreamable,
    /// 16: the service is temporarily unavailable.
    TemporarilyUnavailable,
    /// 17: the user must be logged in.
    LoginRequired,
    /// 18: the radio trial expired.
    TrialExpired,
    /// 20: not enough content for the station.
    NotEnoughContent,
    /// 21: not enough group members for radio.
    NotEnoughMembers,
    /// 22: not enough fans for radio.
    NotEnoughFans,
    /// 23: not enough neighbours for radio.
    NotEnoughNeighbours,
    /// 24: no radio at peak times.
    NoPeakRadio,
    /// 25: the radio station was not found.
    RadioNotFound,
    /// 26: the API key is suspended.
    ApiKeySuspended,
    /// 27: the request type is deprecated.
    Deprecated,
    /// 29: rate limit exceeded.
    RateLimitExceeded,
}

impl ErrorCode {
    /// The numeric code.
    #[must_use]
    pub fn number(self) -> u8 {
        self.details().0
    }

    /// The message Last.fm sends when the method gives no specific one.
    #[must_use]
    pub fn default_message(self) -> &'static str {
        self.details().1
    }

    /// The HTTP status Last.fm sends with this code.
    #[must_use]
    pub fn http_status(self) -> u16 {
        self.details().2
    }

    // Observed against Last.fm on 2026-11-12: the messages and statuses of 3, 5, 6, 10 and 13.
    // The other messages come from the API docs, the other statuses are guesses.
    fn details(self) -> (u8, &'static str, u16) {
        match self {
            Self::InvalidService => (2, "Invalid service - This service does not exist", 400),
            Self::InvalidMethod => (
                3,
                "Invalid Method - No method with that name in this package",
                400,
            ),
            Self::AuthenticationFailed => (
                4,
                "Authentication Failed - You do not have permissions to access the service",
                403,
            ),
            Self::InvalidFormat => (
                5,
                "Invalid format - This service doesn't exist in that format",
                400,
            ),
            Self::InvalidParameters => (
                6,
                "Invalid parameters - Your request is missing a required parameter",
                400,
            ),
            Self::InvalidResource => (7, "Invalid resource specified", 400),
            Self::OperationFailed => (
                8,
                "Operation failed - Most likely the backend service failed. Please try again.",
                500,
            ),
            Self::InvalidSessionKey => (9, "Invalid session key - Please re-authenticate", 403),
            Self::InvalidApiKey => (
                10,
                "Invalid API key - You must be granted a valid key by last.fm",
                403,
            ),
            Self::ServiceOffline => (
                11,
                "Service Offline - This service is temporarily offline. Try again later.",
                503,
            ),
            other => other.radio_and_limits(),
        }
    }

    fn radio_and_limits(self) -> (u8, &'static str, u16) {
        match self {
            Self::SubscribersOnly => (
                12,
                "Subscribers Only - This station is only available to paid last.fm subscribers",
                403,
            ),
            Self::InvalidSignature => (13, "Invalid method signature supplied", 400),
            Self::UnauthorizedToken => (
                14,
                "Unauthorized Token - This token has not been authorized",
                403,
            ),
            Self::NotStreamable => (15, "This item is not available for streaming.", 403),
            Self::TemporarilyUnavailable => (
                16,
                "The service is temporarily unavailable, please try again.",
                503,
            ),
            Self::LoginRequired => (17, "Login: User requires to be logged in", 403),
            Self::TrialExpired => (
                18,
                "Trial Expired - This user has no free radio plays left. Subscription required.",
                403,
            ),
            Self::NotEnoughContent => (
                20,
                "Not Enough Content - There is not enough content to play this station",
                400,
            ),
            Self::NotEnoughMembers => (
                21,
                "Not Enough Members - This group does not have enough members for radio",
                400,
            ),
            Self::NotEnoughFans => (
                22,
                "Not Enough Fans - This artist does not have enough fans for for radio",
                400,
            ),
            other => other.rest(),
        }
    }

    fn rest(self) -> (u8, &'static str, u16) {
        match self {
            Self::NotEnoughNeighbours => (
                23,
                "Not Enough Neighbours - There are not enough neighbours for radio",
                400,
            ),
            Self::NoPeakRadio => (
                24,
                "No Peak Radio - This user is not allowed to listen to radio during peak usage",
                403,
            ),
            Self::RadioNotFound => (25, "Radio Not Found - Radio station not found", 404),
            Self::ApiKeySuspended => (
                26,
                "API Key Suspended - This application is not allowed to make requests to the web \
                 services",
                403,
            ),
            Self::Deprecated => (
                27,
                "Deprecated - This type of request is no longer supported",
                400,
            ),
            _ => (
                29,
                "Rate Limit Exceded - Your IP has made too many requests in a short period, \
                 exceeding our API guidelines",
                429,
            ),
        }
    }
}

/// A Last.fm error response: a code, plus the message and HTTP status when a method overrides
/// the defaults (for example `user.getInfo` sends code 6 as "User not found" with 404).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Last.fm error {}: {}", .code.number(), self.message())]
pub struct ApiError {
    code: ErrorCode,
    message: Option<Str>,
    status: Option<u16>,
}

impl ApiError {
    /// An error with the code's default message and status.
    #[must_use]
    pub fn new(code: ErrorCode) -> Self {
        Self {
            code,
            message: None,
            status: None,
        }
    }

    /// Replaces the default message.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<Str>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Replaces the default HTTP status.
    #[must_use]
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    /// Error 6 with the default "missing a required parameter" message.
    #[must_use]
    pub fn missing_parameter() -> Self {
        Self::new(ErrorCode::InvalidParameters)
    }

    /// Error 6 for an unknown resource, such as `"User not found"`, with HTTP 404.
    #[must_use]
    pub fn not_found(message: &'static str) -> Self {
        Self::new(ErrorCode::InvalidParameters)
            .with_message(Cow::Borrowed(message))
            .with_status(404)
    }

    /// The error code.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        self.code
    }

    /// The message that goes into the response.
    #[must_use]
    pub fn message(&self) -> &str {
        self.message
            .as_deref()
            .unwrap_or_else(|| self.code.default_message())
    }

    /// The HTTP status of the response.
    #[must_use]
    pub fn http_status(&self) -> u16 {
        self.status.unwrap_or_else(|| self.code.http_status())
    }
}

impl From<ErrorCode> for ApiError {
    fn from(code: ErrorCode) -> Self {
        Self::new(code)
    }
}
