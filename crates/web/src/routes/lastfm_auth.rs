//! Last.fm web auth: the user approves this instance on last.fm, which proves they own the
//! linked profile and gives the session key that forwarded writes need.
//!
//! `GET /lastfm/auth/start` returns Last.fm's approval URL, with a random `state` bound to the
//! session in the callback URL. Last.fm sends the browser back to the UI route
//! `/lastfm/callback?state=…&token=…`, and the UI calls `GET /lastfm/auth/callback` with both
//! and the CSRF header. The cookie is `SameSite=Strict`, so it does not travel on Last.fm's
//! redirect itself, only on the UI's own request. A missing, foreign, expired or reused state
//! is refused, so a callback link someone else crafted cannot link their profile here.
//!
//! Last.fm confirming the user also proves ownership, so the link takes the name over from
//! another account that only claimed it.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, header};
use serde::{Deserialize, Serialize};

use crate::error::WebError;
use crate::link::{self, Linked};
use crate::routes::lastfm::{LinkStatus, link_status, relink};
use crate::session::{CsrfChecked, Member, Secure, stored_session};
use crate::state::{AppState, unix_now};

/// Where Last.fm's approval page sends the user back to, under the public URL.
pub const CALLBACK_ROUTE: &str = "/lastfm/callback";

/// `{"url": "https://www.last.fm/api/auth/?api_key=…&cb=…"}`.
#[derive(Debug, Serialize)]
pub(crate) struct StartBody {
    url: String,
}

/// `GET /lastfm/auth/start`.
pub(crate) async fn start(
    State(state): State<AppState>,
    Member(account): Member,
    Secure(secure): Secure,
    headers: HeaderMap,
) -> Result<Json<StartBody>, WebError> {
    let app = Linked::open(&state.vault, &account)
        .app(&state.config)
        .ok_or_else(not_configured)?;
    let session = stored_session(&headers).ok_or_else(WebError::not_logged_in)?;
    let flow = state.auth_states.start(&session, unix_now())?;
    let base = if let Some(url) = &state.config.public_url {
        url.clone()
    } else {
        let host = headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| WebError::invalid("no_host", "The request has no Host header."))?;
        let scheme = if secure { "https" } else { "http" };
        format!("{scheme}://{host}")
    };
    let callback = format!("{base}{CALLBACK_ROUTE}?state={flow}");
    let url = format!(
        "https://www.last.fm/api/auth/?api_key={}&cb={}",
        encode(app.api_key.expose()),
        encode(&callback)
    );
    Ok(Json(StartBody { url }))
}

/// `?state=…&token=…`: our state from the start, and Last.fm's token.
#[derive(Debug, Deserialize)]
pub(crate) struct CallbackQuery {
    #[serde(default)]
    state: String,
    token: String,
}

/// `GET /lastfm/auth/callback?token=`: trades the token for a session key, signed with the
/// same application as the start, stores it sealed, and links the confirmed Last.fm user.
pub(crate) async fn callback(
    State(state): State<AppState>,
    Member(account): Member,
    _: CsrfChecked,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> Result<Json<LinkStatus>, WebError> {
    let web_session = stored_session(&headers).ok_or_else(WebError::not_logged_in)?;
    if !state
        .auth_states
        .finish(&query.state, &web_session, unix_now())
    {
        return Err(WebError::invalid(
            "invalid_state",
            "This Last.fm link did not start here, or it expired. Connect Last.fm again.",
        ));
    }
    let app = Linked::open(&state.vault, &account)
        .app(&state.config)
        .ok_or_else(not_configured)?;
    let session = state
        .upstream
        .get_session(&query.token, &app)
        .await
        .map_err(|e| WebError::lastfm(&e))?;
    link::release_unconfirmed(&state, account.id, &session.name).await?;
    let account = relink(&state, account, &session.name).await?;
    link::update(&state.store, &state.vault, &account, |l| {
        l.session_key = Some(session.key);
    })
    .await?;
    tracing::info!(account = account.id.0, "Last.fm web auth done");
    let account = state
        .store
        .account(account.id)
        .await?
        .ok_or_else(WebError::not_found)?;
    link_status(&state, &account).await.map(Json)
}

fn not_configured() -> WebError {
    WebError::new(
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "lastfm_not_configured",
        "This instance has no Last.fm shared secret. Add your own API key first.",
    )
}

fn encode(text: &str) -> String {
    form_urlencoded::byte_serialize(text.as_bytes()).collect()
}
