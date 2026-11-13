//! Signed calls: `auth.getSession`, an application check, and the write methods.
//!
//! Writes are sent once, without the client's retries; the caller decides what to queue.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::Client;
use crate::error::{Error, Result};
use crate::request::{Format, HttpMethod, Priority, RawRequest};
use crate::signed::{AppCredentials, Secret, WriteCredentials};

/// Track fields a client sends with a scrobble or a now-playing update. Optional fields are
/// passed through as the client sent them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackItem {
    /// Artist name.
    pub artist: String,
    /// Track title.
    pub track: String,
    /// Album title.
    pub album: Option<String>,
    /// Album artist, when it differs from the track artist.
    pub album_artist: Option<String>,
    /// Position on the album.
    pub track_number: Option<String>,
    /// Track MBID.
    pub mbid: Option<String>,
    /// Length in seconds.
    pub duration: Option<String>,
}

/// One scrobble of a `track.scrobble` batch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrobbleItem {
    /// What was played.
    pub track: TrackItem,
    /// When it started playing, in Unix seconds.
    pub timestamp: i64,
    /// `chosenByUser` as the client sent it.
    pub chosen_by_user: Option<String>,
}

/// A name in a write response, and whether Last.fm corrected it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Corrected {
    /// The name as Last.fm stored it.
    pub text: String,
    /// Whether Last.fm changed it.
    pub corrected: bool,
}

impl Corrected {
    fn kept(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            corrected: false,
        }
    }
}

/// Why a scrobble was ignored: Last.fm's `ignoredMessage` code and text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ignored {
    /// 1 artist ignored, 2 track ignored, 3 too old, 4 too new, 5 daily limit.
    pub code: u32,
    /// The message.
    pub message: String,
}

/// What became of one scrobble.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrobbleOutcome {
    /// The track title.
    pub track: Corrected,
    /// The artist.
    pub artist: Corrected,
    /// The album; empty when none was sent.
    pub album: Corrected,
    /// The album artist; empty when none was sent.
    pub album_artist: Corrected,
    /// The scrobble's time.
    pub timestamp: i64,
    /// Set when the scrobble was not accepted.
    pub ignored: Option<Ignored>,
}

impl ScrobbleOutcome {
    /// The item accepted as sent.
    #[must_use]
    pub fn accepted(item: &ScrobbleItem) -> Self {
        let t = &item.track;
        Self {
            track: Corrected::kept(&t.track),
            artist: Corrected::kept(&t.artist),
            album: Corrected::kept(t.album.clone().unwrap_or_default()),
            album_artist: Corrected::kept(t.album_artist.clone().unwrap_or_default()),
            timestamp: item.timestamp,
            ignored: None,
        }
    }

    /// The item ignored for this reason.
    #[must_use]
    pub fn ignored(item: &ScrobbleItem, code: u32, message: &str) -> Self {
        Self {
            ignored: Some(Ignored {
                code,
                message: message.to_owned(),
            }),
            ..Self::accepted(item)
        }
    }

    fn from_wire(entry: &Value, item: &ScrobbleItem) -> Self {
        let sent = Self::accepted(item);
        let ignored = entry
            .get("ignoredMessage")
            .map(|m| (number(m.get("code")), text(m)))
            .filter(|(code, _)| *code != 0)
            .map(|(code, message)| Ignored { code, message });
        Self {
            track: corrected(entry.get("track"), sent.track),
            artist: corrected(entry.get("artist"), sent.artist),
            album: corrected(entry.get("album"), sent.album),
            album_artist: corrected(entry.get("albumArtist"), sent.album_artist),
            timestamp: item.timestamp,
            ignored,
        }
    }
}

/// A Last.fm session from `auth.getSession`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastfmSession {
    /// The Last.fm username that approved the token.
    pub name: String,
    /// The session key.
    pub key: Secret,
}

impl Client {
    /// `auth.getSession`: trades a token the user approved on Last.fm for a session key.
    ///
    /// # Errors
    ///
    /// [`Error::LastFm`] with 14 or 15 for an unapproved or expired token, or a transport error.
    pub async fn get_session(&self, token: &str, app: &AppCredentials) -> Result<LastfmSession> {
        let request = RawRequest::new("auth.getSession")
            .param("token", token)
            .signed(app);
        let body = self.signed_json(request).await?;
        let session = body.get("session").ok_or(Error::Http { status: 200 })?;
        let field = |name: &str| {
            session
                .get(name)
                .and_then(Value::as_str)
                .unwrap_or_default()
        };
        Ok(LastfmSession {
            name: field("name").to_owned(),
            key: Secret::new(field("key")),
        })
    }

    /// Checks an API key and shared secret with one signed `auth.getToken`.
    ///
    /// # Errors
    ///
    /// [`Error::LastFm`] with 10 for a bad key or 13 for a bad secret.
    pub async fn check_app(&self, app: &AppCredentials) -> Result<()> {
        self.signed_json(RawRequest::new("auth.getToken").signed(app))
            .await
            .map(drop)
    }

    /// `track.scrobble` for up to 50 items, one outcome per item in the order given.
    ///
    /// # Errors
    ///
    /// A Last.fm error for the whole batch, or a transport error.
    pub async fn scrobble(
        &self,
        items: &[ScrobbleItem],
        creds: &WriteCredentials,
    ) -> Result<Vec<ScrobbleOutcome>> {
        let params = items
            .iter()
            .enumerate()
            .flat_map(|(i, item)| scrobble_params(item, &format!("[{i}]")));
        let body = self.write("track.scrobble", params, creds).await?;
        let list = body.get("scrobbles").and_then(|s| s.get("scrobble"));
        let entries: Vec<&Value> = match list {
            Some(Value::Array(entries)) => entries.iter().collect(),
            Some(entry) => vec![entry],
            None => Vec::new(),
        };
        Ok(items
            .iter()
            .enumerate()
            .map(|(i, item)| match entries.get(i) {
                Some(entry) => ScrobbleOutcome::from_wire(entry, item),
                None => ScrobbleOutcome::accepted(item),
            })
            .collect())
    }

    /// `track.updateNowPlaying`.
    ///
    /// # Errors
    ///
    /// A Last.fm error or a transport error.
    pub async fn update_now_playing(
        &self,
        item: &TrackItem,
        creds: &WriteCredentials,
    ) -> Result<()> {
        let params = track_params(item, "");
        self.write("track.updateNowPlaying", params, creds)
            .await
            .map(drop)
    }

    /// `track.love`, or `track.unlove` when `loved` is false.
    ///
    /// # Errors
    ///
    /// A Last.fm error or a transport error.
    pub async fn set_love(
        &self,
        artist: &str,
        track: &str,
        loved: bool,
        creds: &WriteCredentials,
    ) -> Result<()> {
        let method = if loved { "track.love" } else { "track.unlove" };
        let params = [("artist", artist), ("track", track)];
        self.write(
            method,
            params.map(|(n, v)| (n.to_owned(), v.to_owned())),
            creds,
        )
        .await
        .map(drop)
    }

    async fn write(
        &self,
        method: &str,
        params: impl IntoIterator<Item = (String, String)>,
        creds: &WriteCredentials,
    ) -> Result<Value> {
        let request = RawRequest::new(method)
            .params(params)
            .param("sk", creds.session_key.expose())
            .http_method(HttpMethod::Post)
            .signed(&creds.app)
            .single_attempt();
        self.signed_json(request).await
    }

    async fn signed_json(&self, request: RawRequest) -> Result<Value> {
        let request = request.format(Format::Json).priority(Priority::Interactive);
        let response = self.raw(&request).await?;
        if let Some(err) = response.lastfm_error() {
            return Err(err.into());
        }
        if !response.is_success() {
            return Err(Error::Http {
                status: response.status,
            });
        }
        Ok(serde_json::from_slice(&response.body)?)
    }
}

fn scrobble_params(item: &ScrobbleItem, suffix: &str) -> Vec<(String, String)> {
    let mut params = track_params(&item.track, suffix);
    params.push((format!("timestamp{suffix}"), item.timestamp.to_string()));
    if let Some(chosen) = &item.chosen_by_user {
        params.push((format!("chosenByUser{suffix}"), chosen.clone()));
    }
    params
}

fn track_params(item: &TrackItem, suffix: &str) -> Vec<(String, String)> {
    let optional = [
        ("album", &item.album),
        ("albumArtist", &item.album_artist),
        ("trackNumber", &item.track_number),
        ("mbid", &item.mbid),
        ("duration", &item.duration),
    ];
    [("artist", &item.artist), ("track", &item.track)]
        .into_iter()
        .map(|(name, value)| (name, value.clone()))
        .chain(
            optional
                .into_iter()
                .filter_map(|(name, value)| value.clone().map(|v| (name, v))),
        )
        .map(|(name, value)| (format!("{name}{suffix}"), value))
        .collect()
}

fn corrected(entry: Option<&Value>, sent: Corrected) -> Corrected {
    let Some(entry) = entry else {
        return sent;
    };
    let flag = entry.get("corrected").and_then(Value::as_str) == Some("1");
    if flag {
        Corrected {
            text: text(entry),
            corrected: true,
        }
    } else {
        sent
    }
}

fn text(entry: &Value) -> String {
    entry
        .get("#text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn number(value: Option<&Value>) -> u32 {
    match value {
        Some(Value::String(text)) => text.trim().parse().unwrap_or(0),
        Some(Value::Number(n)) => n.as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0),
        _ => 0,
    }
}
