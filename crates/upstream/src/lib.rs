//! The only crate that talks to Last.fm: a rate-limited, prioritised client with a key pool,
//! retries, a raw pass-through call, and typed helpers for the user methods the importer needs.
//! [`TimedClient`] is the exception to the rate limit: it times single requests for the speed
//! comparison, whose caller paces them.

mod client;
mod config;
mod error;
mod gate;
mod library;
mod methods;
mod model;
mod pool;
mod request;
mod retry;
mod signed;
mod timed;
mod writes;

pub use client::Client;
pub use config::{ApiKey, BucketMode, ClientConfig, DEFAULT_BASE_URL, RetryPolicy};
pub use error::{Error, LastFmError, Result};
pub use methods::{CallOptions, PageRequest, RecentTracksQuery};
pub use model::{
    AlbumRef, ArtistRef, ChartRange, Friends, Image, LovedTrack, LovedTracks, PageInfo,
    RecentTracks, Scrobble, TopAlbum, TopArtist, TopPage, TopTrack, Track, User,
};
pub use request::{Format, HttpMethod, Priority, RawRequest, RawResponse};
pub use signed::{AppCredentials, Secret, WriteCredentials};
pub use timed::{Timed, TimedClient};
pub use writes::{Corrected, Ignored, LastfmSession, ScrobbleItem, ScrobbleOutcome, TrackItem};
