//! SQLite storage for the Last.fm mirror.
//!
//! [`Store`] is the only way in. It is `Clone + Send + Sync`, and every query runs on the
//! store's own threads behind an async method. This crate is the only one that contains SQL.
#![expect(
    clippy::missing_errors_doc,
    reason = "every method fails only with StoreError, whose variants are documented once"
)]

mod accounts;
mod buckets;
mod cache;
mod counts;
mod db;
mod delta;
mod dims;
mod error;
mod ingest;
mod jobs;
mod keys;
mod lookup;
mod loved;
mod names;
mod periods;
mod pool;
mod range;
mod recent;
mod sessions;
mod settings;
mod text;
mod top;
mod types;
mod users;
mod weekly;

pub use accounts::{Account, AccountStatus, LastfmCredentials, Role};
pub use cache::CachedResponse;
pub use error::{Result, StoreError};
pub use jobs::{Job, JobStatus, JobUpdate};
pub use keys::{ApiKey, KeyScope, NewApiKey};
pub use lookup::{Lookup, UserPlays};
pub use loved::{Love, LovedTrack};
pub use names::{fold, image_key};
pub use pool::Store;
pub use sessions::{ApiSession, AuthToken, WebSession};
pub use settings::SignupMode;
pub use top::TopQuery;
pub use types::{
    AccountId, Entity, JobId, Kind, Listing, MAX_LIMIT, Named, Page, Period, Ranked, RecentTrack,
    Scrobble, TimeRange, UserId, UserStats,
};
pub use users::{MirroredUser, Profile};
