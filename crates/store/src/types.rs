//! Types shared by the store's public methods.

use rusqlite::types::{FromSql, FromSqlResult, ToSql, ToSqlOutput, ValueRef};

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub i64);

        impl ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
                self.0.to_sql()
            }
        }

        impl FromSql for $name {
            fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
                i64::column_result(value).map(Self)
            }
        }
    };
}

id_type!(
    /// A mirrored Last.fm user.
    UserId
);
id_type!(
    /// A local account.
    AccountId
);
id_type!(
    /// A persisted job.
    JobId
);

/// The three entity kinds that have totals, rollups, and top lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// An artist.
    Artist,
    /// An album, keyed by its artist and name.
    Album,
    /// A track, keyed by its artist and name.
    Track,
}

impl Kind {
    /// All kinds, in a fixed order.
    pub const ALL: [Self; 3] = [Self::Artist, Self::Album, Self::Track];

    /// The value of a `kind` column.
    pub(crate) fn code(self) -> i64 {
        match self {
            Self::Artist => 0,
            Self::Album => 1,
            Self::Track => 2,
        }
    }

    /// The word used in table and column names.
    pub(crate) fn word(self) -> &'static str {
        match self {
            Self::Artist => "artist",
            Self::Album => "album",
            Self::Track => "track",
        }
    }
}

/// A top-list period as Last.fm names them. Every period but `Overall` is a rolling window:
/// the last N times 86,400 seconds before the request, as Last.fm counts them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Period {
    /// All time.
    Overall,
    /// `7day`: the last 7 days.
    Days7,
    /// `1month`: the last 30 days.
    Month1,
    /// `3month`: the last 90 days.
    Month3,
    /// `6month`: the last 180 days.
    Month6,
    /// `12month`: the last 365 days.
    Month12,
}

impl Period {
    /// All periods, in storage order.
    pub const ALL: [Self; 6] = [
        Self::Overall,
        Self::Days7,
        Self::Month1,
        Self::Month3,
        Self::Month6,
        Self::Month12,
    ];

    /// Length in days; `None` for overall.
    #[must_use]
    pub fn days(self) -> Option<i64> {
        match self {
            Self::Overall => None,
            Self::Days7 => Some(7),
            Self::Month1 => Some(30),
            Self::Month3 => Some(90),
            Self::Month6 => Some(180),
            Self::Month12 => Some(365),
        }
    }

    /// The time range the period covers at `now`: from `now` minus its days on.
    #[must_use]
    pub fn range(self, now: i64) -> TimeRange {
        let from = self
            .days()
            .map_or(0, |days| now.saturating_sub(days * crate::buckets::DAY));
        TimeRange::new(from.max(0), TimeRange::ALL.to)
    }

    /// The first second of the stored totals when they end on UTC day `day`: the start of the
    /// first whole day of the period. Reads add the part of the day before it that the rolling
    /// window still covers.
    pub(crate) fn start(self, day: i64) -> i64 {
        self.days()
            .map_or(0, |days| (day - days + 1) * crate::buckets::DAY)
    }

    /// The value of the `period` column.
    pub(crate) fn index(self) -> i64 {
        match self {
            Self::Overall => 0,
            Self::Days7 => 1,
            Self::Month1 => 2,
            Self::Month3 => 3,
            Self::Month6 => 4,
            Self::Month12 => 5,
        }
    }
}

/// A name with its optional MBID and Last.fm URL, as received.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Named {
    /// Name exactly as given.
    pub name: String,
    /// MBID; an empty string counts as none.
    pub mbid: Option<String>,
    /// Last.fm URL; an empty string counts as none.
    pub url: Option<String>,
}

impl Named {
    /// A name with no mbid or URL.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            mbid: None,
            url: None,
        }
    }
}

/// One scrobble to ingest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scrobble {
    /// Unix time of the scrobble.
    pub uts: i64,
    /// The track's artist.
    pub artist: Named,
    /// The album, if any. Albums are keyed by the track's artist and this name.
    pub album: Option<Named>,
    /// The track.
    pub track: Named,
    /// Cover art: a Last.fm image URL or an image key.
    pub image: Option<String>,
}

/// A half-open time range `[from, to)` in unix seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeRange {
    /// Start, inclusive.
    pub from: i64,
    /// End, exclusive.
    pub to: i64,
}

impl TimeRange {
    /// Every timestamp the store accepts.
    pub const ALL: Self = Self {
        from: 0,
        to: crate::buckets::MAX_UTS,
    };

    /// A range from `from` (inclusive) to `to` (exclusive).
    #[must_use]
    pub fn new(from: i64, to: i64) -> Self {
        Self { from, to }
    }
}

/// Largest page size the store serves.
pub const MAX_LIMIT: u32 = 10_000;

/// A page request. Pages count from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Page {
    /// Page number, at least 1.
    pub page: u32,
    /// Rows per page, 1 to `MAX_LIMIT`.
    pub limit: u32,
}

impl Page {
    /// A page request, clamped to valid values.
    #[must_use]
    pub fn new(page: u32, limit: u32) -> Self {
        Self {
            page: page.max(1),
            limit: limit.clamp(1, MAX_LIMIT),
        }
    }

    /// Rows before this page.
    #[must_use]
    pub fn offset(self) -> u64 {
        u64::from(self.page - 1) * u64::from(self.limit)
    }
}

/// One page of a list, with the size of the whole list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing<T> {
    /// The rows on this page.
    pub items: Vec<T>,
    /// The page that was served.
    pub page: Page,
    /// Rows in the whole list.
    pub total: u64,
}

impl<T> Listing<T> {
    /// Number of pages in the whole list, as Last.fm counts them.
    #[must_use]
    pub fn total_pages(&self) -> u64 {
        self.total.div_ceil(u64::from(self.page.limit))
    }
}

/// An artist, album, or track with its names, as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entity {
    /// Store id, unique within the kind.
    pub id: i64,
    /// Name as first seen.
    pub name: String,
    /// MBID.
    pub mbid: Option<String>,
    /// Last.fm URL (artists and tracks).
    pub url: Option<String>,
    /// Image key (albums).
    pub image: Option<String>,
    /// The artist (albums and tracks).
    pub artist: Option<Named>,
    /// Length in seconds (tracks, when known).
    pub duration: Option<i64>,
}

/// An entity with its play count and rank in a top list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ranked {
    /// Position in the list, from 1.
    pub rank: u64,
    /// Plays in the requested range.
    pub plays: u64,
    /// The artist, album, or track.
    pub entity: Entity,
}

/// One row of a user's history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentTrack {
    /// Unix time of the scrobble.
    pub uts: i64,
    /// The track, with its artist.
    pub track: Entity,
    /// The album, with its image. Its artist is the track's, so `album.artist` is `None`.
    pub album: Option<Entity>,
    /// Whether the user has loved the track.
    pub loved: bool,
}

/// Counts for one user.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UserStats {
    /// Scrobbles.
    pub playcount: u64,
    /// Distinct artists.
    pub artists: u64,
    /// Distinct albums.
    pub albums: u64,
    /// Distinct tracks.
    pub tracks: u64,
}

/// Converts a stored count to `u64`; counts are never negative.
pub(crate) fn count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}
