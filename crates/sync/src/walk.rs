//! Walking a time range backwards in windows that never split a second.
//!
//! One request asks for the newest page of `[from, cursor)`. If the page holds the whole range,
//! that is the window. Otherwise the oldest second on the page may continue on the next page, so
//! the window is `[oldest + 1, cursor)` and the oldest second is fetched again as the newest
//! second of the next window. Only when a whole page falls in one second does a window need
//! more than one request: that second is then read page by page on its own.

use lfm_store::{Scrobble, Store, TimeRange, UserId};
use lfm_upstream::{Priority, RecentTracksQuery};
use tracing::debug;

use crate::convert::{self, param, uts};
use crate::error::{Result, SyncError};
use crate::upstream::Upstream;

/// A time range together with every scrobble Last.fm has in it, oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Window {
    pub(crate) range: TimeRange,
    pub(crate) scrobbles: Vec<Scrobble>,
}

/// Fetches windows of one user's history.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Walker<'a> {
    pub(crate) upstream: &'a Upstream,
    pub(crate) user: &'a str,
    pub(crate) priority: Priority,
    pub(crate) limit: u32,
}

impl Walker<'_> {
    /// The newest fully covered window of `[from, cursor)`. It ends at `cursor`; its start is
    /// the next cursor, and equals `from` once the range is done.
    pub(crate) async fn step(&self, from: i64, cursor: i64) -> Result<Window> {
        let query = self.query(from, cursor);
        let page = self.upstream.recent(&query, self.priority).await?;
        let inside = |s: &&lfm_upstream::Scrobble| (from..cursor).contains(&uts(s.uts));
        let fetched: Vec<_> = page.scrobbles.iter().filter(inside).collect();
        if u64::try_from(fetched.len()).unwrap_or(u64::MAX) >= page.page.total {
            let scrobbles = convert::scrobbles(fetched.into_iter());
            return Ok(Window {
                range: TimeRange::new(from, cursor),
                scrobbles,
            });
        }
        let oldest = fetched
            .iter()
            .map(|s| uts(s.uts))
            .min()
            .ok_or_else(|| empty_page(from, cursor, page.page.total))?;
        if oldest + 1 < cursor {
            let newer = fetched.into_iter().filter(|s| uts(s.uts) > oldest);
            return Ok(Window {
                range: TimeRange::new(oldest + 1, cursor),
                scrobbles: convert::scrobbles(newer),
            });
        }
        self.whole_second(oldest).await
    }

    /// Every scrobble of one second, for the rare page that falls entirely inside it.
    async fn whole_second(&self, second: i64) -> Result<Window> {
        debug!(
            second,
            "a full page shares one second; reading it page by page"
        );
        let mut newest_first = Vec::new();
        let mut page_number = 1;
        loop {
            let query = self.query(second, second + 1).page(page_number);
            let page = self.upstream.recent(&query, self.priority).await?;
            let empty = page.scrobbles.is_empty();
            newest_first.extend(page.scrobbles);
            if empty || page_number >= page.page.total_pages {
                break;
            }
            page_number += 1;
        }
        Ok(Window {
            range: TimeRange::new(second, second + 1),
            scrobbles: convert::scrobbles(newest_first.iter()),
        })
    }

    /// Replaces `range` in the store with Last.fm's contents, window by window. Returns the
    /// number of scrobbles written.
    pub(crate) async fn copy(&self, store: &Store, user: UserId, range: TimeRange) -> Result<u64> {
        let mut cursor = range.to;
        let mut written = 0;
        while cursor > range.from {
            let window = self.step(range.from, cursor).await?;
            cursor = window.range.from;
            written += u64::try_from(window.scrobbles.len()).unwrap_or_default();
            store
                .replace_window(user, window.range, window.scrobbles)
                .await?;
        }
        Ok(written)
    }

    fn query(&self, from: i64, to: i64) -> RecentTracksQuery {
        RecentTracksQuery::new(self.user)
            .limit(self.limit)
            .from(param(from))
            .to(param(to))
    }
}

fn empty_page(from: i64, cursor: i64, total: u64) -> SyncError {
    SyncError::Inconsistent(format!(
        "no scrobbles on the first page of {from}..{cursor} although total is {total}"
    ))
}
