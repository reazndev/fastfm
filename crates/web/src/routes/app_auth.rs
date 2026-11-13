//! The approval page of the emulated Last.fm auth flow.
//!
//! A Last.fm client calls `auth.getToken` and opens `/api/auth/?api_key=…&token=…` on this host,
//! as it would open last.fm. The server redirects that to the UI route `/authorize` with the
//! same query; the UI shows which application asks (`GET /auth/request`) and the user approves
//! it (`POST /auth/approve`). The client then calls `auth.getSession`.

use axum::Json;
use axum::extract::{Query, RawQuery, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use lfm_store::StoreError;
use serde::{Deserialize, Serialize};

use crate::error::WebError;
use crate::session::Member;
use crate::state::{AppState, unix_now};

/// The UI route that shows the approval page.
pub const AUTHORIZE_ROUTE: &str = "/authorize";

/// `?token=…`.
#[derive(Debug, Deserialize)]
pub(crate) struct TokenQuery {
    token: String,
}

/// Which application asks for access.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthRequest {
    api_key: String,
    /// The key's label, or `null` for a key registered in the server's environment.
    label: Option<String>,
    /// The username of the account that owns the key.
    owner: Option<String>,
    expires_at: i64,
    /// Whether this account already approved the token.
    approved: bool,
}

/// `GET /auth/request?token=`.
pub(crate) async fn request(
    State(state): State<AppState>,
    Member(account): Member,
    Query(query): Query<TokenQuery>,
) -> Result<Json<AuthRequest>, WebError> {
    let token = state
        .store
        .find_auth_token(&query.token)
        .await?
        .ok_or_else(token_not_found)?;
    if token.expires_at <= unix_now() {
        return Err(WebError::new(
            StatusCode::GONE,
            "token_expired",
            "This request has expired. Start again from the app.",
        ));
    }
    let key = state
        .store
        .active_api_keys()
        .await?
        .into_iter()
        .find(|key| key.key == token.api_key);
    let owner = match &key {
        Some(key) => state.store.account(key.account).await?.map(|a| a.name),
        None => None,
    };
    Ok(Json(AuthRequest {
        label: key.map(|key| key.label),
        owner,
        expires_at: token.expires_at,
        approved: token.account == Some(account.id),
        api_key: token.api_key,
    }))
}

/// `{"token": "…"}`.
#[derive(Debug, Deserialize)]
pub(crate) struct ApproveBody {
    token: String,
}

/// `POST /auth/approve`: lets the application act for this account.
pub(crate) async fn approve(
    State(state): State<AppState>,
    Member(account): Member,
    Json(body): Json<ApproveBody>,
) -> Result<StatusCode, WebError> {
    let token = state
        .store
        .find_auth_token(&body.token)
        .await?
        .ok_or_else(token_not_found)?;
    if token.account.is_some_and(|other| other != account.id) {
        return Err(WebError::conflict(
            "token_taken",
            "Another account already approved this request.",
        ));
    }
    let approved = state
        .store
        .authorize_auth_token(&body.token, account.id, unix_now())
        .await;
    match approved {
        Err(StoreError::NotFound) => return Err(token_not_found()),
        other => other?,
    }
    Ok(StatusCode::NO_CONTENT)
}

fn token_not_found() -> WebError {
    WebError::new(
        StatusCode::NOT_FOUND,
        "token_not_found",
        "This request is unknown or has expired. Start again from the app.",
    )
}

/// `GET /api/auth/?…`: sends Last.fm clients to the UI's approval route with the same query.
pub(crate) async fn redirect(RawQuery(query): RawQuery) -> Response {
    let target = match query.filter(|q| !q.is_empty()) {
        Some(query) => format!("{AUTHORIZE_ROUTE}?{query}"),
        None => AUTHORIZE_ROUTE.to_owned(),
    };
    match HeaderValue::from_str(&target) {
        Ok(location) => (StatusCode::SEE_OTHER, [(header::LOCATION, location)]).into_response(),
        Err(_) => StatusCode::BAD_REQUEST.into_response(),
    }
}
