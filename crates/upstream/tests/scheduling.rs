//! Rate limiting, key rotation, priority lanes, retries and the raw pass-through call.

#[cfg(test)]
mod support;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lfm_upstream::{
    ApiKey, BucketMode, CallOptions, Error, Format, HttpMethod, Priority, RawRequest,
};
use support::{client, fast_client, json, mount_json, seen};
use wiremock::matchers::{body_string_contains, method, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const USER_JSON: &str =
    r##"{"user":{"name":"alice","playcount":"10","registered":{"unixtime":"1","#text":1}}}"##;

#[tokio::test]
async fn retries_error_29_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(query_param("method", "user.getInfo"))
        .respond_with(json(403, r#"{"message":"Rate Limit Exceeded","error":29}"#))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    mount_json(&server, "user.getInfo", USER_JSON).await;
    let user = fast_client(&server)
        .user_info("alice", &CallOptions::default())
        .await
        .unwrap();
    assert_eq!(user.name, "alice");
    assert_eq!(seen(&server, "method").await.len(), 2);
}

#[tokio::test]
async fn retries_http_503_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(query_param("method", "user.getInfo"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    mount_json(&server, "user.getInfo", USER_JSON).await;
    let user = fast_client(&server)
        .user_info("alice", &CallOptions::default())
        .await;
    assert!(user.is_ok());
    assert_eq!(seen(&server, "method").await.len(), 2);
}

#[tokio::test]
async fn gives_up_after_max_attempts() {
    let server = MockServer::start().await;
    Mock::given(query_param("method", "user.getInfo"))
        .respond_with(json(500, r#"{"message":"Operation failed","error":8}"#))
        .mount(&server)
        .await;
    let err = fast_client(&server)
        .user_info("alice", &CallOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::LastFm { code: 8, .. }));
    assert_eq!(seen(&server, "method").await.len(), 3);
}

#[tokio::test]
async fn does_not_retry_other_errors() {
    let server = MockServer::start().await;
    Mock::given(query_param("method", "user.getInfo"))
        .respond_with(json(404, r#"{"message":"User not found","error":6}"#))
        .mount(&server)
        .await;
    let err = fast_client(&server)
        .user_info("nobody", &CallOptions::default())
        .await
        .unwrap_err();
    match err {
        Error::LastFm { code, message } => {
            assert_eq!(code, 6);
            assert_eq!(message, "User not found");
        }
        other => panic!("unexpected error {other:?}"),
    }
    assert_eq!(seen(&server, "method").await.len(), 1);
}

#[tokio::test]
async fn rotates_through_the_key_pool() {
    let server = MockServer::start().await;
    mount_json(&server, "user.getInfo", USER_JSON).await;
    let client = client(&server, &["k1", "k2", "k3"], 1000, BucketMode::PerKey);
    for _ in 0..6 {
        client
            .user_info("alice", &CallOptions::default())
            .await
            .unwrap();
    }
    let keys = seen(&server, "api_key").await;
    assert_eq!(keys, ["k1", "k2", "k3", "k1", "k2", "k3"]);
}

#[tokio::test]
async fn per_call_key_overrides_the_pool() {
    let server = MockServer::start().await;
    mount_json(&server, "user.getInfo", USER_JSON).await;
    let options = CallOptions::default().api_key(ApiKey::new("own"));
    fast_client(&server)
        .user_info("alice", &options)
        .await
        .unwrap();
    assert_eq!(seen(&server, "api_key").await, ["own"]);
}

#[tokio::test]
async fn shared_bucket_paces_all_keys_together() {
    let server = MockServer::start().await;
    mount_json(&server, "user.getInfo", USER_JSON).await;
    let timed = |mode| {
        let client = client(&server, &["k1", "k2", "k3"], 5, mode);
        async move {
            let start = Instant::now();
            for _ in 0..3 {
                client
                    .user_info("a", &CallOptions::default())
                    .await
                    .unwrap();
            }
            start.elapsed()
        }
    };
    // Three keys at 5 rps each: three calls need no wait. One shared bucket: two waits of 200 ms.
    assert!(timed(BucketMode::PerKey).await < Duration::from_millis(300));
    assert!(timed(BucketMode::Shared).await >= Duration::from_millis(350));
    let keys: HashSet<String> = seen(&server, "api_key").await.into_iter().collect();
    assert_eq!(keys.len(), 3);
}

#[tokio::test]
async fn higher_priority_waiters_go_first() {
    let server = MockServer::start().await;
    mount_json(&server, "user.getInfo", USER_JSON).await;
    let client = Arc::new(client(&server, &["k1"], 5, BucketMode::PerKey));
    let call = |lane: Priority, tag: &'static str| {
        let client = Arc::clone(&client);
        tokio::spawn(async move {
            let request = RawRequest::new("user.getInfo")
                .param("tag", tag)
                .priority(lane);
            client.raw(&request).await.unwrap();
        })
    };
    // The first call takes the only token; the second holds the turn while it waits for the
    // next one, so the three after it queue up in their lanes.
    call(Priority::Prewarm, "first").await.unwrap();
    let holder = call(Priority::Prewarm, "holder");
    tokio::time::sleep(Duration::from_millis(30)).await;
    let mut waiting = Vec::new();
    for (lane, tag) in [
        (Priority::Prewarm, "prewarm"),
        (Priority::Import, "import"),
        (Priority::Live, "live"),
        (Priority::Interactive, "interactive"),
    ] {
        waiting.push(call(lane, tag));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    holder.await.unwrap();
    for task in waiting {
        task.await.unwrap();
    }
    let order = seen(&server, "tag").await;
    assert_eq!(
        order,
        [
            "first",
            "holder",
            "interactive",
            "live",
            "import",
            "prewarm"
        ]
    );
}

#[tokio::test]
async fn raw_returns_the_body_untouched() {
    let server = MockServer::start().await;
    let body = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<lfm status=\"failed\">\n  \
                <error code=\"6\">User not found</error>\n</lfm>\n";
    Mock::given(query_param("method", "user.getInfo"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_raw(body.as_bytes().to_vec(), "text/xml; charset=UTF-8"),
        )
        .mount(&server)
        .await;
    let request = RawRequest::new("user.getInfo")
        .param("user", "nobody")
        .format(Format::Xml);
    let response = fast_client(&server).raw(&request).await.unwrap();
    assert_eq!(response.status, 404);
    assert_eq!(
        response.content_type.as_deref(),
        Some("text/xml; charset=UTF-8")
    );
    assert_eq!(response.body.as_ref(), body.as_bytes());
    assert_eq!(response.lastfm_error().unwrap().code, 6);
    assert_eq!(seen(&server, "format").await, [""]);
}

#[tokio::test]
async fn post_sends_a_form_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("method=user.getInfo"))
        .and(body_string_contains("api_key=k1"))
        .and(body_string_contains("format=json"))
        .respond_with(json(200, USER_JSON))
        .mount(&server)
        .await;
    let request = RawRequest::new("user.getInfo")
        .param("user", "alice")
        .format(Format::Json)
        .http_method(HttpMethod::Post);
    let response = fast_client(&server).raw(&request).await.unwrap();
    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn can_omit_the_key() {
    let server = MockServer::start().await;
    mount_json(&server, "user.getInfo", USER_JSON).await;
    let request = RawRequest::new("user.getInfo").without_api_key();
    fast_client(&server).raw(&request).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    assert!(requests[0].url.query_pairs().all(|(k, _)| k != "api_key"));
}
