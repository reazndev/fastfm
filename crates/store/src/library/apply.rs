//! Writing a [`Library`]: the library's names and details, the user's map, and the plays that
//! move with it.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, params};

use super::{LibAlbum, LibTrack, Library};
use crate::delta::Delta;
use crate::dims::Resolver;
use crate::error::Result;
use crate::lookup::find;
use crate::names::{fold, image_key, non_empty};
use crate::pool::Store;
use crate::types::{Kind, Named, UserId};
use crate::users::period_day;

/// Scrobbled entity to library entity, by kind; identity entries are left out.
type Map = HashMap<(Kind, i64), i64>;

impl Store {
    /// Stores what enrichment learned about `user` in one transaction: the details of every
    /// listed artist, album and track (creating the ones never scrobbled under that name), and
    /// the user's map, replacing the old one. Plays of every entity whose target changed move
    /// to the new target in the totals, rollups and counters. Bumps every user's version, since
    /// the details show in other users' lists too.
    pub async fn apply_library(&self, user: UserId, library: Library) -> Result<()> {
        self.write(move |tx| {
            let mut resolver = Resolver::new(tx);
            for artist in &library.artists {
                write_artist(tx, &mut resolver, artist)?;
            }
            for album in &library.albums {
                write_album(tx, &mut resolver, album)?;
            }
            for track in &library.tracks {
                write_track(tx, &mut resolver, track)?;
            }
            let mut new = Map::new();
            for change in &library.moves {
                if let Some(lib) = find(tx, &change.to)?.filter(|lib| *lib != change.raw) {
                    new.insert((change.to.kind(), change.raw), lib);
                }
            }
            fill_variants(tx, user, &mut new)?;
            let old = load_map(tx, user)?;
            let mut delta = Delta::new(period_day(tx, user)?);
            for kind in Kind::ALL {
                let moved = changes(&old, &new, kind);
                shift_plays(tx, user, (kind, &moved), &mut delta)?;
            }
            save_map(tx, user, &new)?;
            delta.apply(tx, user)?;
            tx.prepare_cached("UPDATE users SET version = version + 1")?
                .execute([])?;
            Ok(())
        })
        .await
    }
}

fn write_artist(conn: &Connection, resolver: &mut Resolver<'_>, artist: &Named) -> Result<()> {
    let id = resolver.artist(artist)?;
    let sql = "UPDATE artists SET mbid = ?2, url = coalesce(?3, url), \
               lib_name = nullif(?4, name) WHERE id = ?1";
    let values = params![
        id,
        given(artist.mbid.as_ref()),
        given(artist.url.as_ref()),
        artist.name
    ];
    conn.prepare_cached(sql)?.execute(values)?;
    Ok(())
}

fn write_album(conn: &Connection, resolver: &mut Resolver<'_>, lib: &LibAlbum) -> Result<()> {
    // The URL only fills in for an artist seen nowhere else, such as `Various Artists`; the
    // artist MBID shown with an album belongs to the album.
    let artist = resolver.artist(&Named::new(lib.artist.name.clone()))?;
    let sql = "UPDATE artists SET url = coalesce(url, ?2) WHERE id = ?1";
    conn.prepare_cached(sql)?
        .execute(params![artist, given(lib.artist.url.as_ref())])?;
    let image = non_empty(lib.image.as_deref())
        .and_then(image_key)
        .unwrap_or_default();
    let sql = "INSERT INTO albums (artist_id, name, name_fold, mbid, image, variant, url, \
               artist_mbid) VALUES (?1, ?2, ?3, ?4, ?5, '', ?6, ?7) \
               ON CONFLICT (artist_id, name_fold, variant) DO UPDATE SET name = excluded.name, \
               mbid = excluded.mbid, image = excluded.image, url = excluded.url, \
               artist_mbid = excluded.artist_mbid";
    let album = &lib.album;
    let values = params![
        artist,
        album.name,
        fold(&album.name),
        given(album.mbid.as_ref()),
        image,
        given(album.url.as_ref()),
        given(lib.artist.mbid.as_ref()).unwrap_or_default()
    ];
    conn.prepare_cached(sql)?.execute(values)?;
    Ok(())
}

fn write_track(conn: &Connection, resolver: &mut Resolver<'_>, lib: &LibTrack) -> Result<()> {
    let artist = resolver.artist(&Named::new(lib.artist.name.clone()))?;
    let id = resolver.track(artist, &lib.track, &lib.artist)?;
    let sql = "UPDATE tracks SET duration = ?2, artist_mbid = ?3 WHERE id = ?1";
    let artist_mbid = given(lib.artist.mbid.as_ref()).unwrap_or_default();
    conn.prepare_cached(sql)?
        .execute(params![id, lib.duration, artist_mbid])?;
    Ok(())
}

/// A value Last.fm gave, with `""` meaning none.
fn given(value: Option<&String>) -> Option<String> {
    value.filter(|v| !v.is_empty()).cloned()
}

/// Gives every scrobbled album variant without a target the target of another variant (one
/// with the same spelling first), or the oldest variant, as ingest does for a new variant.
fn fill_variants(conn: &Connection, user: UserId, map: &mut Map) -> Result<()> {
    let sql = "SELECT DISTINCT a.id, b.id, b.name != a.name FROM scrobbles s \
               JOIN albums a ON a.id = s.album_id \
               JOIN albums b ON b.artist_id = a.artist_id AND b.name_fold = a.name_fold \
               AND b.variant != '' AND b.id != a.id WHERE s.user_id = ?1 ORDER BY a.id, 3, b.id";
    let mut stmt = conn.prepare(sql)?;
    let pairs = stmt
        .query_map([user], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut siblings: HashMap<i64, Vec<i64>> = HashMap::new();
    for (raw, sibling) in pairs {
        siblings.entry(raw).or_default().push(sibling);
    }
    let targets: Map = siblings
        .iter()
        .filter(|(raw, _)| !map.contains_key(&(Kind::Album, **raw)))
        .map(|(raw, others)| {
            let mapped = others
                .iter()
                .find_map(|other| map.get(&(Kind::Album, *other)).copied());
            let oldest = others.iter().copied().min().unwrap_or(*raw).min(*raw);
            ((Kind::Album, *raw), mapped.unwrap_or(oldest))
        })
        .filter(|((_, raw), target)| raw != target)
        .collect();
    map.extend(targets);
    Ok(())
}

fn load_map(conn: &Connection, user: UserId) -> Result<Map> {
    let sql = "SELECT kind, raw_id, lib_id FROM user_library_map WHERE user_id = ?1";
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map([user], |row| {
        Ok((row.get::<_, i64>(0)?, row.get(1)?, row.get(2)?))
    })?;
    let mut map = Map::new();
    for row in rows {
        let (code, raw, lib) = row?;
        if let Some(kind) = Kind::ALL.into_iter().find(|kind| kind.code() == code) {
            map.insert((kind, raw), lib);
        }
    }
    Ok(map)
}

fn save_map(conn: &Connection, user: UserId, map: &Map) -> Result<()> {
    conn.prepare_cached("DELETE FROM user_library_map WHERE user_id = ?1")?
        .execute([user])?;
    let sql = "INSERT INTO user_library_map (user_id, kind, raw_id, lib_id) \
               VALUES (?1, ?2, ?3, ?4)";
    let mut insert = conn.prepare_cached(sql)?;
    for ((kind, raw), lib) in map {
        insert.execute(params![user, kind.code(), raw, lib])?;
    }
    Ok(())
}

/// The scrobbled entities of `kind` whose target differs between the maps, with the old and
/// the new target.
fn changes(old: &Map, new: &Map, kind: Kind) -> HashMap<i64, (i64, i64)> {
    let target = |map: &Map, raw: i64| map.get(&(kind, raw)).copied().unwrap_or(raw);
    old.keys()
        .chain(new.keys())
        .filter(|(of, _)| *of == kind)
        .map(|(_, raw)| (*raw, (target(old, *raw), target(new, *raw))))
        .filter(|(_, (from, to))| from != to)
        .collect()
}

/// Moves every scrobble of the changed entities to their new target.
fn shift_plays(
    conn: &Connection,
    user: UserId,
    (kind, moved): (Kind, &HashMap<i64, (i64, i64)>),
    delta: &mut Delta,
) -> Result<()> {
    if moved.is_empty() {
        return Ok(());
    }
    let word = kind.word();
    // Artists and tracks have an index per entity; albums do not, so one scan finds them all.
    let rows: Vec<(i64, i64)> = if kind == Kind::Album {
        let sql = "SELECT album_id, uts FROM scrobbles WHERE user_id = ?1 AND album_id IS NOT NULL";
        let wanted: HashSet<i64> = moved.keys().copied().collect();
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([user], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.filter(|row| row.as_ref().map_or(true, |(id, _)| wanted.contains(id)))
            .collect::<rusqlite::Result<_>>()?
    } else {
        let sql = format!("SELECT uts FROM scrobbles WHERE user_id = ?1 AND {word}_id = ?2");
        let mut stmt = conn.prepare_cached(&sql)?;
        let mut rows = Vec::new();
        for raw in moved.keys() {
            let found = stmt.query_map(params![user, raw], |row| Ok((*raw, row.get(0)?)))?;
            rows.extend(found.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        rows
    };
    for (raw, uts) in rows {
        if let Some(targets) = moved.get(&raw) {
            delta.shift(uts, kind, *targets);
        }
    }
    Ok(())
}
