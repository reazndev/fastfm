//! Moves a user's Last.fm data into the mirror's store.
//!
//! - **Import** copies a whole history with `user.getRecentTracks`, walking time ranges
//!   backwards in parallel and writing whole-second windows, then the profile, loved tracks, and
//!   weekly chart list. It saves its cursors after every window and resumes after a restart.
//! - **Live sync** polls each followed user for new scrobbles and the now-playing track, more
//!   often the more recently they scrobbled, with jitter so users do not poll in lockstep.
//! - **Reconciliation** compares counts with Last.fm and repairs the ranges that differ: the
//!   whole history once per interval, counted from the last pass across restarts, and the last
//!   14 days below the sync cursor every 10 minutes for active users and hourly for idle ones,
//!   where late (offline) scrobbles land. Ranges where Last.fm's own totals disagree are
//!   recorded and left alone for a week.
//! - **Enrichment** reads the user's all-time top lists to learn which artists, albums and
//!   tracks Last.fm counts the scrobbles under, so local top lists and charts match.
//!
//! Background requests on one Last.fm key (the instance pool, or an account's own key) share a
//! budget per second: live polls first, then imports, then enrichment and reconciliation.
//! A profile that hides its history stops the import with a clear error and is retried daily.
//!
//! [`SyncService`] owns all of it; the server embeds one.

mod budget;
mod clock;
mod config;
mod context;
mod convert;
mod enrich;
mod error;
mod import;
mod live;
mod now_playing;
mod reconcile;
mod service;
mod tasks;
mod tiers;
mod upstream;
mod walk;

pub use config::SyncConfig;
pub use enrich::EnrichReport;
pub use error::{Result, SyncError};
pub use import::{ImportProgress, PRIVATE_MESSAGE};
pub use now_playing::{NowPlaying, Playing};
pub use reconcile::ReconcileReport;
pub use service::SyncService;
