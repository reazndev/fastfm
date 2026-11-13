//! Extension: a user's whole history in one streamed response, as NDJSON.
//!
//! `GET /2.0/export?user=<name>&api_key=<key>[&from=<uts>&to=<uts>]` answers
//! `application/x-ndjson`, one scrobble per line, newest first, `from` inclusive and `to`
//! exclusive as in `user.getRecentTracks`. Last.fm has no such endpoint, so no Last.fm request
//! changes. A 1.5M-scrobble history is one request instead of 1,500 pages of 1000.

use bytes::Bytes;
use lfm_core::{ApiError, Format, ImageSize, Node, Output, Params, Text, image_url, json};
use lfm_store::{Page, RecentTrack, Store, StoreError, TimeRange, UserId};

use crate::dispatch::Dispatcher;
use crate::methods::shared::{OrFailed, artist_of, history_range};
use crate::reply::{CLASSIC, Reply, error_reply};

/// Scrobbles per store read: one read takes a few milliseconds and about 2 MB of output.
const CHUNK: u32 = 5000;

/// The content type of an export.
pub const NDJSON: &str = "application/x-ndjson";

/// One export in progress. Each [`Export::next_chunk`] reads one page of the history.
#[derive(Debug)]
pub struct Export {
    store: Store,
    user: UserId,
    range: TimeRange,
    page: u32,
    done: bool,
}

impl Dispatcher {
    /// Starts an export after checking the key and the user, or returns the error reply
    /// (in JSON, Last.fm's codes) to send instead.
    ///
    /// # Errors
    /// A missing or invalid key, or a user this instance does not mirror.
    pub async fn export(&self, params: &Params) -> Result<Export, Reply> {
        let fail = |error: ApiError| error_reply(&Output::plain(Format::Json), &error, CLASSIC);
        let key = params.required("api_key").map_err(fail)?;
        self.api_key(key).await.map_err(fail)?;
        let not_found = || fail(ApiError::not_found("User not found"));
        let name = params.optional("user").ok_or_else(not_found)?;
        let store = &self.services().store;
        let user = store.user_by_name(name).await.or_failed().map_err(fail)?;
        let user = user.ok_or_else(not_found)?;
        // Ending the range now keeps the pages still while new scrobbles arrive.
        let now = i64::try_from(self.services().clock.now()).unwrap_or(i64::MAX);
        let range = history_range(params);
        Ok(Export {
            store: store.clone(),
            user: user.id,
            range: TimeRange::new(range.from, range.to.min(now.saturating_add(1))),
            page: 1,
            done: false,
        })
    }
}

impl Export {
    /// The next lines of the export, or `None` when it is complete.
    pub async fn next_chunk(&mut self) -> Option<Result<Bytes, StoreError>> {
        if self.done {
            return None;
        }
        let page = Page::new(self.page, CHUNK);
        let listing = match self.store.recent_tracks(self.user, self.range, page).await {
            Ok(listing) => listing,
            Err(error) => {
                self.done = true;
                return Some(Err(error));
            }
        };
        self.page = self.page.saturating_add(1);
        self.done = listing.items.len() < usize::try_from(CHUNK).unwrap_or(usize::MAX);
        if listing.items.is_empty() {
            return None;
        }
        let mut out = Vec::with_capacity(listing.items.len() * 320);
        for scrobble in &listing.items {
            json::render_value(&line(scrobble), &mut out);
            out.push(b'\n');
        }
        Some(Ok(Bytes::from(out)))
    }
}

/// One scrobble as an export line, with the names as scrobbled and the cover's largest URL.
fn line(scrobble: &RecentTrack) -> Node {
    let track = &scrobble.track;
    let artist = artist_of(track);
    let album = scrobble.album.as_ref();
    let text = |value: Option<&String>| value.cloned().unwrap_or_default();
    let cover = album
        .and_then(|a| a.image.as_deref())
        .map(|file| image_url(file, ImageSize::ExtraLarge));
    Node::new("scrobble")
        .child(Node::leaf("uts", Text::number(scrobble.uts)))
        .child(Node::leaf("artist", artist.name.clone()))
        .child(Node::leaf("artist_mbid", text(artist.mbid.as_ref())))
        .child(Node::leaf("album", text(album.map(|a| &a.name))))
        .child(Node::leaf(
            "album_mbid",
            text(album.and_then(|a| a.mbid.as_ref())),
        ))
        .child(Node::leaf("track", track.name.clone()))
        .child(Node::leaf("track_mbid", text(track.mbid.as_ref())))
        .child(Node::leaf("image", cover.unwrap_or_default()))
        .child(Node::leaf("loved", Text::number(i64::from(scrobble.loved))))
}
