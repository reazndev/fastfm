/// Everything that stops a recording run.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RecordError {
    /// `.env` is missing or lacks a variable.
    #[error("environment: {0}")]
    Env(String),
    /// Command line not understood.
    #[error("usage: lfm-record [record [--force] [--only <method-prefix>] | probe] ({0})")]
    Usage(String),
    /// A file could not be read or written.
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    /// The Last.fm client failed.
    #[error(transparent)]
    Upstream(#[from] lfm_upstream::Error),
    /// A discovery response did not have the expected shape.
    #[error("unexpected Last.fm data: {0}")]
    Data(String),
    /// JSON encoding or decoding failed.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}
