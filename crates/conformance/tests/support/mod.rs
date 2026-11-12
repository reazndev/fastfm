//! An in-process server that replays the fixtures verbatim, with an optional mutation, and
//! helpers to run the harness against it.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail the test the same way the tests do"
)]
#![allow(
    dead_code,
    reason = "each test binary uses a different subset of the helpers"
)]

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Form, Query, State};
use axum::http::{HeaderName, HeaderValue, Response, StatusCode};
use axum::routing::get;
use lfm_conformance::client::Target;
use lfm_conformance::fixture::{self, Case, HttpMethod, REDACTED};
use lfm_conformance::ignore::Rules;
use lfm_conformance::judge::CaseResult;
use lfm_conformance::report::Report;
use lfm_conformance::run::{self, Pace};

/// The key the harness sends and the replay server accepts in place of `REDACTED`.
pub(crate) const KEY: &str = "selftest-key-0123456789";

/// What the replay server sends for one case, before it goes on the wire.
#[derive(Debug, Clone)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

/// Changes the reply of the case with this id.
pub(crate) type Mutation = Arc<dyn Fn(&str, &mut Reply) + Send + Sync>;

struct Replay {
    cases: Vec<Case>,
    mutate: Mutation,
}

/// Every fixture in the repository.
pub(crate) fn all_cases() -> Vec<Case> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    fixture::load(&dir).expect("fixtures load")
}

/// The fixtures whose id starts with `prefix`.
pub(crate) fn cases(prefix: &str) -> Vec<Case> {
    fixture::select(all_cases(), &[prefix.to_owned()], None)
}

/// Starts a replay server for `cases` and returns its API base URL.
pub(crate) async fn serve(cases: Vec<Case>, mutate: Mutation) -> String {
    let state = Arc::new(Replay { cases, mutate });
    let app = Router::new()
        .route("/2.0/", get(answer_get).post(answer_post))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("address");
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://{addr}/2.0/")
}

/// Replays `cases` with `mutate` applied and judges them with `rules`.
pub(crate) async fn run_with(cases: Vec<Case>, rules: &Rules, mutate: Mutation) -> Report {
    let base = serve(cases.clone(), mutate).await;
    let target = Target::new(&base, KEY).expect("target");
    let results = run::run(&target, &cases, rules, Pace::default()).await;
    Report::build(&base, &cases, results, rules)
}

/// No mutation.
pub(crate) fn verbatim() -> Mutation {
    Arc::new(|_, _| {})
}

/// Applies `change` to the reply of the case with id `id` only.
pub(crate) fn on(
    id: &'static str,
    change: impl Fn(&mut Reply) + Send + Sync + 'static,
) -> Mutation {
    Arc::new(move |case, reply| {
        if case == id {
            change(reply);
        }
    })
}

/// Replaces `from` with `to` in the body; panics when `from` is absent so a test cannot pass
/// vacuously.
pub(crate) fn replace_body(reply: &mut Reply, from: &str, to: &str) {
    let text = String::from_utf8(reply.body.clone()).expect("UTF-8 body");
    assert!(text.contains(from), "body does not contain {from:?}");
    reply.body = text.replace(from, to).into_bytes();
}

/// The result for one case id.
pub(crate) fn result<'a>(report: &'a Report, id: &str) -> &'a CaseResult {
    report
        .cases
        .iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("no result for {id}"))
}

async fn answer_get(
    State(state): State<Arc<Replay>>,
    Query(params): Query<Vec<(String, String)>>,
) -> Response<Body> {
    answer(&state, HttpMethod::Get, params)
}

async fn answer_post(
    State(state): State<Arc<Replay>>,
    Form(params): Form<Vec<(String, String)>>,
) -> Response<Body> {
    answer(&state, HttpMethod::Post, params)
}

fn answer(state: &Replay, method: HttpMethod, params: Vec<(String, String)>) -> Response<Body> {
    let params: Vec<(String, String)> = params
        .into_iter()
        .map(|(k, v)| match (k.as_str(), v.as_str()) {
            ("api_key", KEY) => (k, REDACTED.to_owned()),
            _ => (k, v),
        })
        .collect();
    let Some(case) = state
        .cases
        .iter()
        .find(|c| c.http_method == method && c.params == params)
    else {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("no fixture for this request"))
            .expect("response");
    };
    let mut reply = Reply {
        status: case.expected.status,
        headers: case
            .expected
            .headers
            .iter()
            .filter(|(name, _)| !matches!(name.as_str(), "content-length" | "date"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        body: case.expected.body.clone(),
    };
    (state.mutate)(&case.id(), &mut reply);
    let mut response = Response::new(Body::from(reply.body));
    *response.status_mut() = StatusCode::from_u16(reply.status).expect("status");
    for (name, value) in reply.headers {
        response.headers_mut().insert(
            HeaderName::try_from(name).expect("header name"),
            HeaderValue::try_from(value).expect("header value"),
        );
    }
    response
}
