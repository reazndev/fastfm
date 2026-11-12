//! A user's play count and loved flag for one artist, album, or track.

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Result;
use crate::names::fold;
use crate::pool::Store;
use crate::types::{Kind, UserId, count};

/// How to find an entity: by name, case-insensitively, or by MBID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// An artist by name.
    Artist {
        /// Artist name.
        name: String,
    },
    /// An album by artist and album name.
    Album {
        /// Artist name.
        artist: String,
        /// Album name.
        name: String,
    },
    /// A track by artist and track name.
    Track {
        /// Artist name.
        artist: String,
        /// Track name.
        name: String,
    },
    /// Any kind by MBID.
    Mbid {
        /// Which table to search.
        kind: Kind,
        /// The MBID.
        mbid: String,
    },
}

impl Lookup {
    fn kind(&self) -> Kind {
        match self {
            Self::Artist { .. } => Kind::Artist,
            Self::Album { .. } => Kind::Album,
            Self::Track { .. } => Kind::Track,
            Self::Mbid { kind, .. } => *kind,
        }
    }
}

/// A user's relation to one entity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UserPlays {
    /// Store id of the entity, when it is known at all.
    pub id: Option<i64>,
    /// The user's plays; 0 when unknown.
    pub plays: u64,
    /// Whether the user loved it; tracks only.
    pub loved: bool,
}

impl Store {
    /// The user's play count and loved flag for the entity `lookup` finds.
    pub async fn user_plays(&self, user: UserId, lookup: Lookup) -> Result<UserPlays> {
        self.read(move |conn| user_plays(conn, user, &lookup)).await
    }
}

fn user_plays(conn: &Connection, user: UserId, lookup: &Lookup) -> Result<UserPlays> {
    let kind = lookup.kind();
    let Some(id) = find(conn, lookup)? else {
        return Ok(UserPlays::default());
    };
    let word = kind.word();
    let sql = format!("SELECT plays FROM user_{word}_total WHERE user_id = ?1 AND {word}_id = ?2");
    let plays: Option<i64> = conn
        .prepare_cached(&sql)?
        .query_row(params![user, id], |row| row.get(0))
        .optional()?;
    let loved = kind == Kind::Track && {
        let sql = "SELECT 1 FROM loved WHERE user_id = ?1 AND track_id = ?2";
        conn.prepare_cached(sql)?.exists(params![user, id])?
    };
    Ok(UserPlays {
        id: Some(id),
        plays: count(plays.unwrap_or(0)),
        loved,
    })
}

fn find(conn: &Connection, lookup: &Lookup) -> Result<Option<i64>> {
    let get = |row: &rusqlite::Row<'_>| row.get(0);
    let found = match lookup {
        Lookup::Artist { name } => {
            let sql = "SELECT id FROM artists WHERE name_fold = ?1";
            conn.prepare_cached(sql)?.query_row([fold(name)], get)
        }
        Lookup::Album { artist, name } => {
            let sql = "SELECT al.id FROM albums al JOIN artists ar ON ar.id = al.artist_id \
                       WHERE ar.name_fold = +?1 AND al.name_fold = +?2";
            conn.prepare_cached(sql)?
                .query_row([fold(artist), fold(name)], get)
        }
        Lookup::Track { artist, name } => {
            let sql = "SELECT t.id FROM tracks t JOIN artists ar ON ar.id = t.artist_id \
                       WHERE ar.name_fold = +?1 AND t.name_fold = +?2";
            conn.prepare_cached(sql)?
                .query_row([fold(artist), fold(name)], get)
        }
        Lookup::Mbid { kind, mbid } => {
            let sql = format!(
                "SELECT id FROM {}s WHERE mbid = ?1 ORDER BY id LIMIT 1",
                kind.word()
            );
            conn.prepare_cached(&sql)?.query_row([mbid], get)
        }
    };
    Ok(found.optional()?)
}
