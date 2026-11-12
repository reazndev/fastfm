//! Top artists, albums, and tracks.
//!
//! Periods read the per-period totals, so any page costs a walk of the play-count histogram and
//! one index range over the play counts the page covers. Arbitrary ranges and weekly charts merge
//! the rollups instead. Either way, ties are ordered by name once the names are loaded.

use rusqlite::{Connection, params};

use crate::buckets::day_of;
use crate::error::Result;
use crate::pool::Store;
use crate::range::{Window, range_plays, rank, window};
use crate::types::{Kind, Listing, Page, Period, Ranked, TimeRange, UserId, count};
use crate::users::period_day;

/// Which top list to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TopQuery {
    /// Artists, albums, or tracks.
    pub kind: Kind,
    /// The period.
    pub period: Period,
    /// The current time; periods end on its UTC day.
    pub now: i64,
}

impl Store {
    /// One page of the user's top list, by plays, ties by name as Last.fm orders them.
    ///
    /// Reads the precomputed period totals when `advance_periods` has moved them to the day of
    /// `query.now`, and merges the rollups otherwise. Both give the same list.
    pub async fn top(&self, user: UserId, query: TopQuery, page: Page) -> Result<Listing<Ranked>> {
        let TopQuery { kind, period, now } = query;
        self.read(move |conn| {
            let current = period_day(conn, user)? == Some(day_of(now));
            if period == Period::Overall || current {
                from_totals(conn, user, (kind, period), page)
            } else {
                from_range(conn, user, kind, period.range(now), page)
            }
        })
        .await
    }

    /// One page of the user's top `kind` in an arbitrary `range`, by plays, ties by name.
    pub async fn top_in_range(
        &self,
        user: UserId,
        kind: Kind,
        range: TimeRange,
        page: Page,
    ) -> Result<Listing<Ranked>> {
        self.read(move |conn| from_range(conn, user, kind, range, page))
            .await
    }

    /// The whole weekly-chart style list of `kind` for `range`.
    pub async fn chart(&self, user: UserId, kind: Kind, range: TimeRange) -> Result<Vec<Ranked>> {
        self.read(move |conn| {
            let rows = range_plays(conn, user, kind, range)?.into_iter().collect();
            rank(conn, kind, Window { rows, before: 0 }, None)
        })
        .await
    }
}

fn from_range(
    conn: &Connection,
    user: UserId,
    kind: Kind,
    range: TimeRange,
    page: Page,
) -> Result<Listing<Ranked>> {
    let plays = range_plays(conn, user, kind, range)?;
    let total = u64::try_from(plays.len()).unwrap_or(u64::MAX);
    let items = rank(conn, kind, window(plays, page), Some(page))?;
    Ok(Listing { items, page, total })
}

fn from_totals(
    conn: &Connection,
    user: UserId,
    (kind, period): (Kind, Period),
    page: Page,
) -> Result<Listing<Ranked>> {
    let seek = seek_plays(conn, user, (kind, period), page)?;
    let Some((high, low)) = seek.plays else {
        return Ok(Listing {
            items: Vec::new(),
            page,
            total: seek.total,
        });
    };
    let word = kind.word();
    let sql = format!(
        "SELECT {word}_id, plays FROM user_{word}_total WHERE user_id = ?1 AND period = ?2 \
         AND plays <= ?3 AND plays >= ?4"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt
        .query_map(params![user, period.index(), high, low], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let window = Window {
        rows,
        before: seek.before,
    };
    let items = rank(conn, kind, window, Some(page))?;
    Ok(Listing {
        items,
        page,
        total: seek.total,
    })
}

/// Where a page of a top list lies, found from the play-count histogram.
struct Seek {
    /// Entities in the list.
    total: u64,
    /// The play counts at the page's first and last positions; `None` past the end.
    plays: Option<(i64, i64)>,
    /// Entities with more plays than the page's first position.
    before: u64,
}

fn seek_plays(
    conn: &Connection,
    user: UserId,
    (kind, period): (Kind, Period),
    page: Page,
) -> Result<Seek> {
    let sql = format!(
        "SELECT plays, n FROM user_{}_hist WHERE user_id = ?1 AND period = ?2 \
         ORDER BY plays DESC",
        kind.word()
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut rows = stmt.query(params![user, period.index()])?;
    let first = page.offset();
    let last = first + u64::from(page.limit) - 1;
    let (mut total, mut before) = (0, 0);
    let (mut high, mut low) = (None, None);
    while let Some(row) = rows.next()? {
        let (plays, entities): (i64, u64) = (row.get(0)?, count(row.get(1)?));
        if high.is_none() && first < total + entities {
            high = Some(plays);
            before = total;
        }
        if total <= last {
            low = Some(plays);
        }
        total += entities;
    }
    Ok(Seek {
        total,
        plays: high.zip(low),
        before,
    })
}
