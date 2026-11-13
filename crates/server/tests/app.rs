//! The wired service: keys from the store and from `LFM_DEV_API_KEY`, the frozen clock, and the
//! extra header of modern-style methods.
#![allow(clippy::unwrap_used, reason = "tests fail on a broken in-memory store")]

use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use lfm_mirror::{App, Config};
use lfm_store::{KeyScope, NewApiKey, Store};
use tower::ServiceExt;

/// Sun 2026-11-08 12:00 UTC plus one second: the last listed week has just ended.
const NOW: &str = "1794139201";

/// The router, and the data directory that must outlive it.
async fn app() -> (axum::Router, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().to_string_lossy().into_owned();
    let store = Store::open_in_memory().unwrap();
    store.ensure_user("Syntiiix").await.unwrap();
    let account = store.create_account("root", "hash", 1).await.unwrap();
    let key = NewApiKey {
        key: "stored".into(),
        secret: "s".into(),
        label: "test".into(),
        scope: KeyScope::Read,
    };
    store.create_api_key(account.id, key, 1).await.unwrap();
    let config = Config::from_lookup(|name| match name {
        "LFM_NOW" => Some(NOW.into()),
        "LFM_DEV_API_KEY" => Some("dev".into()),
        "LFM_DATA_DIR" => Some(data_dir.clone()),
        _ => None,
    })
    .unwrap();
    let app = App::new(store, &config).await.unwrap();
    (lfm_mirror::router(app.dispatcher), dir)
}

async fn get(app: &axum::Router, query: &str) -> (u16, Option<String>, String) {
    let request = Request::get(format!("/2.0/?{query}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let extra = response
        .headers()
        .get("access-control-request-headers")
        .map(|v| v.to_str().unwrap().to_owned());
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, extra, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn dev_and_stored_keys_work_and_unknown_keys_do_not() {
    let (app, _dir) = app().await;
    let query = |key: &str| format!("method=user.getWeeklyChartList&user=syntiiix&api_key={key}");
    let (status, extra, body) = get(&app, &query("dev")).await;
    assert_eq!(status, 200);
    assert!(
        extra
            .unwrap()
            .starts_with("Origin, X-Atmosphere-tracking-id")
    );
    // The frozen clock decides the last listed week.
    assert!(body.ends_with(
        "<chart from=\"1793534400\" to=\"1794139200\"></chart>\n</weeklychartlist>\n</lfm>\n"
    ));
    assert_eq!(get(&app, &query("stored")).await.0, 200);
    assert_eq!(get(&app, &query("nope")).await.0, 403);
}

#[tokio::test]
async fn unmirrored_user_is_not_found() {
    let (app, _dir) = app().await;
    let (status, _, body) = get(&app, "method=user.getInfo&user=someone&api_key=dev").await;
    assert_eq!(status, 404);
    assert!(
        body.contains("<error code=\"6\">User not found</error>"),
        "{body}"
    );
}

#[tokio::test]
async fn replies_say_where_they_came_from() {
    let (app, _dir) = app().await;
    let request = Request::get("/2.0/?method=user.getInfo&user=syntiiix&api_key=dev")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let source = response.headers().get("x-mirror-source").unwrap();
    assert_eq!(source, "local");
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .unwrap(),
        "*"
    );
}
