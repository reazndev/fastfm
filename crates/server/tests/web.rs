//! The whole HTTP surface as the binary serves it: the web API under `/app/api`, keys made there
//! working on `/2.0/`, and the web UI behind every other path.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "tests fail on a broken store or an unexpected reply"
)]

use axum::body::Body;
use axum::http::{Request, header};
use http_body_util::BodyExt;
use lfm_mirror::{App, Config, app_router};
use lfm_store::Store;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Reply {
    status: u16,
    headers: axum::http::HeaderMap,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap()
    }

    fn header(&self, name: &str) -> &str {
        self.headers.get(name).unwrap().to_str().unwrap()
    }
}

/// The router, and the data directory that must outlive it.
async fn app() -> (axum::Router, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().to_string_lossy().into_owned();
    let store = Store::open_in_memory().unwrap();
    store.ensure_user("Syntiiix").await.unwrap();
    let config = Config::from_lookup(|name| match name {
        "LFM_DATA_DIR" => Some(data_dir.clone()),
        // A frozen clock runs no sync, so nothing calls Last.fm.
        "LFM_NOW" => Some("1794139201".into()),
        _ => None,
    })
    .unwrap();
    let app = App::new(store, &config).await.unwrap();
    (app_router(&app), dir)
}

async fn send(app: &axum::Router, request: Request<Body>) -> Reply {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8_lossy(&bytes).into_owned();
    Reply {
        status,
        headers,
        body,
    }
}

/// A web API call with the CSRF header, a JSON body and the session cookie.
async fn web(app: &axum::Router, method: &str, path: &str, cookie: &str, body: Value) -> Reply {
    let request = Request::builder()
        .method(method)
        .uri(format!("/app/api{path}"))
        .header("x-requested-with", "XMLHttpRequest")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie)
        .body(Body::from(body.to_string()))
        .unwrap();
    send(app, request).await
}

async fn get(app: &axum::Router, path: &str) -> Reply {
    send(app, Request::get(path).body(Body::empty()).unwrap()).await
}

async fn signup(app: &axum::Router, name: &str) -> (Value, String) {
    let body = json!({ "username": name, "password": "a long password" });
    let reply = web(app, "POST", "/signup", "", body).await;
    assert_eq!(reply.status, 201, "{}", reply.body);
    let cookie = reply.header("set-cookie").split(';').next().unwrap();
    (reply.json()["account"].clone(), cookie.to_owned())
}

#[tokio::test]
async fn a_key_created_in_the_web_api_reads_through_the_last_fm_api() {
    let (app, _dir) = app().await;
    let (admin, cookie) = signup(&app, "root").await;
    assert_eq!(
        (admin["role"].as_str(), admin["status"].as_str()),
        (Some("admin"), Some("active"))
    );
    let (second, _) = signup(&app, "guest").await;
    assert_eq!(second["status"], "pending");

    let body = json!({ "label": "test client", "scope": "read" });
    let created = web(&app, "POST", "/keys", &cookie, body).await;
    assert_eq!(created.status, 201, "{}", created.body);
    let key = created.json()["key"]["key"].as_str().unwrap().to_owned();

    let path = format!("/2.0/?method=user.getInfo&user=syntiiix&api_key={key}&format=json");
    let reply = get(&app, &path).await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert_eq!(reply.json()["user"]["name"], "Syntiiix");
}

#[tokio::test]
async fn the_apis_keep_priority_over_the_ui() {
    let (app, _dir) = app().await;
    let unknown = get(&app, "/app/api/nope").await;
    assert_eq!(unknown.status, 404);
    assert_eq!(unknown.json()["error"]["code"], "not_found");
    assert_eq!(get(&app, "/2.0/nope").await.status, 404);
    assert_eq!(get(&app, "/healthz").await.body, "ok");

    let redirect = get(&app, "/api/auth/?api_key=k&token=t").await;
    assert_eq!(redirect.status, 303);
    assert_eq!(redirect.header("location"), "/authorize?api_key=k&token=t");

    // UI routes get the page, built or not; missing files do not.
    let page = get(&app, "/keys").await;
    assert_eq!(page.status, 200);
    assert!(page.header("content-type").starts_with("text/html"));
    assert_eq!(page.header("cache-control"), "no-cache");
    assert_eq!(get(&app, "/main-ABCDEFGH.js").await.status, 404);
    let post = Request::post("/keys").body(Body::empty()).unwrap();
    assert_eq!(send(&app, post).await.status, 405);
}
