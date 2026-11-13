//! `track.scrobble`: one scrobble, or a batch of up to 50 as `artist[0]`, `track[0]`, ….

use std::sync::Arc;

use async_trait::async_trait;
use lfm_core::{ApiError, Node, Params, Response, Text};
use lfm_upstream::{ScrobbleItem, ScrobbleOutcome};

use super::{corrected, ignored_message, session_account, track_item};
use crate::accounts::AccountServices;
use crate::method::{Access, Context, Method};

/// Last.fm's batch limit; indexes from 50 on are not read.
const MAX_BATCH: usize = 50;

/// Answers `<scrobbles accepted="n" ignored="m">` with one `<scrobble>` per item.
///
/// Unverified against Last.fm (no recorded success): the JSON key order, and `accepted` and
/// `ignored` as bare numbers, follow Last.fm's documentation and published client logs.
#[derive(Debug, Clone)]
pub struct Scrobble {
    accounts: Arc<dyn AccountServices>,
}

impl Scrobble {
    /// The method, writing through `accounts`.
    #[must_use]
    pub fn new(accounts: Arc<dyn AccountServices>) -> Self {
        Self { accounts }
    }
}

#[async_trait]
impl Method for Scrobble {
    fn name(&self) -> &'static str {
        "track.scrobble"
    }

    fn access(&self) -> Access {
        Access::Session
    }

    /// Without a signature Last.fm hands the request to its old submissions protocol, which
    /// fails it in plain text: observed in the `error-unsigned` fixture, in both formats.
    fn unsigned_answer(&self) -> Option<&'static str> {
        Some("FAILED Incorrect protocol version (missing clientID/version/username)\n")
    }

    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError> {
        let items = items(ctx.params)?;
        let account = session_account(ctx).await?;
        let outcomes = self.accounts.scrobble(account, items).await?;
        Ok(Response::new(scrobbles_node(&outcomes)))
    }
}

/// The batch in index order, or the single unindexed scrobble.
fn items(params: &Params) -> Result<Vec<ScrobbleItem>, ApiError> {
    let batch = (0..MAX_BATCH)
        .map(|i| format!("[{i}]"))
        .filter(|suffix| sent_any(params, suffix))
        .map(|suffix| item(params, &suffix))
        .collect::<Result<Vec<_>, _>>()?;
    if !batch.is_empty() {
        return Ok(batch);
    }
    item(params, "").map(|item| vec![item])
}

fn sent_any(params: &Params, suffix: &str) -> bool {
    ["artist", "track", "timestamp"]
        .iter()
        .any(|name| params.get(&format!("{name}{suffix}")).is_some())
}

fn item(params: &Params, suffix: &str) -> Result<ScrobbleItem, ApiError> {
    let timestamp = params
        .optional(&format!("timestamp{suffix}"))
        .and_then(|t| t.trim().parse().ok())
        .ok_or_else(ApiError::missing_parameter)?;
    Ok(ScrobbleItem {
        track: track_item(params, suffix)?,
        timestamp,
        chosen_by_user: params
            .optional(&format!("chosenByUser{suffix}"))
            .map(str::to_owned),
    })
}

fn scrobbles_node(outcomes: &[ScrobbleOutcome]) -> Node {
    let ignored = outcomes.iter().filter(|o| o.ignored.is_some()).count();
    let accepted = outcomes.len() - ignored;
    let count = |n: usize| Text::number(i64::try_from(n).unwrap_or(i64::MAX));
    Node::new("scrobbles")
        .attr("accepted", count(accepted))
        .attr("ignored", count(ignored))
        .children(outcomes.iter().map(scrobble_node))
        .json_order(&["scrobble", "@attr", "ignored", "accepted"])
}

fn scrobble_node(outcome: &ScrobbleOutcome) -> Node {
    Node::new("scrobble")
        .child(corrected("track", &outcome.track))
        .child(corrected("artist", &outcome.artist))
        .child(corrected("album", &outcome.album))
        .child(corrected("albumArtist", &outcome.album_artist))
        .child(Node::leaf("timestamp", outcome.timestamp.to_string()))
        .child(ignored_message(outcome.ignored.as_ref()))
        .json_order(&[
            "artist",
            "album",
            "track",
            "ignoredMessage",
            "albumArtist",
            "timestamp",
        ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_batches_and_single_scrobbles() {
        let single = Params::new()
            .with("artist", "Cher")
            .with("track", "Believe")
            .with("timestamp", "100");
        let one = items(&single).unwrap();
        assert_eq!((one.len(), one[0].timestamp), (1, 100));
        let batch = Params::new()
            .with("artist[1]", "B")
            .with("track[1]", "b")
            .with("timestamp[1]", "2")
            .with("artist[0]", "A")
            .with("track[0]", "a")
            .with("timestamp[0]", "1")
            .with("album[0]", "Album");
        let two = items(&batch).unwrap();
        assert_eq!(two.len(), 2);
        assert_eq!(two[0].track.album.as_deref(), Some("Album"));
        assert_eq!(two[1].track.artist, "B");
    }

    #[test]
    fn missing_fields_are_error_6() {
        let no_time = Params::new().with("artist[0]", "A").with("track[0]", "a");
        assert_eq!(items(&no_time).unwrap_err().code().number(), 6);
        assert!(items(&Params::new()).is_err());
    }
}
