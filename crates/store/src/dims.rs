//! Artists, albums, and tracks: resolving names to ids on write, and loading them on read.

use std::collections::HashMap;

use rusqlite::{Connection, Row, params};

use crate::error::Result;
use crate::names::{fold, image_key, non_empty};
use crate::types::{Entity, Kind, Named};

/// Ids of one scrobble's entities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ids {
    pub(crate) artist: i64,
    pub(crate) album: Option<i64>,
    pub(crate) track: i64,
}

impl Ids {
    /// The ids by kind. A scrobble without an album has no album id.
    pub(crate) fn by_kind(self) -> impl Iterator<Item = (Kind, i64)> {
        [
            (Kind::Artist, Some(self.artist)),
            (Kind::Album, self.album),
            (Kind::Track, Some(self.track)),
        ]
        .into_iter()
        .filter_map(|(kind, id)| Some((kind, id?)))
    }
}

/// What the resolver has already written: kind, parent artist id (0 for artists), folded name,
/// and the optional values given with it. A later sighting with other values upserts again, so
/// an MBID or image that turns up later in the same batch is not lost.
type Seen = (Kind, i64, String, [Option<String>; 3]);

/// Upserts entities inside one transaction, remembering ids it has already resolved.
pub(crate) struct Resolver<'c> {
    conn: &'c Connection,
    seen: HashMap<Seen, i64>,
}

impl<'c> Resolver<'c> {
    pub(crate) fn new(conn: &'c Connection) -> Self {
        Self {
            conn,
            seen: HashMap::new(),
        }
    }

    fn upsert(&mut self, key: Seen, sql: &str, values: impl rusqlite::Params) -> Result<i64> {
        if let Some(id) = self.seen.get(&key) {
            return Ok(*id);
        }
        let id = self
            .conn
            .prepare_cached(sql)?
            .query_row(values, |row| row.get(0))?;
        self.seen.insert(key, id);
        Ok(id)
    }

    /// Resolves an artist by folded name, creating it if needed.
    pub(crate) fn artist(&mut self, artist: &Named) -> Result<i64> {
        let name_fold = fold(&artist.name);
        let (mbid, url) = (opt(artist.mbid.as_ref()), opt(artist.url.as_ref()));
        let key = (Kind::Artist, 0, name_fold.clone(), owned([mbid, url, None]));
        let sql = "INSERT INTO artists (name, name_fold, mbid, url) VALUES (?1, ?2, ?3, ?4) \
                   ON CONFLICT (name_fold) DO UPDATE SET mbid = coalesce(excluded.mbid, mbid), \
                   url = coalesce(excluded.url, url) RETURNING id";
        self.upsert(key, sql, params![artist.name, name_fold, mbid, url])
    }

    /// Resolves an album of `artist_id` as scrobbled: one row per spelling and cover, since
    /// Last.fm may file them under different library albums.
    pub(crate) fn album(
        &mut self,
        artist_id: i64,
        album: &Named,
        image: Option<&str>,
    ) -> Result<i64> {
        let name_fold = fold(&album.name);
        let image = non_empty(image).and_then(image_key);
        let variant = scrobbled_variant(&album.name, image.as_deref());
        let mbid = opt(album.mbid.as_ref());
        let key = (
            Kind::Album,
            artist_id,
            variant.clone(),
            owned([mbid, None, None]),
        );
        let sql = "INSERT INTO albums (artist_id, name, name_fold, mbid, image, variant) \
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                   ON CONFLICT (artist_id, name_fold, variant) DO UPDATE \
                   SET mbid = coalesce(excluded.mbid, mbid) RETURNING id";
        let values = params![artist_id, album.name, name_fold, mbid, image, variant];
        self.upsert(key, sql, values)
    }

    /// Resolves a track of `artist_id`, creating it if needed.
    ///
    /// `artist` is the artist as Last.fm showed it with this track: its MBID may be empty even
    /// when the artist has one (recent and loved tracks repeat it per track). Its exact
    /// spelling is part of the track's key, since Last.fm keeps tracks of artist spellings that
    /// differ only in case apart, and is shown with the track when it differs from the artist
    /// row's.
    pub(crate) fn track(&mut self, artist_id: i64, track: &Named, artist: &Named) -> Result<i64> {
        let name_fold = fold(&track.name);
        let (mbid, url) = (opt(track.mbid.as_ref()), opt(track.url.as_ref()));
        let artist_mbid = Some(shown_mbid(artist));
        let key = (
            Kind::Track,
            artist_id,
            format!("{name_fold}\u{1f}{}", artist.name),
            owned([mbid, url, artist_mbid]),
        );
        let sql = "INSERT INTO tracks (artist_id, name, name_fold, mbid, url, artist_mbid, \
                   artist_name, artist_spelling) VALUES (?1, ?2, ?3, ?4, ?5, ?6, \
                   nullif(?7, (SELECT name FROM artists WHERE id = ?1)), ?7) \
                   ON CONFLICT (artist_id, name_fold, artist_spelling) DO UPDATE \
                   SET mbid = coalesce(excluded.mbid, mbid), url = coalesce(excluded.url, url), \
                   artist_mbid = coalesce(excluded.artist_mbid, artist_mbid) RETURNING id";
        let values = params![
            artist_id,
            track.name,
            name_fold,
            mbid,
            url,
            artist_mbid,
            artist.name
        ];
        self.upsert(key, sql, values)
    }

    /// Resolves all entities of one play. An album with an empty name counts as none.
    pub(crate) fn ids(
        &mut self,
        artist: &Named,
        album: Option<&Named>,
        track: &Named,
        image: Option<&str>,
    ) -> Result<Ids> {
        let artist_id = self.artist(artist)?;
        let album = album.filter(|album| !album.name.trim().is_empty());
        let album_id = album
            .map(|album| self.album(artist_id, album, image))
            .transpose()?;
        let track_id = self.track(artist_id, track, artist)?;
        Ok(Ids {
            artist: artist_id,
            album: album_id,
            track: track_id,
        })
    }
}

/// The variant of a scrobbled album: its exact spelling and cover.
pub(crate) fn scrobbled_variant(name: &str, image: Option<&str>) -> String {
    format!("{name}\u{1f}{}", image.unwrap_or_default())
}

/// The artist MBID as shown next to a track: empty when none was given.
pub(crate) fn shown_mbid(artist: &Named) -> &str {
    opt(artist.mbid.as_ref()).unwrap_or_default()
}

fn owned(values: [Option<&str>; 3]) -> [Option<String>; 3] {
    values.map(|value| value.map(str::to_owned))
}

fn opt(value: Option<&String>) -> Option<&str> {
    non_empty(value.map(String::as_str))
}

/// The artist columns of a track joined as `t` with its artist as `ar`: the spelling, MBID and
/// URL Last.fm shows with the track. `entity_at` replaces the URL with the track URL's artist
/// part when there is one.
macro_rules! track_artist_columns {
    () => {
        "coalesce(t.artist_name, ar.name), coalesce(t.artist_mbid, ar.mbid), ar.url"
    };
}
pub(crate) use track_artist_columns;

/// In statements that join, parameters compared with `=` are written `+?N`. A plain `?N`
/// lets `SQLite` propagate the value through the join, which makes it re-prepare the statement
/// every time the statement cache clears its bindings, costing several microseconds per use.
///
/// Columns shared by `entity_sql`: id, name, mbid, url, image, then the artist's name, mbid, url,
/// then the duration. Artists show as Last.fm's library spells them; a library album shows its
/// URL and the artist MBID Last.fm shows with it.
pub(crate) fn entity_sql(kind: Kind) -> &'static str {
    match kind {
        Kind::Artist => {
            "SELECT id, coalesce(lib_name, name), mbid, url, NULL, NULL, NULL, NULL, NULL \
             FROM artists WHERE id = ?1"
        }
        Kind::Album => {
            "SELECT al.id, al.name, al.mbid, al.url, al.image, coalesce(al.artist_name, ar.lib_name, ar.name), \
             coalesce(al.artist_mbid, ar.mbid), ar.url, NULL \
             FROM albums al JOIN artists ar ON ar.id = al.artist_id WHERE al.id = +?1"
        }
        Kind::Track => concat!(
            "SELECT t.id, t.name, t.mbid, t.url, NULL, ",
            track_artist_columns!(),
            ", t.duration FROM tracks t JOIN artists ar ON ar.id = t.artist_id WHERE t.id = +?1"
        ),
    }
}

/// The artist name and the name of one entity, as `entity_sql` shows them: what ties sort by.
pub(crate) fn name_sql(kind: Kind) -> &'static str {
    match kind {
        Kind::Artist => "SELECT NULL, coalesce(lib_name, name) FROM artists WHERE id = ?1",
        Kind::Album => {
            "SELECT coalesce(al.artist_name, ar.lib_name, ar.name), al.name \
             FROM albums al JOIN artists ar ON ar.id = al.artist_id WHERE al.id = +?1"
        }
        Kind::Track => {
            "SELECT coalesce(t.artist_name, ar.name), t.name \
             FROM tracks t JOIN artists ar ON ar.id = t.artist_id WHERE t.id = +?1"
        }
    }
}

/// Reads an entity from nine columns starting at `at`, in the order of `entity_sql`.
pub(crate) fn entity_at(row: &Row<'_>, at: usize) -> rusqlite::Result<Entity> {
    let url: Option<String> = row.get(at + 3)?;
    let artist_name: Option<String> = row.get(at + 5)?;
    let artist = artist_name
        .map(|name| -> rusqlite::Result<Named> {
            let row_url: Option<String> = row.get(at + 7)?;
            Ok(Named {
                name,
                mbid: row.get(at + 6)?,
                url: url.as_deref().and_then(artist_url_of).or(row_url),
            })
        })
        .transpose()?;
    Ok(Entity {
        id: row.get(at)?,
        name: row.get(at + 1)?,
        mbid: row.get(at + 2)?,
        url,
        image: row.get(at + 4)?,
        artist,
        duration: row.get(at + 8)?,
    })
}

/// The artist URL Last.fm shows next to an album or track: the item URL's artist part, with
/// its `+noredirect` if it has one. Last.fm never takes it from the artist itself, whose URL can
/// differ in spelling. Of about 7,800 items checked on 2026-11-13, one did not follow this: a
/// `+noredirect` track URL with a plain artist URL, against eight that kept it.
fn artist_url_of(item_url: &str) -> Option<String> {
    const MUSIC: &str = "/music/";
    const NOREDIRECT: &str = "+noredirect/";
    let (base, path) = item_url.split_once(MUSIC)?;
    let (prefix, path) = match path.strip_prefix(NOREDIRECT) {
        Some(rest) => (NOREDIRECT, rest),
        None => ("", path),
    };
    let artist = path.split('/').next().filter(|artist| !artist.is_empty())?;
    Some(format!("{base}{MUSIC}{prefix}{artist}"))
}

#[cfg(test)]
mod tests {
    use super::artist_url_of;

    #[test]
    fn artist_url_is_the_item_urls_artist_part() {
        let cases = [
            (
                "https://www.last.fm/music/tripleS/ASSEMBLE",
                "https://www.last.fm/music/tripleS",
            ),
            (
                "https://www.last.fm/music/+noredirect/Jeezy/_/Seen+It+All",
                "https://www.last.fm/music/+noredirect/Jeezy",
            ),
            (
                "https://www.last.fm/music/Cher/_/Believe",
                "https://www.last.fm/music/Cher",
            ),
        ];
        for (item, artist) in cases {
            assert_eq!(artist_url_of(item).as_deref(), Some(artist), "{item}");
        }
        assert_eq!(artist_url_of(""), None);
        assert_eq!(artist_url_of("https://www.last.fm/music/"), None);
    }
}
