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

/// Upserts entities inside one transaction, remembering ids it has already resolved.
pub(crate) struct Resolver<'c> {
    conn: &'c Connection,
    artists: HashMap<String, i64>,
    children: HashMap<(Kind, i64, String), i64>,
}

impl<'c> Resolver<'c> {
    pub(crate) fn new(conn: &'c Connection) -> Self {
        Self {
            conn,
            artists: HashMap::new(),
            children: HashMap::new(),
        }
    }

    /// Resolves an artist by folded name, creating it if needed.
    pub(crate) fn artist(&mut self, artist: &Named) -> Result<i64> {
        let key = fold(&artist.name);
        if let Some(id) = self.artists.get(&key) {
            return Ok(*id);
        }
        let sql = "INSERT INTO artists (name, name_fold, mbid, url) VALUES (?1, ?2, ?3, ?4) \
                   ON CONFLICT (name_fold) DO UPDATE SET mbid = coalesce(excluded.mbid, mbid), \
                   url = coalesce(excluded.url, url) RETURNING id";
        let id = self.conn.prepare_cached(sql)?.query_row(
            params![
                artist.name,
                key,
                opt(artist.mbid.as_ref()),
                opt(artist.url.as_ref())
            ],
            |row| row.get(0),
        )?;
        self.artists.insert(key, id);
        Ok(id)
    }

    /// Resolves an album of `artist_id`, creating it if needed and keeping the newest image.
    pub(crate) fn album(
        &mut self,
        artist_id: i64,
        album: &Named,
        image: Option<&str>,
    ) -> Result<i64> {
        let key = (Kind::Album, artist_id, fold(&album.name));
        if let Some(id) = self.children.get(&key) {
            return Ok(*id);
        }
        let sql = "INSERT INTO albums (artist_id, name, name_fold, mbid, image) \
                   VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (artist_id, name_fold) DO UPDATE \
                   SET mbid = coalesce(excluded.mbid, mbid), image = coalesce(excluded.image, image) \
                   RETURNING id";
        let image = non_empty(image).and_then(image_key);
        let values = params![
            artist_id,
            album.name,
            key.2,
            opt(album.mbid.as_ref()),
            image
        ];
        let id = self
            .conn
            .prepare_cached(sql)?
            .query_row(values, |row| row.get(0))?;
        self.children.insert(key, id);
        Ok(id)
    }

    /// Resolves a track of `artist_id`, creating it if needed.
    pub(crate) fn track(&mut self, artist_id: i64, track: &Named) -> Result<i64> {
        let key = (Kind::Track, artist_id, fold(&track.name));
        if let Some(id) = self.children.get(&key) {
            return Ok(*id);
        }
        let sql = "INSERT INTO tracks (artist_id, name, name_fold, mbid, url) \
                   VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (artist_id, name_fold) DO UPDATE \
                   SET mbid = coalesce(excluded.mbid, mbid), url = coalesce(excluded.url, url) \
                   RETURNING id";
        let values = params![
            artist_id,
            track.name,
            key.2,
            opt(track.mbid.as_ref()),
            opt(track.url.as_ref())
        ];
        let id = self
            .conn
            .prepare_cached(sql)?
            .query_row(values, |row| row.get(0))?;
        self.children.insert(key, id);
        Ok(id)
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
        let track_id = self.track(artist_id, track)?;
        Ok(Ids {
            artist: artist_id,
            album: album_id,
            track: track_id,
        })
    }
}

fn opt(value: Option<&String>) -> Option<&str> {
    non_empty(value.map(String::as_str))
}

/// In statements that join, parameters compared with `=` are written `+?N`. A plain `?N`
/// lets `SQLite` propagate the value through the join, which makes it re-prepare the statement
/// every time the statement cache clears its bindings, costing several microseconds per use.
///
/// Columns shared by `entity_sql`: id, name, mbid, url, image, then the artist's name, mbid, url.
pub(crate) fn entity_sql(kind: Kind) -> &'static str {
    match kind {
        Kind::Artist => {
            "SELECT id, name, mbid, url, NULL, NULL, NULL, NULL FROM artists WHERE id = ?1"
        }
        Kind::Album => {
            "SELECT al.id, al.name, al.mbid, NULL, al.image, ar.name, ar.mbid, ar.url \
             FROM albums al JOIN artists ar ON ar.id = al.artist_id WHERE al.id = +?1"
        }
        Kind::Track => {
            "SELECT t.id, t.name, t.mbid, t.url, NULL, ar.name, ar.mbid, ar.url \
             FROM tracks t JOIN artists ar ON ar.id = t.artist_id WHERE t.id = +?1"
        }
    }
}

/// Reads an entity from eight columns starting at `at`, in the order of `entity_sql`.
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
    })
}

/// Loads one entity by id.
pub(crate) fn load(conn: &Connection, kind: Kind, id: i64) -> Result<Option<Entity>> {
    let mut stmt = conn.prepare_cached(entity_sql(kind))?;
    Ok(stmt.query_row([id], |row| entity_at(row, 0)).optional()?)
}
