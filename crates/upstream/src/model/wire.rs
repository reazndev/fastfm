//! Serde shapes of Last.fm's JSON, converted into the public model.

use serde::Deserialize;

use super::de::{flag, non_empty, number, one_or_many};
use super::{
    AlbumRef, ArtistRef, ChartRange, Friends, Image, LovedTrack, LovedTracks, PageInfo,
    RecentTracks, Scrobble, Track, User,
};

#[derive(Deserialize)]
pub(super) struct PageAttr {
    #[serde(default)]
    user: String,
    #[serde(default, deserialize_with = "number")]
    page: u32,
    #[serde(default, rename = "perPage", deserialize_with = "number")]
    per_page: u32,
    #[serde(default, rename = "totalPages", deserialize_with = "number")]
    total_pages: u32,
    #[serde(default, deserialize_with = "number")]
    total: u64,
}

impl From<&PageAttr> for PageInfo {
    fn from(attr: &PageAttr) -> Self {
        Self {
            page: attr.page,
            per_page: attr.per_page,
            total_pages: attr.total_pages,
            total: attr.total,
        }
    }
}

#[derive(Deserialize)]
pub(super) struct WireImage {
    #[serde(default)]
    size: String,
    #[serde(rename = "#text", default)]
    url: String,
}

pub(super) fn images(wire: Vec<WireImage>) -> Vec<Image> {
    wire.into_iter()
        .map(|i| Image {
            size: i.size,
            url: i.url,
        })
        .collect()
}

/// An artist as `{name, mbid, url}` (extended) or `{#text, mbid}` (plain).
#[derive(Deserialize)]
pub(super) struct WireArtist {
    #[serde(default)]
    name: Option<String>,
    #[serde(rename = "#text", default)]
    text: String,
    #[serde(default)]
    mbid: String,
    #[serde(default)]
    url: Option<String>,
}

impl From<WireArtist> for ArtistRef {
    fn from(a: WireArtist) -> Self {
        Self {
            name: a.name.unwrap_or(a.text),
            mbid: non_empty(a.mbid),
            url: a.url.and_then(non_empty),
        }
    }
}

#[derive(Deserialize, Default)]
struct WireAlbum {
    #[serde(rename = "#text", default)]
    text: String,
    #[serde(default)]
    mbid: String,
}

#[derive(Deserialize)]
struct WireDate {
    #[serde(deserialize_with = "number")]
    uts: u64,
}

#[derive(Deserialize, Default)]
struct TrackAttr {
    #[serde(default, deserialize_with = "flag")]
    nowplaying: bool,
}

#[derive(Deserialize)]
struct WireRecentTrack {
    artist: WireArtist,
    name: String,
    #[serde(default)]
    mbid: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    album: WireAlbum,
    #[serde(default, deserialize_with = "one_or_many")]
    image: Vec<WireImage>,
    #[serde(default)]
    date: Option<WireDate>,
    #[serde(default, deserialize_with = "flag")]
    loved: bool,
    #[serde(rename = "@attr", default)]
    attr: TrackAttr,
}

impl WireRecentTrack {
    fn into_track(self) -> (Option<u64>, bool, Track) {
        let track = Track {
            artist: self.artist.into(),
            name: self.name,
            mbid: non_empty(self.mbid),
            url: self.url,
            album: AlbumRef {
                name: self.album.text,
                mbid: non_empty(self.album.mbid),
            },
            images: images(self.image),
            loved: self.loved,
        };
        (self.date.map(|d| d.uts), self.attr.nowplaying, track)
    }
}

#[derive(Deserialize)]
struct WireRecentTracks {
    #[serde(default, deserialize_with = "one_or_many")]
    track: Vec<WireRecentTrack>,
    #[serde(rename = "@attr")]
    attr: PageAttr,
}

/// `{"recenttracks": ...}`.
#[derive(Deserialize)]
pub(crate) struct RecentTracksBody {
    recenttracks: WireRecentTracks,
}

impl From<RecentTracksBody> for RecentTracks {
    fn from(body: RecentTracksBody) -> Self {
        let page = PageInfo::from(&body.recenttracks.attr);
        let mut now_playing = None;
        let mut scrobbles = Vec::new();
        for wire in body.recenttracks.track {
            match wire.into_track() {
                (Some(uts), false, track) => scrobbles.push(Scrobble { uts, track }),
                (_, _, track) => now_playing = Some(track),
            }
        }
        Self {
            user: body.recenttracks.attr.user,
            now_playing,
            scrobbles,
            page,
        }
    }
}

#[derive(Deserialize, Default)]
struct Registered {
    #[serde(default, deserialize_with = "number")]
    unixtime: u64,
}

#[derive(Deserialize)]
struct WireUser {
    name: String,
    #[serde(default)]
    realname: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    country: String,
    #[serde(default, deserialize_with = "number")]
    playcount: u64,
    #[serde(default, deserialize_with = "number")]
    artist_count: u64,
    #[serde(default, deserialize_with = "number")]
    album_count: u64,
    #[serde(default, deserialize_with = "number")]
    track_count: u64,
    #[serde(default)]
    registered: Registered,
    #[serde(default, deserialize_with = "one_or_many")]
    image: Vec<WireImage>,
    #[serde(default, deserialize_with = "flag")]
    subscriber: bool,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    gender: String,
    #[serde(default, deserialize_with = "number")]
    age: u64,
    #[serde(default, deserialize_with = "number")]
    playlists: u64,
    #[serde(default, deserialize_with = "number")]
    bootstrap: u64,
    #[serde(default, deserialize_with = "number")]
    corrections: u64,
    #[serde(default)]
    spotify_expiry_estimate: Registered,
}

impl From<WireUser> for User {
    fn from(u: WireUser) -> Self {
        Self {
            name: u.name,
            real_name: u.realname,
            url: u.url,
            country: u.country,
            playcount: u.playcount,
            artist_count: u.artist_count,
            album_count: u.album_count,
            track_count: u.track_count,
            registered: u.registered.unixtime,
            images: images(u.image),
            subscriber: u.subscriber,
            kind: u.kind,
            gender: u.gender,
            age: u.age,
            playlists: u.playlists,
            bootstrap: u.bootstrap,
            corrections: u.corrections,
            spotify_expiry: u.spotify_expiry_estimate.unixtime,
        }
    }
}

/// `{"user": ...}`.
#[derive(Deserialize)]
pub(crate) struct UserBody {
    user: WireUser,
}

impl From<UserBody> for User {
    fn from(body: UserBody) -> Self {
        body.user.into()
    }
}

#[derive(Deserialize)]
struct WireFriends {
    #[serde(default, deserialize_with = "one_or_many")]
    user: Vec<WireUser>,
    #[serde(rename = "@attr")]
    attr: PageAttr,
}

/// `{"friends": ...}`.
#[derive(Deserialize)]
pub(crate) struct FriendsBody {
    friends: WireFriends,
}

impl From<FriendsBody> for Friends {
    fn from(body: FriendsBody) -> Self {
        Self {
            page: PageInfo::from(&body.friends.attr),
            users: body.friends.user.into_iter().map(User::from).collect(),
        }
    }
}

#[derive(Deserialize)]
struct WireLovedTrack {
    artist: WireArtist,
    name: String,
    #[serde(default)]
    mbid: String,
    #[serde(default)]
    url: String,
    #[serde(default, deserialize_with = "one_or_many")]
    image: Vec<WireImage>,
    date: WireDate,
}

#[derive(Deserialize)]
struct WireLovedTracks {
    #[serde(default, deserialize_with = "one_or_many")]
    track: Vec<WireLovedTrack>,
    #[serde(rename = "@attr")]
    attr: PageAttr,
}

/// `{"lovedtracks": ...}`.
#[derive(Deserialize)]
pub(crate) struct LovedTracksBody {
    lovedtracks: WireLovedTracks,
}

impl From<LovedTracksBody> for LovedTracks {
    fn from(body: LovedTracksBody) -> Self {
        let tracks = body
            .lovedtracks
            .track
            .into_iter()
            .map(|t| LovedTrack {
                artist: t.artist.into(),
                name: t.name,
                mbid: non_empty(t.mbid),
                url: t.url,
                images: images(t.image),
                uts: t.date.uts,
            })
            .collect();
        Self {
            tracks,
            page: PageInfo::from(&body.lovedtracks.attr),
        }
    }
}

#[derive(Deserialize)]
struct WireChart {
    #[serde(deserialize_with = "number")]
    from: u64,
    #[serde(deserialize_with = "number")]
    to: u64,
}

#[derive(Deserialize)]
struct WireChartList {
    #[serde(default, deserialize_with = "one_or_many")]
    chart: Vec<WireChart>,
}

/// `{"weeklychartlist": ...}`.
#[derive(Deserialize)]
pub(crate) struct ChartListBody {
    weeklychartlist: WireChartList,
}

impl From<ChartListBody> for Vec<ChartRange> {
    fn from(body: ChartListBody) -> Self {
        body.weeklychartlist
            .chart
            .into_iter()
            .map(|c| ChartRange {
                from: c.from,
                to: c.to,
            })
            .collect()
    }
}
