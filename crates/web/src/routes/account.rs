//! Sign-up, login, logout, and the current account.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use lfm_api::methods::auth::random_hex;
use lfm_store::{AccountStatus, WebSession, fold};
use serde::{Deserialize, Serialize};

use crate::attempts::Origin;
use crate::error::WebError;
use crate::password::{MAX_PASSWORD, check_password, check_username};
use crate::session::{
    ClientIp, SESSION_LIFETIME, Secure, Viewer, cookie, stored_id, stored_session,
};
use crate::state::{AppState, unix_now};
use crate::views::AccountBody;

/// A username and password.
#[derive(Debug, Deserialize)]
pub(crate) struct Credentials {
    username: String,
    password: String,
}

/// A sign-up: credentials, and the setup token for the first account when one is configured.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignupBody {
    username: String,
    password: String,
    #[serde(default)]
    setup_token: Option<String>,
}

/// `{"mode": "approval" | "open" | "closed", "hasAccounts": bool, "setupTokenRequired": bool}`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignupModeView {
    mode: &'static str,
    has_accounts: bool,
    setup_token_required: bool,
}

/// `GET /signup-mode`, public: what the sign-up form will do, before anyone fills it in.
pub(crate) async fn signup_mode(
    State(state): State<AppState>,
) -> Result<Json<SignupModeView>, WebError> {
    let has_accounts = has_accounts(&state).await?;
    Ok(Json(SignupModeView {
        mode: state.store.signup_mode().await?.as_str(),
        has_accounts,
        setup_token_required: !has_accounts && state.config.setup_token.is_some(),
    }))
}

async fn has_accounts(state: &AppState) -> Result<bool, WebError> {
    Ok(!state.store.list_accounts(None).await?.is_empty())
}

/// Refuses the first sign-up without the configured setup token.
async fn check_setup_token(state: &AppState, given: Option<&str>) -> Result<(), WebError> {
    let Some(expected) = &state.config.setup_token else {
        return Ok(());
    };
    if has_accounts(state).await? {
        return Ok(());
    }
    let given = given.unwrap_or_default().trim();
    // Compares every byte, so the time taken does not show how much of a guess matched.
    let same = given.len() == expected.len()
        && given
            .bytes()
            .zip(expected.bytes())
            .fold(0, |diff, (a, b)| diff | (a ^ b))
            == 0;
    if same {
        Ok(())
    } else {
        Err(WebError::new(
            StatusCode::FORBIDDEN,
            "setup_token_invalid",
            "Enter the setup token from the server's configuration.",
        ))
    }
}

/// `POST /signup`: creates the account and logs it in. The first account becomes the admin,
/// and must give the setup token when one is configured; later ones follow the sign-up mode.
pub(crate) async fn signup(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Secure(secure): Secure,
    Json(body): Json<SignupBody>,
) -> Result<impl IntoResponse, WebError> {
    let now = unix_now();
    if !state.attempts.allows_ip(&ip, now) {
        return Err(WebError::rate_limited());
    }
    check_username(&body.username)?;
    check_password(&body.password)?;
    // Every sign-up costs a hash, so it counts against the IP like a failed login.
    state.attempts.count_ip(&ip, now);
    check_setup_token(&state, body.setup_token.as_deref()).await?;
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
    if !state.attempts.admit(&name_key, Origin::Web(&ip), now).await {
        return Err(WebError::rate_limited());
    }
    let account = state.store.account_by_name(&body.username).await?;
    let hash = account.as_ref().map(|a| a.password_hash.as_str());
    let verified =
        body.password.len() <= MAX_PASSWORD && state.passwords.verify(hash, &body.password).await;
    let Some(account) = account.filter(|_| verified) else {
        state.attempts.failed(&name_key, Origin::Web(&ip), now);
        return Err(WebError::bad_credentials());
    };
    state.attempts.succeeded(&name_key, Origin::Web(&ip));
    if account.status == AccountStatus::Disabled {
        return Err(WebError::disabled());
    }
    let old = stored_session(&headers);
    let set_cookie = start_session(&state, account.id, old, secure).await?;
    Ok((
        [(header::SET_COOKIE, set_cookie)],
        Json(AccountBody::of(&account)),
    ))
}

/// `POST /logout`: ends the session and clears the cookie. Succeeds without a session too.
pub(crate) async fn logout(
    State(state): State<AppState>,
    Secure(secure): Secure,
    headers: HeaderMap,
) -> Result<impl IntoResponse, WebError> {
    if let Some(id) = stored_session(&headers) {
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
    let id = random_hex(32)?;
    let session = WebSession {
        id: stored_id(&id),
        account,
        created_at: now,
        expires_at: now + SESSION_LIFETIME,
    };
    let value = cookie(Some(&id), secure)?;
    state.store.create_session(session).await?;
    Ok(value)
}
