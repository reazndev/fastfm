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
    /// The parameters Last.fm documents (or was seen to honour) for the method. Only these
    /// are forwarded and keyed on, so made-up parameters can neither bust the cache nor turn
    /// the mirror into a proxy for arbitrary requests.
    pub params: &'static [&'static str],
}

const fn method(
    name: &'static str,
    ttl: Duration,
    fold: bool,
    params: &'static [&'static str],
) -> Forwarded {
    Forwarded {
        name,
        ttl,
        fold,
        params,
    }
}

const ARTIST: &[&str] = &["artist", "mbid", "autocorrect"];
const ARTIST_INFO: &[&str] = &["artist", "mbid", "autocorrect", "lang", "username", "user"];
const ARTIST_TAGS: &[&str] = &["artist", "mbid", "autocorrect", "user"];
const ARTIST_LIST: &[&str] = &["artist", "mbid", "autocorrect", "limit", "page"];
const ARTIST_SEARCH: &[&str] = &["artist", "limit", "page"];
const ALBUM: &[&str] = &["artist", "album", "mbid", "autocorrect"];
const ALBUM_INFO: &[&str] = &[
    "artist",
    "album",
    "mbid",
    "autocorrect",
    "lang",
    "username",
    "user",
];
const ALBUM_TAGS: &[&str] = &["artist", "album", "mbid", "autocorrect", "user"];
const ALBUM_SEARCH: &[&str] = &["album", "limit", "page"];
const TRACK: &[&str] = &["artist", "track", "mbid", "autocorrect"];
const TRACK_INFO: &[&str] = &[
    "artist",
    "track",
    "mbid",
    "autocorrect",
    "lang",
    "username",
    "user",
];
const TRACK_TAGS: &[&str] = &["artist", "track", "mbid", "autocorrect", "user"];
const TRACK_LIST: &[&str] = &["artist", "track", "mbid", "autocorrect", "limit"];
const TRACK_SEARCH: &[&str] = &["track", "artist", "limit", "page"];
const PAGED: &[&str] = &["limit", "page"];
const GEO: &[&str] = &["country", "location", "limit", "page"];
const TAG_PAGED: &[&str] = &["tag", "limit", "page"];
const USER_PAGED: &[&str] = &["user", "limit", "page"];
const USER_TOP: &[&str] = &["user", "period", "limit", "page"];
const USER_CHART: &[&str] = &["user", "from", "to", "limit"];

/// Every read method Last.fm documents. The local `user.*` and `library.*` methods are listed
/// too: they are forwarded for users this instance does not mirror.
static FORWARDED: &[Forwarded] = &[
    method("album.getInfo", INFO, true, ALBUM_INFO),
    method("album.getTags", HOUR, true, ALBUM_TAGS),
    method("album.getTopTags", LISTS, true, ALBUM),
    method("album.search", SEARCH, false, ALBUM_SEARCH),
    method("artist.getCorrection", LISTS, true, &["artist"]),
    method("artist.getInfo", INFO, true, ARTIST_INFO),
    method("artist.getSimilar", LISTS, true, ARTIST_LIST),
    method("artist.getTags", HOUR, true, ARTIST_TAGS),
    method("artist.getTopAlbums", LISTS, true, ARTIST_LIST),
    method("artist.getTopTags", LISTS, true, ARTIST),
    method("artist.getTopTracks", LISTS, true, ARTIST_LIST),
    method("artist.search", SEARCH, false, ARTIST_SEARCH),
    method("chart.getTopArtists", CHARTS, false, PAGED),
    method("chart.getTopTags", CHARTS, false, PAGED),
    method("chart.getTopTracks", CHARTS, false, PAGED),
    method("geo.getTopArtists", CHARTS, true, GEO),
    method("geo.getTopTracks", CHARTS, true, GEO),
    method("library.getArtists", USER, false, USER_PAGED),
    method("tag.getInfo", INFO, false, &["tag", "lang"]),
    method("tag.getSimilar", LISTS, false, &["tag"]),
    method("tag.getTopAlbums", LISTS, false, TAG_PAGED),
    method("tag.getTopArtists", LISTS, false, TAG_PAGED),
    method("tag.getTopTags", LISTS, false, PAGED),
    method("tag.getTopTracks", LISTS, false, TAG_PAGED),
    method("tag.getWeeklyChartList", LISTS, false, &["tag"]),
    method("track.getCorrection", LISTS, true, &["artist", "track"]),
    method("track.getInfo", INFO, true, TRACK_INFO),
    method("track.getSimilar", LISTS, true, TRACK_LIST),
    method("track.getTags", HOUR, true, TRACK_TAGS),
    method("track.getTopTags", LISTS, true, TRACK),
    method("track.search", SEARCH, false, TRACK_SEARCH),
    method(
        "user.getFriends",
        USER,
        false,
        &["user", "recenttracks", "limit", "page"],
    ),
    method("user.getInfo", USER, false, &["user"]),
    method("user.getLovedTracks", USER, false, USER_PAGED),
    method(
        "user.getPersonalTags",
        USER,
        false,
        &["user", "tag", "taggingtype", "limit", "page"],
    ),
    method(
        "user.getRecentTracks",
        USER,
        false,
        &["user", "limit", "page", "from", "to", "extended"],
    ),
    method("user.getTopAlbums", USER, false, USER_TOP),
    method("user.getTopArtists", USER, false, USER_TOP),
    method("user.getTopTags", USER, false, &["user", "limit"]),
    method("user.getTopTracks", USER, false, USER_TOP),
    method("user.getWeeklyAlbumChart", USER, false, USER_CHART),
    method("user.getWeeklyArtistChart", USER, false, USER_CHART),
    method("user.getWeeklyChartList", USER, false, &["user"]),
    method("user.getWeeklyTrackChart", USER, false, USER_CHART),
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
    /// The Last.fm key to call with; `None` uses the client's pool.
    pub(crate) api_key: Option<lfm_upstream::ApiKey>,
}

impl Forward {
    /// The request for `method` with the client's parameters, minus the ones that stay here.
    #[must_use]
    pub fn new(method: &'static Forwarded, params: &Params, format: Format) -> Self {
        let params: Vec<(String, String)> = params
            .effective()
            .into_iter()
            .filter(|(name, _)| method.params.contains(name))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        let key = cache_key(method, &params);
        Self {
            method,
            params,
            format,
            key,
            api_key: None,
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
    fn undocumented_parameters_are_neither_sent_nor_keyed() {
        let plain = Params::new().with("artist", "Cher");
        let padded = plain.clone().with("_n", "123").with("api_sig", "x");
        assert_eq!(
            key("artist.getInfo", &padded),
            key("artist.getInfo", &plain)
        );
        let method = forwarded("artist.getInfo").unwrap();
        let sent = Forward::new(method, &padded, Format::Json).params;
        assert_eq!(sent, [("artist".to_owned(), "Cher".to_owned())]);
    }

    #[test]
    fn search_terms_keep_their_case() {
        let upper = key("artist.search", &Params::new().with("artist", "Cher"));
        let lower = key("artist.search", &Params::new().with("artist", "cher"));
        assert_ne!(upper, lower);
    }
}
