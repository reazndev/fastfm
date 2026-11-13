//! `track.love`: loves a track for the session's user.

use std::sync::Arc;

use async_trait::async_trait;
use lfm_core::{ApiError, Response};

use super::session_account;
use crate::accounts::AccountServices;
use crate::method::{Access, Context, Method};

/// Needs `artist` and `track`; answers an empty `<lfm status="ok">`.
#[derive(Debug, Clone)]
pub struct Love {
    accounts: Arc<dyn AccountServices>,
}

impl Love {
    /// The method, writing through `accounts`.
    #[must_use]
    pub fn new(accounts: Arc<dyn AccountServices>) -> Self {
        Self { accounts }
    }
}

#[async_trait]
impl Method for Love {
    fn name(&self) -> &'static str {
        "track.love"
    }

    fn access(&self) -> Access {
        Access::Session
    }

    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError> {
        let artist = ctx.params.required("artist")?;
        let track = ctx.params.required("track")?;
        let account = session_account(ctx).await?;
        self.accounts.set_love(account, artist, track, true).await?;
        Ok(Response::empty())
    }
}
