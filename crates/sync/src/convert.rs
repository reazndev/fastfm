//! Turning Last.fm's answers into the store's input types.

use lfm_store::{Love, Named, Profile, ProfileExtras, Scrobble};
use lfm_upstream::{ArtistRef, Image, LovedTrack, User};

/// Converts a Unix time from Last.fm to the store's signed seconds.
pub(crate) fn uts(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Converts the store's signed seconds to a Last.fm parameter; negative times become 0.
pub(crate) fn param(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

/// Converts a scrobble.
pub(crate) fn scrobble(scrobble: &lfm_upstream::Scrobble) -> Scrobble {
    let track = &scrobble.track;
    let album = (!track.album.name.is_empty()).then(|| Named {
        name: track.album.name.clone(),
        mbid: track.album.mbid.clone(),
        url: None,
    });
    let mut artist = artist(&track.artist);
    // Plain pages carry no artist URL; the track URL starts with it.
    if artist.url.is_none() {
        artist.url = track
            .url
            .split_once("/_/")
            .map(|(artist_url, _)| artist_url.to_owned());
    }
    Scrobble {
        uts: uts(scrobble.uts),
        artist,
        album,
        track: Named {
            name: track.name.clone(),
            mbid: track.mbid.clone(),
            url: Some(track.url.clone()),
        },
        image: largest_image(&track.images),
    }
}

/// Converts a page of scrobbles, newest first as Last.fm sends them, into oldest-first order
/// so same-second scrobbles are numbered the same way by every writer.
pub(crate) fn scrobbles<'a>(
    newest_first: impl DoubleEndedIterator<Item = &'a lfm_upstream::Scrobble>,
) -> Vec<Scrobble> {
    newest_first.rev().map(scrobble).collect()
}

/// Converts a loved track.
pub(crate) fn love(loved: &LovedTrack) -> Love {
    Love {
        uts: uts(loved.uts),
        artist: artist(&loved.artist),
        track: Named {
            name: loved.name.clone(),
            mbid: loved.mbid.clone(),
            url: Some(loved.url.clone()),
        },
    }
}

/// Converts the `user.getInfo` fields the store keeps apart from the profile.
pub(crate) fn profile_extras(user: &User) -> ProfileExtras {
    ProfileExtras {
        corrections: uts(user.corrections),
        spotify_expiry: uts(user.spotify_expiry),
    }
}

/// Converts the `user.getInfo` fields the store keeps.
pub(crate) fn profile(user: &User) -> Profile {
    Profile {
        realname: user.real_name.clone(),
        url: user.url.clone(),
        country: user.country.clone(),
        gender: user.gender.clone(),
        age: uts(user.age),
        subscriber: user.subscriber,
        playlists: uts(user.playlists),
        bootstrap: uts(user.bootstrap),
        registered: uts(user.registered),
        image: largest_image(&user.images),
        user_type: user.kind.clone(),
    }
}

fn artist(artist: &ArtistRef) -> Named {
    Named {
        name: artist.name.clone(),
        mbid: artist.mbid.clone(),
        url: artist.url.clone(),
    }
}

/// The last non-empty URL; Last.fm lists sizes smallest first.
fn largest_image(images: &[Image]) -> Option<String> {
    images
        .iter()
        .rev()
        .find(|image| !image.url.is_empty())
        .map(|image| image.url.clone())
}
