//! A plain HTTP client that times single `GET` requests, for the speed comparison.
//!
//! It bypasses the key pool, the rate limiter and the retries on purpose: the caller measures
//! one request as a client would see it, and paces its own calls.

use std::fmt;
use std::time::{Duration, Instant};

use bytes::Bytes;
use reqwest::header::HeaderMap;

use crate::error::Result;

/// A keep-alive HTTP client whose requests report their wall-clock time.
pub struct TimedClient {
    http: reqwest::Client,
}

impl fmt::Debug for TimedClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TimedClient").finish_non_exhaustive()
    }
}

/// One timed answer.
#[derive(Debug, Clone)]
pub struct Timed {
    /// The HTTP status.
    pub status: u16,
    /// The whole body.
    pub body: Bytes,
    /// From sending the request to the last byte of the body.
    pub elapsed: Duration,
    headers: HeaderMap,
}

impl Timed {
    /// A response header as text.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

impl TimedClient {
    /// A client that gives up on a request after `timeout`.
    ///
    /// # Errors
    ///
    /// Fails when the HTTP client cannot be built.
    pub fn new(user_agent: &str, timeout: Duration) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(user_agent)
            .timeout(timeout)
            .build()?;
        Ok(Self { http })
    }

    /// Sends `GET url?params` and reads the whole body. Errors never carry the URL, which may
    /// hold an API key.
    ///
    /// # Errors
    ///
    /// Fails when the request does not complete.
    pub async fn get(&self, url: &str, params: &[(&str, &str)]) -> Result<Timed> {
        let started = Instant::now();
        let response = self.http.get(url).query(params).send().await?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response.bytes().await?;
        Ok(Timed {
            status,
            body,
            elapsed: started.elapsed(),
            headers,
        })
    }
}
