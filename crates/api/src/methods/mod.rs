//! Last.fm methods, one file per method under `methods/<package>/<method>.rs`.
//!
//! To add a method: create `methods/<package>/<method>.rs` with a unit struct that implements
//! [`Method`](crate::Method), declare it in `methods/<package>/mod.rs`, and add it to
//! [`registry`]. See [`example::Echo`] for the shape, and `shared` for the behaviour the local
//! methods have in common.

pub mod auth;
pub mod example;
pub mod library;
pub(crate) mod shared;
pub mod track;
pub mod user;

use std::sync::Arc;

use crate::accounts::AccountServices;
use crate::registry::Registry;

/// Every Last.fm method the mirror serves.
#[must_use]
pub fn registry() -> Registry {
    Registry::new()
        .with(auth::GetSession)
        .with(auth::GetToken::default())
        .with(user::GetInfo)
        .with(user::GetLovedTracks)
        .with(user::GetRecentTracks)
        .with(user::GetTopAlbums)
        .with(user::GetTopArtists)
        .with(user::GetTopTracks)
        .with(user::GetWeeklyAlbumChart)
        .with(user::GetWeeklyArtistChart)
        .with(user::GetWeeklyChartList)
        .with(user::GetWeeklyTrackChart)
        .with(library::GetArtists)
}

/// `registry` plus the methods that act for a local account: `auth.getMobileSession` and the
/// `track.*` writes, which reach accounts and Last.fm through `accounts`, and `auth.getToken`
/// limited per API key.
#[must_use]
pub fn with_accounts(registry: Registry, accounts: &Arc<dyn AccountServices>) -> Registry {
    registry
        .with(auth::GetMobileSession::new(Arc::clone(accounts)))
        .with(auth::GetToken::limited(Arc::clone(accounts)))
        .with(track::Love::new(Arc::clone(accounts)))
        .with(track::Scrobble::new(Arc::clone(accounts)))
        .with(track::Unlove::new(Arc::clone(accounts)))
        .with(track::UpdateNowPlaying::new(Arc::clone(accounts)))
}
