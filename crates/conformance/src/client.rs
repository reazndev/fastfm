//! Rebuilds a fixture's request and sends it to the server under test.

use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::Url;

use crate::error::Error;
use crate::fixture::{Case, HttpMethod, REDACTED};

const TIMEOUT: Duration = Duration::from_secs(60);

/// What the server under test answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received {
    /// HTTP status.
    pub status: u16,
    /// Response headers, names in lower case, repeated headers joined with `, `.
    pub headers: BTreeMap<String, String>,
    /// Body bytes, with the key replaced by `REDACTED`.
    pub body: Vec<u8>,
}

/// The server under test and the key to use on it.
#[derive(Debug, Clone)]
pub struct Target {
    base_url: Url,
    api_key: String,
    http: reqwest::Client,
}

impl Target {
    /// A target at `base_url` (such as `http://127.0.0.1:3055/2.0/`).
    pub fn new(base_url: &str, api_key: &str) -> Result<Self, Error> {
        let base_url = Url::parse(base_url).map_err(|e| Error::BaseUrl(e.to_string()))?;
        // Redirects would hide the status the server really sent.
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT)
            .user_agent(concat!("lfm-conformance/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            base_url,
            api_key: api_key.to_owned(),
            http,
        })
    }

    /// The base URL, for the report.
    #[must_use]
    pub fn base_url(&self) -> &str {
        self.base_url.as_str()
    }

    /// Sends the case's request: same HTTP method, same parameters in the same order, the real
    /// key where the fixture had `REDACTED`, and recorded invalid or missing keys left as they
    /// were.
    pub async fn send(&self, case: &Case) -> Result<Received, reqwest::Error> {
        let params = self.wire_params(case);
        let request = match case.http_method {
            HttpMethod::Get => self.http.get(self.base_url.clone()).query(&params),
            HttpMethod::Post => self.http.post(self.base_url.clone()).form(&params),
        };
        let response = request.send().await?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                let text = String::from_utf8_lossy(value.as_bytes());
                (name.as_str().to_owned(), self.scrub_text(&text))
            })
            .fold(BTreeMap::new(), join_repeated);
        let body = response.bytes().await?;
        Ok(Received {
            status,
            headers,
            body: scrub(&body, self.api_key.as_bytes()),
        })
    }

    fn wire_params(&self, case: &Case) -> Vec<(String, String)> {
        case.params
            .iter()
            .map(|(k, v)| {
                let value = if k == "api_key" && v == REDACTED {
                    self.api_key.clone()
                } else {
                    v.clone()
                };
                (k.clone(), value)
            })
            .collect()
    }

    fn scrub_text(&self, text: &str) -> String {
        if self.api_key.is_empty() {
            return text.to_owned();
        }
        text.replace(&self.api_key, REDACTED)
    }
}

fn join_repeated(
    mut headers: BTreeMap<String, String>,
    (name, value): (String, String),
) -> BTreeMap<String, String> {
    headers
        .entry(name)
        .and_modify(|v| {
            v.push_str(", ");
            v.push_str(&value);
        })
        .or_insert(value);
    headers
}

/// Replaces every occurrence of `key` with `REDACTED`, as the recorder did, so a server that
/// echoes the key matches the fixture and the report never holds the key.
fn scrub(body: &[u8], key: &[u8]) -> Vec<u8> {
    if key.is_empty() {
        return body.to_vec();
    }
    let mut out = Vec::with_capacity(body.len());
    let mut rest = body;
    while let Some(pos) = rest.windows(key.len()).position(|w| w == key) {
        let (before, after) = rest.split_at(pos);
        out.extend_from_slice(before);
        out.extend_from_slice(REDACTED.as_bytes());
        rest = after.get(key.len()..).unwrap_or_default();
    }
    out.extend_from_slice(rest);
    out
}
