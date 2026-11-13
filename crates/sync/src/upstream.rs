//! The Last.fm calls the sync jobs make, counted.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use lfm_store::{TimeRange, fold};
use lfm_upstream::{
    ApiKey, ArtistRef, CallOptions, ChartRange, Client, LovedTracks, PageRequest, Priority,
    RecentTracks, RecentTracksQuery, TopAlbum, TopArtist, TopPage, TopTrack, User,
};

use crate::budget::Budgets;
use crate::convert::param;
use crate::error::Result;

/// The shared client, the users' own keys, the request budgets, and a count of the requests
/// sync has made.
#[derive(Debug, Clone)]
pub(crate) struct Upstream {
    client: Arc<Client>,
    requests: Arc<AtomicU64>,
    /// Own Last.fm keys by folded user name; users without one share the pool.
    keys: Arc<RwLock<HashMap<String, ApiKey>>>,
    budgets: Arc<Budgets>,
}

impl Upstream {
    pub(crate) fn new(client: Arc<Client>, budgets: Budgets) -> Self {
        Self {
            client,
            requests: Arc::default(),
            keys: Arc::default(),
            budgets: Arc::new(budgets),
        }
    }

    /// Makes every later call for `user` use `key`, or the pool again with `None`.
    pub(crate) fn use_key(&self, user: &str, key: Option<ApiKey>) {
        let mut keys = self.keys.write().unwrap_or_else(PoisonError::into_inner);
        match key {
            Some(key) => keys.insert(fold(user), key),
            None => keys.remove(&fold(user)),
        };
    }

    /// Counts a call made on `user`'s behalf, waits for its key's budget, and returns its
    /// options: their own key, if they have one.
    async fn options(&self, user: &str, priority: Priority) -> CallOptions {
        self.requests.fetch_add(1, Ordering::Relaxed);
        let key = self
            .keys
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&fold(user))
            .cloned();
        self.budgets.of(key.as_ref()).take(priority).await;
        let options = CallOptions::new(priority);
        match key {
            Some(key) => options.api_key(key),
            None => options,
        }
    }

    /// Requests made so far, retries not included.
    pub(crate) fn requests(&self) -> u64 {
        self.requests.load(Ordering::Relaxed)
    }

    pub(crate) async fn recent(
        &self,
        user: &str,
        query: &RecentTracksQuery,
        priority: Priority,
    ) -> Result<RecentTracks> {
        let options = self.options(user, priority).await;
        Ok(self.client.recent_tracks(query, &options).await?)
    }

    /// Last.fm's scrobble count for `range`, from `@attr.total` of a one-item page.
    pub(crate) async fn total(
        &self,
        user: &str,
        range: TimeRange,
        priority: Priority,
    ) -> Result<u64> {
        let query = recent_query(user)
            .limit(1)
            .from(param(range.from))
            .to(param(range.to));
        Ok(self.recent(user, &query, priority).await?.page.total)
    }

    pub(crate) async fn user_info(&self, user: &str, priority: Priority) -> Result<User> {
        let options = self.options(user, priority).await;
        Ok(self.client.user_info(user, &options).await?)
    }

    pub(crate) async fn loved(
        &self,
        user: &str,
        page: PageRequest,
        priority: Priority,
    ) -> Result<LovedTracks> {
        let options = self.options(user, priority).await;
        Ok(self.client.loved_tracks(user, page, &options).await?)
    }

    pub(crate) async fn chart_list(
        &self,
        user: &str,
        priority: Priority,
    ) -> Result<Vec<ChartRange>> {
        let options = self.options(user, priority).await;
        Ok(self.client.weekly_chart_list(user, &options).await?)
    }

    /// One page of the user's all-time top artists, `LIST_LIMIT` per page.
    pub(crate) async fn top_artists(&self, user: &str, page: u32) -> Result<TopPage<TopArtist>> {
        let options = self.options(user, Priority::Prewarm).await;
        Ok(self
            .client
            .top_artists(user, list_page(page), &options)
            .await?)
    }

    /// One page of the user's all-time top albums, `LIST_LIMIT` per page.
    pub(crate) async fn top_albums(&self, user: &str, page: u32) -> Result<TopPage<TopAlbum>> {
        let options = self.options(user, Priority::Prewarm).await;
        Ok(self
            .client
            .top_albums(user, list_page(page), &options)
            .await?)
    }

    /// One page of the user's all-time top tracks, `LIST_LIMIT` per page.
    pub(crate) async fn top_tracks(&self, user: &str, page: u32) -> Result<TopPage<TopTrack>> {
        let options = self.options(user, Priority::Prewarm).await;
        Ok(self
            .client
            .top_tracks(user, list_page(page), &options)
            .await?)
    }

    /// The artist Last.fm files `artist` under; `None` when it does not know the name.
    pub(crate) async fn artist_correction(
        &self,
        user: &str,
        artist: &str,
    ) -> Result<Option<ArtistRef>> {
        let options = self.options(user, Priority::Prewarm).await;
        Ok(self.client.artist_correction(artist, &options).await?)
    }
}

/// A `user.getRecentTracks` query for `user`. Sync reads plain pages: extended ones blank the
/// artist MBID Last.fm shows per track, and add nothing the plain ones lack but loved flags
/// (synced from the loved list) and the artist URL (the start of the track URL).
pub(crate) fn recent_query(user: &str) -> RecentTracksQuery {
    RecentTracksQuery::new(user).extended(false)
}

/// Entries per top-list page: Last.fm's largest, so a list of n entries costs n / 1000 calls.
const LIST_LIMIT: u32 = 1000;

fn list_page(page: u32) -> PageRequest {
    PageRequest {
        page: Some(page),
        limit: Some(LIST_LIMIT),
    }
}
