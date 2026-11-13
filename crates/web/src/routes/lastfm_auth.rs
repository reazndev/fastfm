//! Last.fm web auth: the user approves this instance on last.fm, which proves they own the
//! linked profile and gives the session key that forwarded writes need.
//!
//! `GET /lastfm/auth/start` returns Last.fm's approval URL. Last.fm sends the browser back to
//! the UI route `/lastfm/callback?token=…`, and the UI calls `GET /lastfm/auth/callback` with
//! that token and the CSRF header. The cookie is `SameSite=Strict`, so it does not travel on
//! Last.fm's redirect itself, only on the UI's own request.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, header};
use serde::{Deserialize, Serialize};

use crate::error::WebError;
use crate::link::{self, Linked};
use crate::routes::lastfm::{LinkStatus, link_status, relink};
use crate::session::{CsrfChecked, Member, Secure};
use crate::state::AppState;

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
    let app = Linked::open(&state.vault, &account)?
        .app(&state.config)
        .ok_or_else(not_configured)?;
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
    let callback = format!("{base}{CALLBACK_ROUTE}");
    let url = format!(
        "https://www.last.fm/api/auth/?api_key={}&cb={}",
        encode(app.api_key.expose()),
        encode(&callback)
    );
    Ok(Json(StartBody { url }))
}

/// `?token=…` from Last.fm.
#[derive(Debug, Deserialize)]
pub(crate) struct CallbackQuery {
    token: String,
}

/// `GET /lastfm/auth/callback?token=`: trades the token for a session key, signed with the
/// same application as the start, stores it sealed, and links the confirmed Last.fm user.
pub(crate) async fn callback(
    State(state): State<AppState>,
    Member(account): Member,
    _: CsrfChecked,
    Query(query): Query<CallbackQuery>,
) -> Result<Json<LinkStatus>, WebError> {
    let app = Linked::open(&state.vault, &account)?
        .app(&state.config)
        .ok_or_else(not_configured)?;
    let session = state
        .upstream
        .get_session(&query.token, &app)
        .await
        .map_err(|e| WebError::lastfm(&e))?;
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
