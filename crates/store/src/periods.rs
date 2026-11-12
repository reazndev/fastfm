//! Moving the per-period totals forward as days pass.
//!
//! Each period table holds the plays from the period's first day on, as of `users.period_day`.
//! Ingest adds to every period a scrobble falls in. When the day changes, the days that leave a
//! period are subtracted using the rollups, so a daily move costs a few hundred row updates per
//! user instead of a rebuild.

use rusqlite::{Connection, params};

use crate::buckets::day_of;
use crate::delta::Delta;
use crate::error::Result;
use crate::pool::Store;
use crate::range::range_plays;
use crate::types::{Kind, Period, TimeRange, UserId};
use crate::users::{bump_version, period_day};

impl Store {
    /// Moves every user's period totals to the UTC day of `now`, and builds them for users that
    /// have none yet. Call it every few minutes; it does nothing while the day is unchanged.
    pub async fn advance_periods(&self, now: i64) -> Result<()> {
        let day = day_of(now);
        let users: Vec<UserId> = self
            .read(move |conn| {
                let sql = "SELECT id FROM users WHERE period_day IS NULL OR period_day != ?1";
                let mut stmt = conn.prepare_cached(sql)?;
                let rows = stmt.query_map([day], |row| row.get(0))?;
                Ok(rows.collect::<rusqlite::Result<_>>()?)
            })
            .await?;
        for user in users {
            self.write(move |tx| advance(tx, user, day)).await?;
        }
        Ok(())
    }
}

fn advance(conn: &Connection, user: UserId, day: i64) -> Result<()> {
    let old = period_day(conn, user)?;
    if old == Some(day) {
        return Ok(());
    }
    let mut delta = Delta::new(None);
    let periods = Period::ALL
        .into_iter()
        .filter(|period| *period != Period::Overall);
    match old {
        Some(old) if old < day && day - old < 365 => {
            for (period, kind) in periods.flat_map(|period| Kind::ALL.map(|kind| (period, kind))) {
                let leaving = TimeRange::new(period.start(old), period.start(day));
                delta.add_plays(kind, period, &range_plays(conn, user, kind, leaving)?, -1);
            }
        }
        _ => {
            clear_periods(conn, user)?;
            for (period, kind) in periods.flat_map(|period| Kind::ALL.map(|kind| (period, kind))) {
                let range = TimeRange::new(period.start(day), TimeRange::ALL.to);
                delta.add_plays(kind, period, &range_plays(conn, user, kind, range)?, 1);
            }
        }
    }
    delta.apply(conn, user)?;
    let sql = "UPDATE users SET period_day = ?2 WHERE id = ?1";
    conn.prepare_cached(sql)?.execute(params![user, day])?;
    bump_version(conn, user)
}

fn clear_periods(conn: &Connection, user: UserId) -> Result<()> {
    for kind in Kind::ALL {
        for table in ["total", "hist"] {
            let sql = format!(
                "DELETE FROM user_{}_{table} WHERE user_id = ?1 AND period > 0",
                kind.word()
            );
            conn.prepare_cached(&sql)?.execute([user])?;
        }
    }
    Ok(())
}
