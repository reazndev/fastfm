//! Local `userplaycount` and `userloved` on a cached `artist/album/track.getInfo`.
//!
//! Last.fm's answer with `username` differs from the one without in more than the two fields:
//! `artist.getInfo` shows the artist's real image instead of the placeholder, and
//! `album.getInfo` orders its JSON keys differently (see the `username` fixtures). So the mirror
//! forwards the request with the user name and only replaces the two values, keeping Last.fm's
//! bytes, field order and JSON types around them.

use bytes::Bytes;
use lfm_core::{Format, Params};
use lfm_store::{Kind, Lookup};

/// The user name of a `getInfo` request: `username`, or `user` as some clients send it.
pub(crate) fn overlay_user(params: &Params) -> Option<&str> {
    params
        .optional("username")
        .or_else(|| params.optional("user"))
}

/// What to look up in the store for a `getInfo` method, or `None` for any other method.
pub(crate) fn lookup(method: &str, params: &Params) -> Option<Lookup> {
    let (kind, name_param) = match method {
        "artist.getInfo" => (Kind::Artist, None),
        "album.getInfo" => (Kind::Album, Some("album")),
        "track.getInfo" => (Kind::Track, Some("track")),
        _ => return None,
    };
    if let Some(mbid) = params.optional("mbid") {
        let mbid = mbid.to_owned();
        return Some(Lookup::Mbid { kind, mbid });
    }
    let artist = params.optional("artist")?.to_owned();
    let Some(name_param) = name_param else {
        return Some(Lookup::Artist { name: artist });
    };
    let name = params.optional(name_param)?.to_owned();
    Some(match kind {
        Kind::Album => Lookup::Album { artist, name },
        _ => Lookup::Track { artist, name },
    })
}

/// The user's own values for one entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Overlay {
    /// Plays of the entity.
    pub(crate) plays: u64,
    /// Whether the user loves it; only `track.getInfo` shows this.
    pub(crate) loved: bool,
}

impl Overlay {
    /// `body` with the user's values in place of Last.fm's. Fields the body lacks stay absent.
    pub(crate) fn apply(self, body: &Bytes, format: Format) -> Bytes {
        let plays = self.plays.to_string();
        let loved = if self.loved { "1" } else { "0" };
        let body = replace(body, format, "userplaycount", &plays);
        Bytes::from(replace(&body, format, "userloved", loved))
    }
}

fn replace(body: &[u8], format: Format, field: &str, value: &str) -> Vec<u8> {
    let span = match format {
        Format::Xml => xml_span(body, field),
        Format::Json => json_span(body, field),
    };
    let Some((start, end)) = span else {
        return body.to_vec();
    };
    let (Some(head), Some(tail)) = (body.get(..start), body.get(end..)) else {
        return body.to_vec();
    };
    [head, value.as_bytes(), tail].concat()
}

/// The text of `<field>…</field>`. A `<` inside text is escaped, so the tag is unambiguous.
fn xml_span(body: &[u8], field: &str) -> Option<(usize, usize)> {
    let open = format!("<{field}>");
    let start = find(body, open.as_bytes(), 0)? + open.len();
    let end = find(body, format!("</{field}>").as_bytes(), start)?;
    Some((start, end))
}

/// The digits of `"field":"123"` or `"field":123`. A quote inside a JSON string is escaped, so
/// a match not preceded by a backslash is the key itself.
fn json_span(body: &[u8], field: &str) -> Option<(usize, usize)> {
    let key = format!("\"{field}\":");
    let mut from = 0;
    let at = loop {
        let at = find(body, key.as_bytes(), from)?;
        if at == 0 || body.get(at - 1) != Some(&b'\\') {
            break at;
        }
        from = at + 1;
    };
    let mut start = at + key.len();
    if body.get(start) == Some(&b'"') {
        start += 1;
    }
    let digits = body
        .get(start..)?
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    Some((start, start + digits))
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| at + from)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL: Overlay = Overlay {
        plays: 42,
        loved: false,
    };

    fn apply(body: &str, format: Format) -> String {
        let out = LOCAL.apply(&Bytes::from(body.to_owned()), format);
        String::from_utf8(out.to_vec()).unwrap()
    }

    #[test]
    fn replaces_string_and_number_values_in_json() {
        let track = r#"{"x":"\"userloved\":1","track":{"userplaycount":"7462","userloved":"1"}}"#;
        let want = r#"{"x":"\"userloved\":1","track":{"userplaycount":"42","userloved":"0"}}"#;
        assert_eq!(apply(track, Format::Json), want);
        let album = r#"{"album":{"name":"A","userplaycount":9264,"image":[]}}"#;
        assert_eq!(
            apply(album, Format::Json),
            r#"{"album":{"name":"A","userplaycount":42,"image":[]}}"#
        );
    }

    #[test]
    fn replaces_xml_text_and_leaves_bodies_without_the_fields() {
        let body = "<stats><playcount>5</playcount><userplaycount>34955</userplaycount>\n</stats>";
        assert_eq!(
            apply(body, Format::Xml),
            "<stats><playcount>5</playcount><userplaycount>42</userplaycount>\n</stats>"
        );
        assert_eq!(apply("<artist/>", Format::Xml), "<artist/>");
    }

    #[test]
    fn looks_up_by_mbid_first_then_by_names() {
        let params = Params::new()
            .with("artist", "Cher")
            .with("track", "Believe");
        assert_eq!(
            lookup("track.getInfo", &params),
            Some(Lookup::Track {
                artist: "Cher".into(),
                name: "Believe".into()
            })
        );
        let by_mbid = params.with("mbid", "m");
        assert!(matches!(
            lookup("artist.getInfo", &by_mbid),
            Some(Lookup::Mbid {
                kind: Kind::Artist,
                ..
            })
        ));
        assert_eq!(lookup("artist.getSimilar", &Params::new()), None);
    }
}
