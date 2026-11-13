//! What client writes need beyond the ingest paths: settling scrobbles after Last.fm answered,
//! marking a user changed, and reading a token whatever its expiry.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};

use crate::delta::Delta;
use crate::dims::Ids;
use crate::error::Result;
use crate::ingest::append;
use crate::library::LibMap;
use crate::names::fold;
use crate::pool::Store;
use crate::sessions::AuthToken;
use crate::types::{Scrobble, UserId};
use crate::users::{bump_version, period_day};

/// Scrobbles a user's row matches on: time, artist and track, case-folded.
type Key = (i64, String, String);

impl Store {
    /// Brings locally written scrobbles in line with Last.fm's answer, in one transaction:
    /// removes one stored row for each of `remove` (ignored or corrected by Last.fm), then makes
    /// sure each of `keep` is stored as often as it appears there. A scrobble that live sync
    /// already replaced is neither removed twice nor stored twice.
    pub async fn settle_scrobbles(
        &self,
        user: UserId,
        remove: Vec<Scrobble>,
        keep: Vec<Scrobble>,
    ) -> Result<()> {
        self.write(move |tx| {
            let removed = remove_each(tx, user, &remove)?;
            let missing = missing(tx, user, &keep)?;
            append(tx, user, &missing)?;
            if removed && missing.is_empty() {
                bump_version(tx, user)?;
            }
            Ok(())
        })
        .await
    }

    /// Marks the user's data as changed, so cached responses drop, as a new now-playing track
    /// needs.
    pub async fn touch_user(&self, user: UserId) -> Result<()> {
        self.write(move |tx| bump_version(tx, user)).await
    }

    /// The token whether or not it has expired, so `auth.getSession` can tell an expired token
    /// (15) from an unknown one (4).
    pub async fn find_auth_token(&self, token: &str) -> Result<Option<AuthToken>> {
        let token = token.to_owned();
        self.read(move |conn| {
            let sql = "SELECT token, api_key, account_id, expires_at FROM auth_tokens \
                       WHERE token = ?1";
            let found = conn.prepare_cached(sql)?.query_row([token], |row| {
                Ok(AuthToken {
                    token: row.get(0)?,
                    api_key: row.get(1)?,
                    account: row.get(2)?,
                    expires_at: row.get(3)?,
                })
            });
            Ok(found.optional()?)
        })
        .await
    }
}

fn key(scrobble: &Scrobble) -> Key {
    (
        scrobble.uts,
        fold(&scrobble.artist.name),
        fold(&scrobble.track.name),
    )
}

/// Removes the newest-numbered row of each scrobble. Returns whether any row went.
fn remove_each(conn: &Connection, user: UserId, scrobbles: &[Scrobble]) -> Result<bool> {
    let find = "SELECT s.seq, s.track_id, s.album_id, s.artist_id FROM scrobbles s \
                JOIN tracks t ON t.id = s.track_id JOIN artists ar ON ar.id = t.artist_id \
                WHERE s.user_id = ?1 AND s.uts = ?2 AND ar.name_fold = ?3 AND t.name_fold = ?4 \
                ORDER BY s.seq DESC LIMIT 1";
    let delete = "DELETE FROM scrobbles WHERE user_id = ?1 AND uts = ?2 AND seq = ?3";
    let mut delta = Delta::new(period_day(conn, user)?);
    // Aggregates count under library entities, as in every ingest.
    let mut map = LibMap::new(user);
    let mut removed = false;
    for scrobble in scrobbles {
        let (uts, artist, track) = key(scrobble);
        let row: Option<(i64, Ids)> = conn
            .prepare_cached(find)?
            .query_row(params![user, uts, artist, track], |row| {
                let ids = Ids {
                    track: row.get(1)?,
                    album: row.get(2)?,
                    artist: row.get(3)?,
                };
                Ok((row.get(0)?, ids))
            })
            .optional()?;
        if let Some((seq, ids)) = row {
            conn.prepare_cached(delete)?
                .execute(params![user, uts, seq])?;
            delta.add(uts, map.ids(conn, ids)?, -1);
            removed = true;
        }
    }
    delta.apply(conn, user)?;
    Ok(removed)
}

/// The scrobbles of `keep` that are stored fewer times than they appear in it.
fn missing(conn: &Connection, user: UserId, keep: &[Scrobble]) -> Result<Vec<Scrobble>> {
    let count = "SELECT count(*) FROM scrobbles s JOIN tracks t ON t.id = s.track_id \
                 JOIN artists ar ON ar.id = t.artist_id \
                 WHERE s.user_id = ?1 AND s.uts = ?2 AND ar.name_fold = ?3 AND t.name_fold = ?4";
    let mut wanted: HashMap<Key, i64> = HashMap::new();
    let mut out = Vec::new();
    for scrobble in keep {
        let key = key(scrobble);
        let stored: i64 = conn
            .prepare_cached(count)?
            .query_row(params![user, key.0, key.1, key.2], |row| row.get(0))?;
        let n = wanted.entry(key).or_insert(0);
        *n += 1;
        if stored < *n {
            out.push(scrobble.clone());
        }
    }
    Ok(out)
}
