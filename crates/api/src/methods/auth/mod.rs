//! `auth.*`: Last.fm's token and session flow, emulated with local accounts.
//!
//! A client calls `auth.getToken`, sends the user to `/api/auth/?api_key=…&token=…` on this
//! host, where the web UI shows the request and the user approves it, then trades the token for
//! a session key with `auth.getSession`. `auth.getMobileSession` skips the page and takes the
//! local account's name and password. Session keys do not expire, as on Last.fm; revoking the
//! API key or disabling the account ends them.

mod get_mobile_session;
mod get_session;
mod get_token;
mod random;

pub use get_mobile_session::GetMobileSession;
pub use get_session::GetSession;
pub use get_token::GetToken;
pub use random::{RandomUnavailable, random_hex, random_token};

use lfm_core::{ApiError, ErrorCode, Node, Response, Text};
use lfm_store::{Account, ApiSession};

use crate::method::Context;
use crate::methods::shared::OrFailed;

/// How long a token from `auth.getToken` may wait for approval, as on Last.fm.
pub const TOKEN_LIFETIME: i64 = 3600;

impl From<RandomUnavailable> for ApiError {
    fn from(error: RandomUnavailable) -> Self {
        tracing::error!(%error, "cannot make a key");
        ErrorCode::OperationFailed.into()
    }
}

/// The calling API key, which the dispatcher has already checked.
fn api_key<'c>(ctx: &'c Context<'_>) -> Result<&'c str, ApiError> {
    ctx.params.required("api_key")
}

/// The request time as the store keeps it.
fn now(ctx: &Context<'_>) -> i64 {
    i64::try_from(ctx.now).unwrap_or(i64::MAX)
}

/// Issues a new session key for `account`, bound to the calling API key, and renders it.
async fn new_session(ctx: &Context<'_>, account: &Account) -> Result<Response, ApiError> {
    let session = ApiSession {
        session_key: random_hex(16)?,
        account: account.id,
        api_key: api_key(ctx)?.to_owned(),
        created_at: now(ctx),
    };
    ctx.services
        .store
        .create_api_session(session.clone())
        .await
        .or_failed()?;
    Ok(session_response(account, &session.session_key))
}

/// `<session><name/><key/><subscriber/></session>`, where `name` is the linked Last.fm user.
fn session_response(account: &Account, key: &str) -> Response {
    let name = account
        .lastfm_user
        .clone()
        .unwrap_or_else(|| account.name.clone());
    // Unverified against Last.fm: JSON order and `subscriber` as a bare number come from
    // published client logs, not from a recorded fixture.
    let session = Node::new("session")
        .child(Node::leaf("name", name))
        .child(Node::leaf("key", key.to_owned()))
        .child(Node::leaf("subscriber", Text::number(0)))
        .json_order(&["subscriber", "name", "key"]);
    Response::new(session)
}
