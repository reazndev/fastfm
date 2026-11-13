use std::fmt;

use reqwest::Url;
use reqwest::header::CONTENT_TYPE;
use tracing::{debug, warn};

use crate::config::{ApiKey, ClientConfig, RetryPolicy};
use crate::error::{Error, Result};
use crate::pool::KeyPool;
use crate::request::{HttpMethod, RawRequest, RawResponse};
use crate::retry::{backoff, is_retryable};

/// The rate-limited Last.fm client. Cheap to share behind an `Arc`.
pub struct Client {
    http: reqwest::Client,
    base_url: Url,
    pool: KeyPool,
    retry: RetryPolicy,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.base_url.as_str())
            .field("retry", &self.retry)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// Builds a client from its config.
    ///
    /// # Errors
    ///
    /// Fails when the base URL does not parse or the HTTP client cannot be built.
    pub fn new(config: ClientConfig) -> Result<Self> {
        let base_url = Url::parse(&config.base_url).map_err(|e| Error::BaseUrl(e.to_string()))?;
        let http = reqwest::Client::builder()
            .user_agent(config.user_agent)
            .timeout(config.timeout)
            .build()?;
        Ok(Self {
            http,
            base_url,
            pool: KeyPool::new(
                config.api_keys,
                config.requests_per_second,
                config.bucket_mode,
            ),
            retry: config.retry,
        })
    }

    /// Sends a request and returns Last.fm's answer untouched, error bodies included.
    ///
    /// Transient failures (Last.fm codes 8, 11, 16, 29, HTTP 429 and 5xx, network errors) are
    /// retried with backoff; after the last attempt the last response is returned as is.
    ///
    /// # Errors
    ///
    /// Fails when no key is available or every attempt failed at the network level.
    pub async fn raw(&self, request: &RawRequest) -> Result<RawResponse> {
        let mut attempt = 1;
        loop {
            let key = self.pool.acquire(&request.key, request.priority).await?;
            let outcome = self.send_once(request, key.as_ref()).await;
            let retry = match &outcome {
                Ok(response) => is_retryable(response.status, response.lastfm_error().as_ref()),
                Err(_) => true,
            };
            if !retry || request.single_attempt || attempt >= self.retry.max_attempts {
                return outcome;
            }
            let delay = backoff(&self.retry, attempt);
            warn!(
                method = request.method,
                attempt,
                ?delay,
                reason = %describe(&outcome),
                "retrying Last.fm call"
            );
            tokio::time::sleep(delay).await;
            attempt += 1;
        }
    }

    async fn send_once(&self, request: &RawRequest, key: Option<&ApiKey>) -> Result<RawResponse> {
        let mut params = request.wire_params(key.map(ApiKey::expose));
        if let Some(secret) = &request.secret {
            params.push(("api_sig".to_owned(), secret.sign(&params)));
        }
        let builder = match request.http_method {
            HttpMethod::Get => self.http.get(self.base_url.clone()).query(&params),
            HttpMethod::Post => self.http.post(self.base_url.clone()).form(&params),
        };
        let response = builder.send().await?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let content_type = headers
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let body = response.bytes().await?;
        debug!(
            method = request.method,
            status,
            bytes = body.len(),
            "Last.fm answered"
        );
        Ok(RawResponse {
            status,
            content_type,
            headers,
            body,
        })
    }
}

fn describe(outcome: &Result<RawResponse>) -> String {
    match outcome {
        Ok(response) => match response.lastfm_error() {
            Some(err) => format!("Last.fm error {}", err.code),
            None => format!("HTTP {}", response.status),
        },
        Err(err) => err.to_string(),
    }
}
