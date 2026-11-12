//! Top artists, albums, and tracks.
//!
//! Periods read the per-period totals, so any page costs a walk of the play-count histogram and
//! one short index range. Arbitrary ranges and weekly charts merge the rollups instead.

use rusqlite::{Connection, params};

use crate::buckets::day_of;
use crate::delta::TIE_BLOCK;
use crate::error::Result;
use crate::pool::Store;
use crate::range::{range_plays, rank, skip, sorted, take};
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
    /// One page of the user's top list, by plays, ties by id.
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

    /// One page of the user's top `kind` in an arbitrary `range`, by plays, ties by id.
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
            let plays = sorted(range_plays(conn, user, kind, range)?, None);
            rank(conn, kind, 0, plays)
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
    let plays = sorted(range_plays(conn, user, kind, range)?, Some(page));
    let total = u64::try_from(plays.len()).unwrap_or(u64::MAX);
    let window = plays.into_iter().skip(skip(page)).take(take(page));
    let items = rank(conn, kind, page.offset(), window)?;
    Ok(Listing { items, page, total })
}

fn from_totals(
    conn: &Connection,
    user: UserId,
    (kind, period): (Kind, Period),
    page: Page,
) -> Result<Listing<Ranked>> {
    let seek = seek_plays(conn, user, (kind, period), page.offset())?;
    let Some((cap, skip)) = seek.start else {
        return Ok(Listing {
            items: Vec::new(),
            page,
            total: seek.total,
        });
    };
    let (block, skip) = seek_block(conn, user, (kind, period, cap), skip)?;
    let word = kind.word();
    let key = "user_id = ?1 AND period = ?2";
    let tie = format!(
        "SELECT {word}_id, plays FROM user_{word}_total WHERE {key} AND plays = ?3 \
         AND {word}_id >= ?4 ORDER BY {word}_id LIMIT ?5 OFFSET ?6"
    );
    let first = block * TIE_BLOCK;
    let mut rows = pairs(
        conn,
        &tie,
        params![user, period.index(), cap, first, page.limit, skip],
    )?;
    let missing = take(page).saturating_sub(rows.len());
    if missing > 0 {
        let rest = format!(
            "SELECT {word}_id, plays FROM user_{word}_total WHERE {key} AND plays < ?3 \
             ORDER BY plays DESC, {word}_id LIMIT ?4"
        );
        rows.extend(pairs(
            conn,
            &rest,
            params![user, period.index(), cap, missing],
        )?);
    }
    let items = rank(conn, kind, page.offset(), rows)?;
    Ok(Listing {
        items,
        page,
        total: seek.total,
    })
}

fn pairs(conn: &Connection, sql: &str, values: impl rusqlite::Params) -> Result<Vec<(i64, i64)>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(values, |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Finds the id block where the `skip`-th entity with `plays` plays lies, and how many of the
/// block's entities to skip.
fn seek_block(
    conn: &Connection,
    user: UserId,
    (kind, period, plays): (Kind, Period, i64),
    skip: u64,
) -> Result<(i64, u64)> {
    if skip == 0 {
        return Ok((0, 0));
    }
    let sql = format!(
        "SELECT block, n FROM user_{}_tie WHERE user_id = ?1 AND period = ?2 AND plays = ?3 \
         ORDER BY block",
        kind.word()
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut rows = stmt.query(params![user, period.index(), plays])?;
    let mut passed = 0;
    while let Some(row) = rows.next()? {
        let entities = count(row.get(1)?);
        if skip < passed + entities {
            return Ok((row.get(0)?, skip - passed));
        }
        passed += entities;
    }
    Ok((0, skip))
}

/// Where a page of a top list starts, found from the play-count histogram.
struct Seek {
    /// Entities in the list.
    total: u64,
    /// The highest play count on the page and how many entities with that count come before
    /// it; `None` past the end of the list.
    start: Option<(i64, u64)>,
}

fn seek_plays(
    conn: &Connection,
    user: UserId,
    (kind, period): (Kind, Period),
    offset: u64,
) -> Result<Seek> {
    let sql = format!(
        "SELECT plays, n FROM user_{}_hist WHERE user_id = ?1 AND period = ?2 \
         ORDER BY plays DESC",
        kind.word()
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut rows = stmt.query(params![user, period.index()])?;
    let mut seek = Seek {
        total: 0,
        start: None,
    };
    while let Some(row) = rows.next()? {
        let entities = count(row.get(1)?);
        if seek.start.is_none() && offset < seek.total + entities {
            seek.start = Some((row.get(0)?, offset - seek.total));
        }
        seek.total += entities;
    }
    Ok(seek)
}
