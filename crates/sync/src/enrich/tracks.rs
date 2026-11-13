//! Matching scrobbled tracks with the entries of the top-tracks list.
//!
//! Last.fm keeps tracks apart by the exact artist spelling they were scrobbled with
//! (`João Bosco & Vinícius` and `JOÃO BOSCO & VINÍCIUS` are two tracks of one artist), and so
//! does the store. A scrobbled track matches the entry with its exact artist spelling and title
//! ignoring case. What is left is matched ignoring the artist's case where that pairs exactly
//! one track with one entry, then by title alone where each side has one track of that title
//! left and both have the same plays (a track Last.fm files under a longer artist name).
//!
//! The store may still hold one track for scrobbles Last.fm splits by spelling, from before it
//! kept spellings apart. [`merged`] finds them, so enrichment can read their scrobbles again.

use std::collections::{HashMap, HashSet};

use lfm_store::{LibMove, LibraryInputs, Lookup, Scrobbled, fold};
use lfm_upstream::TopTrack;

use super::plan::{Key, Lists, key};

/// Exact artist spelling and folded title.
type Exact = (String, String);

fn exact(artist: &str, name: &str) -> Exact {
    (artist.to_owned(), fold(name))
}

/// The moves that put each scrobbled track under its list entry.
pub(crate) fn moves(inputs: &LibraryInputs, lists: &Lists) -> Vec<LibMove> {
    let listed: HashSet<Exact> = lists
        .tracks
        .iter()
        .map(|t| exact(&t.artist.name, &t.name))
        .collect();
    let scrobbled: HashSet<Exact> = inputs
        .tracks
        .iter()
        .map(|t| exact(&t.artist, &t.name))
        .collect();
    let mut ours: Vec<&Scrobbled> = inputs
        .tracks
        .iter()
        .filter(|t| !listed.contains(&exact(&t.artist, &t.name)))
        .collect();
    let mut theirs: Vec<&TopTrack> = lists
        .tracks
        .iter()
        .filter(|t| !scrobbled.contains(&exact(&t.artist.name, &t.name)))
        .collect();
    let by_key = take_pairs(
        (&mut ours, &mut theirs),
        (
            |t| key(&t.artist, &t.name),
            |t| key(&t.artist.name, &t.name),
        ),
        |_, _| true,
    );
    let by_title = take_pairs(
        (&mut ours, &mut theirs),
        (|t| fold(&t.name), |t| fold(&t.name)),
        |ours, theirs| ours.plays == theirs.playcount,
    );
    by_key
        .into_iter()
        .chain(by_title)
        .map(|(ours, theirs)| LibMove {
            raw: ours.id,
            to: Lookup::Track {
                artist: theirs.artist.name.clone(),
                name: theirs.name.clone(),
            },
        })
        .collect()
}

/// How a track and a list entry are grouped.
type KeyFns<K> = (fn(&Scrobbled) -> K, fn(&TopTrack) -> K);

/// Pairs each track with the entry of the same key where the key has exactly one of each and
/// `fits` accepts them, and removes the pairs from both lists.
fn take_pairs<'a, 'l, K: std::hash::Hash + Eq>(
    (ours, theirs): (&mut Vec<&'a Scrobbled>, &mut Vec<&'l TopTrack>),
    (our_key, their_key): KeyFns<K>,
    fits: fn(&Scrobbled, &TopTrack) -> bool,
) -> Vec<(&'a Scrobbled, &'l TopTrack)> {
    let mut groups: HashMap<K, (Vec<usize>, Vec<usize>)> = HashMap::new();
    for (at, track) in ours.iter().enumerate() {
        groups.entry(our_key(track)).or_default().0.push(at);
    }
    for (at, track) in theirs.iter().enumerate() {
        groups.entry(their_key(track)).or_default().1.push(at);
    }
    let pairs: Vec<(usize, usize)> = groups
        .into_values()
        .filter_map(
            |(ours, theirs)| match (ours.as_slice(), theirs.as_slice()) {
                ([ours], [theirs]) => Some((*ours, *theirs)),
                _ => None,
            },
        )
        .filter(|(o, t)| {
            let (Some(o), Some(t)) = (ours.get(*o), theirs.get(*t)) else {
                return false;
            };
            fits(o, t)
        })
        .collect();
    let found = pairs
        .iter()
        .filter_map(|(o, t)| Some((*ours.get(*o)?, *theirs.get(*t)?)))
        .collect();
    let (used_ours, used_theirs): (HashSet<usize>, HashSet<usize>) = pairs.into_iter().unzip();
    let mut at = 0..;
    ours.retain(|_| !at.next().is_some_and(|i| used_ours.contains(&i)));
    let mut at = 0..;
    theirs.retain(|_| !at.next().is_some_and(|i| used_theirs.contains(&i)));
    found
}

/// Scrobbled tracks whose plays Last.fm splits over several artist spellings while the store
/// counts them under one: the tracks whose scrobbles need reading again.
pub(crate) fn merged(inputs: &LibraryInputs, lists: &Lists) -> Vec<i64> {
    let mut spellings: HashMap<Key, HashSet<&str>> = HashMap::new();
    for track in &lists.tracks {
        let spellings = spellings.entry(key(&track.artist.name, &track.name));
        spellings.or_default().insert(&track.artist.name);
    }
    let mut ours: HashMap<Key, HashSet<&str>> = HashMap::new();
    for track in &inputs.tracks {
        ours.entry(key(&track.artist, &track.name))
            .or_default()
            .insert(&track.artist);
    }
    inputs
        .tracks
        .iter()
        .filter(|track| {
            let key = key(&track.artist, &track.name);
            let listed = spellings.get(&key).map_or(0, HashSet::len);
            listed > 1 && ours.get(&key).map_or(0, HashSet::len) < listed
        })
        .map(|track| track.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use lfm_upstream::ArtistRef;

    use super::*;

    fn ours(id: i64, artist: &str, name: &str, plays: u64) -> Scrobbled {
        Scrobbled {
            id,
            artist: artist.to_owned(),
            name: name.to_owned(),
            image: None,
            plays,
        }
    }

    fn theirs(artist: &str, name: &str, playcount: u64) -> TopTrack {
        TopTrack {
            artist: ArtistRef {
                name: artist.to_owned(),
                mbid: None,
                url: None,
            },
            name: name.to_owned(),
            mbid: None,
            url: String::new(),
            duration: 0,
            playcount,
        }
    }

    fn lists(tracks: Vec<TopTrack>) -> Lists {
        Lists {
            tracks,
            ..Lists::default()
        }
    }

    fn inputs(tracks: Vec<Scrobbled>) -> LibraryInputs {
        LibraryInputs {
            tracks,
            ..LibraryInputs::default()
        }
    }

    fn target(artist: &str, name: &str) -> Lookup {
        Lookup::Track {
            artist: artist.to_owned(),
            name: name.to_owned(),
        }
    }

    #[test]
    fn spellings_match_exactly_first_then_ignoring_case() {
        let title = "Chora, me liga - Ao vivo";
        let listed = lists(vec![
            theirs("João Bosco & Vinícius", title, 2),
            theirs("JOÃO BOSCO & VINÍCIUS", title, 1),
            theirs("Twenty One Pilots", "Heathens", 5),
            theirs("Udo Lindenberg & das Panik-Orchester", "Horizont", 1),
        ]);
        let scrobbled = inputs(vec![
            ours(1, "João Bosco & Vinícius", "chora, me liga - ao vivo", 2),
            ours(2, "JOÃO BOSCO & VINÍCIUS", title, 1),
            ours(3, "twenty one pilots", "Heathens", 5),
            ours(4, "Udo Lindenberg", "Horizont", 1),
        ]);
        let mut moves = moves(&scrobbled, &listed);
        moves.sort_by_key(|m| m.raw);
        let want = [
            (3, target("Twenty One Pilots", "Heathens")),
            (
                4,
                target("Udo Lindenberg & das Panik-Orchester", "Horizont"),
            ),
        ];
        let got: Vec<(i64, Lookup)> = moves.into_iter().map(|m| (m.raw, m.to)).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn a_track_merged_over_spellings_is_read_again() {
        let title = "Chora, me liga - Ao vivo";
        let listed = lists(vec![
            theirs("João Bosco & Vinícius", title, 2),
            theirs("JOÃO BOSCO & VINÍCIUS", title, 1),
            theirs("Other", "Song", 4),
        ]);
        let merged_once = inputs(vec![
            ours(7, "JOÃO BOSCO & VINÍCIUS", title, 3),
            ours(8, "Other", "Song", 4),
        ]);
        assert_eq!(merged(&merged_once, &listed), [7]);
        let split = inputs(vec![
            ours(7, "JOÃO BOSCO & VINÍCIUS", title, 1),
            ours(9, "João Bosco & Vinícius", title, 2),
        ]);
        assert_eq!(merged(&split, &listed), Vec::<i64>::new());
    }
}
