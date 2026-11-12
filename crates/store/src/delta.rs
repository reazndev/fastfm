//! Keeping totals, histograms, rollups, and user counters equal to the scrobbles table.
//!
//! An ingest collects the net change per row first, so a window that deletes and re-inserts the
//! same scrobbles touches no aggregate at all.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};

use crate::buckets::buckets_of;
use crate::dims::Ids;
use crate::error::{Result, changed};
use crate::range::PlayMap;
use crate::types::{Kind, Period, UserId};

/// Net changes to one user's aggregates.
#[derive(Debug, Default)]
pub(crate) struct Delta {
    /// The user's `period_day`; scrobbles from a period's first day on count towards it.
    period_day: Option<i64>,
    totals: HashMap<(Kind, Period, i64), i64>,
    rollups: HashMap<(Kind, i64, i64, i64), i64>,
    counts: HashMap<(i64, i64), i64>,
    plays: i64,
}

impl Delta {
    /// A delta for a user whose period totals end on `period_day`.
    pub(crate) fn new(period_day: Option<i64>) -> Self {
        Self {
            period_day,
            ..Self::default()
        }
    }

    /// Counts one scrobble in (`sign` 1) or out (`sign` -1).
    pub(crate) fn add(&mut self, uts: i64, ids: Ids, sign: i64) {
        self.plays += sign;
        let periods: Vec<Period> = Period::ALL
            .into_iter()
            .filter(|period| match (period, self.period_day) {
                (Period::Overall, _) => true,
                (_, Some(day)) => uts >= period.start(day),
                (_, None) => false,
            })
            .collect();
        for (kind, id) in ids.by_kind() {
            for period in &periods {
                *self.totals.entry((kind, *period, id)).or_default() += sign;
            }
            for (level, bucket) in buckets_of(uts) {
                *self.rollups.entry((kind, level, bucket, id)).or_default() += sign;
            }
        }
        for key in buckets_of(uts) {
            *self.counts.entry(key).or_default() += sign;
        }
    }

    /// Adds `sign` times `plays` (entity id to plays) to the totals of one period.
    pub(crate) fn add_plays(&mut self, kind: Kind, period: Period, plays: &PlayMap, sign: i64) {
        for (id, count) in plays {
            *self.totals.entry((kind, period, *id)).or_default() += sign * count;
        }
    }

    /// Writes the changes and updates the user's counters.
    pub(crate) fn apply(self, conn: &Connection, user: UserId) -> Result<()> {
        let mut distinct: HashMap<Kind, i64> = HashMap::new();
        for ((kind, period, id), change) in self.totals {
            if change != 0 {
                let entities = apply_total(conn, user, (kind, period, id), change)?;
                if period == Period::Overall {
                    *distinct.entry(kind).or_default() += entities;
                }
            }
        }
        for ((kind, level, bucket, id), change) in self.rollups {
            if change != 0 {
                let table = format!("user_{}_rollup", kind.word());
                let word = format!("{}_id", kind.word());
                let key = [
                    ("user_id", user.0),
                    ("lvl", level),
                    ("bucket", bucket),
                    (&word, id),
                ];
                add_counter(conn, &table, &key, change)?;
            }
        }
        for ((level, bucket), change) in self.counts {
            if change != 0 {
                let key = [("user_id", user.0), ("lvl", level), ("bucket", bucket)];
                add_counter(conn, "user_scrobble_rollup", &key, change)?;
            }
        }
        let sql = "UPDATE users SET playcount = playcount + ?2, artist_count = artist_count + ?3, \
                   album_count = album_count + ?4, track_count = track_count + ?5 WHERE id = ?1";
        let by_kind = |kind| distinct.get(&kind).copied().unwrap_or_default();
        let values = params![
            user,
            self.plays,
            by_kind(Kind::Artist),
            by_kind(Kind::Album),
            by_kind(Kind::Track)
        ];
        changed(conn.prepare_cached(sql)?.execute(values)?)
    }
}

/// Ids per block of the tie tables.
pub(crate) const TIE_BLOCK: i64 = 4096;

/// Changes one total and its histograms. Returns the change in distinct entities (-1, 0, or 1).
fn apply_total(
    conn: &Connection,
    user: UserId,
    (kind, period, id): (Kind, Period, i64),
    change: i64,
) -> Result<i64> {
    let word = kind.word();
    let key = "user_id = ?1 AND period = ?2";
    let select = format!("SELECT plays FROM user_{word}_total WHERE {key} AND {word}_id = ?3");
    let values = params![user, period.index(), id];
    let old: i64 = conn
        .prepare_cached(&select)?
        .query_row(values, |row| row.get(0))
        .optional()?
        .unwrap_or(0);
    let new = (old + change).max(0);
    if new == 0 {
        let sql = format!("DELETE FROM user_{word}_total WHERE {key} AND {word}_id = ?3");
        conn.prepare_cached(&sql)?.execute(values)?;
    } else {
        let sql = format!(
            "INSERT INTO user_{word}_total (user_id, period, {word}_id, plays) \
             VALUES (?1, ?2, ?3, ?4) ON CONFLICT DO UPDATE SET plays = excluded.plays"
        );
        conn.prepare_cached(&sql)?
            .execute(params![user, period.index(), id, new])?;
    }
    for (plays, step) in [(old, -1), (new, 1)]
        .into_iter()
        .filter(|(plays, _)| *plays > 0)
    {
        let key = [
            ("user_id", user.0),
            ("period", period.index()),
            ("plays", plays),
        ];
        add_counter(conn, &format!("user_{word}_hist"), &key, step)?;
        let block = [key[0], key[1], key[2], ("block", id.div_euclid(TIE_BLOCK))];
        add_counter(conn, &format!("user_{word}_tie"), &block, step)?;
    }
    Ok(i64::from(new > 0) - i64::from(old > 0))
}

/// Adds `change` to the counter of the row `key` in `table`, and deletes the row at zero.
/// Counter tables name the counter `n` or `plays`.
fn add_counter(conn: &Connection, table: &str, key: &[(&str, i64)], change: i64) -> Result<()> {
    let counter = if table.ends_with("_hist") || table.ends_with("_tie") {
        "n"
    } else {
        "plays"
    };
    let names: Vec<&str> = key.iter().map(|(name, _)| *name).collect();
    let marks: Vec<String> = (1..=key.len() + 1).map(|at| format!("?{at}")).collect();
    let upsert = format!(
        "INSERT INTO {table} ({}, {counter}) VALUES ({}) ON CONFLICT DO UPDATE \
         SET {counter} = {counter} + excluded.{counter} RETURNING {counter}",
        names.join(", "),
        marks.join(", ")
    );
    let values: Vec<i64> = key
        .iter()
        .map(|(_, value)| *value)
        .chain([change])
        .collect();
    let left: i64 = conn
        .prepare_cached(&upsert)?
        .query_row(rusqlite::params_from_iter(&values), |row| row.get(0))?;
    if left <= 0 {
        let matches: Vec<String> = names
            .iter()
            .zip(&marks)
            .map(|(name, mark)| format!("{name} = {mark}"))
            .collect();
        let delete = format!("DELETE FROM {table} WHERE {}", matches.join(" AND "));
        let keys = values.iter().take(key.len());
        conn.prepare_cached(&delete)?
            .execute(rusqlite::params_from_iter(keys))?;
    }
    Ok(())
}
