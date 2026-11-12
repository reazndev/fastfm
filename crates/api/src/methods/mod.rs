//! Last.fm methods, one file per method under `methods/<package>/<method>.rs`.
//!
//! To add a method: create `methods/<package>/<method>.rs` with a unit struct that implements
//! [`Method`](crate::Method), declare it in `methods/<package>/mod.rs`, and add it to
//! [`registry`]. See [`example::Echo`] for the shape, and `shared` for the behaviour the local
//! methods have in common.

pub mod example;
pub mod library;
pub(crate) mod shared;
pub mod user;

use crate::registry::Registry;

/// Every Last.fm method the mirror serves.
#[must_use]
pub fn registry() -> Registry {
    Registry::new()
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
