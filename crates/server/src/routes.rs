//! HTTP routes: the Last.fm API at `/2.0` and `/2.0/`, `/healthz`, the web API at `/app/api`,
//! the `/api/auth` approval redirect, and the web UI for every other path.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use lfm_api::Dispatcher;
use lfm_core::Params;

use crate::{App, ui};

/// CORS headers Last.fm sends on every API response, preflight included.
const CORS: [(&str, &str); 3] = [
    ("access-control-allow-methods", "POST, GET, OPTIONS"),
    ("access-control-allow-origin", "*"),
    ("access-control-max-age", "86400"),
];

/// Where a reply came from: `local`, `cache`, `stale` or `upstream`.
const SOURCE: &str = "x-mirror-source";

/// The Last.fm API and `/healthz`.
pub fn router(dispatcher: Arc<Dispatcher>) -> Router {
    let api = get(api).post(api).options(preflight);
    Router::new()
        .route("/2.0", api.clone())
        .route("/2.0/", api)
        .route("/healthz", get(healthz))
        .with_state(dispatcher)
}

/// The full HTTP surface: [`router`], the web API and the web UI. Serve it with connect info,
/// which the login rate limit reads the client's address from.
pub fn app_router(app: &App) -> Router {
    router(Arc::clone(&app.dispatcher))
        .nest("/app/api", app.web.router())
        .merge(lfm_web::auth_page_redirect())
        .fallback(ui::serve)
}

async fn api(
    State(dispatcher): State<Arc<Dispatcher>>,
    method: Method,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let mut params = Params::new().with_urlencoded(query.unwrap_or_default().as_bytes());
    if method == Method::POST && is_form(&headers) {
        params = params.with_urlencoded(&body);
    }
    let reply = dispatcher.dispatch(&params).await;
    tracing::debug!(
        method = params.get("method").unwrap_or_default(),
        status = reply.status,
        "api request"
    );
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = (status, Body::from(reply.body)).into_response();
    let headers = response.headers_mut();
    set(headers, "content-type", &reply.content_type);
    let replayed = reply
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("access-control-allow-origin"));
    if !replayed {
        add_cors(headers);
    }
    for (name, value) in &reply.headers {
        set(headers, name, value);
    }
    headers.insert(SOURCE, HeaderValue::from_static(reply.source.as_str()));
    response
}

/// Sets a header from text, skipping one that is not valid HTTP, which only a broken upstream
/// answer could carry.
fn set(headers: &mut HeaderMap, name: &str, value: &str) {
    if let (Ok(name), Ok(value)) = (
        HeaderName::from_bytes(name.as_bytes()),
        HeaderValue::from_str(value),
    ) {
        headers.insert(name, value);
    } else {
        tracing::warn!(name, "skipping an invalid header");
    }
}

// Last.fm answers a preflight with an empty octet-stream body.
async fn preflight() -> Response {
    let mut response = (
        [(header::CONTENT_TYPE, "application/octet-stream")],
        Body::empty(),
    )
        .into_response();
    add_cors(response.headers_mut());
    response
}

async fn healthz() -> &'static str {
    "ok"
}

fn add_cors(headers: &mut HeaderMap) {
    for (name, value) in CORS {
        headers.insert(name, HeaderValue::from_static(value));
    }
}

// A POST without a content type is still read as a form, as most scrobblers send it that way.
fn is_form(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|v| v.starts_with("application/x-www-form-urlencoded"))
}
