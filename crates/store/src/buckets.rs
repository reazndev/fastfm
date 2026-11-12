//! Time buckets for the rollup tables.
//!
//! Rollups exist at three widths: 1, 8, and 64 UTC days. A time range splits into raw edges
//! shorter than a day, which are read from the scrobbles table, and runs of whole buckets, taking
//! the widest bucket that fits. A year is then about 5 wide buckets plus at most 14 narrower ones
//! on each side instead of 365 days or 100,000 scrobbles.

/// Seconds per day.
pub(crate) const DAY: i64 = 86_400;

/// Bucket widths in days, finest first. Each width divides the next.
pub(crate) const LEVELS: [i64; 3] = [1, 8, 64];

/// Upper bound for timestamps. Keeps bucket arithmetic far from overflow.
pub(crate) const MAX_UTS: i64 = 1 << 40;

/// A piece of a time range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Segment {
    /// Scrobbles in `[from, to)`, read from the scrobbles table.
    Raw { from: i64, to: i64 },
    /// Buckets `first..end` at `level`, read from the rollup tables.
    Buckets { level: usize, first: i64, end: i64 },
}

/// The UTC day number of `uts`.
pub(crate) fn day_of(uts: i64) -> i64 {
    uts.div_euclid(DAY)
}

/// Width of a bucket at `level`, in days.
pub(crate) fn level_days(level: usize) -> i64 {
    LEVELS.get(level).copied().unwrap_or(1)
}

/// The bucket of `uts` at every level, as `(level, bucket)`.
pub(crate) fn buckets_of(uts: i64) -> impl Iterator<Item = (i64, i64)> {
    LEVELS
        .iter()
        .zip(0..)
        .map(move |(days, level)| (level, uts.div_euclid(DAY * days)))
}

/// Splits `[from, to)` into segments, oldest first.
pub(crate) fn segments(from: i64, to: i64) -> Vec<Segment> {
    let (from, to) = (from.clamp(0, MAX_UTS), to.clamp(0, MAX_UTS));
    let first_day = ceil_div(from, DAY);
    let end_day = to.div_euclid(DAY);
    let mut out = Vec::new();
    if first_day >= end_day {
        push_raw(&mut out, from, to);
        return out;
    }
    push_raw(&mut out, from, first_day * DAY);
    push_days(&mut out, first_day, end_day, 0);
    push_raw(&mut out, end_day * DAY, to);
    out
}

/// Splits one bucket into the next finer level, or into a raw day.
pub(crate) fn split(level: usize, bucket: i64) -> Segment {
    match level.checked_sub(1) {
        None => Segment::Raw {
            from: bucket * DAY,
            to: (bucket + 1) * DAY,
        },
        Some(finer) => {
            let ratio = level_days(level) / level_days(finer);
            Segment::Buckets {
                level: finer,
                first: bucket * ratio,
                end: (bucket + 1) * ratio,
            }
        }
    }
}

/// Covers days `[first, end)`, both multiples of the width of `level`.
fn push_days(out: &mut Vec<Segment>, first: i64, end: i64, level: usize) {
    let width = level_days(level);
    let coarser = level + 1;
    if coarser < LEVELS.len() {
        let outer = level_days(coarser);
        let (inner_first, inner_end) = (
            ceil_div(first, outer) * outer,
            end.div_euclid(outer) * outer,
        );
        if inner_first < inner_end {
            push_buckets(out, level, first / width, inner_first / width);
            push_days(out, inner_first, inner_end, coarser);
            push_buckets(out, level, inner_end / width, end / width);
            return;
        }
    }
    push_buckets(out, level, first / width, end / width);
}

fn push_raw(out: &mut Vec<Segment>, from: i64, to: i64) {
    if from < to {
        out.push(Segment::Raw { from, to });
    }
}

fn push_buckets(out: &mut Vec<Segment>, level: usize, first: i64, end: i64) {
    if first < end {
        out.push(Segment::Buckets { level, first, end });
    }
}

fn ceil_div(value: i64, by: i64) -> i64 {
    -(-value).div_euclid(by)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The time span a segment covers.
    fn span(segment: Segment) -> (i64, i64) {
        match segment {
            Segment::Raw { from, to } => (from, to),
            Segment::Buckets { level, first, end } => {
                let width = level_days(level) * DAY;
                (first * width, end * width)
            }
        }
    }

    fn assert_tiles(from: i64, to: i64) {
        let pieces = segments(from, to);
        let mut cursor = from;
        for piece in &pieces {
            let (start, end) = span(*piece);
            assert_eq!(start, cursor, "gap in {pieces:?}");
            assert!(end > start);
            cursor = end;
        }
        assert_eq!(cursor, to.max(from), "range not covered: {pieces:?}");
    }

    #[test]
    fn tiles_ranges_exactly() {
        let year = 365 * DAY;
        for (from, to) in [
            (0, 1),
            (5, 5),
            (100, DAY + 100),
            (DAY, 2 * DAY),
            (1_700_000_123, 1_700_000_123 + year),
            (1_700_000_123, 1_700_000_123 + 7 * DAY),
            (0, 30 * year),
        ] {
            assert_tiles(from, to);
        }
    }

    #[test]
    fn a_year_uses_few_buckets() {
        let pieces = segments(1_700_000_123, 1_700_000_123 + 365 * DAY);
        assert!(pieces.len() <= 7, "{pieces:?}");
    }

    #[test]
    fn split_covers_parent() {
        assert_eq!(
            split(2, 3),
            Segment::Buckets {
                level: 1,
                first: 24,
                end: 32
            }
        );
        assert_eq!(
            split(1, 3),
            Segment::Buckets {
                level: 0,
                first: 24,
                end: 32
            }
        );
        assert_eq!(
            split(0, 3),
            Segment::Raw {
                from: 3 * DAY,
                to: 4 * DAY
            }
        );
    }
}
