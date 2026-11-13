//! The hook through which live sync's now-playing state reaches `user.getRecentTracks`.

use lfm_store::{Entity, Named, RecentTrack, UserId, image_key};
use lfm_upstream::Track;

/// What each mirrored user is playing right now. The server implements it over live sync.
pub trait NowPlayingSource: Send + Sync + std::fmt::Debug {
    /// The track `user` is playing, if any. Its `uts` is not shown.
    fn now_playing(&self, user: UserId) -> Option<RecentTrack>;
}

/// A track as Last.fm reported it in recent tracks, in the store's shape.
#[must_use]
pub fn recent_track_of(track: &Track) -> RecentTrack {
    let artist = Named {
        name: track.artist.name.clone(),
        mbid: track.artist.mbid.clone(),
        url: track.artist.url.clone(),
    };
    let cover = track
        .images
        .iter()
        .rev()
        .find_map(|image| image_key(&image.url));
    let album = Some(&track.album)
        .filter(|album| !album.name.is_empty())
        .map(|album| Entity {
            id: 0,
            name: album.name.clone(),
            mbid: album.mbid.clone(),
            url: None,
            image: cover,
            artist: None,
            duration: None,
        });
    RecentTrack {
        uts: 0,
        track: Entity {
            id: 0,
            name: track.name.clone(),
            mbid: track.mbid.clone(),
            url: Some(track.url.clone()).filter(|url| !url.is_empty()),
            image: None,
            artist: Some(artist),
            duration: None,
        },
        album,
        loved: track.loved,
    }
}
