//! Admin endpoints: accounts and instance settings.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use lfm_store::{AccountId, AccountStatus, Role, SignupMode};
use serde::{Deserialize, Serialize};

use crate::error::WebError;
use crate::session::Admin;
use crate::state::AppState;
use crate::views::AccountView;

/// `?status=pending|active|disabled`.
#[derive(Debug, Deserialize)]
pub(crate) struct StatusFilter {
    status: Option<String>,
}

/// `{"accounts": […]}`.
#[derive(Debug, Serialize)]
pub(crate) struct AccountList {
    accounts: Vec<AccountView>,
}

/// `GET /admin/accounts?status=`: oldest first.
pub(crate) async fn list(
    State(state): State<AppState>,
    _: Admin,
    Query(filter): Query<StatusFilter>,
) -> Result<Json<AccountList>, WebError> {
    let status = filter
        .status
        .map(|s| s.parse::<AccountStatus>())
        .transpose()
        .map_err(|_| WebError::invalid("invalid_status", "Use pending, active or disabled."))?;
    let accounts = state.store.list_accounts(status).await?;
    Ok(Json(AccountList {
        accounts: accounts.iter().map(AccountView::from).collect(),
    }))
}

/// `POST /admin/accounts/{id}/approve`: activates a pending account.
pub(crate) async fn approve(
    State(state): State<AppState>,
    _: Admin,
    Path(id): Path<i64>,
) -> Result<StatusCode, WebError> {
    state.store.approve_account(AccountId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /admin/accounts/{id}/reject`: deletes a pending account.
pub(crate) async fn reject(
    State(state): State<AppState>,
    _: Admin,
    Path(id): Path<i64>,
) -> Result<StatusCode, WebError> {
    state.store.reject_account(AccountId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /admin/accounts/{id}/disable`: stops the account's sync and rejects its API keys and
/// session keys at once. The last active admin cannot be disabled.
pub(crate) async fn disable(
    State(state): State<AppState>,
    _: Admin,
    Path(id): Path<i64>,
) -> Result<StatusCode, WebError> {
    let id = AccountId(id);
    state.store.disable_account(id).await?;
    state.refresh_keys().await;
    if let Some(name) = linked_user(&state, id).await?
        && let Some(user) = state.store.user_by_name(&name).await?
    {
        state.sync.unfollow(user.id).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /admin/accounts/{id}/enable`: re-activates a disabled account and resumes its sync.
pub(crate) async fn enable(
    State(state): State<AppState>,
    _: Admin,
    Path(id): Path<i64>,
) -> Result<StatusCode, WebError> {
    let id = AccountId(id);
    state.store.enable_account(id).await?;
    state.refresh_keys().await;
    if let Some(name) = linked_user(&state, id).await? {
        state.sync.follow(&name).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn linked_user(state: &AppState, id: AccountId) -> Result<Option<String>, WebError> {
    Ok(state
        .store
        .account(id)
        .await?
        .and_then(|account| account.lastfm_user))
}

/// `{"role": "admin" | "user"}`.
#[derive(Debug, Deserialize)]
pub(crate) struct RoleBody {
    role: String,
}

/// `PUT /admin/accounts/{id}/role`. The last active admin cannot be demoted.
pub(crate) async fn set_role(
    State(state): State<AppState>,
    _: Admin,
    Path(id): Path<i64>,
    Json(body): Json<RoleBody>,
) -> Result<StatusCode, WebError> {
    let role: Role = body
        .role
        .parse()
        .map_err(|_| WebError::invalid("invalid_role", "Use admin or user."))?;
    state.store.set_role(AccountId(id), role).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Instance settings. Keys are never shown, only how many there are.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Settings {
    signup_mode: &'static str,
    lastfm_key_pool: KeyPool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct KeyPool {
    /// Keys in the instance pool.
    keys: usize,
    /// Whether a shared secret is set, which web auth and forwarded writes need.
    can_sign: bool,
}

/// `GET /admin/settings`.
pub(crate) async fn settings(
    State(state): State<AppState>,
    _: Admin,
) -> Result<Json<Settings>, WebError> {
    current(&state).await.map(Json)
}

/// `{"signupMode": "approval" | "open" | "closed"}`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SettingsBody {
    signup_mode: String,
}

/// `PUT /admin/settings`.
pub(crate) async fn update_settings(
    State(state): State<AppState>,
    _: Admin,
    Json(body): Json<SettingsBody>,
) -> Result<Json<Settings>, WebError> {
    let mode: SignupMode = body
        .signup_mode
        .parse()
        .map_err(|_| WebError::invalid("invalid_signup_mode", "Use approval, open or closed."))?;
    state.store.set_signup_mode(mode).await?;
    current(&state).await.map(Json)
}

async fn current(state: &AppState) -> Result<Settings, WebError> {
    Ok(Settings {
        signup_mode: state.store.signup_mode().await?.as_str(),
        lastfm_key_pool: KeyPool {
            keys: state.config.key_pool_size,
            can_sign: state.config.lastfm_app.is_some(),
        },
    })
}
