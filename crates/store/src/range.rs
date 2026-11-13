//! Summing plays per entity over a time range, and turning `(id, plays)` rows into a ranked list.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use rusqlite::{Connection, OptionalExtension, params};

use crate::buckets::{Segment, segments};
use crate::collate::tie_key;
use crate::dims::{entity_at, entity_sql, name_sql};
use crate::error::Result;
use crate::types::{Kind, Page, Ranked, TimeRange, UserId, count};

/// Plays per entity id.
pub(crate) type PlayMap = HashMap<i64, i64, BuildHasherDefault<IdHasher>>;

/// Hashes integer ids with one multiplication; the default `SipHash` costs more than the rest of a merge.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct IdHasher(u64);

impl Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0.rotate_left(8) ^ u64::from(*byte)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }

    fn write_i64(&mut self, value: i64) {
        let value = u64::from_ne_bytes(value.to_ne_bytes());
        self.0 = (self.0 ^ value).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

/// Sums plays per entity over `range`, from rollups and from raw scrobbles at the edges.
pub(crate) fn range_plays(
    conn: &Connection,
    user: UserId,
    kind: Kind,
    range: TimeRange,
) -> Result<PlayMap> {
    let word = kind.word();
    // Rollups count library entities; raw rows are mapped to them the same way.
    let raw = format!(
        "SELECT coalesce(m.lib_id, s.{word}_id), 1 FROM scrobbles s \
         LEFT JOIN user_library_map m ON m.user_id = s.user_id AND m.kind = {code} \
         AND m.raw_id = s.{word}_id \
         WHERE s.user_id = ?1 AND s.uts >= ?2 AND s.uts < ?3 AND s.{word}_id IS NOT NULL",
        code = kind.code()
    );
    let rolled = format!(
        "SELECT {word}_id, plays FROM user_{word}_rollup \
         WHERE user_id = ?1 AND lvl = ?2 AND bucket >= ?3 AND bucket < ?4"
    );
    let mut plays = PlayMap::default();
    for segment in segments(range.from, range.to) {
        let mut stmt;
        let mut rows = match segment {
            Segment::Raw { from, to } => {
                stmt = conn.prepare_cached(&raw)?;
                stmt.query(params![user, from, to])?
            }
            Segment::Buckets { level, first, end } => {
                stmt = conn.prepare_cached(&rolled)?;
                stmt.query(params![user, level, first, end])?
            }
        };
        while let Some(row) = rows.next()? {
            *plays.entry(row.get(0)?).or_insert(0) += row.get::<_, i64>(1)?;
        }
    }
    Ok(plays)
}

/// A slice of a ranked list: rows that may fall on the page, and how many rows rank above them.
#[derive(Debug, Default)]
pub(crate) struct Window {
    /// `(id, plays)` rows whose play counts overlap the page, in any order.
    pub(crate) rows: Vec<(i64, i64)>,
    /// Rows in the list with more plays than any row in `rows`.
    pub(crate) before: u64,
}

/// The rows of `plays` that can fall on `page` once ties are ordered by name: every row whose
/// play count lies between those at the page's first and last positions.
pub(crate) fn window(plays: PlayMap, page: Page) -> Window {
    let mut counts: Vec<i64> = plays.values().copied().collect();
    counts.sort_unstable_by(|a, b| b.cmp(a));
    let first = skip(page);
    let last = first
        .saturating_add(take(page))
        .min(counts.len())
        .saturating_sub(1);
    let (Some(&high), Some(&low)) = (counts.get(first), counts.get(last)) else {
        return Window::default();
    };
    let before = counts.partition_point(|plays| *plays > high);
    Window {
        rows: plays
            .into_iter()
            .filter(|(_, plays)| (low..=high).contains(plays))
            .collect(),
        before: u64::try_from(before).unwrap_or(u64::MAX),
    }
}

/// Rows before `page`, as an index.
pub(crate) fn skip(page: Page) -> usize {
    usize::try_from(page.offset()).unwrap_or(usize::MAX)
}

/// Rows on `page`, as a count.
pub(crate) fn take(page: Page) -> usize {
    usize::try_from(page.limit).unwrap_or(usize::MAX)
}

/// Orders a window's rows by plays and ties by name, and returns the rows of `page` numbered
/// from its offset; with no page, all rows numbered from 1. Only the names are read for the
/// whole window: a deep page can sit in a tie group of tens of thousands of entities, of which
/// only the page's rows need loading and exact ordering.
pub(crate) fn rank(
    conn: &Connection,
    kind: Kind,
    window: Window,
    page: Option<Page>,
) -> Result<Vec<Ranked>> {
    let mut names = conn.prepare_cached(name_sql(kind))?;
    let mut keyed = Vec::with_capacity(window.rows.len());
    for (id, plays) in window.rows {
        let found = names
            .query_row([id], |row| {
                Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?))
            })
            .optional()?;
        if let Some((artist, name)) = found {
            keyed.push((plays, tie_key(artist.as_deref(), &name), id));
        }
    }
    let offset = page.map_or(0, Page::offset);
    let skip_in_window = usize::try_from(offset.saturating_sub(window.before)).unwrap_or(0);
    let rows = page_slice(&mut keyed, skip_in_window, page.map_or(usize::MAX, take));
    let mut entities = conn.prepare_cached(entity_sql(kind))?;
    let mut ranked = Vec::with_capacity(rows.len());
    for ((plays, _, id), rank) in rows.iter().zip(offset + 1..) {
        let entity = entities
            .query_row([id], |row| entity_at(row, 0))
            .optional()?;
        if let Some(entity) = entity {
            ranked.push(Ranked {
                rank,
                plays: count(*plays),
                entity,
            });
        }
    }
    Ok(ranked)
}

/// A row to rank: plays, tie key and id.
type Keyed = (i64, (Vec<u32>, Vec<u32>), i64);

/// Rows `skip..skip + take` of `rows` in rank order: more plays first, ties by name. Selects
/// the slice before sorting it, so the rest of the rows stay unsorted.
fn page_slice(rows: &mut [Keyed], skip: usize, take: usize) -> &mut [Keyed] {
    let order = |a: &Keyed, b: &Keyed| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1));
    let end = skip.saturating_add(take).min(rows.len());
    if skip >= end {
        return &mut [];
    }
    if skip > 0 {
        rows.select_nth_unstable_by(skip, order);
    }
    let Some(tail) = rows.get_mut(skip..) else {
        return &mut [];
    };
    let len = end - skip;
    if len < tail.len() {
        tail.select_nth_unstable_by(len, order);
    }
    let Some(page) = tail.get_mut(..len) else {
        return &mut [];
    };
    page.sort_unstable_by(order);
    page
}
