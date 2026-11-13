//! The login cookie, the extractors that find the logged-in account, and the CSRF header rule.
//!
//! The cookie holds an opaque random id that the store maps to an account. It is `HttpOnly`,
//! `SameSite=Strict`, and `Secure` per [`CookiePolicy`]. State-changing requests must also carry
//! an `X-Requested-With` header: a cross-site form cannot set one, and a cross-site script
//! cannot send one without a CORS preflight, which this API never grants.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use lfm_store::{Account, AccountStatus, Role};

use crate::config::CookiePolicy;
use crate::error::WebError;
use crate::state::{AppState, unix_now};

/// The cookie's name.
pub(crate) const COOKIE: &str = "lfm_session";
/// How long a login lasts, in seconds.
pub(crate) const SESSION_LIFETIME: i64 = 30 * 86_400;
/// The header every state-changing request must carry.
pub(crate) const CSRF_HEADER: &str = "x-requested-with";

/// The session id in the request's cookies.
pub(crate) fn session_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|cookies| cookies.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE)
        .map(|(_, value)| value.to_owned())
}

/// The key a session is stored under: the SHA-256 of the cookie value, so a leaked database
/// does not hand out logins.
pub(crate) fn stored_id(cookie_value: &str) -> String {
    use sha2::Digest;
    use std::fmt::Write;
    sha2::Sha256::digest(cookie_value.as_bytes()).iter().fold(
        String::with_capacity(64),
        |mut hex, byte| {
            // Writing to a String cannot fail.
            let _ = write!(hex, "{byte:02x}");
            hex
        },
    )
}

/// The stored key of the request's session, if it has a cookie.
pub(crate) fn stored_session(headers: &HeaderMap) -> Option<String> {
    session_id(headers).map(|id| stored_id(&id))
}

/// `Set-Cookie` for a new session, or one that clears it when `id` is `None`.
pub(crate) fn cookie(id: Option<&str>, secure: bool) -> Result<HeaderValue, WebError> {
    let (value, age) = id.map_or(("", 0), |id| (id, SESSION_LIFETIME));
    let secure = if secure { "; Secure" } else { "" };
    let text =
        format!("{COOKIE}={value}; Path=/; Max-Age={age}; HttpOnly; SameSite=Strict{secure}");
    HeaderValue::from_str(&text).map_err(|error| WebError::internal(&error))
}

/// Whether the cookie should be `Secure` for this request.
fn secure(state: &AppState, parts: &Parts) -> bool {
    match state.config.cookies {
        CookiePolicy::Always => true,
        CookiePolicy::Never => false,
        CookiePolicy::Auto => {
            parts.uri.scheme_str() == Some("https")
                || (state.config.trust_proxy
                    && header_text(&parts.headers, "x-forwarded-proto")
                        .is_some_and(|proto| proto.eq_ignore_ascii_case("https")))
        }
    }
}

fn header_text<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
}

/// Whether a cookie set in this response should be `Secure`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Secure(pub(crate) bool);

impl FromRequestParts<AppState> for Secure {
    type Rejection = WebError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, WebError>> + Send {
        std::future::ready(Ok(Self(secure(state, parts))))
    }
}

/// A logged-in account that is not disabled. Pending accounts pass; see [`Member`].
#[derive(Debug)]
pub(crate) struct Viewer(pub(crate) Account);

impl FromRequestParts<AppState> for Viewer {
    type Rejection = WebError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, WebError> {
        let id = stored_session(&parts.headers).ok_or_else(WebError::not_logged_in)?;
        let session = state
            .store
            .session(&id, unix_now())
            .await?
            .ok_or_else(WebError::not_logged_in)?;
        let account = state
            .store
            .account(session.account)
            .await?
            .ok_or_else(WebError::not_logged_in)?;
        if account.status == AccountStatus::Disabled {
            return Err(WebError::disabled());
        }
        Ok(Self(account))
    }
}

/// A logged-in, active account: what every endpoint but `/me` and `/logout` needs.
#[derive(Debug)]
pub(crate) struct Member(pub(crate) Account);

impl FromRequestParts<AppState> for Member {
    type Rejection = WebError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, WebError> {
        let Viewer(account) = Viewer::from_request_parts(parts, state).await?;
        match account.status {
            AccountStatus::Active => Ok(Self(account)),
            AccountStatus::Pending => Err(WebError::pending()),
            AccountStatus::Disabled => Err(WebError::disabled()),
        }
    }
}

/// An active admin.
#[derive(Debug)]
pub(crate) struct Admin;

impl FromRequestParts<AppState> for Admin {
    type Rejection = WebError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, WebError> {
        let Member(account) = Member::from_request_parts(parts, state).await?;
        if account.role == Role::Admin {
            Ok(Self)
        } else {
            Err(WebError::admin_only())
        }
    }
}

/// The client's IP, for the login rate limit. Needs the server to run with connect info. When
/// the proxy is trusted, the `X-Forwarded-For` entry its last trusted hop added wins: entries
/// left of it come from the client, which can write anything there.
#[derive(Debug)]
pub(crate) struct ClientIp(pub(crate) String);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = WebError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, WebError>> + Send {
        let forwarded = state
            .config
            .trust_proxy
            .then(|| header_text(&parts.headers, "x-forwarded-for"))
            .flatten()
            .and_then(|list| forwarded_client(list, state.config.proxy_hops));
        let direct = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0.ip().to_string());
        let ip = forwarded.or(direct).unwrap_or_else(|| "unknown".to_owned());
        std::future::ready(Ok(Self(ip)))
    }
}

/// The address `hops` trusted proxies saw: the `hops`-th entry from the right, or the leftmost
/// when the list is shorter.
fn forwarded_client(list: &str, hops: usize) -> Option<String> {
    let entries: Vec<&str> = list
        .split(',')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .collect();
    let index = entries.len().saturating_sub(hops.max(1));
    entries.get(index).map(|ip| (*ip).to_owned())
}

/// Proof that the request carries the CSRF header, for the one `GET` that changes state.
#[derive(Debug)]
pub(crate) struct CsrfChecked;

impl FromRequestParts<AppState> for CsrfChecked {
    type Rejection = WebError;

    fn from_request_parts(
        parts: &mut Parts,
        _: &AppState,
    ) -> impl Future<Output = Result<Self, WebError>> + Send {
        let checked = if parts.headers.contains_key(CSRF_HEADER) {
            Ok(Self)
        } else {
            Err(csrf_missing())
        };
        std::future::ready(checked)
    }
}

/// Middleware: refuses `POST`, `PUT`, `PATCH` and `DELETE` without the CSRF header.
pub(crate) async fn require_csrf_header(request: Request, next: Next) -> Response {
    let safe = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    if safe || request.headers().contains_key(CSRF_HEADER) {
        next.run(request).await
    } else {
        csrf_missing().into_response()
    }
}

fn csrf_missing() -> WebError {
    WebError::new(
        axum::http::StatusCode::FORBIDDEN,
        "csrf_header_missing",
        "Send the X-Requested-With header with this request.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_address_is_the_one_the_trusted_proxy_added() {
        let list = "6.6.6.6, 10.0.0.7, 192.0.2.1";
        assert_eq!(forwarded_client(list, 1).as_deref(), Some("192.0.2.1"));
        assert_eq!(forwarded_client(list, 2).as_deref(), Some("10.0.0.7"));
        assert_eq!(forwarded_client(list, 9).as_deref(), Some("6.6.6.6"));
        assert_eq!(forwarded_client(" ", 1), None);
    }

    #[test]
    fn sessions_are_stored_hashed() {
        let stored = stored_id("abc");
        assert_eq!(stored.len(), 64);
        assert_ne!(stored, "abc");
        assert_eq!(stored, stored_id("abc"));
    }
}
