//! HTTP routes: the Last.fm API at `/2.0` and `/2.0/`, the history export at `/2.0/export`,
//! `/healthz`, the web API at `/app/api`, the `/api/auth` approval redirect, and the web UI for
//! every other path.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use lfm_api::{Dispatcher, Encoding, NDJSON, Reply};
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

/// The Last.fm API, the export and `/healthz`.
pub fn router(dispatcher: Arc<Dispatcher>) -> Router {
    let api = get(api).post(api).options(preflight);
    Router::new()
        .route("/2.0", api.clone())
        .route("/2.0/", api)
        .route("/2.0/export", get(export).options(preflight))
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
    respond(&reply, &headers)
}

/// Turns a reply into HTTP. Beyond what Last.fm sends, a success carries an `ETag`; a client
/// that sends `Accept-Encoding` gets the best coding it accepts and `Vary`, and one whose
/// `If-None-Match` names the body gets a 304 without it. A request with neither header gets
/// Last.fm's headers and bytes.
fn respond(reply: &Reply, request: &HeaderMap) -> Response {
    let accept = text(request, header::ACCEPT_ENCODING);
    let wanted = accept.map_or(Encoding::Identity, Encoding::negotiate);
    let (bytes, encoding) = reply.body.encoded(wanted);
    let not_modified = reply.status == 200
        && text(request, header::IF_NONE_MATCH).is_some_and(|tags| reply.body.matches(tags));
    let mut response = if not_modified {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let status =
            StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut response = (status, Body::from(bytes)).into_response();
        set(response.headers_mut(), "content-type", &reply.content_type);
        if let Some(coding) = encoding.header() {
            let headers = response.headers_mut();
            headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static(coding));
        }
        response
    };
    let headers = response.headers_mut();
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
    if reply.status == 200 {
        set(headers, "etag", &reply.body.etag(encoding));
    }
    if accept.is_some() {
        headers.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    }
    headers.insert(SOURCE, HeaderValue::from_static(reply.source.as_str()));
    response
}

/// Streams a user's history as NDJSON, one store page at a time. A store failure midway
/// aborts the response, so a client never takes a cut-off export for a complete one.
async fn export(State(dispatcher): State<Arc<Dispatcher>>, RawQuery(query): RawQuery) -> Response {
    let params = Params::new().with_urlencoded(query.unwrap_or_default().as_bytes());
    let export = match dispatcher.export(&params).await {
        Ok(export) => export,
        Err(reply) => return respond(&reply, &HeaderMap::new()),
    };
    let chunks = futures_util::stream::unfold(export, |mut export| async move {
        let chunk = export.next_chunk().await?;
        let chunk = chunk.map_err(|error| {
            tracing::error!(%error, "export failed");
            std::io::Error::other(error)
        });
        Some((chunk, export))
    });
    let mut response = Body::from_stream(chunks).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(NDJSON));
    add_cors(headers);
    response
}

fn text(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
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
