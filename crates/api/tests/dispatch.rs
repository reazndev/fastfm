//! Dispatcher behaviour that the HTTP tests in `lfm-mirror` do not cover: key states, scopes
//! and the response cache.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use lfm_api::{
    Access, ApiKey, Context, Dispatcher, InMemoryApiKeys, InMemorySessions, MemoryCache, Method,
    Registry, ResponseCache, Scope, Services, Session,
};
use lfm_core::{ApiError, Node, Params, Response, api_signature};

const SECRET: &str = "secret";

/// Counts calls and returns the count, cached per `user`.
#[derive(Default)]
struct Counter(AtomicU32);

#[async_trait]
impl Method for Counter {
    fn name(&self) -> &'static str {
        "test.count"
    }

    fn cache_user<'p>(&self, params: &'p Params) -> Option<&'p str> {
        params.optional("user")
    }

    async fn call(&self, _ctx: &Context<'_>) -> Result<Response, ApiError> {
        let n = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(Response::new(Node::leaf("count", n)))
    }
}

struct Write;

#[async_trait]
impl Method for Write {
    fn name(&self) -> &'static str {
        "test.write"
    }

    fn access(&self) -> Access {
        Access::Session
    }

    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError> {
        let user = ctx.session.map(|s| s.user.clone()).unwrap_or_default();
        Ok(Response::new(Node::leaf("user", user)))
    }
}

struct Signed;

#[async_trait]
impl Method for Signed {
    fn name(&self) -> &'static str {
        "test.signed"
    }

    fn access(&self) -> Access {
        Access::Signed
    }

    async fn call(&self, _ctx: &Context<'_>) -> Result<Response, ApiError> {
        Ok(Response::empty())
    }
}

fn key(scope: Scope, active: bool) -> ApiKey {
    ApiKey {
        account_id: 1,
        secret: SECRET.into(),
        scope,
        active,
    }
}

fn dispatcher(cache: Arc<MemoryCache>) -> Dispatcher {
    let keys = InMemoryApiKeys::default()
        .with("rw", key(Scope::ReadWrite, true))
        .with("ro", key(Scope::Read, true))
        .with("off", key(Scope::ReadWrite, false));
    let session = Session {
        account_id: 1,
        user: "syntiiix".into(),
    };
    let services = Services {
        api_keys: Arc::new(keys),
        sessions: Arc::new(InMemorySessions::default().with("sk", session)),
        cache,
    };
    let registry = Registry::new()
        .with(Counter::default())
        .with(Write)
        .with(Signed);
    Dispatcher::new(registry, services)
}

fn signed(params: Params) -> Params {
    let sig = api_signature(&params, SECRET);
    params.with("api_sig", sig)
}

async fn body(d: &Dispatcher, params: &Params) -> (u16, String) {
    let reply = d.dispatch(params).await;
    (
        reply.status,
        String::from_utf8_lossy(&reply.body).into_owned(),
    )
}

fn new_cache() -> Arc<MemoryCache> {
    Arc::new(MemoryCache::new(1 << 20, Duration::from_secs(60)))
}

#[tokio::test]
async fn inactive_key_is_suspended() {
    let d = dispatcher(new_cache());
    let params = Params::new()
        .with("method", "test.count")
        .with("api_key", "off")
        .with("format", "json");
    let (status, body) = body(&d, &params).await;
    assert_eq!(status, 403);
    assert!(body.ends_with(r#","error":26}"#), "{body}");
}

#[tokio::test]
async fn write_needs_write_scope_and_gets_the_session() {
    let d = dispatcher(new_cache());
    let base = |key: &str| {
        Params::new()
            .with("method", "test.write")
            .with("api_key", key)
            .with("sk", "sk")
    };
    let (status, allowed) = body(&d, &signed(base("rw")).with("format", "json")).await;
    assert_eq!((status, allowed.as_str()), (200, r#"{"user":"syntiiix"}"#));
    let (status, refused) = body(&d, &signed(base("ro")).with("format", "json")).await;
    assert_eq!(status, 403);
    assert!(refused.ends_with(r#","error":4}"#), "{refused}");
}

#[tokio::test]
async fn signed_method_needs_api_sig_but_no_session() {
    let d = dispatcher(new_cache());
    let base = Params::new()
        .with("method", "test.signed")
        .with("api_key", "ro");
    assert_eq!(body(&d, &base).await.0, 400);
    assert_eq!(body(&d, &signed(base)).await.0, 200);
}

#[tokio::test]
async fn cache_serves_repeats_until_the_user_is_invalidated() {
    let cache = new_cache();
    let d = dispatcher(Arc::clone(&cache));
    let params = Params::new()
        .with("method", "test.count")
        .with("api_key", "rw")
        .with("user", "Syntiiix")
        .with("format", "json");
    assert_eq!(body(&d, &params).await.1, r#"{"count":"1"}"#);
    assert_eq!(body(&d, &params).await.1, r#"{"count":"1"}"#);
    let jsonp = params.clone().with("callback", "cb");
    assert_eq!(body(&d, &jsonp).await.1, r#"cb({"count":"1"});"#);
    let xml = params.clone().with("format", "xml");
    assert!(body(&d, &xml).await.1.contains("<count>2</count>"));
    cache.invalidate_user("syntiiix");
    assert_eq!(body(&d, &params).await.1, r#"{"count":"3"}"#);
}
