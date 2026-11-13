//! The read-through cache against a mock Last.fm: replay, single flight, stale-while-revalidate,
//! failures, the `getInfo` overlay and pass-through for users this instance does not mirror.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "tests fail on a broken mock or store"
)]

#[allow(dead_code, reason = "this binary uses only part of the shared helpers")]
mod support;

use std::sync::Arc;
use std::time::Duration;

use lfm_api::{Dispatcher, Passthrough, Reply, Source};
use lfm_core::Params;
use lfm_store::{CachedResponse, Store};
use lfm_upstream::{ApiKey, Client, ClientConfig, RetryPolicy};
use wiremock::matchers::query_param;
use wiremock::{Mock, MockServer, ResponseTemplate};

const ARTIST_JSON: &str = r#"{"artist":{"name":"Cher","url":"https://www.last.fm/music/Cher"}}"#;
const UTF8_JSON: &str = "application/json; charset=UTF-8";

fn client(server: &MockServer) -> Arc<Client> {
    let config = ClientConfig::new([ApiKey::new("instance-key")])
        .base_url(format!("{}/2.0/", server.uri()))
        .requests_per_second(std::num::NonZeroU32::new(1000).unwrap())
        .retry(RetryPolicy {
            max_attempts: 2,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
        });
    Arc::new(Client::new(config).unwrap())
}

fn dispatcher(store: Store, server: &MockServer) -> Dispatcher {
    let passthrough = Passthrough::new(store.clone(), client(server));
    support::dispatcher(store).with_passthrough(passthrough)
}

/// Last.fm answering `method` with `body`, its JSON content type and CORS headers, after `delay`.
fn answer(status: u16, body: &str, delay: Duration) -> ResponseTemplate {
    ResponseTemplate::new(status)
        .set_body_raw(body.as_bytes().to_vec(), UTF8_JSON)
        .insert_header("access-control-allow-origin", "*")
        .insert_header("access-control-allow-methods", "POST, GET, PUT, OPTIONS")
        .set_delay(delay)
}

async fn mount(server: &MockServer, method: &str, template: ResponseTemplate) {
    Mock::given(query_param("method", method))
        .respond_with(template)
        .mount(server)
        .await;
}

async fn calls(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().len()
}

fn query(pairs: &[(&str, &str)]) -> Params {
    pairs
        .iter()
        .fold(Params::new().with("api_key", support::KEY), |p, (k, v)| {
            p.with(*k, *v)
        })
}

fn artist_info(artist: &str) -> Params {
    query(&[
        ("method", "artist.getInfo"),
        ("artist", artist),
        ("format", "json"),
    ])
}

fn body(reply: &Reply) -> &str {
    std::str::from_utf8(&reply.body).unwrap()
}

#[tokio::test]
async fn a_second_identical_request_makes_no_upstream_call() {
    let server = MockServer::start().await;
    mount(
        &server,
        "artist.getInfo",
        answer(200, ARTIST_JSON, Duration::ZERO),
    )
    .await;
    let d = dispatcher(Store::open_in_memory().unwrap(), &server);
    let first = d.dispatch(&artist_info("Cher")).await;
    assert_eq!(first.source, Source::Upstream);
    // Another spelling of the same artist hits the same entry.
    let second = d.dispatch(&artist_info("cHER")).await;
    assert_eq!(second.source, Source::Cache);
    assert_eq!(calls(&server).await, 1);
    for reply in [&first, &second] {
        assert_eq!((reply.status, body(reply)), (200, ARTIST_JSON));
        assert_eq!(reply.content_type, UTF8_JSON);
        assert!(
            reply
                .headers
                .iter()
                .any(|(n, v)| n == "access-control-allow-methods" && v.contains("PUT"))
        );
    }
    let request = &server.received_requests().await.unwrap()[0];
    let sent: Vec<(String, String)> = request.url.query_pairs().into_owned().collect();
    assert!(sent.contains(&("api_key".into(), "instance-key".into())));
    assert!(!sent.iter().any(|(_, v)| v == support::KEY), "{sent:?}");
}

#[tokio::test]
async fn json_and_xml_are_cached_apart_and_jsonp_is_wrapped_here() {
    let server = MockServer::start().await;
    mount(
        &server,
        "artist.getInfo",
        answer(200, ARTIST_JSON, Duration::ZERO),
    )
    .await;
    let d = dispatcher(Store::open_in_memory().unwrap(), &server);
    let jsonp = d
        .dispatch(&artist_info("Cher").with("callback", "cb"))
        .await;
    assert_eq!(body(&jsonp), format!("cb({ARTIST_JSON});"));
    assert_eq!(jsonp.content_type, "text/javascript; charset=utf-8");
    let plain = d.dispatch(&artist_info("Cher")).await;
    assert_eq!((plain.source, body(&plain)), (Source::Cache, ARTIST_JSON));
    let xml = query(&[("method", "artist.getInfo"), ("artist", "Cher")]);
    assert_eq!(d.dispatch(&xml).await.source, Source::Upstream);
    let sent = server.received_requests().await.unwrap();
    assert_eq!(sent.len(), 2);
    assert!(
        !sent
            .iter()
            .any(|r| r.url.query().unwrap().contains("callback"))
    );
}

#[tokio::test]
async fn concurrent_identical_requests_share_one_call() {
    let server = MockServer::start().await;
    let slow = answer(200, ARTIST_JSON, Duration::from_millis(200));
    mount(&server, "artist.getInfo", slow).await;
    let d = Arc::new(dispatcher(Store::open_in_memory().unwrap(), &server));
    let tasks: Vec<_> = (0..10)
        .map(|_| {
            let d = Arc::clone(&d);
            tokio::spawn(async move { d.dispatch(&artist_info("Cher")).await })
        })
        .collect();
    for task in tasks {
        let reply = task.await.unwrap();
        assert_eq!((reply.status, body(&reply)), (200, ARTIST_JSON));
    }
    assert_eq!(calls(&server).await, 1);
}

#[tokio::test]
async fn a_stale_entry_is_served_at_once_and_refreshed_once() {
    let server = MockServer::start().await;
    let fresh = answer(200, ARTIST_JSON, Duration::from_millis(200));
    mount(&server, "artist.getInfo", fresh).await;
    let store = Store::open_in_memory().unwrap();
    let old = CachedResponse {
        body: br#"{"artist":{"name":"old"}}"#.to_vec(),
        status: 200,
        content_type: "application/json".into(),
        headers: "access-control-allow-origin: *".into(),
        fetched_at: 1,
        expires_at: 2,
    };
    store
        .cache_put("artist.getinfo?artist=cher", "json", old)
        .await
        .unwrap();
    let d = dispatcher(store.clone(), &server);
    for _ in 0..5 {
        let reply = d.dispatch(&artist_info("Cher")).await;
        assert_eq!(reply.source, Source::Stale);
        assert_eq!(body(&reply), r#"{"artist":{"name":"old"}}"#);
        assert_eq!(reply.content_type, "application/json");
    }
    let refreshed = async {
        loop {
            let reply = d.dispatch(&artist_info("Cher")).await;
            if reply.source == Source::Cache {
                return reply;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let reply = tokio::time::timeout(Duration::from_secs(5), refreshed)
        .await
        .unwrap();
    assert_eq!(body(&reply), ARTIST_JSON);
    assert_eq!(calls(&server).await, 1);
    let stored = store
        .cache_get("artist.getinfo?artist=cher", "json")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.body, ARTIST_JSON.as_bytes());
}

#[tokio::test]
async fn unknown_methods_never_reach_last_fm() {
    let server = MockServer::start().await;
    let d = dispatcher(Store::open_in_memory().unwrap(), &server);
    for method in [
        "artist.noSuchMethod",
        "user.noSuchMethod",
        "track.noSuchMethod",
    ] {
        let reply = d.dispatch(&query(&[("method", method)])).await;
        assert_eq!(reply.status, 400, "{method}");
        assert!(body(&reply).contains("<error code=\"3\">"), "{method}");
        assert_eq!(reply.source, Source::Local);
    }
    assert_eq!(calls(&server).await, 0);
}

#[tokio::test]
async fn failures_give_error_29_or_16_and_are_not_cached() {
    let server = MockServer::start().await;
    let limited = r#"{"message":"Rate Limit Exceeded","error":29}"#;
    mount(
        &server,
        "artist.getInfo",
        answer(429, limited, Duration::ZERO),
    )
    .await;
    mount(&server, "track.getInfo", answer(503, "", Duration::ZERO)).await;
    let d = dispatcher(Store::open_in_memory().unwrap(), &server);
    let reply = d.dispatch(&artist_info("Cher")).await;
    assert_eq!(reply.status, 429);
    assert!(body(&reply).contains("\"error\":29"), "{}", body(&reply));
    let track = query(&[("method", "track.getInfo"), ("artist", "A"), ("track", "B")]);
    let reply = d.dispatch(&track).await;
    assert!(
        body(&reply).contains("<error code=\"16\">"),
        "{}",
        body(&reply)
    );
    // Two attempts each, then again on the next request, since failures are not kept.
    d.dispatch(&track).await;
    assert_eq!(calls(&server).await, 6);
}

#[tokio::test]
async fn last_fm_errors_are_replayed_with_their_status() {
    let server = MockServer::start().await;
    let missing =
        r#"{"error":6,"message":"The artist you supplied could not be found","links":[]}"#;
    mount(
        &server,
        "artist.getInfo",
        answer(200, missing, Duration::ZERO),
    )
    .await;
    let d = dispatcher(Store::open_in_memory().unwrap(), &server);
    let reply = d.dispatch(&artist_info("zzz")).await;
    assert_eq!((reply.status, body(&reply)), (200, missing));
    assert_eq!(d.dispatch(&artist_info("zzz")).await.source, Source::Cache);
    assert_eq!(calls(&server).await, 1);
}

#[tokio::test]
async fn get_info_with_a_mirrored_user_shows_local_counts() {
    let server = MockServer::start().await;
    for format in ["json", "xml"] {
        let recorded = support::recorded("track.getInfo", "username", format);
        let template = ResponseTemplate::new(200)
            .set_body_raw(recorded.body.into_bytes(), recorded.content_type.as_str());
        let format_matcher = query_param("method", "track.getInfo");
        let mock = Mock::given(format_matcher).and(query_param("username", "syntiiix"));
        let mock = if format == "json" {
            mock.and(query_param("format", "json"))
        } else {
            mock.and(wiremock::matchers::query_param_is_missing("format"))
        };
        mock.respond_with(template).mount(&server).await;
    }
    let (store, user) = support::store().await;
    let d = dispatcher(store.clone(), &server);
    let lookup = lfm_store::Lookup::Track {
        artist: "Juice WRLD".into(),
        name: "Doom".into(),
    };
    let local = store.user_plays(user, lookup).await.unwrap();
    assert!(local.id.is_some());
    for format in ["json", "xml"] {
        let want = support::recorded("track.getInfo", "username", format);
        let reply = d.dispatch(&want.params).await;
        let (plays, loved) = (local.plays.to_string(), u8::from(local.loved).to_string());
        let expected = match format {
            "json" => want
                .body
                .replace(
                    "\"userplaycount\":\"7462\"",
                    &format!("\"userplaycount\":\"{plays}\""),
                )
                .replace("\"userloved\":\"1\"", &format!("\"userloved\":\"{loved}\"")),
            _ => want
                .body
                .replace(">7462<", &format!(">{plays}<"))
                .replace("<userloved>1<", &format!("<userloved>{loved}<")),
        };
        assert_eq!(body(&reply), expected, "{format}");
        assert_eq!(reply.content_type, want.content_type);
    }
}

#[tokio::test]
async fn local_user_methods_forward_only_users_not_mirrored_here() {
    let server = MockServer::start().await;
    let elsewhere = r#"{"user":{"name":"RJ"}}"#;
    mount(
        &server,
        "user.getInfo",
        answer(200, elsewhere, Duration::ZERO),
    )
    .await;
    let (store, _) = support::store().await;
    let d = dispatcher(store, &server);
    let info = |user: &str| {
        query(&[
            ("method", "user.getInfo"),
            ("user", user),
            ("format", "json"),
        ])
    };
    let mirrored = d.dispatch(&info("SYNTIIIX")).await;
    assert_eq!(mirrored.source, Source::Local);
    let forwarded = d.dispatch(&info("RJ")).await;
    assert_eq!(
        (forwarded.source, body(&forwarded)),
        (Source::Upstream, elsewhere)
    );
    // Without `user` the local "User not found" stands.
    let missing = d.dispatch(&query(&[("method", "user.getInfo")])).await;
    assert_eq!((missing.status, missing.source), (404, Source::Local));
    assert_eq!(calls(&server).await, 1);
}
