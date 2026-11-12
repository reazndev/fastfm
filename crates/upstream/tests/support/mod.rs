//! Shared helpers for the mock-server tests.

use std::num::NonZeroU32;
use std::time::Duration;

use lfm_upstream::{ApiKey, BucketMode, Client, ClientConfig, RetryPolicy};
use wiremock::matchers::query_param;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A client against the mock server with fast retries.
pub(crate) fn client(server: &MockServer, keys: &[&str], rps: u32, mode: BucketMode) -> Client {
    let config = ClientConfig::new(keys.iter().map(|k| ApiKey::new(*k)))
        .base_url(format!("{}/2.0/", server.uri()))
        .requests_per_second(NonZeroU32::new(rps).unwrap())
        .bucket_mode(mode)
        .retry(RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(40),
        });
    Client::new(config).unwrap()
}

/// A fast client with one key.
pub(crate) fn fast_client(server: &MockServer) -> Client {
    client(server, &["k1"], 1000, BucketMode::PerKey)
}

/// Answers every call to `method` with this JSON body.
pub(crate) async fn mount_json(server: &MockServer, method: &str, body: &str) {
    Mock::given(query_param("method", method))
        .respond_with(json(200, body))
        .mount(server)
        .await;
}

/// A JSON response as Last.fm sends it.
pub(crate) fn json(status: u16, body: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(body.as_bytes().to_vec(), "application/json")
}

/// The value of one query parameter of each request the server saw, in arrival order.
pub(crate) async fn seen(server: &MockServer, name: &str) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| {
            r.url
                .query_pairs()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default()
        })
        .collect()
}
