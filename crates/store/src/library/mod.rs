//! Last.fm's library: the artists, albums and tracks its top lists count a user's scrobbles
//! under, which are not always the ones scrobbled.
//!
//! Scrobbles keep the entities they were scrobbled with. `user_library_map` names, per user,
//! each scrobbled entity that counts under another one, and every aggregate (totals,
//! histograms, rollups, user counters) is kept in library entities. Enrichment learns the map
//! from Last.fm's overall top lists and applies it with [`Store::apply_library`], which moves
//! the plays of every entity whose target changed. Ingest maps each new scrobble through the
//! stored map, so a known name counts in the right place at once and an unknown one counts as
//! itself until the next enrichment.

mod apply;

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};

use crate::dims::Ids;
use crate::error::Result;
use crate::lookup::Lookup;
use crate::pool::Store;
use crate::types::{Kind, Named, UserId, count};

/// An artist, album or track a user has scrobbled, with the user's plays of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scrobbled {
    /// Store id.
    pub id: i64,
    /// The artist's name; for an artist, its own name.
    pub artist: String,
    /// The album or track name; for an artist, its own name again.
    pub name: String,
    /// An album's cover as scrobbled, as an image key; `None` for artists and tracks.
    pub image: Option<String>,
    /// The user's scrobbles of it.
    pub plays: u64,
}

/// What enrichment needs to know about one user's scrobbles.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LibraryInputs {
    /// Scrobbled artists.
    pub artists: Vec<Scrobbled>,
    /// Scrobbled albums, keyed by the track artist, one per spelling and cover.
    pub albums: Vec<Scrobbled>,
    /// Scrobbled tracks.
    pub tracks: Vec<Scrobbled>,
    /// Scrobbled artists that count under another artist now: id to that artist's name.
    pub artist_targets: HashMap<i64, String>,
}

/// An album as a top-albums list shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LibAlbum {
    /// The album artist, with the artist MBID shown next to this album.
    pub artist: Named,
    /// The album, with its MBID and URL.
    pub album: Named,
    /// The cover: a Last.fm image URL or key; `None` for none.
    pub image: Option<String>,
}

/// A track as a top-tracks list shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LibTrack {
    /// The artist as spelled with this track, with the artist MBID shown next to it.
    pub artist: Named,
    /// The track, with its MBID and URL.
    pub track: Named,
    /// Length in seconds; 0 when Last.fm does not know it.
    pub duration: i64,
}

/// A scrobbled entity that counts under another one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibMove {
    /// Store id of the scrobbled artist, album or track.
    pub raw: i64,
    /// The library entity it counts under, of the same kind, by name.
    pub to: Lookup,
}

/// Last.fm's library view of one user's scrobbles, as enrichment reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Library {
    /// Artists as the top-artists list shows them.
    pub artists: Vec<Named>,
    /// Albums as the top-albums list shows them.
    pub albums: Vec<LibAlbum>,
    /// Tracks as the top-tracks list shows them.
    pub tracks: Vec<LibTrack>,
    /// The complete set of scrobbled entities that count under another one. Entities left
    /// out count as themselves.
    pub moves: Vec<LibMove>,
}

impl Store {
    /// The user's scrobbled artists, albums and tracks with their plays, and where artists
    /// count now.
    pub async fn library_inputs(&self, user: UserId) -> Result<LibraryInputs> {
        self.read(move |conn| {
            Ok(LibraryInputs {
                artists: scrobbled(conn, user, Kind::Artist)?,
                albums: scrobbled(conn, user, Kind::Album)?,
                tracks: scrobbled(conn, user, Kind::Track)?,
                artist_targets: artist_targets(conn, user)?,
            })
        })
        .await
    }
}

fn scrobbled(conn: &Connection, user: UserId, kind: Kind) -> Result<Vec<Scrobbled>> {
    let plays = |word: &str| {
        format!(
            "(SELECT {word}_id AS id, count(*) AS n FROM scrobbles \
             WHERE user_id = ?1 AND {word}_id IS NOT NULL GROUP BY {word}_id) x"
        )
    };
    let sql = match kind {
        Kind::Artist => format!(
            "SELECT a.id, a.name, a.name, NULL, x.n FROM {} JOIN artists a ON a.id = x.id",
            plays("artist")
        ),
        Kind::Album => format!(
            "SELECT e.id, ar.name, e.name, e.image, x.n FROM {} JOIN albums e ON e.id = x.id \
             JOIN artists ar ON ar.id = e.artist_id",
            plays("album")
        ),
        Kind::Track => format!(
            "SELECT e.id, ar.name, e.name, NULL, x.n FROM {} JOIN tracks e ON e.id = x.id \
             JOIN artists ar ON ar.id = e.artist_id",
            plays("track")
        ),
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([user], |row| {
        Ok(Scrobbled {
            id: row.get(0)?,
            artist: row.get(1)?,
            name: row.get(2)?,
            image: row.get(3)?,
            plays: count(row.get(4)?),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn artist_targets(conn: &Connection, user: UserId) -> Result<HashMap<i64, String>> {
    let sql = "SELECT m.raw_id, coalesce(a.lib_name, a.name) FROM user_library_map m \
               JOIN artists a ON a.id = m.lib_id WHERE m.user_id = ?1 AND m.kind = ?2";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![user, Kind::Artist.code()], |row| {
        Ok((row.get(0)?, row.get(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Maps scrobbled entity ids to the library entities they count under, for one user, caching
/// each answer for the length of one write.
#[derive(Debug)]
pub(crate) struct LibMap {
    user: UserId,
    assign: bool,
    cache: HashMap<(Kind, i64), i64>,
}

impl LibMap {
    /// A map that only reads the stored targets.
    pub(crate) fn new(user: UserId) -> Self {
        Self {
            user,
            assign: false,
            cache: HashMap::new(),
        }
    }

    /// A map for ingest, which also gives an album spelling or cover seen for the first time
    /// the target of the others, and stores it.
    pub(crate) fn assigning(user: UserId) -> Self {
        Self {
            assign: true,
            ..Self::new(user)
        }
    }

    /// The library entity `raw` counts under.
    pub(crate) fn id(&mut self, conn: &Connection, kind: Kind, raw: i64) -> Result<i64> {
        if let Some(id) = self.cache.get(&(kind, raw)) {
            return Ok(*id);
        }
        let sql = "SELECT lib_id FROM user_library_map \
                   WHERE user_id = ?1 AND kind = ?2 AND raw_id = ?3";
        let stored = conn
            .prepare_cached(sql)?
            .query_row(params![self.user, kind.code(), raw], |row| row.get(0))
            .optional()?;
        let id = match stored {
            Some(id) => id,
            None if self.assign && kind == Kind::Album => assign_album(conn, self.user, raw)?,
            None => raw,
        };
        self.cache.insert((kind, raw), id);
        Ok(id)
    }

    /// The library entities a scrobble of `ids` counts under.
    pub(crate) fn ids(&mut self, conn: &Connection, ids: Ids) -> Result<Ids> {
        Ok(Ids {
            artist: self.id(conn, Kind::Artist, ids.artist)?,
            album: ids
                .album
                .map(|album| self.id(conn, Kind::Album, album))
                .transpose()?,
            track: self.id(conn, Kind::Track, ids.track)?,
        })
    }
}

/// Where a scrobbled album without a stored target counts: where another variant of it counts
/// for the user (one with the same spelling first), else under the oldest variant. Stores the answer, so the plays can
/// be taken out the same way.
fn assign_album(conn: &Connection, user: UserId, raw: i64) -> Result<i64> {
    let target: Option<i64> = conn
        .prepare_cached(SIBLING_SQL)?
        .query_row(params![user, raw], |row| row.get(0))
        .optional()?;
    match target {
        Some(target) if target != raw => {
            let sql = "INSERT INTO user_library_map (user_id, kind, raw_id, lib_id) \
                       VALUES (?1, ?2, ?3, ?4)";
            conn.prepare_cached(sql)?
                .execute(params![user, Kind::Album.code(), raw, target])?;
            Ok(target)
        }
        _ => Ok(raw),
    }
}

/// The target of the other scrobbled variants of album `?2` for user `?1`: a stored one first,
/// preferring the same spelling, else the oldest variant, which may be the album itself.
const SIBLING_SQL: &str = "\
    SELECT coalesce( \
        (SELECT m.lib_id FROM albums b \
         JOIN user_library_map m ON m.user_id = ?1 AND m.kind = 1 AND m.raw_id = b.id \
         WHERE b.artist_id = a.artist_id AND b.name_fold = a.name_fold \
         AND b.variant != '' AND b.id != a.id ORDER BY b.name != a.name, b.id LIMIT 1), \
        (SELECT min(b.id) FROM albums b WHERE b.artist_id = a.artist_id \
         AND b.name_fold = a.name_fold AND b.variant != '')) \
    FROM albums a WHERE a.id = ?2";
