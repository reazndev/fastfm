//! The Last.fm API surface: method registry, dispatcher, auth and response cache.

pub mod auth;
pub mod cache;
pub mod dispatch;
pub mod method;
pub mod methods;
pub mod registry;

pub use auth::{
    ApiKey, ApiKeys, AuthError, InMemoryApiKeys, InMemorySessions, Scope, Session, Sessions,
};
pub use cache::{CacheKey, MemoryCache, ResponseCache};
pub use dispatch::{Dispatcher, Reply};
pub use method::{Access, Context, Method, Services};
pub use registry::Registry;
