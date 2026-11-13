//! The Last.fm API surface: method registry, dispatcher, auth and response cache.

pub mod accounts;
pub mod auth;
pub mod body;
pub mod cache;
pub mod clock;
pub mod dispatch;
pub mod export;
mod forward;
pub mod method;
pub mod methods;
pub mod now_playing;
pub mod passthrough;
pub mod prewarm;
pub mod registry;
pub mod reply;
pub mod store_auth;

pub use accounts::AccountServices;
pub use auth::{
    ApiKey, ApiKeys, AuthError, InMemoryApiKeys, InMemorySessions, Scope, Session, Sessions,
};
pub use body::{Body, Encoding};
pub use cache::{CacheKey, MemoryCache, ResponseCache};
pub use clock::Clock;
pub use dispatch::{Dispatcher, Reply, Source};
pub use export::{Export, NDJSON};
pub use method::{Access, Context, Method, Services};
pub use now_playing::{NowPlayingSource, recent_track_of};
pub use passthrough::{Caller, CallerKeys, Forward, Passthrough, Unavailable};
pub use registry::Registry;
pub use store_auth::{StoreKeys, StoreSessions};
