//! The live race: one page of a user's history at a time, from Last.fm or from this mirror,
//! trimmed to what the UI shows.

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use super::Demo;
use crate::error::WebError;

/// Scrobbles per page. Small pages keep both lanes moving visibly.
const PAGE: u32 = 200;

/// Which API a race page comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Side {
    Lastfm,
    Mirror,
}

/// One page as the UI needs it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RacePage {
    /// From sending the request to the last byte of the body.
    ms: f64,
    /// Scrobbles on this page, without a track playing now.
    count: usize,
    /// Scrobbles in the whole history.
    total: u64,
    total_pages: u32,
    /// The last `tail` scrobbles of the page, newest first.
    tracks: Vec<RaceTrack>,
}

/// One scrobble on a race page.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct RaceTrack {
    name: String,
    artist: String,
    /// The largest cover, or empty when there is none.
    image: String,
    uts: i64,
}

impl Demo {
    /// Fetches `page` of `user`'s recent tracks from `side`, keeping the last `tail` scrobbles.
    /// Unlike [`Demo::measure`], it does not wait for its turn, so both lanes run at once;
    /// Last.fm requests are still spaced.
    pub(crate) async fn race_page(
        &self,
        user: &str,
        side: Side,
        page: u32,
        tail: usize,
    ) -> Result<RacePage, WebError> {
        let params = [
            ("method", "user.getRecentTracks".to_owned()),
            ("user", user.to_owned()),
            ("limit", PAGE.to_string()),
            ("page", page.to_string()),
        ];
        let answer = match side {
            Side::Lastfm => self.lastfm(&params).await?,
            Side::Mirror => self.mirror(&params).await?,
        };
        let parsed = (answer.status == 200)
            .then(|| serde_json::from_slice::<Body>(&answer.body).ok())
            .flatten()
            .ok_or_else(|| {
                WebError::new(
                    StatusCode::BAD_GATEWAY,
                    "race_page_failed",
                    "This page did not load. Last.fm sometimes fails on deep pages.",
                )
            })?;
        Ok(race_page(
            parsed.recenttracks,
            answer.elapsed.as_secs_f64() * 1000.0,
            tail,
        ))
    }
}

fn race_page(recent: Recent, ms: f64, tail: usize) -> RacePage {
    let scrobbles: Vec<RaceTrack> = recent
        .track
        .into_iter()
        .filter_map(RawTrack::scrobble)
        .collect();
    let count = scrobbles.len();
    RacePage {
        ms,
        count,
        total: recent.attr.total.parse().unwrap_or_default(),
        total_pages: recent.attr.total_pages.parse().unwrap_or_default(),
        tracks: scrobbles
            .into_iter()
            .skip(count.saturating_sub(tail))
            .collect(),
    }
}

#[derive(Deserialize)]
struct Body {
    recenttracks: Recent,
}

#[derive(Deserialize)]
struct Recent {
    #[serde(default)]
    track: OneOrMany,
    #[serde(rename = "@attr")]
    attr: Attr,
}

/// Last.fm's JSON turns a list of one into a bare object.
#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    Many(Vec<RawTrack>),
    One(Box<RawTrack>),
}

impl Default for OneOrMany {
    fn default() -> Self {
        Self::Many(Vec::new())
    }
}

impl IntoIterator for OneOrMany {
    type Item = RawTrack;
    type IntoIter = std::vec::IntoIter<RawTrack>;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            Self::Many(tracks) => tracks.into_iter(),
            Self::One(track) => vec![*track].into_iter(),
        }
    }
}

#[derive(Deserialize)]
struct Attr {
    #[serde(rename = "totalPages")]
    total_pages: String,
    total: String,
}

#[derive(Deserialize)]
struct RawTrack {
    name: String,
    artist: Text,
    #[serde(default)]
    image: Vec<Text>,
    /// Missing on the track playing now.
    date: Option<Date>,
}

#[derive(Deserialize)]
struct Text {
    #[serde(rename = "#text")]
    text: String,
}

#[derive(Deserialize)]
struct Date {
    uts: String,
}

impl RawTrack {
    /// The scrobble, or `None` for the track playing now, which is not part of the history.
    fn scrobble(self) -> Option<RaceTrack> {
        let uts = self.date?.uts.parse().ok()?;
        // Last.fm lists images from small to extralarge.
        let image = self.image.into_iter().next_back().map(|i| i.text);
        Some(RaceTrack {
            name: self.name,
            artist: self.artist.text,
            image: image.unwrap_or_default(),
            uts,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn track(name: &str, uts: Option<i64>) -> serde_json::Value {
        let mut track = json!({"name": name, "artist": {"#text": "Cher"},
            "image": [{"size": "small", "#text": "s.jpg"}, {"size": "large", "#text": "l.jpg"}]});
        if let Some(uts) = uts {
            track["date"] = json!({"uts": uts.to_string(), "#text": "01 Jan 2020, 00:00"});
        }
        track
    }

    fn parse(tracks: &serde_json::Value, tail: usize) -> RacePage {
        let body = json!({"recenttracks": {"track": tracks,
            "@attr": {"total": "450", "totalPages": "3", "page": "1", "perPage": "200"}}});
        let body: Body = serde_json::from_value(body).unwrap();
        race_page(body.recenttracks, 1.0, tail)
    }

    #[test]
    fn keeps_the_tail_and_skips_the_track_playing_now() {
        let tracks = json!([
            track("Now", None),
            track("A", Some(3)),
            track("B", Some(2)),
            track("C", Some(1))
        ]);
        let page = parse(&tracks, 2);
        assert_eq!((page.count, page.total, page.total_pages), (3, 450, 3));
        let names: Vec<&str> = page.tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["B", "C"]);
        assert_eq!(page.tracks[0].image, "l.jpg");
    }

    #[test]
    fn reads_a_single_track_sent_as_an_object() {
        let page = parse(&track("A", Some(3)), 30);
        assert_eq!(page.count, 1);
        assert_eq!(page.tracks[0].artist, "Cher");
    }
}
