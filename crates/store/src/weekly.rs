//! The list of weekly chart ranges, as `user.getWeeklyChartList` returns it.

use rusqlite::params;

use crate::error::Result;
use crate::pool::Store;
use crate::types::{TimeRange, UserId};
use crate::users::bump_version;

impl Store {
    /// Replaces the user's weekly chart list.
    pub async fn replace_weekly_charts(&self, user: UserId, charts: Vec<TimeRange>) -> Result<()> {
        self.write(move |tx| {
            tx.prepare_cached("DELETE FROM weekly_chart_list WHERE user_id = ?1")?.execute([user])?;
            let sql = "INSERT INTO weekly_chart_list (user_id, from_uts, to_uts) VALUES (?1, ?2, ?3) \
                       ON CONFLICT DO UPDATE SET to_uts = excluded.to_uts";
            let mut insert = tx.prepare_cached(sql)?;
            for chart in &charts {
                insert.execute(params![user, chart.from, chart.to])?;
            }
            bump_version(tx, user)
        })
        .await
    }

    /// The user's weekly chart list, oldest first.
    pub async fn weekly_charts(&self, user: UserId) -> Result<Vec<TimeRange>> {
        self.read(move |conn| {
            let sql = "SELECT from_uts, to_uts FROM weekly_chart_list WHERE user_id = ?1 \
                       ORDER BY from_uts";
            let mut stmt = conn.prepare_cached(sql)?;
            let rows =
                stmt.query_map([user], |row| Ok(TimeRange::new(row.get(0)?, row.get(1)?)))?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
        .await
    }
}
