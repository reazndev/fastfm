//! This crate's own view of Last.fm data, independent of the rest of the workspace.

mod de;
pub(crate) mod wire;

/// One image URL of a given size (`small`, `medium`, `large`, `extralarge`, `mega`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Size name as Last.fm reports it.
    pub size: String,
    /// The URL, often empty when Last.fm has no picture.
    pub url: String,
}

/// Paging attributes from `@attr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PageInfo {
    /// Current page, from 1.
    pub page: u32,
    /// Items per page.
    pub per_page: u32,
    /// Number of pages.
    pub total_pages: u32,
    /// Number of items across all pages.
    pub total: u64,
}

/// An artist as referenced from a track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistRef {
    /// Display name.
    pub name: String,
    /// `MusicBrainz` ID; `None` when Last.fm sends an empty string.
    pub mbid: Option<String>,
    /// Last.fm page URL.
    pub url: Option<String>,
}

/// An album as referenced from a scrobble.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumRef {
    /// Display name, empty when the scrobble has no album.
    pub name: String,
    /// `MusicBrainz` ID.
    pub mbid: Option<String>,
}

/// A track as it appears in recent tracks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    /// The artist.
    pub artist: ArtistRef,
    /// Track title.
    pub name: String,
    /// `MusicBrainz` ID.
    pub mbid: Option<String>,
    /// Last.fm page URL.
    pub url: String,
    /// The album.
    pub album: AlbumRef,
    /// Cover art URLs.
    pub images: Vec<Image>,
    /// Whether the user loves the track.
    pub loved: bool,
}

/// One scrobble: a track and when it was played.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scrobble {
    /// Unix time of the scrobble.
    pub uts: u64,
    /// What was played.
    pub track: Track,
}

/// One page of `user.getRecentTracks`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentTracks {
    /// The user name from `@attr`.
    pub user: String,
    /// The track playing now, which Last.fm puts first and without a date.
    pub now_playing: Option<Track>,
    /// Scrobbles, newest first.
    pub scrobbles: Vec<Scrobble>,
    /// Paging attributes.
    pub page: PageInfo,
}

/// A Last.fm user profile, from `user.getInfo` or a friends list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// User name.
    pub name: String,
    /// Real name, often empty.
    pub real_name: String,
    /// Profile URL.
    pub url: String,
    /// Country name, or the text `None` when unset.
    pub country: String,
    /// Total scrobbles; 0 where Last.fm leaves it out.
    pub playcount: u64,
    /// Distinct artists; 0 where Last.fm leaves it out.
    pub artist_count: u64,
    /// Distinct albums; 0 where Last.fm leaves it out.
    pub album_count: u64,
    /// Distinct tracks; 0 where Last.fm leaves it out.
    pub track_count: u64,
    /// Unix time of registration.
    pub registered: u64,
    /// Avatar URLs.
    pub images: Vec<Image>,
    /// Whether the user pays for Last.fm Pro.
    pub subscriber: bool,
    /// Account type, such as `user` or `subscriber`.
    pub kind: String,
    /// Gender code, such as `n`; empty where Last.fm leaves it out.
    pub gender: String,
    /// Age; 0 when hidden or left out.
    pub age: u64,
    /// Playlist count; 0 where Last.fm leaves it out.
    pub playlists: u64,
    /// Bootstrap flag as Last.fm reports it; 0 where left out.
    pub bootstrap: u64,
}

/// One loved track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LovedTrack {
    /// The artist.
    pub artist: ArtistRef,
    /// Track title.
    pub name: String,
    /// `MusicBrainz` ID.
    pub mbid: Option<String>,
    /// Last.fm page URL.
    pub url: String,
    /// Cover art URLs.
    pub images: Vec<Image>,
    /// Unix time the track was loved.
    pub uts: u64,
}

/// One page of `user.getLovedTracks`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LovedTracks {
    /// Loved tracks, newest first.
    pub tracks: Vec<LovedTrack>,
    /// Paging attributes.
    pub page: PageInfo,
}

/// One week from `user.getWeeklyChartList`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChartRange {
    /// Unix time the week starts.
    pub from: u64,
    /// Unix time the week ends.
    pub to: u64,
}

/// One page of `user.getFriends`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Friends {
    /// The friends on this page.
    pub users: Vec<User>,
    /// Paging attributes.
    pub page: PageInfo,
}
