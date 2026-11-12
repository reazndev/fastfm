//! `www.last.fm` URLs for artists, albums, tracks and users.

use std::fmt::Write as _;

const MUSIC: &str = "https://www.last.fm/music/";

/// `https://www.last.fm/music/Artist`.
#[must_use]
pub fn artist_url(artist: &str) -> String {
    let mut out = String::from(MUSIC);
    encode_name(artist, &mut out);
    out
}

/// `https://www.last.fm/music/Artist/Album`.
#[must_use]
pub fn album_url(artist: &str, album: &str) -> String {
    let mut out = artist_url(artist);
    out.push('/');
    encode_name(album, &mut out);
    out
}

/// `https://www.last.fm/music/Artist/_/Track`.
#[must_use]
pub fn track_url(artist: &str, track: &str) -> String {
    let mut out = artist_url(artist);
    out.push_str("/_/");
    encode_name(track, &mut out);
    out
}

/// `https://www.last.fm/user/Name`.
#[must_use]
pub fn user_url(user: &str) -> String {
    let mut out = String::from("https://www.last.fm/user/");
    encode_name(user, &mut out);
    out
}

/// Encodes one path segment the way Last.fm does: space becomes `+`, letters, digits and
/// `-_.!~*(),&:$;=@` stay, everything else is percent-encoded as UTF-8 with uppercase hex.
///
/// Observed in the 2026-11-12 fixtures: `,()&:$;=@*` stay; space, `/`, `'`, `"`, `#`, `%`, `?`,
/// `[]{}<>` and non-ASCII are encoded. `!` appears both ways (Last.fm keeps each entity's URL as
/// first created), so stored URLs win over this; `~` is a guess.
pub fn encode_name(name: &str, out: &mut String) {
    for byte in name.bytes() {
        match byte {
            b' ' => out.push('+'),
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'('
            | b')'
            | b','
            | b'&'
            | b':'
            | b'$'
            | b';'
            | b'='
            | b'@' => out.push(char::from(byte)),
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
}
