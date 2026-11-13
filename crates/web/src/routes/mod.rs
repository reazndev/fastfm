//! The routes under `/app/api`.

pub(crate) mod account;
pub(crate) mod admin;
pub(crate) mod app_auth;
pub(crate) mod keys;
pub(crate) mod lastfm;
pub(crate) mod lastfm_auth;

use axum::Router;
use axum::middleware;
use axum::routing::{delete, get, post, put};

use crate::session::require_csrf_header;
use crate::state::AppState;

/// Every endpoint, with the CSRF rule applied.
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/signup", post(account::signup))
        .route("/login", post(account::login))
        .route("/logout", post(account::logout))
        .route("/me", get(account::me))
        .route("/admin/accounts", get(admin::list))
        .route("/admin/accounts/{id}/approve", post(admin::approve))
        .route("/admin/accounts/{id}/reject", post(admin::reject))
        .route("/admin/accounts/{id}/disable", post(admin::disable))
        .route("/admin/accounts/{id}/enable", post(admin::enable))
        .route("/admin/accounts/{id}/role", put(admin::set_role))
        .route(
            "/admin/settings",
            get(admin::settings).put(admin::update_settings),
        )
        .route("/keys", get(keys::list).post(keys::create))
        .route("/keys/{key}", delete(keys::revoke))
        .route(
            "/lastfm",
            get(lastfm::status).put(lastfm::link).delete(lastfm::unlink),
        )
        .route(
            "/lastfm/key",
            put(lastfm::set_own_key).delete(lastfm::clear_own_key),
        )
        .route("/lastfm/auth/start", get(lastfm_auth::start))
        .route("/lastfm/auth/callback", get(lastfm_auth::callback))
        .route("/auth/request", get(app_auth::request))
        .route("/auth/approve", post(app_auth::approve))
        .layer(middleware::from_fn(require_csrf_header))
}
