//! Summing plays per entity over a time range, and turning `(id, plays)` rows into a ranked list.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use rusqlite::{Connection, params};

use crate::buckets::{Segment, segments};
use crate::collate::entity_key;
use crate::dims::load;
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
    let raw = format!(
        "SELECT {word}_id, 1 FROM scrobbles WHERE user_id = ?1 AND uts >= ?2 AND uts < ?3 \
         AND {word}_id IS NOT NULL"
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

/// Loads names for a window's rows, orders them by plays and ties by name, and returns the rows
/// of `page` numbered from its offset; with no page, all rows numbered from 1.
pub(crate) fn rank(
    conn: &Connection,
    kind: Kind,
    window: Window,
    page: Option<Page>,
) -> Result<Vec<Ranked>> {
    let mut loaded = window
        .rows
        .into_iter()
        .filter_map(|(id, plays)| {
            let entity = load(conn, kind, id).transpose()?;
            Some(entity.map(|entity| (plays, entity_key(&entity), entity)))
        })
        .collect::<Result<Vec<_>>>()?;
    loaded.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let offset = page.map_or(0, Page::offset);
    let skip_in_window = usize::try_from(offset.saturating_sub(window.before)).unwrap_or(0);
    Ok(loaded
        .into_iter()
        .skip(skip_in_window)
        .take(page.map_or(usize::MAX, take))
        .zip(offset + 1..)
        .map(|((plays, _, entity), rank)| Ranked {
            rank,
            plays: count(plays),
            entity,
        })
        .collect())
}
