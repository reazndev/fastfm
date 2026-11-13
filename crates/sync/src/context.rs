//! What every sync job works with.

use std::collections::HashSet;
use std::sync::{Arc, PoisonError, RwLock};

use lfm_store::{Store, UserId};
use lfm_upstream::Priority;

use crate::config::SyncConfig;
use crate::error::{Result, SyncError};
use crate::now_playing::NowPlaying;
use crate::upstream::Upstream;
use crate::walk::Walker;

/// The store, the counted Last.fm client, the settings, the now-playing map, and the users
/// whose profiles hide their history.
#[derive(Debug, Clone)]
pub(crate) struct Context {
    pub(crate) store: Store,
    pub(crate) upstream: Upstream,
    pub(crate) config: SyncConfig,
    pub(crate) now_playing: NowPlaying,
    pub(crate) private: Arc<RwLock<HashSet<UserId>>>,
}

impl Context {
    /// A walker over `user`'s history in the given lane.
    pub(crate) fn walker<'a>(&'a self, user: &'a str, priority: Priority) -> Walker<'a> {
        Walker {
            upstream: &self.upstream,
            user,
            priority,
            limit: self.config.page_limit,
        }
    }

    /// Records whether Last.fm last refused `user`'s history as private.
    pub(crate) fn set_private(&self, user: UserId, private: bool) {
        let mut users = self.private.write().unwrap_or_else(PoisonError::into_inner);
        if private {
            users.insert(user);
        } else {
            users.remove(&user);
        }
    }

    /// Whether Last.fm last refused `user`'s history as private.
    pub(crate) fn is_private(&self, user: UserId) -> bool {
        self.private
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&user)
    }

    /// The stored name of a user, which is the one to send to Last.fm.
    pub(crate) async fn user_name(&self, user: UserId) -> Result<String> {
        self.store
            .user(user)
            .await?
            .map(|u| u.name)
            .ok_or_else(|| SyncError::UnknownUser(format!("id {}", user.0)))
    }
}
