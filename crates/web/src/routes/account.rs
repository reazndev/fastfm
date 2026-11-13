//! Sign-up, login, logout, and the current account.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use lfm_api::methods::auth::random_hex;
use lfm_store::{AccountStatus, WebSession, fold};
use serde::Deserialize;

use crate::error::WebError;
use crate::password::{MAX_PASSWORD, check_password, check_username};
use crate::session::{ClientIp, SESSION_LIFETIME, Secure, Viewer, cookie, session_id};
use crate::state::{AppState, unix_now};
use crate::views::AccountBody;

/// A username and password.
#[derive(Debug, Deserialize)]
pub(crate) struct Credentials {
    username: String,
    password: String,
}

/// `POST /signup`: creates the account and logs it in. The first account becomes the admin;
/// later ones follow the sign-up mode.
pub(crate) async fn signup(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Secure(secure): Secure,
    Json(body): Json<Credentials>,
) -> Result<impl IntoResponse, WebError> {
    let now = unix_now();
    if !state.ip_attempts.allows(&ip, now) {
        return Err(WebError::rate_limited());
    }
    check_username(&body.username)?;
    check_password(&body.password)?;
    // Every sign-up costs a hash, so it counts against the IP like a failed login.
    state.ip_attempts.fail(&ip, now);
    let hash = state.passwords.hash(&body.password).await?;
    let account = state
        .store
        .create_account(&body.username, &hash, now)
        .await?;
    tracing::info!(
        account = account.id.0,
        status = account.status.as_str(),
        "signed up"
    );
    let set_cookie = start_session(&state, account.id, None, secure).await?;
    let headers = [(header::SET_COOKIE, set_cookie)];
    Ok((
        StatusCode::CREATED,
        headers,
        Json(AccountBody::of(&account)),
    ))
}

/// `POST /login`. Unknown names and wrong passwords get the same answer in about the same
/// time. Failures are limited per IP and per username.
pub(crate) async fn login(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Secure(secure): Secure,
    headers: HeaderMap,
    Json(body): Json<Credentials>,
) -> Result<impl IntoResponse, WebError> {
    let now = unix_now();
    let name_key = fold(&body.username);
    if !state.ip_attempts.allows(&ip, now) || !state.name_attempts.allows(&name_key, now) {
        return Err(WebError::rate_limited());
    }
    let account = state.store.account_by_name(&body.username).await?;
    let hash = account.as_ref().map(|a| a.password_hash.as_str());
    let verified =
        body.password.len() <= MAX_PASSWORD && state.passwords.verify(hash, &body.password).await;
    let Some(account) = account.filter(|_| verified) else {
        return Err(failed(&state, &ip, &name_key, now));
    };
    state.name_attempts.clear(&name_key);
    if account.status == AccountStatus::Disabled {
        return Err(WebError::disabled());
    }
    let old = session_id(&headers);
    let set_cookie = start_session(&state, account.id, old, secure).await?;
    Ok((
        [(header::SET_COOKIE, set_cookie)],
        Json(AccountBody::of(&account)),
    ))
}

fn failed(state: &AppState, ip: &str, name_key: &str, now: i64) -> WebError {
    state.ip_attempts.fail(ip, now);
    state.name_attempts.fail(name_key, now);
    WebError::bad_credentials()
}

/// `POST /logout`: ends the session and clears the cookie. Succeeds without a session too.
pub(crate) async fn logout(
    State(state): State<AppState>,
    Secure(secure): Secure,
    headers: HeaderMap,
) -> Result<impl IntoResponse, WebError> {
    if let Some(id) = session_id(&headers) {
        state.store.delete_session(&id).await?;
    }
    let headers = [(header::SET_COOKIE, cookie(None, secure)?)];
    Ok((StatusCode::NO_CONTENT, headers))
}

/// `GET /me`: the logged-in account, pending ones included.
pub(crate) async fn me(Viewer(account): Viewer) -> Json<AccountBody> {
    Json(AccountBody::of(&account))
}

/// Stores a new session, ending the one the browser had, and returns its `Set-Cookie` value.
async fn start_session(
    state: &AppState,
    account: lfm_store::AccountId,
    old: Option<String>,
    secure: bool,
) -> Result<axum::http::HeaderValue, WebError> {
    if let Some(old) = old {
        state.store.delete_session(&old).await?;
    }
    let now = unix_now();
    let session = WebSession {
        id: random_hex(32)?,
        account,
        created_at: now,
        expires_at: now + SESSION_LIFETIME,
    };
    let value = cookie(Some(&session.id), secure)?;
    state.store.create_session(session).await?;
    Ok(value)
}
