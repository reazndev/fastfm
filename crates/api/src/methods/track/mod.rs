//! `track.*` writes: written locally, then forwarded to Last.fm by [`AccountServices`].
//!
//! Each method parses the client's parameters and renders the outcome; the local write, the
//! forwarding and the retry queue live behind [`AccountServices`].
//!
//! [`AccountServices`]: crate::accounts::AccountServices

mod love;
mod scrobble;
mod unlove;
mod update_now_playing;

pub use love::Love;
pub use scrobble::Scrobble;
pub use unlove::Unlove;
pub use update_now_playing::UpdateNowPlaying;

use lfm_core::{ApiError, ErrorCode, Node, Params};
use lfm_store::AccountId;
use lfm_upstream::{Corrected, Ignored, TrackItem};

use crate::method::Context;
use crate::methods::shared::OrFailed;

/// The account a write acts for. The session key must belong to the calling API key, as a
/// Last.fm session key belongs to the application that obtained it.
async fn session_account(ctx: &Context<'_>) -> Result<AccountId, ApiError> {
    let session = ctx.session.ok_or(ErrorCode::InvalidSessionKey)?;
    let session_key = ctx.params.required("sk")?;
    let caller = ctx.params.required("api_key")?;
    let stored = ctx
        .services
        .store
        .api_session(session_key)
        .await
        .or_failed()?;
    match stored {
        Some(stored) if stored.api_key == caller => Ok(AccountId(session.account_id)),
        _ => Err(ErrorCode::InvalidSessionKey.into()),
    }
}

/// The track fields of a request, with `suffix` after each name (`[0]` in a scrobble batch).
/// `artist` and `track` are required.
fn track_item(params: &Params, suffix: &str) -> Result<TrackItem, ApiError> {
    let get = |name: &str| {
        params
            .optional(&format!("{name}{suffix}"))
            .map(str::to_owned)
    };
    Ok(TrackItem {
        artist: get("artist").ok_or_else(ApiError::missing_parameter)?,
        track: get("track").ok_or_else(ApiError::missing_parameter)?,
        album: get("album"),
        album_artist: get("albumArtist"),
        track_number: get("trackNumber"),
        mbid: get("mbid"),
        duration: get("duration"),
    })
}

/// `<artist corrected="0">Cher</artist>`.
fn corrected(name: &'static str, value: &Corrected) -> Node {
    let flag = if value.corrected { "1" } else { "0" };
    Node::leaf(name, value.text.clone()).attr("corrected", flag)
}

/// `<ignoredMessage code="0"></ignoredMessage>`, or the reason a scrobble was ignored.
fn ignored_message(ignored: Option<&Ignored>) -> Node {
    let (code, message) = ignored.map_or((0, String::new()), |i| (i.code, i.message.clone()));
    Node::leaf("ignoredMessage", message).attr("code", code)
}
