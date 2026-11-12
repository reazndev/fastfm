use std::path::PathBuf;

/// Everything that stops a conformance run before it can produce a report.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The command line was not understood.
    #[error("{0}\n\n{USAGE}")]
    Usage(String),
    /// A file could not be read or written.
    #[error("{path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// A fixture's metadata is missing or malformed.
    #[error("fixture {path}: {message}")]
    Fixture {
        /// The metadata file.
        path: PathBuf,
        /// What is wrong.
        message: String,
    },
    /// The ignore file is malformed or has a rule without a reason.
    #[error("ignore file {path}: {message}")]
    Ignore {
        /// The ignore file.
        path: PathBuf,
        /// What is wrong.
        message: String,
    },
    /// The base URL does not parse or the HTTP client cannot start.
    #[error("http client: {0}")]
    Http(#[from] reqwest::Error),
    /// The base URL is not a URL.
    #[error("base URL: {0}")]
    BaseUrl(String),
    /// The report could not be encoded.
    #[error("report: {0}")]
    Report(#[from] serde_json::Error),
}

impl Error {
    /// Wraps an I/O error with the path it concerns.
    pub fn io(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io { path, source }
    }
}

/// The command-line help.
pub const USAGE: &str = "\
usage: lfm-conformance run --base-url <url> [--api-key <key>] [options]

  --base-url <url>        API root, e.g. http://127.0.0.1:8080/2.0/
  --api-key <key>         key sent where a fixture had the real key
                          (or set LFM_CONFORMANCE_API_KEY, which stays out of `ps`)
  --only <prefix>         run cases whose id (method/variant.format) starts with this;
                          repeat to add more
  --format xml|json       run one format only
  --report <path>         write the JSON report here
  --ignore <path>         ignore rules (default conformance/ignore.toml if present)
  --fixtures <dir>        fixture directory (default fixtures)
  --expect <file>         baseline of case ids expected to pass; fail only on regressions
  --write-expect <file>   write the passing case ids (merged with ids not run) to this file
  --concurrency <n>       requests in flight (default 8)
  --rps <n>               at most n requests per second (default unlimited)";
