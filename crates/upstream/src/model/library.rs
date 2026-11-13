//! A user's overall top lists and Last.fm's name corrections: the entities Last.fm counts a
//! user's scrobbles under.

use serde::Deserialize;

use super::de::{non_empty, number, one_or_many};
use super::wire::{PageAttr, WireArtist, WireImage, images};
use super::{ArtistRef, Image, PageInfo};

/// One page of a top list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopPage<T> {
    /// The entries on this page, by play count.
    pub items: Vec<T>,
    /// Paging attributes.
    pub page: PageInfo,
}

/// One entry of `user.getTopArtists`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopArtist {
    /// Name as Last.fm's library spells it.
    pub name: String,
    /// `MusicBrainz` ID.
    pub mbid: Option<String>,
    /// Last.fm page URL.
    pub url: String,
    /// The user's plays.
    pub playcount: u64,
}

/// One entry of `user.getTopAlbums`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopAlbum {
    /// The album artist, with the MBID Last.fm shows for this album.
    pub artist: ArtistRef,
    /// Album title.
    pub name: String,
    /// `MusicBrainz` ID.
    pub mbid: Option<String>,
    /// Last.fm page URL.
    pub url: String,
    /// Cover art URLs; empty URLs when there is none.
    pub images: Vec<Image>,
    /// The user's plays.
    pub playcount: u64,
}

/// One entry of `user.getTopTracks`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopTrack {
    /// The artist, with the MBID Last.fm shows for this track.
    pub artist: ArtistRef,
    /// Track title.
    pub name: String,
    /// `MusicBrainz` ID.
    pub mbid: Option<String>,
    /// Last.fm page URL.
    pub url: String,
    /// Length in seconds; 0 when unknown.
    pub duration: u64,
    /// The user's plays.
    pub playcount: u64,
}

#[derive(Deserialize)]
struct WireTopArtist {
    name: String,
    #[serde(default)]
    mbid: String,
    #[serde(default)]
    url: String,
    #[serde(default, deserialize_with = "number")]
    playcount: u64,
}

#[derive(Deserialize)]
struct WireTopAlbum {
    artist: WireArtist,
    name: String,
    #[serde(default)]
    mbid: String,
    #[serde(default)]
    url: String,
    #[serde(default, deserialize_with = "one_or_many")]
    image: Vec<WireImage>,
    #[serde(default, deserialize_with = "number")]
    playcount: u64,
}

#[derive(Deserialize)]
struct WireTopTrack {
    artist: WireArtist,
    name: String,
    #[serde(default)]
    mbid: String,
    #[serde(default)]
    url: String,
    #[serde(default, deserialize_with = "number")]
    duration: u64,
    #[serde(default, deserialize_with = "number")]
    playcount: u64,
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
struct WireList<T> {
    #[serde(default = "Vec::new", deserialize_with = "one_or_many")]
    artist: Vec<T>,
    #[serde(default = "Vec::new", deserialize_with = "one_or_many")]
    album: Vec<T>,
    #[serde(default = "Vec::new", deserialize_with = "one_or_many")]
    track: Vec<T>,
    #[serde(rename = "@attr")]
    attr: PageAttr,
}

impl<T> WireList<T> {
    fn into_page<U>(self, convert: impl Fn(T) -> U) -> TopPage<U> {
        let page = PageInfo::from(&self.attr);
        let items = [self.artist, self.album, self.track]
            .into_iter()
            .flatten()
            .map(convert)
            .collect();
        TopPage { items, page }
    }
}

/// `{"topartists": ...}`.
#[derive(Deserialize)]
pub(crate) struct TopArtistsBody {
    topartists: WireList<WireTopArtist>,
}

impl From<TopArtistsBody> for TopPage<TopArtist> {
    fn from(body: TopArtistsBody) -> Self {
        body.topartists.into_page(|a| TopArtist {
            name: a.name,
            mbid: non_empty(a.mbid),
            url: a.url,
            playcount: a.playcount,
        })
    }
}

/// `{"topalbums": ...}`.
#[derive(Deserialize)]
pub(crate) struct TopAlbumsBody {
    topalbums: WireList<WireTopAlbum>,
}

impl From<TopAlbumsBody> for TopPage<TopAlbum> {
    fn from(body: TopAlbumsBody) -> Self {
        body.topalbums.into_page(|a| TopAlbum {
            artist: a.artist.into(),
            name: a.name,
            mbid: non_empty(a.mbid),
            url: a.url,
            images: images(a.image),
            playcount: a.playcount,
        })
    }
}

/// `{"toptracks": ...}`.
#[derive(Deserialize)]
pub(crate) struct TopTracksBody {
    toptracks: WireList<WireTopTrack>,
}

impl From<TopTracksBody> for TopPage<TopTrack> {
    fn from(body: TopTracksBody) -> Self {
        body.toptracks.into_page(|t| TopTrack {
            artist: t.artist.into(),
            name: t.name,
            mbid: non_empty(t.mbid),
            url: t.url,
            duration: t.duration,
            playcount: t.playcount,
        })
    }
}

#[derive(Deserialize)]
struct WireCorrection {
    artist: WireArtist,
}

#[derive(Deserialize)]
struct WireCorrections {
    #[serde(default, deserialize_with = "one_or_many")]
    correction: Vec<WireCorrection>,
}

/// `{"corrections": ...}` of `artist.getCorrection`. Without a correction Last.fm sends
/// `"corrections":"\n"` or similar text, which reads as no correction.
#[derive(Deserialize)]
pub(crate) struct ArtistCorrectionBody {
    #[serde(default, deserialize_with = "corrections")]
    corrections: Vec<WireCorrection>,
}

fn corrections<'de, D>(deserializer: D) -> Result<Vec<WireCorrection>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        List(WireCorrections),
        Text(serde::de::IgnoredAny),
    }
    Ok(match Either::deserialize(deserializer)? {
        Either::List(list) => list.correction,
        Either::Text(_) => Vec::new(),
    })
}

impl From<ArtistCorrectionBody> for Option<ArtistRef> {
    fn from(body: ArtistCorrectionBody) -> Self {
        body.corrections.into_iter().next().map(|c| c.artist.into())
    }
}
