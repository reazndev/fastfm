//! Typed helpers for the user methods the importer and live sync need.

use serde::de::DeserializeOwned;

use crate::client::Client;
use crate::config::ApiKey;
use crate::error::{Error, Result};
use crate::model::wire::{ChartListBody, FriendsBody, LovedTracksBody, RecentTracksBody, UserBody};
use crate::model::{ChartRange, Friends, LovedTracks, RecentTracks, User};
use crate::request::{Format, Priority, RawRequest};

/// Scheduling lane and optional own key for one typed call.
#[derive(Debug, Clone, Default)]
pub struct CallOptions {
    priority: Priority,
    api_key: Option<ApiKey>,
}

impl CallOptions {
    /// Options for the given lane, using the key pool.
    #[must_use]
    pub fn new(priority: Priority) -> Self {
        Self {
            priority,
            api_key: None,
        }
    }

    /// Uses this key, with its own token bucket, instead of the pool.
    #[must_use]
    pub fn api_key(self, api_key: ApiKey) -> Self {
        Self {
            api_key: Some(api_key),
            ..self
        }
    }

    fn apply(&self, request: RawRequest) -> RawRequest {
        let request = request.priority(self.priority).format(Format::Json);
        match &self.api_key {
            Some(key) => request.api_key(key.clone()),
            None => request,
        }
    }
}

/// Page number and size for paged methods. Unset values use Last.fm's defaults.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PageRequest {
    /// Page number, from 1.
    pub page: Option<u32>,
    /// Items per page.
    pub limit: Option<u32>,
}

impl PageRequest {
    fn params(self) -> impl Iterator<Item = (&'static str, String)> {
        let page = self.page.map(|p| ("page", p.to_string()));
        let limit = self.limit.map(|l| ("limit", l.to_string()));
        limit.into_iter().chain(page)
    }
}

/// Parameters of `user.getRecentTracks`, built by chaining setters. Always sends
/// `extended=1` so loved flags, artist URLs and MBIDs come back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentTracksQuery {
    user: String,
    page: PageRequest,
    from: Option<u64>,
    to: Option<u64>,
}

impl RecentTracksQuery {
    /// A query for this user's newest page.
    #[must_use]
    pub fn new(user: impl Into<String>) -> Self {
        Self {
            user: user.into(),
            page: PageRequest::default(),
            from: None,
            to: None,
        }
    }

    /// Sets the page number.
    #[must_use]
    pub fn page(mut self, page: u32) -> Self {
        self.page.page = Some(page);
        self
    }

    /// Sets the page size. Last.fm documents 200 as the maximum but accepts up to 1000.
    #[must_use]
    pub fn limit(mut self, limit: u32) -> Self {
        self.page.limit = Some(limit);
        self
    }

    /// Only scrobbles at or after this Unix time.
    #[must_use]
    pub fn from(self, from: u64) -> Self {
        Self {
            from: Some(from),
            ..self
        }
    }

    /// Only scrobbles before this Unix time; Last.fm treats `to` as exclusive.
    #[must_use]
    pub fn to(self, to: u64) -> Self {
        Self {
            to: Some(to),
            ..self
        }
    }

    fn request(&self) -> RawRequest {
        let from = self.from.map(|f| ("from", f.to_string()));
        let to = self.to.map(|t| ("to", t.to_string()));
        RawRequest::new("user.getRecentTracks")
            .param("user", self.user.as_str())
            .param("extended", "1")
            .params(self.page.params())
            .params(from.into_iter().chain(to))
    }
}

impl Client {
    /// `user.getRecentTracks` with `extended=1`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::LastFm`] for a Last.fm error body, or a transport or parse error.
    pub async fn recent_tracks(
        &self,
        query: &RecentTracksQuery,
        options: &CallOptions,
    ) -> Result<RecentTracks> {
        self.typed::<RecentTracksBody, _>(options.apply(query.request()))
            .await
    }

    /// `user.getInfo` for a named user.
    ///
    /// # Errors
    ///
    /// Returns [`Error::LastFm`] for a Last.fm error body, or a transport or parse error.
    pub async fn user_info(&self, user: &str, options: &CallOptions) -> Result<User> {
        let request = RawRequest::new("user.getInfo").param("user", user);
        self.typed::<UserBody, _>(options.apply(request)).await
    }

    /// One page of `user.getLovedTracks`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::LastFm`] for a Last.fm error body, or a transport or parse error.
    pub async fn loved_tracks(
        &self,
        user: &str,
        page: PageRequest,
        options: &CallOptions,
    ) -> Result<LovedTracks> {
        let request = RawRequest::new("user.getLovedTracks")
            .param("user", user)
            .params(page.params());
        self.typed::<LovedTracksBody, _>(options.apply(request))
            .await
    }

    /// `user.getWeeklyChartList`: every week Last.fm has a chart for, oldest first.
    ///
    /// # Errors
    ///
    /// Returns [`Error::LastFm`] for a Last.fm error body, or a transport or parse error.
    pub async fn weekly_chart_list(
        &self,
        user: &str,
        options: &CallOptions,
    ) -> Result<Vec<ChartRange>> {
        let request = RawRequest::new("user.getWeeklyChartList").param("user", user);
        self.typed::<ChartListBody, _>(options.apply(request)).await
    }

    /// One page of `user.getFriends`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::LastFm`] for a Last.fm error body, or a transport or parse error.
    pub async fn friends(
        &self,
        user: &str,
        page: PageRequest,
        options: &CallOptions,
    ) -> Result<Friends> {
        let request = RawRequest::new("user.getFriends")
            .param("user", user)
            .params(page.params());
        self.typed::<FriendsBody, _>(options.apply(request)).await
    }

    async fn typed<W, T>(&self, request: RawRequest) -> Result<T>
    where
        W: DeserializeOwned + Into<T>,
    {
        let response = self.raw(&request).await?;
        if let Some(err) = response.lastfm_error() {
            return Err(err.into());
        }
        if !response.is_success() {
            return Err(Error::Http {
                status: response.status,
            });
        }
        Ok(serde_json::from_slice::<W>(&response.body)?.into())
    }
}
