//! Write-through: apply a client's write locally, then forward it to Last.fm.
//!
//! The local write comes first and the client is answered from it if Last.fm cannot be reached.
//! A transient failure goes to the store's outbox and is retried with backoff by
//! [`retry_due`]; a permanent one is recorded there as failed. Without a Last.fm session key
//! the write is applied locally only and recorded as held, which `GET /lastfm` shows.
//!
//! Scrobbles are not counted twice. Live sync replaces whole windows from the sync cursor on
//! with what Last.fm has, so a scrobble that reached Last.fm replaces the local copy rather
//! than joining it. When Last.fm answers, [`settle`] removes the local copy of each scrobble it
//! ignored or corrected and makes sure each accepted one is stored exactly once, which also
//! restores one that a poll dropped while it was still on its way. Anything that slips past,
//! such as a queued scrobble Last.fm later rejects, is repaired by the nightly reconciliation.

use lfm_core::{ApiError, ErrorCode, artist_url, track_url};
use lfm_store::{Account, AccountId, AccountStatus, Love, Named, Scrobble, UserId};
use lfm_upstream::{AlbumRef, ArtistRef, ScrobbleItem, ScrobbleOutcome, TrackItem};
use lfm_upstream::{Track, WriteCredentials};

use crate::link::Linked;
use crate::outbox::{Disposition, Pending, record};
use crate::state::{AppState, unix_now};

/// Scrobbles older than this are ignored with code 3, as Last.fm does.
const MAX_AGE: i64 = 14 * 86_400;
/// Scrobbles further ahead than this are ignored with code 4.
const MAX_AHEAD: i64 = 86_400;

/// The account a write is for, its mirrored user, and how to forward.
#[derive(Debug)]
pub(crate) struct Target {
    pub(crate) account: Account,
    pub(crate) user: UserId,
    pub(crate) creds: Option<WriteCredentials>,
}

/// Looks up what a write for `account` needs. Writes need a linked Last.fm user, since that
/// user's mirror is where they land.
pub(crate) async fn target(state: &AppState, account: AccountId) -> Result<Target, ApiError> {
    let account = state
        .store
        .account(account)
        .await
        .map_err(failed)?
        .filter(|a| a.status == AccountStatus::Active)
        .ok_or(ErrorCode::InvalidSessionKey)?;
    let name = account.lastfm_user.clone().ok_or_else(|| {
        ApiError::new(ErrorCode::AuthenticationFailed)
            .with_message("Link a Last.fm user to this account first")
    })?;
    let user = state.store.ensure_user(&name).await.map_err(failed)?;
    let creds = Linked::open(&state.vault, &account)
        .map_err(failed)?
        .write_credentials(&state.config);
    Ok(Target {
        account,
        user,
        creds,
    })
}

/// `track.scrobble`: stores what passes the local checks, forwards it, and answers with
/// Last.fm's verdict when it has one.
pub(crate) async fn scrobble(
    state: &AppState,
    account: AccountId,
    items: Vec<ScrobbleItem>,
) -> Result<Vec<ScrobbleOutcome>, ApiError> {
    let target = target(state, account).await?;
    let now = unix_now();
    let verdicts: Vec<Option<ScrobbleOutcome>> =
        items.iter().map(|item| local_verdict(item, now)).collect();
    let accepted: Vec<ScrobbleItem> = items
        .iter()
        .zip(&verdicts)
        .filter(|(_, ignored)| ignored.is_none())
        .map(|(item, _)| item.clone())
        .collect();
    if accepted.is_empty() {
        return Ok(verdicts.into_iter().flatten().collect());
    }
    let local: Vec<Scrobble> = accepted.iter().map(stored).collect();
    state
        .store
        .append_scrobbles(target.user, local)
        .await
        .map_err(failed)?;
    let pending = Pending::Scrobble {
        items: accepted.clone(),
    };
    let answered = forward(state, &target, pending).await;
    let mut sent = answered
        .unwrap_or_else(|| accepted.iter().map(ScrobbleOutcome::accepted).collect())
        .into_iter();
    Ok(verdicts
        .into_iter()
        .zip(&items)
        .map(|(ignored, item)| {
            ignored
                .or_else(|| sent.next())
                .unwrap_or_else(|| ScrobbleOutcome::accepted(item))
        })
        .collect())
}

/// `track.updateNowPlaying`: shown at once as the user's now-playing track. Not queued when
/// Last.fm cannot be reached, since it would be stale by the retry.
pub(crate) async fn now_playing(
    state: &AppState,
    account: AccountId,
    track: TrackItem,
) -> Result<(), ApiError> {
    let target = target(state, account).await?;
    let now = unix_now();
    state
        .sync
        .now_playing()
        .set(target.user, Some(playing(&track)), now);
    state.store.touch_user(target.user).await.map_err(failed)?;
    if let Some(creds) = &target.creds
        && let Err(error) = state.upstream.update_now_playing(&track, creds).await
    {
        tracing::warn!(%error, account = account.0, "forwarding now playing failed");
    }
    Ok(())
}

/// `track.love` and `track.unlove`.
pub(crate) async fn set_love(
    state: &AppState,
    account: AccountId,
    artist: &str,
    track: &str,
    loved: bool,
) -> Result<(), ApiError> {
    let target = target(state, account).await?;
    if loved {
        let love = Love {
            uts: unix_now(),
            artist: Named::new(artist),
            track: Named::new(track),
        };
        state.store.love(target.user, love).await.map_err(failed)?;
    } else {
        state
            .store
            .unlove(target.user, artist, track)
            .await
            .map_err(failed)?;
    }
    let pending = Pending::Love {
        artist: artist.to_owned(),
        track: track.to_owned(),
        loved,
    };
    forward(state, &target, pending).await;
    Ok(())
}

/// Sends a write now. Queues it after a transient failure, records a permanent one, and holds
/// it when the account cannot forward. Returns Last.fm's scrobble outcomes when it answered.
async fn forward(
    state: &AppState,
    target: &Target,
    pending: Pending,
) -> Option<Vec<ScrobbleOutcome>> {
    let now = unix_now();
    let id = target.account.id;
    let Some(creds) = &target.creds else {
        record(state, id, &pending, Disposition::Held, now).await;
        return None;
    };
    match send(state, target.user, &pending, creds).await {
        Ok(outcomes) => Some(outcomes),
        Err(error) => {
            tracing::warn!(%error, account = id.0, method = pending.method(), "forward failed");
            let message = error.to_string();
            let disposition = if error.is_transient() {
                Disposition::Queued(message)
            } else {
                Disposition::Failed(message)
            };
            record(state, id, &pending, disposition, now).await;
            None
        }
    }
}

/// Sends one write to Last.fm, and settles scrobbles locally with its answer.
pub(crate) async fn send(
    state: &AppState,
    user: UserId,
    pending: &Pending,
    creds: &WriteCredentials,
) -> Result<Vec<ScrobbleOutcome>, lfm_upstream::Error> {
    match pending {
        Pending::Scrobble { items } => {
            let outcomes = state.upstream.scrobble(items, creds).await?;
            settle(state, user, items, &outcomes).await;
            Ok(outcomes)
        }
        Pending::Love {
            artist,
            track,
            loved,
        } => {
            let upstream = &state.upstream;
            upstream.set_love(artist, track, *loved, creds).await?;
            Ok(Vec::new())
        }
    }
}

/// Makes the local copies match Last.fm's verdict: ignored ones go, corrected ones are
/// replaced, accepted ones are stored exactly once.
async fn settle(
    state: &AppState,
    user: UserId,
    items: &[ScrobbleItem],
    outcomes: &[ScrobbleOutcome],
) {
    let mut remove = Vec::new();
    let mut keep = Vec::new();
    for (item, outcome) in items.iter().zip(outcomes) {
        let corrected = outcome.artist.corrected || outcome.track.corrected;
        if outcome.ignored.is_some() || corrected {
            remove.push(stored(item));
        }
        if outcome.ignored.is_none() {
            keep.push(as_stored(item, outcome));
        }
    }
    if let Err(error) = state.store.settle_scrobbles(user, remove, keep).await {
        tracing::warn!(%error, user = user.0, "settling scrobbles failed");
    }
}

/// The reason Last.fm would ignore this scrobble, as an outcome, or `None` to send it.
fn local_verdict(item: &ScrobbleItem, now: i64) -> Option<ScrobbleOutcome> {
    // Messages from Last.fm's documentation of `ignoredMessage`; unverified against the API.
    let (code, message) = if item.track.artist.trim().is_empty() {
        (1, "Artist was ignored")
    } else if item.track.track.trim().is_empty() {
        (2, "Track was ignored")
    } else if item.timestamp < now - MAX_AGE {
        (3, "Timestamp was too old")
    } else if item.timestamp > now + MAX_AHEAD {
        (4, "Timestamp was too new")
    } else {
        return None;
    };
    Some(ScrobbleOutcome::ignored(item, code, message))
}

/// The scrobble as the client sent it, in the store's shape.
fn stored(item: &ScrobbleItem) -> Scrobble {
    let t = &item.track;
    Scrobble {
        uts: item.timestamp,
        artist: Named::new(&t.artist),
        album: t.album.as_deref().map(Named::new),
        track: Named {
            mbid: t.mbid.clone(),
            ..Named::new(&t.track)
        },
        image: None,
    }
}

/// The scrobble with Last.fm's corrections.
fn as_stored(item: &ScrobbleItem, outcome: &ScrobbleOutcome) -> Scrobble {
    let sent = stored(item);
    let album = Some(&outcome.album.text)
        .filter(|name| !name.is_empty())
        .map(|name| Named::new(name.as_str()));
    Scrobble {
        artist: Named::new(&outcome.artist.text),
        track: Named {
            name: outcome.track.text.clone(),
            ..sent.track.clone()
        },
        album: album.or(sent.album.clone()),
        ..sent
    }
}

/// The now-playing track in live sync's shape.
fn playing(track: &TrackItem) -> Track {
    Track {
        artist: ArtistRef {
            name: track.artist.clone(),
            mbid: None,
            url: Some(artist_url(&track.artist)),
        },
        name: track.track.clone(),
        mbid: track.mbid.clone(),
        url: track_url(&track.artist, &track.track),
        album: AlbumRef {
            name: track.album.clone().unwrap_or_default(),
            mbid: None,
        },
        images: Vec::new(),
        loved: false,
    }
}

/// Logs a failure and turns it into Last.fm's error 8.
pub(crate) fn failed(error: impl std::fmt::Display) -> ApiError {
    tracing::error!(%error, "write-through failed");
    ErrorCode::OperationFailed.into()
}
