//! Top artists, albums, and tracks.
//!
//! Periods read the per-period totals, so any page costs a walk of the play-count histogram and
//! one index range over the play counts the page covers. Arbitrary ranges and weekly charts merge
//! the rollups instead. Either way, ties are ordered by name once the names are loaded.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{Connection, OptionalExtension, params};

use crate::buckets::day_of;
use crate::error::Result;
use crate::pool::Store;
use crate::range::{PlayMap, Window, range_plays, rank, window};
use crate::types::{Kind, Listing, Page, Period, Ranked, TimeRange, UserId, count};
use crate::users::period_day;

/// Which top list to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TopQuery {
    /// Artists, albums, or tracks.
    pub kind: Kind,
    /// The period.
    pub period: Period,
    /// The request time; periods are the rolling windows that end at it.
    pub now: i64,
}

impl Store {
    /// One page of the user's top list, by plays, ties by name as Last.fm orders them.
    ///
    /// Reads the precomputed period totals when `advance_periods` has moved them to the day of
    /// `query.now`, plus the plays of the rolling window's first, partial day, and merges the
    /// rollups otherwise. Both give the same list.
    pub async fn top(&self, user: UserId, query: TopQuery, page: Page) -> Result<Listing<Ranked>> {
        let TopQuery { kind, period, now } = query;
        self.read(move |conn| {
            let day = day_of(now);
            if period == Period::Overall {
                from_totals(conn, user, (kind, period), &PlayMap::default(), page)
            } else if period_day(conn, user)? == Some(day) {
                // The totals start on a UTC day; the rolling window starts earlier that day.
                let edge = TimeRange::new(period.range(now).from, period.start(day));
                let edge = range_plays(conn, user, kind, edge)?;
                from_totals(conn, user, (kind, period), &edge, page)
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

/// A page from the stored totals, with `edge` (plays outside them) added on top. The edge
/// holds one partial day, a few hundred entities at most: their totals are read one by one and
/// the histogram is corrected for them before the page is located.
fn from_totals(
    conn: &Connection,
    user: UserId,
    (kind, period): (Kind, Period),
    edge: &PlayMap,
    page: Page,
) -> Result<Listing<Ranked>> {
    let changed = changed_totals(conn, user, (kind, period), edge)?;
    let seek = seek_plays(conn, user, (kind, period), &changed, page)?;
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
    let stored = stmt
        .query_map(params![user, period.index(), high, low], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<(i64, i64)>>>()?;
    let in_range = |plays: &i64| (low..=high).contains(plays);
    let rows = stored
        .into_iter()
        .filter(|(id, _)| !changed.contains_key(id))
        .chain(
            changed
                .iter()
                .map(|(id, (_, new))| (*id, *new))
                .filter(|(_, plays)| in_range(plays)),
        )
        .collect();
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

/// Entity id to its stored plays and its plays with the edge added.
type Changed = HashMap<i64, (i64, i64)>;

fn changed_totals(
    conn: &Connection,
    user: UserId,
    (kind, period): (Kind, Period),
    edge: &PlayMap,
) -> Result<Changed> {
    let word = kind.word();
    let sql = format!(
        "SELECT plays FROM user_{word}_total WHERE user_id = ?1 AND period = ?2 AND {word}_id = ?3"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    edge.iter()
        .filter(|(_, extra)| **extra != 0)
        .map(|(id, extra)| {
            let stored: i64 = stmt
                .query_row(params![user, period.index(), id], |row| row.get(0))
                .optional()?
                .unwrap_or(0);
            Ok((*id, (stored, stored + extra)))
        })
        .collect()
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

/// `+?N`: see `counts::newest_bucket_reaching`; an ordered scan with plain `?N` is re-planned on
/// every use.
fn seek_plays(
    conn: &Connection,
    user: UserId,
    (kind, period): (Kind, Period),
    changed: &Changed,
    page: Page,
) -> Result<Seek> {
    let sql = format!(
        "SELECT plays, n FROM user_{}_hist WHERE user_id = +?1 AND period = +?2 \
         ORDER BY plays DESC",
        kind.word()
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt
        .query_map(params![user, period.index()], |row| {
            Ok((row.get(0)?, count(row.get(1)?)))
        })?
        .collect::<rusqlite::Result<Vec<(i64, u64)>>>()?;
    let first = page.offset();
    let last = first + u64::from(page.limit) - 1;
    let (mut total, mut before) = (0, 0);
    let (mut high, mut low) = (None, None);
    for (plays, entities) in corrected(rows, changed) {
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

/// The histogram rows `(plays, entities)`, most plays first, with each changed entity moved
/// from its stored play count to its new one.
fn corrected(rows: Vec<(i64, u64)>, changed: &Changed) -> Vec<(i64, u64)> {
    if changed.is_empty() {
        return rows;
    }
    let mut hist: BTreeMap<i64, i64> = rows
        .into_iter()
        .map(|(plays, n)| (plays, i64::try_from(n).unwrap_or(i64::MAX)))
        .collect();
    for (old, new) in changed.values() {
        for (plays, step) in [(*old, -1), (*new, 1)] {
            if plays > 0 {
                *hist.entry(plays).or_default() += step;
            }
        }
    }
    hist.into_iter()
        .rev()
        .filter(|(_, n)| *n > 0)
        .map(|(plays, n)| (plays, count(n)))
        .collect()
}
