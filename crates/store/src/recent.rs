//! A user's history, newest first.

use rusqlite::{Connection, Row, params};

use crate::counts::{count_in, locate};
use crate::dims::entity_at;
use crate::error::Result;
use crate::pool::Store;
use crate::types::{Listing, Page, RecentTrack, TimeRange, UserId};

/// The page is cut from the scrobbles table first, so names are joined for its rows only.
const PAGE_SQL: &str = "\
    SELECT s.uts, \
        t.id, t.name, t.mbid, t.url, NULL, ar.name, coalesce(t.artist_mbid, ar.mbid), ar.url, \
        t.duration, \
        al.id, al.name, al.mbid, NULL, al.image, NULL, NULL, NULL, NULL, \
        l.uts IS NOT NULL \
    FROM (SELECT uts, seq, track_id, album_id, artist_id FROM scrobbles \
          WHERE user_id = +?1 AND uts >= ?2 AND uts < ?3 \
          ORDER BY uts DESC, seq DESC LIMIT ?4 OFFSET ?5) s \
    JOIN tracks t ON t.id = s.track_id \
    JOIN artists ar ON ar.id = s.artist_id \
    LEFT JOIN albums al ON al.id = s.album_id \
    LEFT JOIN loved l ON l.user_id = +?1 AND l.track_id = s.track_id \
    ORDER BY s.uts DESC, s.seq DESC";

impl Store {
    /// One page of the user's scrobbles in `range`, newest first, with the range's total.
    pub async fn recent_tracks(
        &self,
        user: UserId,
        range: TimeRange,
        page: Page,
    ) -> Result<Listing<RecentTrack>> {
        self.read(move |conn| recent_tracks(conn, user, range, page))
            .await
    }
}

fn recent_tracks(
    conn: &Connection,
    user: UserId,
    range: TimeRange,
    page: Page,
) -> Result<Listing<RecentTrack>> {
    let total = count_in(conn, user, range)?;
    if page.offset() >= total {
        return Ok(Listing {
            items: Vec::new(),
            page,
            total,
        });
    }
    let seek = locate(conn, user, range, page.offset())?;
    let mut stmt = conn.prepare_cached(PAGE_SQL)?;
    let values = params![user, range.from, seek.end, page.limit, seek.skip];
    let items = stmt
        .query_map(values, recent_from_row)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Listing { items, page, total })
}

fn recent_from_row(row: &Row<'_>) -> rusqlite::Result<RecentTrack> {
    let album_id: Option<i64> = row.get(10)?;
    Ok(RecentTrack {
        uts: row.get(0)?,
        track: entity_at(row, 1)?,
        album: album_id.map(|_| entity_at(row, 10)).transpose()?,
        loved: row.get(19)?,
    })
}
