//! Artists, albums, and tracks: resolving names to ids on write, and loading them on read.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, Row, params};

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

    /// Resolves an album of `artist_id`, creating it if needed and keeping the newest image.
    pub(crate) fn album(
        &mut self,
        artist_id: i64,
        album: &Named,
        image: Option<&str>,
    ) -> Result<i64> {
        let name_fold = fold(&album.name);
        let image = non_empty(image).and_then(image_key);
        let mbid = opt(album.mbid.as_ref());
        let key = (
            Kind::Album,
            artist_id,
            name_fold.clone(),
            owned([mbid, image.as_deref(), None]),
        );
        let sql = "INSERT INTO albums (artist_id, name, name_fold, mbid, image) \
                   VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (artist_id, name_fold) DO UPDATE \
                   SET mbid = coalesce(excluded.mbid, mbid), \
                   image = coalesce(excluded.image, image) RETURNING id";
        self.upsert(
            key,
            sql,
            params![artist_id, album.name, name_fold, mbid, image],
        )
    }

    /// Resolves a track of `artist_id`, creating it if needed.
    ///
    /// `artist_mbid` is the artist MBID Last.fm showed with this track, which may be empty even
    /// when the artist has one; recent and loved tracks repeat it per track.
    pub(crate) fn track(
        &mut self,
        artist_id: i64,
        track: &Named,
        artist_mbid: Option<&str>,
    ) -> Result<i64> {
        let name_fold = fold(&track.name);
        let (mbid, url) = (opt(track.mbid.as_ref()), opt(track.url.as_ref()));
        let key = (
            Kind::Track,
            artist_id,
            name_fold.clone(),
            owned([mbid, url, artist_mbid]),
        );
        let sql = "INSERT INTO tracks (artist_id, name, name_fold, mbid, url, artist_mbid) \
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT (artist_id, name_fold) DO UPDATE \
                   SET mbid = coalesce(excluded.mbid, mbid), url = coalesce(excluded.url, url), \
                   artist_mbid = coalesce(excluded.artist_mbid, artist_mbid) RETURNING id";
        let values = params![artist_id, track.name, name_fold, mbid, url, artist_mbid];
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
        let track_id = self.track(artist_id, track, Some(shown_mbid(artist)))?;
        Ok(Ids {
            artist: artist_id,
            album: album_id,
            track: track_id,
        })
    }
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

/// In statements that join, parameters compared with `=` are written `+?N`. A plain `?N`
/// lets `SQLite` propagate the value through the join, which makes it re-prepare the statement
/// every time the statement cache clears its bindings, costing several microseconds per use.
///
/// Columns shared by `entity_sql`: id, name, mbid, url, image, then the artist's name, mbid, url,
/// then the duration. A track's artist MBID is the one shown with the track.
pub(crate) fn entity_sql(kind: Kind) -> &'static str {
    match kind {
        Kind::Artist => {
            "SELECT id, name, mbid, url, NULL, NULL, NULL, NULL, NULL FROM artists WHERE id = ?1"
        }
        Kind::Album => {
            "SELECT al.id, al.name, al.mbid, NULL, al.image, ar.name, ar.mbid, ar.url, NULL \
             FROM albums al JOIN artists ar ON ar.id = al.artist_id WHERE al.id = +?1"
        }
        Kind::Track => {
            "SELECT t.id, t.name, t.mbid, t.url, NULL, ar.name, \
             coalesce(t.artist_mbid, ar.mbid), ar.url, t.duration \
             FROM tracks t JOIN artists ar ON ar.id = t.artist_id WHERE t.id = +?1"
        }
    }
}

/// Reads an entity from nine columns starting at `at`, in the order of `entity_sql`.
pub(crate) fn entity_at(row: &Row<'_>, at: usize) -> rusqlite::Result<Entity> {
    let artist_name: Option<String> = row.get(at + 5)?;
    let artist = artist_name
        .map(|name| -> rusqlite::Result<Named> {
            Ok(Named {
                name,
                mbid: row.get(at + 6)?,
                url: row.get(at + 7)?,
            })
        })
        .transpose()?;
    Ok(Entity {
        id: row.get(at)?,
        name: row.get(at + 1)?,
        mbid: row.get(at + 2)?,
        url: row.get(at + 3)?,
        image: row.get(at + 4)?,
        artist,
        duration: row.get(at + 8)?,
    })
}

/// Loads one entity by id.
pub(crate) fn load(conn: &Connection, kind: Kind, id: i64) -> Result<Option<Entity>> {
    let mut stmt = conn.prepare_cached(entity_sql(kind))?;
    Ok(stmt.query_row([id], |row| entity_at(row, 0)).optional()?)
}
