//! The `Method` trait every Last.fm method implements, and the context it receives.

use std::sync::Arc;

use async_trait::async_trait;
use lfm_core::{ApiError, Params, Response, Style};
use lfm_store::{MirroredUser, Store};

use crate::auth::{ApiKey, ApiKeys, Session, Sessions};
use crate::cache::ResponseCache;
use crate::clock::Clock;
use crate::now_playing::NowPlayingSource;

/// What a method needs from the caller before it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Access {
    /// An API key. A signed request may also carry `sk`.
    Read,
    /// An API key and a valid `api_sig`, as `auth.getSession` needs.
    Signed,
    /// An API key with write scope, a valid `api_sig` and a valid `sk`, as writes need.
    Session,
}

/// Shared services that methods use.
#[derive(Clone)]
pub struct Services {
    /// The database.
    pub store: Store,
    /// What "now" is for methods.
    pub clock: Clock,
    /// API key lookup.
    pub api_keys: Arc<dyn ApiKeys>,
    /// Session key lookup.
    pub sessions: Arc<dyn Sessions>,
    /// Rendered response cache.
    pub cache: Arc<dyn ResponseCache>,
}

impl std::fmt::Debug for Services {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Services").finish_non_exhaustive()
    }
}

/// Everything a method call can see.
#[derive(Debug)]
pub struct Context<'a> {
    /// Request parameters from the query string and the form body.
    pub params: &'a Params,
    /// The API key that made the call.
    pub api_key: &'a ApiKey,
    /// The session, present for [`Access::Session`] methods and signed calls that send `sk`.
    pub session: Option<&'a Session>,
    /// Shared services.
    pub services: &'a Services,
    /// The request time in Unix seconds, for `period` ranges and dates.
    pub now: u64,
    /// The mirrored user named by [`Method::cache_user`], when the store has them.
    pub user: Option<&'a MirroredUser>,
    /// What mirrored users are playing, when live sync runs.
    pub now_playing: Option<&'a dyn NowPlayingSource>,
}

/// One Last.fm API method. Implementations live in `methods/<package>/<method>.rs`; they parse
/// parameters, call services and return a response tree, and never write XML or JSON.
#[async_trait]
pub trait Method: Send + Sync {
    /// The name as Last.fm documents it, such as `user.getInfo`. Matched case-insensitively.
    fn name(&self) -> &'static str;

    /// What the caller must prove before the method runs.
    fn access(&self) -> Access {
        Access::Read
    }

    /// The user whose data the response shows. The dispatcher looks them up for
    /// [`Context::user`], and caches the rendered response until that user's next ingest.
    /// `None` disables both, which is the default.
    fn cache_user<'p>(&self, params: &'p Params) -> Option<&'p str> {
        let _ = params;
        None
    }

    /// What Last.fm answers, in plain text and with status 200, instead of error 6 when this
    /// [`Access::Session`] method arrives without `api_sig`. `None`, the default, keeps error 6.
    fn unsigned_answer(&self) -> Option<&'static str> {
        None
    }

    /// Which backend's formatting the method copies, for its body and its errors.
    fn style(&self) -> Style {
        Style::Classic
    }

    /// Whether Last.fm serves this method's JSON as `application/json; charset=UTF-8` with an
    /// extra `access-control-request-headers` header, as some modern-style methods do.
    fn utf8_json(&self) -> bool {
        false
    }

    /// Runs the method.
    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError>;
}
