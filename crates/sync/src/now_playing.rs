//! What each followed user is playing right now, as last seen by live sync.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

use lfm_store::UserId;
use lfm_upstream::Track;

/// A track a user is playing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playing {
    /// The track, as Last.fm reported it.
    pub track: Track,
    /// Unix time of the poll that last saw it.
    pub seen_at: i64,
}

/// Shared now-playing state, one entry per user. Cheap to clone; every clone sees the same map.
#[derive(Debug, Clone, Default)]
pub struct NowPlaying {
    tracks: Arc<RwLock<HashMap<UserId, Playing>>>,
}

impl NowPlaying {
    /// The track `user` is playing, if live sync saw one on its last poll.
    #[must_use]
    pub fn get(&self, user: UserId) -> Option<Playing> {
        self.tracks
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&user)
            .cloned()
    }

    /// Records the result of a poll, or a client's `track.updateNowPlaying`: a track, or `None`
    /// when nothing plays.
    pub fn set(&self, user: UserId, track: Option<Track>, now: i64) {
        let mut tracks = self.tracks.write().unwrap_or_else(PoisonError::into_inner);
        match track {
            Some(track) => tracks.insert(
                user,
                Playing {
                    track,
                    seen_at: now,
                },
            ),
            None => tracks.remove(&user),
        };
    }
}
