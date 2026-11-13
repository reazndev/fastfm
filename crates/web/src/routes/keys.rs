//! The account's own API keys for the Last.fm-compatible API.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use lfm_api::methods::auth::random_hex;
use lfm_store::NewApiKey;
use serde::{Deserialize, Serialize};

use crate::error::WebError;
use crate::session::Member;
use crate::state::{AppState, unix_now};
use crate::views::{KeyView, Scope};

/// Longest label accepted.
const MAX_LABEL: usize = 64;

/// `{"keys": […]}`.
#[derive(Debug, Serialize)]
pub(crate) struct KeyList {
    keys: Vec<KeyView>,
}

/// `GET /keys`: oldest first, secrets included.
pub(crate) async fn list(
    State(state): State<AppState>,
    Member(account): Member,
) -> Result<Json<KeyList>, WebError> {
    let keys = state.store.api_keys(account.id).await?;
    Ok(Json(KeyList {
        keys: keys.into_iter().map(KeyView::from).collect(),
    }))
}

/// `{"label": "…", "scope": "read" | "read_write"}`.
#[derive(Debug, Deserialize)]
pub(crate) struct NewKey {
    label: String,
    scope: Scope,
}

/// `{"key": …}`.
#[derive(Debug, Serialize)]
pub(crate) struct KeyBody {
    key: KeyView,
}

/// `POST /keys`: a new key and secret, 32 hex characters each, from the OS random source.
pub(crate) async fn create(
    State(state): State<AppState>,
    Member(account): Member,
    Json(body): Json<NewKey>,
) -> Result<(StatusCode, Json<KeyBody>), WebError> {
    let label = body.label.trim();
    if label.is_empty() || label.chars().count() > MAX_LABEL {
        return Err(WebError::invalid(
            "invalid_label",
            "Use a label of 1 to 64 characters.",
        ));
    }
    let key = NewApiKey {
        key: random_hex(16)?,
        secret: random_hex(16)?,
        label: label.to_owned(),
        scope: body.scope.into(),
    };
    let key = state
        .store
        .create_api_key(account.id, key, unix_now())
        .await?;
    state.refresh_keys().await;
    Ok((StatusCode::CREATED, Json(KeyBody { key: key.into() })))
}

/// `DELETE /keys/{key}`: revokes the key and every session key issued to it.
pub(crate) async fn revoke(
    State(state): State<AppState>,
    Member(account): Member,
    Path(key): Path<String>,
) -> Result<StatusCode, WebError> {
    state.store.revoke_api_key(account.id, &key).await?;
    state.refresh_keys().await;
    Ok(StatusCode::NO_CONTENT)
}
