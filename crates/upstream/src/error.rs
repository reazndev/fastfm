/// Result type of this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// An error reported by Last.fm in the response body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastFmError {
    /// Last.fm's numeric error code, such as 6 or 29.
    pub code: u32,
    /// Last.fm's message, unchanged.
    pub message: String,
}

/// Everything that can go wrong when calling Last.fm.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Last.fm answered with an error body.
    #[error("Last.fm error {code}: {message}")]
    LastFm {
        /// Last.fm's numeric error code.
        code: u32,
        /// Last.fm's message.
        message: String,
    },
    /// Last.fm answered with a non-success status and no recognisable error body.
    #[error("Last.fm returned HTTP {status}")]
    Http {
        /// The HTTP status code.
        status: u16,
    },
    /// The request did not complete. The URL is stripped so the API key cannot leak.
    #[error("request to Last.fm failed: {0}")]
    Transport(reqwest::Error),
    /// The body was not the JSON shape this crate expects.
    #[error("could not parse Last.fm response: {0}")]
    Parse(#[from] serde_json::Error),
    /// No key in the pool and none given for the call.
    #[error("no Last.fm API key available")]
    NoApiKey,
    /// The configured base URL does not parse.
    #[error("invalid base URL: {0}")]
    BaseUrl(String),
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Self::Transport(err.without_url())
    }
}

impl From<LastFmError> for Error {
    fn from(err: LastFmError) -> Self {
        Self::LastFm {
            code: err.code,
            message: err.message,
        }
    }
}
