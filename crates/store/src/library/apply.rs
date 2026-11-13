//! Writing a [`Library`]: the library's names and details, the user's map, and the plays that
//! move with it.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};

use super::{LibAlbum, LibMove, LibTrack, Library};
use crate::delta::Delta;
use crate::dims::Resolver;
use crate::error::Result;
use crate::lookup::find;
use crate::names::{fold, image_key, non_empty};
use crate::pool::Store;
use crate::types::{Kind, Named, UserId, count};
use crate::users::{bump_version, period_day};

/// Scrobbled entity to library entity, by kind; identity entries are left out.
type Map = HashMap<(Kind, i64), i64>;

/// Scrobbled entities whose map entry may change, by kind.
type Pending = Vec<(Kind, Vec<i64>)>;

/// Entities whose details one transaction writes.
const DETAILS_CHUNK: usize = 2_000;

/// Plays one transaction moves at most, unless a single entity has more: about half a second
/// of totals and rollups, plus one scan for albums, which have no index by album. A
/// 1.5M-scrobble user whose every album moves took 18 s in one transaction before, during
/// which live sync and client writes waited.
const MOVE_PLAYS: u64 = 50_000;

impl Store {
    /// Stores what enrichment learned about `user`: the details of every listed artist, album
    /// and track (creating the ones never scrobbled under that name), and the user's map,
    /// replacing the old one. Plays of every entity whose target changed move to the new
    /// target in the totals, rollups and counters.
    ///
    /// The work is split into many short transactions so the single writer stays available.
    /// Each move batch changes map entries and their plays together, so every state between
    /// batches is consistent: an entity counts entirely under its old or its new target.
    /// Bumps every user's version at the end, since the details show in other users' lists.
    pub async fn apply_library(&self, user: UserId, library: Library) -> Result<()> {
        let Library {
            artists,
            albums,
            tracks,
            moves,
        } = library;
        for chunk in artists.chunks(DETAILS_CHUNK).map(<[Named]>::to_vec) {
            self.write(move |tx| write_each(tx, &chunk, write_artist))
                .await?;
        }
        for chunk in albums.chunks(DETAILS_CHUNK).map(<[LibAlbum]>::to_vec) {
            self.write(move |tx| write_each(tx, &chunk, write_album))
                .await?;
        }
        for chunk in tracks.chunks(DETAILS_CHUNK).map(<[LibTrack]>::to_vec) {
            self.write(move |tx| write_each(tx, &chunk, write_track))
                .await?;
        }
        let (new, raws) = self.write(move |tx| plan_map(tx, user, &moves)).await?;
        let new = Arc::new(new);
        for (kind, raws) in raws {
            let plays = self.read(move |conn| raw_plays(conn, user, kind)).await?;
            for batch in batches(raws, &plays) {
                let new = Arc::clone(&new);
                self.write(move |tx| move_batch(tx, user, (kind, &batch), &new))
                    .await?;
            }
        }
        self.write(|tx| {
            tx.prepare_cached("UPDATE users SET version = version + 1")?
                .execute([])?;
            Ok(())
        })
        .await
    }
}

fn write_each<T>(
    tx: &Connection,
    items: &[T],
    write: fn(&Connection, &mut Resolver<'_>, &T) -> Result<()>,
) -> Result<()> {
    let mut resolver = Resolver::new(tx);
    items
        .iter()
        .try_for_each(|item| write(tx, &mut resolver, item))
}

/// The new map, and per kind every scrobbled entity whose entry may change: those in the old
/// map or in the new one.
fn plan_map(conn: &Connection, user: UserId, moves: &[LibMove]) -> Result<(Map, Pending)> {
    let mut new = Map::new();
    for change in moves {
        if let Some(lib) = find(conn, &change.to)?.filter(|lib| *lib != change.raw) {
            new.insert((change.to.kind(), change.raw), lib);
        }
    }
    fill_variants(conn, user, &mut new)?;
    let old = load_map(conn, user)?;
    let raws = Kind::ALL
        .into_iter()
        .map(|kind| {
            let keys: HashSet<i64> = old
                .keys()
                .chain(new.keys())
                .filter(|(of, _)| *of == kind)
                .map(|(_, raw)| *raw)
                .collect();
            let mut keys: Vec<i64> = keys.into_iter().collect();
            keys.sort_unstable();
            (kind, keys)
        })
        .collect();
    Ok((new, raws))
}

/// Scrobbles per scrobbled entity of `kind`. Only sizes the batches, so a read that misses
/// the newest scrobbles does no harm.
fn raw_plays(conn: &Connection, user: UserId, kind: Kind) -> Result<HashMap<i64, u64>> {
    let word = kind.word();
    let sql = format!(
        "SELECT {word}_id, count(*) FROM scrobbles WHERE user_id = ?1 AND {word}_id IS NOT NULL \
         GROUP BY 1"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([user], |row| Ok((row.get(0)?, count(row.get(1)?))))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Splits `raws` into batches of at most [`MOVE_PLAYS`] plays.
fn batches(raws: Vec<i64>, plays: &HashMap<i64, u64>) -> Vec<Vec<i64>> {
    let mut out: Vec<Vec<i64>> = Vec::new();
    let mut size = 0;
    for raw in raws {
        let weight = plays.get(&raw).copied().unwrap_or(0);
        match out.last_mut() {
            Some(batch) if size + weight <= MOVE_PLAYS => {
                batch.push(raw);
                size += weight;
            }
            _ => {
                out.push(vec![raw]);
                size = weight;
            }
        }
    }
    out
}

/// Points `raws` of `kind` at their targets in `new`, moving their plays from the targets the
/// stored map gives them now.
fn move_batch(
    conn: &Connection,
    user: UserId,
    (kind, raws): (Kind, &[i64]),
    new: &Map,
) -> Result<()> {
    let mut current = conn.prepare_cached(
        "SELECT lib_id FROM user_library_map WHERE user_id = ?1 AND kind = ?2 AND raw_id = ?3",
    )?;
    let mut moved = HashMap::new();
    for &raw in raws {
        let from: Option<i64> = current
            .query_row(params![user, kind.code(), raw], |row| row.get(0))
            .optional()?;
        let (from, to) = (
            from.unwrap_or(raw),
            new.get(&(kind, raw)).copied().unwrap_or(raw),
        );
        if from != to {
            moved.insert(raw, (from, to));
        }
    }
    if moved.is_empty() {
        return Ok(());
    }
    let mut delta = Delta::new(period_day(conn, user)?);
    shift_plays(conn, user, (kind, &moved), &mut delta)?;
    save_entries(conn, user, kind, &moved)?;
    delta.apply(conn, user)?;
    bump_version(conn, user)
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

fn save_entries(
    conn: &Connection,
    user: UserId,
    kind: Kind,
    moved: &HashMap<i64, (i64, i64)>,
) -> Result<()> {
    let mut delete = conn.prepare_cached(
        "DELETE FROM user_library_map WHERE user_id = ?1 AND kind = ?2 AND raw_id = ?3",
    )?;
    let mut insert = conn.prepare_cached(
        "INSERT INTO user_library_map (user_id, kind, raw_id, lib_id) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (raw, (_, to)) in moved {
        delete.execute(params![user, kind.code(), raw])?;
        if to != raw {
            insert.execute(params![user, kind.code(), raw, to])?;
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batches_stay_under_the_play_budget_but_keep_big_entities_whole() {
        let plays = HashMap::from([(1, 30_000), (2, 30_000), (3, 70_000), (4, 10), (5, 20)]);
        let got = batches(vec![1, 2, 3, 4, 5, 6], &plays);
        assert_eq!(got, [vec![1], vec![2], vec![3], vec![4, 5, 6]]);
    }
}
