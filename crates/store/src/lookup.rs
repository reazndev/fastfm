//! A user's play count and loved flag for one artist, album, or track.

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Result;
use crate::names::fold;
use crate::pool::Store;
use crate::types::{Kind, Named, UserId, count};

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

/// A track's length, as `track.getInfo` or a top-tracks page reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackDuration {
    /// Artist name.
    pub artist: String,
    /// Track name.
    pub track: String,
    /// Length in seconds.
    pub seconds: i64,
}

impl Store {
    /// Stores the lengths of tracks the store already knows; unknown tracks are skipped.
    /// Bumps every user's version when a length changed, since top lists show it.
    pub async fn set_track_durations(&self, durations: Vec<TrackDuration>) -> Result<()> {
        self.write(move |tx| {
            let sql = "UPDATE tracks SET duration = ?3 WHERE id = (SELECT t.id FROM tracks t \
                       JOIN artists ar ON ar.id = t.artist_id \
                       WHERE ar.name_fold = +?1 AND t.name_fold = +?2) \
                       AND duration IS NOT ?3";
            let mut stmt = tx.prepare_cached(sql)?;
            let mut changed = 0;
            for d in &durations {
                changed += stmt.execute(params![fold(&d.artist), fold(&d.track), d.seconds])?;
            }
            if changed > 0 {
                tx.prepare_cached("UPDATE users SET version = version + 1")?
                    .execute([])?;
            }
            Ok(())
        })
        .await
    }

    /// Fills in the MBID and URL of artists the store already knows, from sources that show
    /// them where recent tracks do not (top artists, weekly artist charts). Unknown artists are
    /// skipped. Bumps every user's version when something changed.
    pub async fn enrich_artists(&self, artists: Vec<Named>) -> Result<()> {
        self.write(move |tx| {
            let sql = "UPDATE artists SET mbid = coalesce(?2, mbid), url = coalesce(?3, url) \
                       WHERE name_fold = ?1 AND ((?2 IS NOT NULL AND mbid IS NOT ?2) \
                       OR (?3 IS NOT NULL AND url IS NOT ?3))";
            let mut stmt = tx.prepare_cached(sql)?;
            let mut changed = 0;
            for artist in &artists {
                let given = |value: &Option<String>| value.clone().filter(|v| !v.is_empty());
                let values = params![fold(&artist.name), given(&artist.mbid), given(&artist.url)];
                changed += stmt.execute(values)?;
            }
            if changed > 0 {
                tx.prepare_cached("UPDATE users SET version = version + 1")?
                    .execute([])?;
            }
            Ok(())
        })
        .await
    }

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
