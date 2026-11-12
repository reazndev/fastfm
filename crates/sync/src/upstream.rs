//! The Last.fm calls the sync jobs make, counted.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use lfm_store::TimeRange;
use lfm_upstream::{
    CallOptions, ChartRange, Client, LovedTracks, PageRequest, Priority, RecentTracks,
    RecentTracksQuery, User,
};

use crate::convert::param;
use crate::error::Result;

/// The shared client plus a count of the requests sync has made.
#[derive(Debug, Clone)]
pub(crate) struct Upstream {
    client: Arc<Client>,
    requests: Arc<AtomicU64>,
}

impl Upstream {
    pub(crate) fn new(client: Arc<Client>) -> Self {
        Self {
            client,
            requests: Arc::default(),
        }
    }

    /// Requests made so far, retries not included.
    pub(crate) fn requests(&self) -> u64 {
        self.requests.load(Ordering::Relaxed)
    }

    pub(crate) async fn recent(
        &self,
        query: &RecentTracksQuery,
        priority: Priority,
    ) -> Result<RecentTracks> {
        self.count();
        Ok(self.client.recent_tracks(query, &options(priority)).await?)
    }

    /// Last.fm's scrobble count for `range`, from `@attr.total` of a one-item page.
    pub(crate) async fn total(
        &self,
        user: &str,
        range: TimeRange,
        priority: Priority,
    ) -> Result<u64> {
        let query = RecentTracksQuery::new(user)
            .limit(1)
            .from(param(range.from))
            .to(param(range.to));
        Ok(self.recent(&query, priority).await?.page.total)
    }

    pub(crate) async fn user_info(&self, user: &str, priority: Priority) -> Result<User> {
        self.count();
        Ok(self.client.user_info(user, &options(priority)).await?)
    }

    pub(crate) async fn loved(
        &self,
        user: &str,
        page: PageRequest,
        priority: Priority,
    ) -> Result<LovedTracks> {
        self.count();
        Ok(self
            .client
            .loved_tracks(user, page, &options(priority))
            .await?)
    }

    pub(crate) async fn chart_list(
        &self,
        user: &str,
        priority: Priority,
    ) -> Result<Vec<ChartRange>> {
        self.count();
        Ok(self
            .client
            .weekly_chart_list(user, &options(priority))
            .await?)
    }

    fn count(&self) {
        self.requests.fetch_add(1, Ordering::Relaxed);
    }
}

fn options(priority: Priority) -> CallOptions {
    CallOptions::new(priority)
}
