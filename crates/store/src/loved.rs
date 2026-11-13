//! Loved tracks.
//!
//! Loved lists hold thousands of rows, not millions, so paging uses the `(user, uts)` index with
//! an offset bounded by the list's size.

use rusqlite::{Connection, OptionalExtension, params};

use crate::dims::{Resolver, entity_sql};
use crate::error::Result;
use crate::names::fold;
use crate::pool::Store;
use crate::types::{Entity, Kind, Listing, Named, Page, UserId, count};
use crate::users::bump_version;

/// A loved track to store.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Love {
    /// When it was loved, in unix seconds.
    pub uts: i64,
    /// The track's artist.
    pub artist: Named,
    /// The track.
    pub track: Named,
}

/// A loved track as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LovedTrack {
    /// When it was loved.
    pub uts: i64,
    /// The track, with its artist.
    pub track: Entity,
}

impl Store {
    /// Replaces all of the user's loved tracks. `loves` is in Last.fm's order, newest first;
    /// tracks loved in the same second keep that order.
    pub async fn replace_loved(&self, user: UserId, loves: Vec<Love>) -> Result<()> {
        self.write(move |tx| {
            tx.prepare_cached("DELETE FROM loved WHERE user_id = ?1")?
                .execute([user])?;
            let mut resolver = Resolver::new(tx);
            for (love, seq) in loves.iter().rev().zip(1..) {
                insert(tx, &mut resolver, user, (love, seq))?;
            }
            bump_version(tx, user)
        })
        .await
    }

    /// Loves one track, or updates the time it was loved. It comes before tracks loved in the
    /// same second.
    pub async fn love(&self, user: UserId, love: Love) -> Result<()> {
        self.write(move |tx| {
            let sql = "SELECT coalesce(max(seq), 0) + 1 FROM loved WHERE user_id = ?1";
            let seq = tx
                .prepare_cached(sql)?
                .query_row([user], |row| row.get(0))?;
            insert(tx, &mut Resolver::new(tx), user, (&love, seq))?;
            bump_version(tx, user)
        })
        .await
    }

    /// Unloves a track found by artist and track name. Returns whether it was loved.
    pub async fn unlove(&self, user: UserId, artist: &str, track: &str) -> Result<bool> {
        let (artist, track) = (fold(artist), fold(track));
        self.write(move |tx| {
            let sql = "DELETE FROM loved WHERE user_id = ?1 AND track_id = \
                       (SELECT t.id FROM tracks t JOIN artists ar ON ar.id = t.artist_id \
                        WHERE ar.name_fold = +?2 AND t.name_fold = +?3)";
            let removed = tx
                .prepare_cached(sql)?
                .execute(params![user, artist, track])?
                > 0;
            if removed {
                bump_version(tx, user)?;
            }
            Ok(removed)
        })
        .await
    }

    /// One page of the user's loved tracks, newest first.
    pub async fn loved_tracks(&self, user: UserId, page: Page) -> Result<Listing<LovedTrack>> {
        self.read(move |conn| loved_tracks(conn, user, page)).await
    }
}

fn insert(
    conn: &Connection,
    resolver: &mut Resolver<'_>,
    user: UserId,
    (love, seq): (&Love, i64),
) -> Result<()> {
    let artist = resolver.artist(&love.artist)?;
    let track = resolver.track(artist, &love.track, &love.artist)?;
    let sql = "INSERT INTO loved (user_id, track_id, uts, seq) VALUES (?1, ?2, ?3, ?4) \
               ON CONFLICT DO UPDATE SET uts = excluded.uts, seq = excluded.seq";
    conn.prepare_cached(sql)?
        .execute(params![user, track, love.uts, seq])?;
    Ok(())
}

fn loved_tracks(conn: &Connection, user: UserId, page: Page) -> Result<Listing<LovedTrack>> {
    let total_sql = "SELECT count(*) FROM loved WHERE user_id = ?1";
    let total = count(
        conn.prepare_cached(total_sql)?
            .query_row([user], |row| row.get(0))?,
    );
    let sql = "SELECT track_id, uts FROM loved WHERE user_id = ?1 \
               ORDER BY uts DESC, seq DESC, track_id LIMIT ?2 OFFSET ?3";
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(params![user, page.limit, page.offset()], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut track = conn.prepare_cached(entity_sql(Kind::Track))?;
    let items = rows
        .map(|row| {
            let (id, uts) = row?;
            let entity = track
                .query_row([id], |row| crate::dims::entity_at(row, 0))
                .optional()?;
            Ok(entity.map(|track| LovedTrack { uts, track }))
        })
        .filter_map(Result::transpose)
        .collect::<Result<_>>()?;
    Ok(Listing { items, page, total })
}
