//! Moves a user's Last.fm data into the mirror's store.
//!
//! - **Import** copies a whole history with `user.getRecentTracks`, walking time ranges
//!   backwards in parallel and writing whole-second windows, then the profile, loved tracks, and
//!   weekly chart list. It saves its cursors after every window and resumes after a restart.
//! - **Live sync** polls each followed user for new scrobbles and the now-playing track.
//! - **Reconciliation** compares counts with Last.fm and repairs the ranges that differ.
//! - **Enrichment** reads the user's all-time top lists to learn which artists, albums and
//!   tracks Last.fm counts the scrobbles under, so local top lists and charts match.
//!
//! [`SyncService`] owns all of it; the server embeds one.

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
mod upstream;
mod walk;

pub use config::SyncConfig;
pub use enrich::EnrichReport;
pub use error::{Result, SyncError};
pub use import::ImportProgress;
pub use now_playing::{NowPlaying, Playing};
pub use reconcile::ReconcileReport;
pub use service::SyncService;
