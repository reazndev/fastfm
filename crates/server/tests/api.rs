//! The router in-process: exact bodies, statuses and headers, matching what Last.fm returned
//! for the same requests on 2026-11-12.
#![allow(clippy::unwrap_used, reason = "tests fail on a broken in-memory store")]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use lfm_api::methods::example::Echo;
use lfm_api::{
    Access, ApiKey, Context, Dispatcher, InMemoryApiKeys, InMemorySessions, MemoryCache, Method,
    Registry, Scope, Services, Session,
};
use lfm_core::{ApiError, Params, Response, api_signature};
use tower::ServiceExt;

const KEY: &str = "0123456789abcdef0123456789abcdef";
const SECRET: &str = "fedcba9876543210fedcba9876543210";
const SK: &str = "session0123456789abcdef012345678";
const XML: &str = "text/xml; charset=UTF-8";
const JSON: &str = "application/json";

/// A write method for the signature tests; `track.love` replies with an empty envelope.
struct Love;

#[async_trait]
impl Method for Love {
    fn name(&self) -> &'static str {
        "track.love"
    }

    fn access(&self) -> Access {
        Access::Session
    }

    async fn call(&self, _ctx: &Context<'_>) -> Result<Response, ApiError> {
        Ok(Response::empty())
    }
}

fn app() -> axum::Router {
    let key = ApiKey {
        account_id: 1,
        secret: SECRET.into(),
        scope: Scope::ReadWrite,
        active: true,
    };
    let session = Session {
        account_id: 1,
        user: "syntiiix".into(),
    };
    let services = Services {
        store: lfm_store::Store::open_in_memory().unwrap(),
        clock: lfm_api::Clock::System,
        api_keys: Arc::new(InMemoryApiKeys::default().with(KEY, key)),
        sessions: Arc::new(InMemorySessions::default().with(SK, session)),
        cache: Arc::new(MemoryCache::new(1 << 20, Duration::from_secs(60))),
    };
    let registry = Registry::new().with(Echo).with(Love);
    lfm_mirror::router(Arc::new(Dispatcher::new(registry, services)))
}

struct Reply {
    status: StatusCode,
    content_type: String,
    cors: [String; 3],
    body: String,
}

impl Reply {
    fn broken(error: &axum::http::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            content_type: String::new(),
            cors: Default::default(),
            body: format!("cannot build request: {error}"),
        }
    }
}

async fn send(request: axum::http::Result<Request<Body>>) -> Reply {
    let request = match request {
        Ok(request) => request,
        Err(error) => return Reply::broken(&error),
    };
    let Ok(response) = app().oneshot(request).await;
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    let content_type = header("content-type");
    let cors = [
        header("access-control-allow-methods"),
        header("access-control-allow-origin"),
        header("access-control-max-age"),
    ];
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .map(http_body_util::Collected::to_bytes)
        .unwrap_or_default();
    Reply {
        status,
        content_type,
        cors,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

async fn get(query: &str) -> Reply {
    send(Request::get(format!("/2.0/?{query}")).body(Body::empty())).await
}

async fn post(body: String) -> Reply {
    let request = Request::post("/2.0/")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body));
    send(request).await
}

fn xml_error(code: u8, message: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<lfm status=\"failed\">\n  \
         <error code=\"{code}\">{message}</error>\n</lfm>\n"
    )
}

fn json_error(code: u8, message: &str) -> String {
    format!(r#"{{"message":"{message}","error":{code}}}"#)
}

const INVALID_METHOD: &str = "Invalid Method - No method with that name in this package";
const MISSING_PARAM: &str = "Invalid parameters - Your request is missing a required parameter";
const INVALID_KEY: &str = "Invalid API key - You must be granted a valid key by last.fm";
const INVALID_SIG: &str = "Invalid method signature supplied";

fn assert_reply(reply: &Reply, status: u16, content_type: &str, body: &str) {
    assert_eq!(reply.status.as_u16(), status, "body: {}", reply.body);
    assert_eq!(reply.content_type, content_type);
    assert_eq!(reply.body, body);
    assert_eq!(reply.cors, ["POST, GET, OPTIONS", "*", "86400"]);
}

#[tokio::test]
async fn unknown_method() {
    let xml = get(&format!("method=user.nosuch&api_key={KEY}")).await;
    assert_reply(&xml, 400, XML, &xml_error(3, INVALID_METHOD));
    let json = get(&format!("method=user.nosuch&api_key={KEY}&format=json")).await;
    assert_reply(&json, 400, JSON, &json_error(3, INVALID_METHOD));
}

#[tokio::test]
async fn missing_method() {
    let xml = get(&format!("api_key={KEY}")).await;
    assert_reply(&xml, 400, XML, &xml_error(3, INVALID_METHOD));
    let json = get(&format!("api_key={KEY}&format=json")).await;
    assert_reply(&json, 400, JSON, &json_error(3, INVALID_METHOD));
}

#[tokio::test]
async fn missing_api_key_comes_before_unknown_method() {
    let xml = get("method=example.echo&text=hi").await;
    assert_reply(&xml, 400, XML, &xml_error(6, MISSING_PARAM));
    let json = get("method=user.nosuch&format=json").await;
    assert_reply(&json, 400, JSON, &json_error(6, MISSING_PARAM));
}

#[tokio::test]
async fn invalid_api_key_comes_after_unknown_method() {
    let bad = "ffffffffffffffffffffffffffffffff";
    let xml = get(&format!("method=example.echo&text=hi&api_key={bad}")).await;
    assert_reply(&xml, 403, XML, &xml_error(10, INVALID_KEY));
    let json = get(&format!(
        "method=example.echo&text=hi&api_key={bad}&format=json"
    ))
    .await;
    assert_reply(&json, 403, JSON, &json_error(10, INVALID_KEY));
    let unknown = get(&format!("method=user.nosuch&api_key={bad}&format=json")).await;
    assert_reply(&unknown, 400, JSON, &json_error(3, INVALID_METHOD));
}

#[tokio::test]
async fn xml_json_and_jsonp() {
    let query = format!("method=Example.ECHO&text=a/b&api_key={KEY}");
    let xml = get(&query).await;
    let expected = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<lfm status=\"ok\">\n  \
                    <echo page=\"1\" perPage=\"50\">\n    <text>a/b</text>\n  </echo>\n</lfm>\n";
    assert_reply(&xml, 200, XML, expected);
    let json = get(&format!("{query}&format=json")).await;
    let expected = r#"{"echo":{"text":"a/b","@attr":{"page":"1","perPage":"50"}}}"#;
    assert_reply(&json, 200, JSON, expected);
    let jsonp = get(&format!("{query}&format=json&callback=cb")).await;
    let content_type = "text/javascript; charset=utf-8";
    assert_reply(&jsonp, 200, content_type, &format!("cb({expected});"));
}

#[tokio::test]
async fn invalid_format_is_error_5_in_xml() {
    let reply = get(&format!(
        "method=example.echo&text=x&api_key={KEY}&format=bogus"
    ))
    .await;
    let message = "Invalid format - This service doesn&apos;t exist in that format";
    assert_reply(&reply, 400, XML, &xml_error(5, message));
}

#[tokio::test]
async fn post_with_form_body() {
    let reply = post(format!(
        "method=example.echo&text=hi&api_key={KEY}&format=json"
    ))
    .await;
    let expected = r#"{"echo":{"text":"hi","@attr":{"page":"1","perPage":"50"}}}"#;
    assert_reply(&reply, 200, JSON, expected);
}

#[tokio::test]
async fn form_body_overrides_query_and_path_without_slash_works() {
    let request = Request::post(format!("/2.0?method=example.echo&text=query&api_key={KEY}"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("text=body&format=json"));
    let reply = send(request).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.body.contains(r#""text":"body""#), "{}", reply.body);
}

fn love_params(sig: Option<&str>) -> Params {
    let params = Params::new()
        .with("method", "track.love")
        .with("artist", "Cher")
        .with("track", "Believe")
        .with("api_key", KEY)
        .with("sk", SK);
    let sig = sig.map_or_else(|| api_signature(&params, SECRET), str::to_owned);
    params.with("api_sig", sig)
}

fn form(params: &Params, format: &str) -> String {
    let mut out = form_urlencoded::Serializer::new(String::new());
    for (name, value) in params.effective() {
        out.append_pair(name, value);
    }
    out.append_pair("format", format);
    out.finish()
}

#[tokio::test]
async fn signed_write_with_bad_signature() {
    let params = love_params(Some("0123456789abcdef0123456789abcdef"));
    let xml = post(form(&params, "xml")).await;
    assert_reply(&xml, 400, XML, &xml_error(13, INVALID_SIG));
    let json = post(form(&params, "json")).await;
    assert_reply(&json, 400, JSON, &json_error(13, INVALID_SIG));
}

#[tokio::test]
async fn write_without_signature_is_missing_parameter() {
    let body = format!("method=track.love&artist=Cher&track=Believe&api_key={KEY}&format=json");
    let reply = post(body).await;
    assert_reply(&reply, 400, JSON, &json_error(6, MISSING_PARAM));
}

#[tokio::test]
async fn signed_write_with_good_signature_and_bad_session() {
    let ok = post(form(&love_params(None), "xml")).await;
    let expected = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<lfm status=\"ok\">\n</lfm>\n";
    assert_reply(&ok, 200, XML, expected);
    let ok_json = post(form(&love_params(None), "json")).await;
    assert_reply(&ok_json, 200, JSON, "{}");

    let params = Params::new()
        .with("method", "track.love")
        .with("api_key", KEY)
        .with("sk", "nope");
    let signed = params
        .clone()
        .with("api_sig", api_signature(&params, SECRET));
    let reply = post(form(&signed, "json")).await;
    let message = "Invalid session key - Please re-authenticate";
    assert_reply(&reply, 403, JSON, &json_error(9, message));
}

#[tokio::test]
async fn preflight_and_healthz() {
    let request = Request::options("/2.0/").body(Body::empty());
    let reply = send(request).await;
    assert_reply(&reply, 200, "application/octet-stream", "");
    let reply = send(Request::get("/healthz").body(Body::empty())).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, "ok");
}
