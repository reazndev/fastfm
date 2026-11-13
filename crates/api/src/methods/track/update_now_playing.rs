//! `track.updateNowPlaying`: the track the user is playing right now.

use std::sync::Arc;

use async_trait::async_trait;
use lfm_core::{ApiError, Node, Response};
use lfm_upstream::{Corrected, TrackItem};

use super::{corrected, ignored_message, session_account, track_item};
use crate::accounts::AccountServices;
use crate::method::{Access, Context, Method};

/// Answers `<nowplaying>` echoing the track, uncorrected.
///
/// Unverified against Last.fm (no recorded success): the JSON key order follows published
/// client logs.
#[derive(Debug, Clone)]
pub struct UpdateNowPlaying {
    accounts: Arc<dyn AccountServices>,
}

impl UpdateNowPlaying {
    /// The method, writing through `accounts`.
    #[must_use]
    pub fn new(accounts: Arc<dyn AccountServices>) -> Self {
        Self { accounts }
    }
}

#[async_trait]
impl Method for UpdateNowPlaying {
    fn name(&self) -> &'static str {
        "track.updateNowPlaying"
    }

    fn access(&self) -> Access {
        Access::Session
    }

    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError> {
        let track = track_item(ctx.params, "")?;
        let account = session_account(ctx).await?;
        let root = now_playing_node(&track);
        self.accounts.update_now_playing(account, track).await?;
        Ok(Response::new(root))
    }
}

fn now_playing_node(track: &TrackItem) -> Node {
    let kept = |text: &str| Corrected {
        text: text.to_owned(),
        corrected: false,
    };
    let album = track.album.as_deref().unwrap_or_default();
    let album_artist = track.album_artist.as_deref().unwrap_or_default();
    Node::new("nowplaying")
        .child(corrected("track", &kept(&track.track)))
        .child(corrected("artist", &kept(&track.artist)))
        .child(corrected("album", &kept(album)))
        .child(corrected("albumArtist", &kept(album_artist)))
        .child(ignored_message(None))
        .json_order(&["artist", "track", "ignoredMessage", "albumArtist", "album"])
}
