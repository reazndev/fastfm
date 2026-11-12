//! The `Method` trait every Last.fm method implements, and the context it receives.

use std::sync::Arc;

use async_trait::async_trait;
use lfm_core::{ApiError, Params, Response};

use crate::auth::{ApiKey, ApiKeys, Session, Sessions};
use crate::cache::ResponseCache;

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

/// Shared services that methods use. The store and the upstream client are added here when
/// those crates land.
#[derive(Clone)]
pub struct Services {
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

    /// The user whose data the response shows, when the rendered response may be cached until
    /// that user's next ingest. `None` disables caching, which is the default.
    fn cache_user<'p>(&self, params: &'p Params) -> Option<&'p str> {
        let _ = params;
        None
    }

    /// Runs the method.
    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError>;
}
