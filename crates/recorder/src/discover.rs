//! Finds real artists, albums, tracks, tags and time ranges in the user's own data, so every
//! fixture asks about something that exists.

use lfm_upstream::{
    CallOptions, ChartRange, Client, Format, PageRequest, RawRequest, RecentTracksQuery, Scrobble,
};
use serde_json::Value;

use crate::error::RecordError;

const DAY: u64 = 86_400;

/// An entity from a top list: its name, its artist (for albums and tracks), and its MBID.
#[derive(Debug, Clone, Default)]
pub(crate) struct Entity {
    pub(crate) name: String,
    pub(crate) artist: String,
    pub(crate) mbid: Option<String>,
}

/// A `from`/`to` pair.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Range {
    pub(crate) from: u64,
    pub(crate) to: u64,
}

/// Last page numbers at the default page size of 50.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LastPages {
    pub(crate) recent: u32,
    pub(crate) loved: u32,
    pub(crate) library: u32,
    pub(crate) top_artists: u32,
}

/// Everything the specs need to know about the user.
#[derive(Debug, Clone)]
pub(crate) struct Discovered {
    pub(crate) user: String,
    pub(crate) artists: Vec<Entity>,
    pub(crate) albums: Vec<Entity>,
    pub(crate) tracks: Vec<Entity>,
    pub(crate) user_tag: Option<String>,
    pub(crate) global_tag: String,
    pub(crate) friend: Option<String>,
    pub(crate) last_pages: LastPages,
    pub(crate) charts: Vec<ChartRange>,
    /// A UTC day that has scrobbles.
    pub(crate) day: Range,
    /// A range that holds exactly one scrobble, at `single_uts`.
    pub(crate) one: Range,
    /// A range between two scrobbles that holds none.
    pub(crate) zero: Range,
    pub(crate) single_uts: u64,
}

impl Discovered {
    /// Asks Last.fm about the user. Costs about a dozen requests.
    pub(crate) async fn load(client: &Client, user: &str) -> Result<Self, RecordError> {
        let options = CallOptions::default();
        let page = PageRequest {
            page: None,
            limit: Some(50),
        };
        let artists = top(client, "user.getTopArtists", user, "artist").await?;
        let recent = client
            .recent_tracks(&RecentTracksQuery::new(user).limit(200).page(2), &options)
            .await?;
        let (one, zero, single_uts) = isolated_scrobble(&recent.scrobbles)?;
        let first_artist = artists.first().map(|a| a.name.clone()).unwrap_or_default();
        Ok(Self {
            user: user.to_owned(),
            albums: top(client, "user.getTopAlbums", user, "album").await?,
            tracks: top(client, "user.getTopTracks", user, "track").await?,
            user_tag: first_name(client, user_req("user.getTopTags", user), "toptags", "tag")
                .await?,
            global_tag: artist_tag(client, &first_artist).await?,
            friend: client
                .friends(user, page, &options)
                .await
                .ok()
                .and_then(|f| f.users.first().map(|u| u.name.clone())),
            last_pages: LastPages {
                recent: recent_pages(client, user).await?,
                loved: client
                    .loved_tracks(user, page, &options)
                    .await?
                    .page
                    .total_pages,
                library: total_pages(client, user_req("library.getArtists", user)).await?,
                top_artists: total_pages(client, user_req("user.getTopArtists", user)).await?,
            },
            charts: client.weekly_chart_list(user, &options).await?,
            day: Range {
                from: single_uts - single_uts % DAY,
                to: single_uts - single_uts % DAY + DAY - 1,
            },
            one,
            zero,
            single_uts,
            artists,
        })
    }
}

fn user_req(method: &str, user: &str) -> RawRequest {
    RawRequest::new(method)
        .param("user", user)
        .format(Format::Json)
}

async fn get_json(client: &Client, request: RawRequest) -> Result<Value, RecordError> {
    let response = client.raw(&request).await?;
    if let Some(err) = response.lastfm_error() {
        return Err(RecordError::Data(format!(
            "{} failed: {} {}",
            request.method_name(),
            err.code,
            err.message
        )));
    }
    Ok(serde_json::from_slice(&response.body)?)
}

/// The first five entries of a user top list, keeping the first one with an MBID among them.
async fn top(
    client: &Client,
    method: &str,
    user: &str,
    item: &str,
) -> Result<Vec<Entity>, RecordError> {
    let request = user_req(method, user).param("limit", "50");
    let body = get_json(client, request).await?;
    let list_key = format!("top{item}s");
    let items = list(&body, &list_key, item);
    let mut picked: Vec<Entity> = items.iter().copied().take(5).map(entity).collect();
    if !picked.iter().any(|e| e.mbid.is_some())
        && let Some(with_mbid) = items.iter().copied().map(entity).find(|e| e.mbid.is_some())
    {
        picked.push(with_mbid);
    }
    if picked.is_empty() {
        return Err(RecordError::Data(format!("{method} returned no {item}")));
    }
    Ok(picked)
}

fn list<'a>(body: &'a Value, outer: &str, inner: &str) -> Vec<&'a Value> {
    match body.get(outer).and_then(|o| o.get(inner)) {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(item @ Value::Object(_)) => vec![item],
        _ => Vec::new(),
    }
}

fn entity(item: &Value) -> Entity {
    let text = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or_default().to_owned();
    Entity {
        name: text(item.get("name")),
        artist: text(item.get("artist").and_then(|a| a.get("name"))),
        mbid: Some(text(item.get("mbid"))).filter(|m| !m.is_empty()),
    }
}

async fn first_name(
    client: &Client,
    request: RawRequest,
    outer: &str,
    inner: &str,
) -> Result<Option<String>, RecordError> {
    let body = get_json(client, request).await?;
    Ok(list(&body, outer, inner)
        .first()
        .and_then(|t| t.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned))
}

async fn artist_tag(client: &Client, artist: &str) -> Result<String, RecordError> {
    let request = RawRequest::new("artist.getTopTags")
        .param("artist", artist)
        .format(Format::Json);
    let tag = first_name(client, request, "toptags", "tag").await?;
    Ok(tag.unwrap_or_else(|| "rock".to_owned()))
}

async fn total_pages(client: &Client, request: RawRequest) -> Result<u32, RecordError> {
    let body = get_json(client, request).await?;
    let pages = body
        .as_object()
        .and_then(|o| o.values().next())
        .and_then(|inner| inner.get("@attr"))
        .and_then(|attr| attr.get("totalPages"))
        .and_then(Value::as_str)
        .and_then(|p| p.parse().ok());
    pages.ok_or_else(|| RecordError::Data("no totalPages in response".to_owned()))
}

async fn recent_pages(client: &Client, user: &str) -> Result<u32, RecordError> {
    let query = RecentTracksQuery::new(user).limit(50);
    let recent = client
        .recent_tracks(&query, &CallOptions::default())
        .await?;
    Ok(recent.page.total_pages)
}

/// Finds a scrobble with more than two seconds to each neighbour and returns a range holding
/// just it, a range holding nothing, and its timestamp.
fn isolated_scrobble(scrobbles: &[Scrobble]) -> Result<(Range, Range, u64), RecordError> {
    let times: Vec<u64> = scrobbles.iter().map(|s| s.uts).collect();
    times
        .windows(3)
        .find_map(|w| match *w {
            [newer, mid, older] if newer > mid + 2 && mid > older + 2 => Some((
                Range {
                    from: older + 1,
                    to: newer - 1,
                },
                Range {
                    from: older + 1,
                    to: mid - 1,
                },
                mid,
            )),
            _ => None,
        })
        .ok_or_else(|| RecordError::Data("no isolated scrobble on page 2".to_owned()))
}
