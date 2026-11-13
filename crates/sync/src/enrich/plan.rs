//! Matching a user's scrobbled entities with the entries of Last.fm's top lists.
//!
//! Observed on the fixture account (160,643 scrobbles, 2026-11-12), and the rules below follow
//! from it:
//!
//! - Artists: every list entry's play count equals the scrobbles of the names that fold to it
//!   plus those `artist.getCorrection` maps to it, so `zzz.` (3,718) and `ZZZ` (1) make `ZZZ`
//!   (3,719). 4,052 scrobbled names become Last.fm's 4,046 artists with exact counts.
//! - Tracks keep the scrobbled artist (`zzz.` tracks stay under `zzz.`); all 15,477 match by
//!   name but one, which Last.fm files under a longer artist name that no correction call
//!   reveals. A scrobbled track and a list entry that are each the only unmatched one with that
//!   title and have the same play count are paired.
//! - Albums are filed under the album artist, which recent tracks do not show. The store keeps
//!   one scrobbled album per spelling and cover, and those tell album artists apart where one
//!   title has several (`FIRST FINALE` and `First Finale` by one track artist belong to two
//!   album artists, with different covers). A scrobbled album goes to the listed album with its
//!   exact spelling and cover when that is the only one with its title, else to the listed
//!   album of its scrobbled or corrected artist, else to the album with that title that has the
//!   most plays still unaccounted for (`Various Artists`, a featured artist). That places all
//!   but 62 scrobbles; those share spelling, cover and artist with scrobbles Last.fm files under
//!   another album artist, so 11 of its 6,198 albums get no plays here.

use std::collections::HashMap;

use lfm_store::{
    LibAlbum, LibMove, LibTrack, Library, LibraryInputs, Lookup, Named, Scrobbled, fold, image_key,
};
use lfm_upstream::{ArtistRef, TopAlbum, TopArtist, TopTrack};

/// The user's three all-time top lists.
#[derive(Debug, Clone, Default)]
pub(crate) struct Lists {
    pub(crate) artists: Vec<TopArtist>,
    pub(crate) albums: Vec<TopAlbum>,
    pub(crate) tracks: Vec<TopTrack>,
}

/// Folded artist and title.
pub(super) type Key = (String, String);

/// Scrobbled artist names the lists do not explain and that have no stored target among them:
/// the ones to ask `artist.getCorrection` about.
pub(crate) fn needs_correction(inputs: &LibraryInputs, lists: &Lists) -> Vec<String> {
    let listed = artist_index(lists);
    inputs
        .artists
        .iter()
        .filter(|artist| !listed.contains_key(&fold(&artist.name)))
        .filter(|artist| {
            let target = inputs.artist_targets.get(&artist.id);
            !target.is_some_and(|target| listed.contains_key(&fold(target)))
        })
        .map(|artist| artist.name.clone())
        .collect()
}

/// The library the lists describe, with the moves that make the user's counts match them.
/// `corrections` maps folded scrobbled names to the names `artist.getCorrection` gave.
pub(crate) fn library(
    inputs: &LibraryInputs,
    lists: &Lists,
    corrections: &HashMap<String, String>,
) -> Library {
    let artists = artist_targets(inputs, lists, corrections);
    let moves = artist_moves(inputs, &artists)
        .chain(album_moves(inputs, lists, &artists))
        .chain(super::tracks::moves(inputs, lists))
        .collect();
    Library {
        artists: lists.artists.iter().map(lib_artist).collect(),
        albums: lists.albums.iter().map(lib_album).collect(),
        tracks: lists.tracks.iter().map(lib_track).collect(),
        moves,
    }
}

/// How many artists, albums and tracks move.
pub(crate) fn moved(library: &Library) -> [usize; 3] {
    let count = |want: fn(&Lookup) -> bool| library.moves.iter().filter(|m| want(&m.to)).count();
    [
        count(|to| matches!(to, Lookup::Artist { .. })),
        count(|to| matches!(to, Lookup::Album { .. })),
        count(|to| matches!(to, Lookup::Track { .. })),
    ]
}

fn artist_index(lists: &Lists) -> HashMap<String, &TopArtist> {
    lists
        .artists
        .iter()
        .map(|artist| (fold(&artist.name), artist))
        .collect()
}

/// Each scrobbled artist name, folded, with the listed artist it counts under.
fn artist_targets<'l>(
    inputs: &LibraryInputs,
    lists: &'l Lists,
    corrections: &HashMap<String, String>,
) -> HashMap<String, &'l TopArtist> {
    let listed = artist_index(lists);
    inputs
        .artists
        .iter()
        .filter_map(|artist| {
            let name = fold(&artist.name);
            let stored = inputs.artist_targets.get(&artist.id).map(|t| fold(t));
            let corrected = corrections.get(&name).map(|c| fold(c));
            let target = [Some(name.clone()), stored, corrected]
                .into_iter()
                .flatten()
                .find_map(|candidate| listed.get(&candidate).copied())?;
            Some((name, target))
        })
        .collect()
}

fn artist_moves<'a>(
    inputs: &'a LibraryInputs,
    targets: &'a HashMap<String, &TopArtist>,
) -> impl Iterator<Item = LibMove> + 'a {
    inputs.artists.iter().filter_map(|artist| {
        let target = targets.get(&fold(&artist.name))?;
        (fold(&target.name) != fold(&artist.name)).then(|| LibMove {
            raw: artist.id,
            to: Lookup::Artist {
                name: target.name.clone(),
            },
        })
    })
}

/// Albums still to account for, by key: plays not yet matched, and the listed album.
type Left<'l> = HashMap<Key, (i128, &'l TopAlbum)>;

fn album_moves(
    inputs: &LibraryInputs,
    lists: &Lists,
    artists: &HashMap<String, &TopArtist>,
) -> Vec<LibMove> {
    let mut left: Left<'_> = lists
        .albums
        .iter()
        .map(|album| {
            (
                key(&album.artist.name, &album.name),
                (plays(album.playcount), album),
            )
        })
        .collect();
    let mut by_title: HashMap<String, Vec<Key>> = HashMap::new();
    for album in &lists.albums {
        let key = key(&album.artist.name, &album.name);
        by_title.entry(key.1.clone()).or_default().push(key);
    }
    let mut targets: Vec<(&Scrobbled, Key)> = Vec::new();
    let mut rest: Vec<&Scrobbled> = Vec::new();
    for album in &inputs.albums {
        let titled = by_title
            .get(&fold(&album.name))
            .map_or(&[][..], Vec::as_slice);
        match direct_album(album, titled, &left, artists) {
            Some(found) => targets.push((album, found)),
            None => rest.push(album),
        }
    }
    for (album, found) in &targets {
        take(&mut left, found, album.plays);
    }
    // Biggest first, so the plays still unaccounted for guide the large albums.
    rest.sort_by(|a, b| b.plays.cmp(&a.plays).then(a.id.cmp(&b.id)));
    for album in rest {
        let candidates = by_title.get(&fold(&album.name)).into_iter().flatten();
        let best = candidates
            .max_by_key(|k| left.get(*k).map_or(i128::MIN, |(n, _)| *n))
            .cloned();
        if let Some(best) = best {
            take(&mut left, &best, album.plays);
            targets.push((album, best));
        }
    }
    targets
        .into_iter()
        .filter_map(|(album, found)| {
            let (_, listed) = left.get(&found)?;
            Some(LibMove {
                raw: album.id,
                to: Lookup::Album {
                    artist: listed.artist.name.clone(),
                    name: listed.name.clone(),
                },
            })
        })
        .collect()
}

/// The listed album a scrobbled one belongs to without weighing play counts. Where several
/// listed albums share the title, the one with the same spelling and cover decides; otherwise
/// the scrobbled artist, then the corrected one.
fn direct_album(
    album: &Scrobbled,
    titled: &[Key],
    left: &Left<'_>,
    artists: &HashMap<String, &TopArtist>,
) -> Option<Key> {
    if titled.len() > 1 {
        let same: Vec<&Key> = titled
            .iter()
            .filter(|k| left.get(*k).is_some_and(|(_, l)| looks_the_same(album, l)))
            .collect();
        if let [only] = same.as_slice() {
            return Some((*only).clone());
        }
    }
    let own = key(&album.artist, &album.name);
    let corrected = artists
        .get(&own.0)
        .map(|artist| (fold(&artist.name), own.1.clone()));
    [Some(own), corrected]
        .into_iter()
        .flatten()
        .find(|k| left.contains_key(k))
}

/// Last.fm's "no cover" picture, which recent tracks show where top albums show none.
const NO_COVER: &str = "2a96cbd8b46e442fc41c2b86b821562f.png";

fn looks_the_same(album: &Scrobbled, listed: &TopAlbum) -> bool {
    let cover = |image: Option<&str>| image.filter(|i| *i != NO_COVER).map(str::to_owned);
    let listed_cover = listed
        .images
        .iter()
        .rev()
        .find_map(|image| image_key(&image.url));
    album.name == listed.name && cover(album.image.as_deref()) == listed_cover
}

fn take(left: &mut Left<'_>, key: &Key, plays_taken: u64) {
    if let Some((n, _)) = left.get_mut(key) {
        *n -= plays(plays_taken);
    }
}

fn plays(count: u64) -> i128 {
    i128::from(count)
}

pub(super) fn key(artist: &str, name: &str) -> Key {
    (fold(artist), fold(name))
}

fn named(name: &str, mbid: Option<&String>, url: &str) -> Named {
    Named {
        name: name.to_owned(),
        mbid: mbid.cloned(),
        url: (!url.is_empty()).then(|| url.to_owned()),
    }
}

fn artist_named(artist: &ArtistRef) -> Named {
    Named {
        name: artist.name.clone(),
        mbid: artist.mbid.clone(),
        url: artist.url.clone(),
    }
}

fn lib_artist(artist: &TopArtist) -> Named {
    named(&artist.name, artist.mbid.as_ref(), &artist.url)
}

fn lib_album(album: &TopAlbum) -> LibAlbum {
    let cover = album
        .images
        .iter()
        .rev()
        .find(|image| !image.url.is_empty());
    LibAlbum {
        artist: artist_named(&album.artist),
        album: named(&album.name, album.mbid.as_ref(), &album.url),
        image: cover.map(|image| image.url.clone()),
    }
}

fn lib_track(track: &TopTrack) -> LibTrack {
    LibTrack {
        artist: Named {
            url: None,
            ..artist_named(&track.artist)
        },
        track: named(&track.name, track.mbid.as_ref(), &track.url),
        duration: i64::try_from(track.duration).unwrap_or(0),
    }
}
