//! Summing plays per entity over a time range, and turning `(id, plays)` rows into a ranked list.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use rusqlite::{Connection, params};

use crate::buckets::{Segment, segments};
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

/// Orders entities by plays, ties by id. Only the rows up to the end of `page` are sorted; the
/// rest only count towards the total.
pub(crate) fn sorted(plays: PlayMap, page: Option<Page>) -> Vec<(i64, i64)> {
    let mut plays: Vec<(i64, i64)> = plays.into_iter().collect();
    let end = page.map_or(usize::MAX, |page| skip(page).saturating_add(take(page)));
    if end < plays.len() {
        plays.select_nth_unstable_by(end, by_plays);
        let (head, _) = plays.split_at_mut(end);
        head.sort_unstable_by(by_plays);
    } else {
        plays.sort_unstable_by(by_plays);
    }
    plays
}

fn by_plays(a: &(i64, i64), b: &(i64, i64)) -> std::cmp::Ordering {
    b.1.cmp(&a.1).then(a.0.cmp(&b.0))
}

/// Rows before `page`, as an index.
pub(crate) fn skip(page: Page) -> usize {
    usize::try_from(page.offset()).unwrap_or(usize::MAX)
}

/// Rows on `page`, as a count.
pub(crate) fn take(page: Page) -> usize {
    usize::try_from(page.limit).unwrap_or(usize::MAX)
}

/// Loads names for `(id, plays)` rows and numbers them from `offset + 1`.
pub(crate) fn rank(
    conn: &Connection,
    kind: Kind,
    offset: u64,
    rows: impl IntoIterator<Item = (i64, i64)>,
) -> Result<Vec<Ranked>> {
    rows.into_iter()
        .zip(offset + 1..)
        .filter_map(|((id, plays), rank)| {
            let entity = load(conn, kind, id).transpose()?;
            Some(entity.map(|entity| Ranked {
                rank,
                plays: count(plays),
                entity,
            }))
        })
        .collect()
}
