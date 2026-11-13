//! A web API and a dispatcher over one temporary store, with Last.fm mocked by `wiremock`.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::missing_panics_doc,
    reason = "test helpers fail the test the same way the tests do"
)]
#![allow(
    unreachable_pub,
    dead_code,
    reason = "shared by test binaries through `mod support`"
)]

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::RawQuery;
use axum::http::{HeaderMap, Request, header};
use axum::response::IntoResponse;
use http_body_util::BodyExt;
use lfm_api::{Clock, Dispatcher, MemoryCache, ResponseCache, Services, StoreKeys, StoreSessions};
use lfm_core::{Params, api_signature};
use lfm_store::Store;
use lfm_sync::{SyncConfig, SyncService};
use lfm_upstream::{ApiKey, AppCredentials, Client, ClientConfig, RetryPolicy, Secret};
use lfm_web::{DemoConfig, Web, WebConfig, WebDeps};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;
use wiremock::MockServer;

pub mod mock;

/// A request budget the tests never reach.
const UNLIMITED: NonZeroU32 = NonZeroU32::new(100_000).unwrap();

/// The instance's Last.fm application, as the mock knows it.
pub const LASTFM_KEY: &str = "1111111111111111111111111111111a";
pub const LASTFM_SECRET: &str = "2222222222222222222222222222222b";
/// The key the speed comparison sends to the mirror.
pub const INTERNAL_KEY: &str = "3333333333333333333333333333333c";
/// The demo import's history cap in these tests.
pub const DEMO_MAX_SCROBBLES: u64 = 1000;

/// Everything a test drives.
pub struct Harness {
    pub web: Web,
    pub sync: Arc<SyncService>,
    pub app: Router,
    pub store: Store,
    pub dispatcher: Dispatcher,
    pub lastfm: MockServer,
    _dir: TempDir,
}

impl Harness {
    pub async fn new() -> Self {
        Self::build(false, None).await
    }

    /// With a setup token that the first sign-up must give.
    pub async fn with_setup_token(token: &str) -> Self {
        Self::build(false, Some(token)).await
    }

    /// With the speed comparison on, timing the mock Last.fm and the dispatcher served over
    /// loopback.
    pub async fn with_demo() -> Self {
        Self::build(true, None).await
    }

    async fn build(demo: bool, setup_token: Option<&str>) -> Self {
        let lastfm = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let client = lastfm_client(&lastfm);
        let sync = Arc::new(SyncService::new(
            store.clone(),
            Arc::clone(&client),
            SyncConfig::default().request_budgets(UNLIMITED, UNLIMITED),
        ));
        let keys = Arc::new(
            StoreKeys::load(store.clone(), internal_key())
                .await
                .unwrap(),
        );
        let cache: Arc<dyn ResponseCache> =
            Arc::new(MemoryCache::new(1 << 20, Duration::from_secs(60)));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = web_config(&dir, setup_token);
        if demo {
            let mirror = listener.local_addr().unwrap();
            config = config.demo(demo_config(&lastfm, mirror, Arc::clone(&cache)));
        }
        let deps = WebDeps {
            store: store.clone(),
            sync: Arc::clone(&sync),
            upstream: client,
            keys: Arc::clone(&keys),
        };
        let web = Web::start(config, deps).await.unwrap();
        let services = Services {
            store: store.clone(),
            clock: Clock::System,
            api_keys: keys,
            sessions: Arc::new(StoreSessions::new(store.clone())),
            cache,
        };
        let registry =
            lfm_api::methods::with_accounts(lfm_api::methods::registry(), &web.accounts());
        let app = Router::new()
            .nest("/app/api", web.router())
            .merge(lfm_web::auth_page_redirect());
        let dispatcher = Dispatcher::new(registry, services);
        tokio::spawn(serve_api(listener, dispatcher.clone()));
        Self {
            web,
            sync,
            app,
            store,
            dispatcher,
            lastfm,
            _dir: dir,
        }
    }

    /// A request to the web API, built by chaining.
    pub fn call(&self, method: &str, path: &str) -> Call<'_> {
        Call {
            harness: self,
            method: method.to_owned(),
            path: path.to_owned(),
            body: None,
            cookie: None,
            csrf: true,
            headers: Vec::new(),
        }
    }

    /// Signs up `name` and returns the session cookie.
    pub async fn signup(&self, name: &str) -> String {
        let reply = self
            .call("POST", "/app/api/signup")
            .json(json!({ "username": name, "password": "a long password" }))
            .send()
            .await;
        assert_eq!(reply.status, 201, "{}", reply.json);
        reply.cookie().unwrap()
    }

    /// A Last.fm API call in JSON, unsigned.
    pub async fn api(&self, pairs: &[(&str, &str)]) -> (u16, Value) {
        let params = pairs
            .iter()
            .fold(Params::new().with("format", "json"), |p, (n, v)| {
                p.with(*n, *v)
            });
        self.dispatch(&params).await
    }

    /// A Last.fm API call in JSON, signed with `secret`.
    pub async fn signed(&self, pairs: &[(&str, &str)], secret: &str) -> (u16, Value) {
        let params = pairs.iter().fold(Params::new(), |p, (n, v)| p.with(*n, *v));
        let sig = api_signature(&params, secret);
        self.dispatch(&params.with("api_sig", sig).with("format", "json"))
            .await
    }

    async fn dispatch(&self, params: &Params) -> (u16, Value) {
        let reply = self.dispatcher.dispatch(params).await;
        let json = serde_json::from_slice(&reply.body).unwrap_or(Value::Null);
        (reply.status, json)
    }

    /// Last.fm requests the mock saw for `method`, as their form or query pairs.
    pub async fn lastfm_calls(&self, method: &str) -> Vec<HashMap<String, String>> {
        self.lastfm
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| {
                let mut pairs: HashMap<String, String> = r.url.query_pairs().into_owned().collect();
                pairs.extend(form_urlencoded::parse(&r.body).into_owned());
                pairs
            })
            .filter(|pairs| pairs.get("method").map(String::as_str) == Some(method))
            .collect()
    }
}

/// The `state` in the callback URL of a Last.fm approval URL from `GET /lastfm/auth/start`.
pub fn callback_state(approval_url: &str) -> String {
    let query = approval_url.split_once('?').unwrap().1;
    let cb = form_urlencoded::parse(query.as_bytes())
        .find(|(name, _)| name == "cb")
        .unwrap()
        .1
        .into_owned();
    let cb_query = cb.split_once('?').unwrap().1;
    form_urlencoded::parse(cb_query.as_bytes())
        .find(|(name, _)| name == "state")
        .unwrap()
        .1
        .into_owned()
}

/// The web API's settings: the instance application, one pool key, and the setup token.
fn web_config(dir: &TempDir, setup_token: Option<&str>) -> WebConfig {
    let config = WebConfig::new(dir.path())
        .lastfm_app(AppCredentials {
            api_key: ApiKey::new(LASTFM_KEY),
            secret: Secret::new(LASTFM_SECRET),
        })
        .key_pool_size(1);
    match setup_token {
        Some(token) => config.setup_token(token),
        None => config,
    }
}

/// The Last.fm client, pointed at the mock, fast and with short retries.
fn lastfm_client(lastfm: &MockServer) -> Arc<Client> {
    let client = ClientConfig::new([ApiKey::new(LASTFM_KEY)])
        .base_url(format!("{}/2.0/", lastfm.uri()))
        .requests_per_second(NonZeroU32::new(1000).unwrap())
        .retry(RetryPolicy {
            max_attempts: 2,
            base_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(10),
        });
    Arc::new(Client::new(client).unwrap())
}

/// The comparison timing the mock Last.fm and the mirror at `mirror`.
fn demo_config(
    lastfm: &MockServer,
    mirror: std::net::SocketAddr,
    cache: Arc<dyn ResponseCache>,
) -> DemoConfig {
    let mirror_url = format!("http://{mirror}/2.0/");
    DemoConfig::new(ApiKey::new(LASTFM_KEY), mirror_url, INTERNAL_KEY, cache)
        .lastfm_url(format!("{}/2.0/", lastfm.uri()))
        .max_scrobbles(DEMO_MAX_SCROBBLES)
}

/// [`INTERNAL_KEY`], registered in code as the server registers its own.
fn internal_key() -> HashMap<String, lfm_api::ApiKey> {
    let record = lfm_api::ApiKey {
        account_id: 0,
        secret: INTERNAL_KEY.to_owned(),
        scope: lfm_api::Scope::Read,
        active: true,
    };
    HashMap::from([(INTERNAL_KEY.to_owned(), record)])
}

/// `/2.0/` over HTTP, with the `X-Mirror-Source` header, as the server crate serves it.
async fn serve_api(listener: tokio::net::TcpListener, dispatcher: Dispatcher) {
    let dispatcher = Arc::new(dispatcher);
    let api = |RawQuery(query): RawQuery| async move {
        let params = Params::new().with_urlencoded(query.unwrap_or_default().as_bytes());
        let reply = dispatcher.dispatch(&params).await;
        let status = axum::http::StatusCode::from_u16(reply.status).unwrap();
        let source = [("x-mirror-source", reply.source.as_str())];
        (status, source, reply.body.bytes().clone()).into_response()
    };
    let app = Router::new().route("/2.0/", axum::routing::get(api));
    axum::serve(listener, app).await.unwrap();
}

/// One web API request.
pub struct Call<'h> {
    harness: &'h Harness,
    method: String,
    path: String,
    body: Option<Value>,
    cookie: Option<String>,
    csrf: bool,
    headers: Vec<(String, String)>,
}

impl Call<'_> {
    pub fn json(self, body: Value) -> Self {
        Self {
            body: Some(body),
            ..self
        }
    }

    pub fn cookie(self, cookie: &str) -> Self {
        Self {
            cookie: Some(cookie.to_owned()),
            ..self
        }
    }

    pub fn without_csrf(self) -> Self {
        Self {
            csrf: false,
            ..self
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub async fn send(self) -> Reply {
        let mut request = Request::builder()
            .method(self.method.as_str())
            .uri(&self.path);
        if let Some(cookie) = &self.cookie {
            request = request.header(header::COOKIE, cookie);
        }
        if self.csrf {
            request = request.header("x-requested-with", "XMLHttpRequest");
        }
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        let body = match &self.body {
            Some(body) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(body.to_string())
            }
            None => Body::empty(),
        };
        let response = self
            .harness
            .app
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            json: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        }
    }
}

/// A web API response.
#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub headers: HeaderMap,
    pub json: Value,
}

impl Reply {
    /// The `name=value` part of the `Set-Cookie` header, for sending back.
    pub fn cookie(&self) -> Option<String> {
        self.set_cookie()
            .and_then(|c| c.split(';').next().map(str::to_owned))
    }

    /// The whole `Set-Cookie` header.
    pub fn set_cookie(&self) -> Option<String> {
        self.headers
            .get(header::SET_COOKIE)
            .map(|v| v.to_str().unwrap().to_owned())
    }

    /// The error code of an error response.
    pub fn code(&self) -> &str {
        self.json["error"]["code"].as_str().unwrap_or_default()
    }
}

/// Polls `check` every 10 ms until it holds, failing after five seconds.
pub async fn eventually<F, Fut>(mut check: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    for _ in 0..500 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not reached in time");
}
