//! The read methods the mirror forwards to Last.fm, how long their answers stay fresh, and how
//! a request becomes a cache key.

use std::time::Duration;

use lfm_core::{Format, Params};

const HOUR: Duration = Duration::from_secs(3_600);
const DAY: Duration = Duration::from_secs(86_400);

/// `getInfo`: an entity's page changes slowly.
const INFO: Duration = Duration::from_secs(7 * 86_400);
/// Similar lists, top lists, tags and corrections.
const LISTS: Duration = Duration::from_secs(30 * 86_400);
/// Search results.
const SEARCH: Duration = DAY;
/// `chart.*` and `geo.*` move by the hour.
const CHARTS: Duration = HOUR;
/// A user's own data, for users this instance does not mirror, and user tags.
const USER: Duration = Duration::from_secs(300);
/// Last.fm error answers such as "artist not found", so a typo is not remembered for a week.
pub(crate) const ERRORS: Duration = Duration::from_secs(300);

/// Parameters that never reach Last.fm and never change the answer: our key, signature and
/// session, the output format (cached apart) and the JSONP callback (applied when serving).
const NOT_FORWARDED: [&str; 6] = ["api_key", "api_sig", "sk", "format", "callback", "method"];

/// Entity names that Last.fm looks up ignoring case and does not echo back as given, so their
/// case can be folded in the cache key. Observed in the `autocorrect-*` and `lowercase-country`
/// fixtures, whose bodies equal the default variant's. Search terms, tags and user names are
/// echoed, so they keep their case.
const FOLDED: [&str; 4] = ["artist", "album", "track", "country"];

/// One method the mirror forwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Forwarded {
    /// The name as Last.fm documents it.
    pub name: &'static str,
    /// How long an answer stays fresh.
    pub ttl: Duration,
    /// Whether the [`FOLDED`] parameters are compared ignoring case.
    pub fold: bool,
}

const fn method(name: &'static str, ttl: Duration, fold: bool) -> Forwarded {
    Forwarded { name, ttl, fold }
}

/// Every read method Last.fm documents. The local `user.*` and `library.*` methods are listed
/// too: they are forwarded for users this instance does not mirror.
static FORWARDED: &[Forwarded] = &[
    method("album.getInfo", INFO, true),
    method("album.getTags", HOUR, true),
    method("album.getTopTags", LISTS, true),
    method("album.search", SEARCH, false),
    method("artist.getCorrection", LISTS, true),
    method("artist.getInfo", INFO, true),
    method("artist.getSimilar", LISTS, true),
    method("artist.getTags", HOUR, true),
    method("artist.getTopAlbums", LISTS, true),
    method("artist.getTopTags", LISTS, true),
    method("artist.getTopTracks", LISTS, true),
    method("artist.search", SEARCH, false),
    method("chart.getTopArtists", CHARTS, false),
    method("chart.getTopTags", CHARTS, false),
    method("chart.getTopTracks", CHARTS, false),
    method("geo.getTopArtists", CHARTS, true),
    method("geo.getTopTracks", CHARTS, true),
    method("library.getArtists", USER, false),
    method("tag.getInfo", INFO, false),
    method("tag.getSimilar", LISTS, false),
    method("tag.getTopAlbums", LISTS, false),
    method("tag.getTopArtists", LISTS, false),
    method("tag.getTopTags", LISTS, false),
    method("tag.getTopTracks", LISTS, false),
    method("tag.getWeeklyChartList", LISTS, false),
    method("track.getCorrection", LISTS, true),
    method("track.getInfo", INFO, true),
    method("track.getSimilar", LISTS, true),
    method("track.getTags", HOUR, true),
    method("track.getTopTags", LISTS, true),
    method("track.search", SEARCH, false),
    method("user.getFriends", USER, false),
    method("user.getInfo", USER, false),
    method("user.getLovedTracks", USER, false),
    method("user.getPersonalTags", USER, false),
    method("user.getRecentTracks", USER, false),
    method("user.getTopAlbums", USER, false),
    method("user.getTopArtists", USER, false),
    method("user.getTopTags", USER, false),
    method("user.getTopTracks", USER, false),
    method("user.getWeeklyAlbumChart", USER, false),
    method("user.getWeeklyArtistChart", USER, false),
    method("user.getWeeklyChartList", USER, false),
    method("user.getWeeklyTrackChart", USER, false),
];

/// The forwarded method with this name, ignoring ASCII case like Last.fm.
#[must_use]
pub fn forwarded(name: &str) -> Option<&'static Forwarded> {
    FORWARDED.iter().find(|m| m.name.eq_ignore_ascii_case(name))
}

/// One cache entry's identity: the key text and the format.
pub(crate) type EntryId = (String, Format);

/// A request to forward: what to send, and where its answer is cached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forward {
    pub(crate) method: &'static Forwarded,
    pub(crate) params: Vec<(String, String)>,
    pub(crate) format: Format,
    pub(crate) key: String,
}

impl Forward {
    /// The request for `method` with the client's parameters, minus the ones that stay here.
    #[must_use]
    pub fn new(method: &'static Forwarded, params: &Params, format: Format) -> Self {
        let params: Vec<(String, String)> = params
            .effective()
            .into_iter()
            .filter(|(name, _)| !NOT_FORWARDED.contains(name))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        let key = cache_key(method, &params);
        Self {
            method,
            params,
            format,
            key,
        }
    }

    /// A request built by the mirror itself, such as a pre-warm.
    #[must_use]
    pub fn of<'a>(
        method: &'static Forwarded,
        params: impl IntoIterator<Item = (&'a str, &'a str)>,
        format: Format,
    ) -> Self {
        let params = params
            .into_iter()
            .fold(Params::new(), |all, (name, value)| all.with(name, value));
        Self::new(method, &params, format)
    }

    /// The cache key, such as `artist.getinfo?artist=cher`.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    pub(crate) fn id(&self) -> EntryId {
        (self.key.clone(), self.format)
    }
}

/// `method?sorted&params`, the method lowercased and folded names case-folded.
fn cache_key(method: &Forwarded, params: &[(String, String)]) -> String {
    let mut query = form_urlencoded::Serializer::new(String::new());
    for (name, value) in params {
        if method.fold && FOLDED.contains(&name.as_str()) {
            query.append_pair(name, &lfm_store::fold(value));
        } else {
            query.append_pair(name, value);
        }
    }
    format!("{}?{}", method.name.to_ascii_lowercase(), query.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(method: &str, params: &Params) -> String {
        let method = forwarded(method).unwrap();
        Forward::new(method, params, Format::Json).key
    }

    #[test]
    fn finds_methods_ignoring_case_and_leaves_writes_out() {
        assert_eq!(forwarded("ARTIST.getinfo").unwrap().name, "artist.getInfo");
        assert!(forwarded("track.scrobble").is_none());
        assert!(forwarded("artist.noSuchMethod").is_none());
    }

    #[test]
    fn key_sorts_drops_local_params_and_folds_names() {
        let a = Params::new()
            .with("method", "artist.getInfo")
            .with("artist", "jUICE wrld")
            .with("api_key", "k")
            .with("callback", "cb")
            .with("lang", "de");
        let b = Params::new()
            .with("lang", "de")
            .with("artist", "Juice WRLD");
        assert_eq!(key("artist.getInfo", &a), key("artist.getInfo", &b));
        assert_eq!(
            key("artist.getInfo", &Params::new().with("artist", "Cher")),
            "artist.getinfo?artist=cher"
        );
    }

    #[test]
    fn search_terms_keep_their_case() {
        let upper = key("artist.search", &Params::new().with("artist", "Cher"));
        let lower = key("artist.search", &Params::new().with("artist", "cher"));
        assert_ne!(upper, lower);
    }
}
