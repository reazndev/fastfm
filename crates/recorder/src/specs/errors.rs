//! Error cases and protocol details: bad keys, unknown methods and entities, missing
//! parameters, POST, JSONP and unsigned writes.

use crate::discover::Discovered;
use crate::spec::Spec;

/// A name nobody uses on Last.fm.
const NONSENSE: &str = "zzqxj-no-such-thing-8172635";

pub(super) fn specs(d: &Discovered) -> Vec<Spec> {
    [protocol(d), missing(d), unsigned_writes()].concat()
}

fn protocol(d: &Discovered) -> Vec<Spec> {
    let info = Spec::new("user.getInfo", "default").param("user", &d.user);
    vec![
        info.with_variant("error-invalid-key").invalid_key(),
        info.with_variant("error-missing-key").omit_key(),
        info.with_variant("post").post(),
        info.with_variant("post-error-invalid-key")
            .post()
            .invalid_key(),
        info.with_variant("jsonp-callback")
            .param("callback", "handleResponse"),
        info.with_variant("jsonp-callback-invalid-name")
            .param("callback", "bad name!"),
        Spec::new("user.getRecentTracks", "jsonp-callback")
            .param("user", &d.user)
            .num("limit", 2)
            .param("callback", "cb"),
        Spec::new("user.getRecentTracks", "post-limit-2")
            .param("user", &d.user)
            .num("limit", 2)
            .post(),
        Spec::new("user.noSuchMethod", "error-unknown-method").param("user", &d.user),
    ]
}

fn missing(d: &Discovered) -> Vec<Spec> {
    vec![
        Spec::new("user.getRecentTracks", "error-missing-user"),
        Spec::new("user.getRecentTracks", "error-unknown-user").param("user", NONSENSE),
        Spec::new("user.getRecentTracks", "error-page-out-of-range")
            .param("user", &d.user)
            .num("page", 999_999),
        Spec::new("user.getTopArtists", "error-unknown-user").param("user", NONSENSE),
        Spec::new("user.getTopArtists", "error-page-out-of-range")
            .param("user", &d.user)
            .num("page", 999_999),
        Spec::new("user.getLovedTracks", "error-page-out-of-range")
            .param("user", &d.user)
            .num("page", 999_999),
        Spec::new("user.getInfo", "error-missing-user"),
        Spec::new("user.getInfo", "error-unknown-user").param("user", NONSENSE),
        Spec::new("user.getWeeklyArtistChart", "error-unknown-user").param("user", NONSENSE),
        Spec::new("artist.getInfo", "error-unknown-artist").param("artist", NONSENSE),
        Spec::new("artist.getInfo", "error-missing-artist"),
        Spec::new("artist.getInfo", "error-unknown-mbid")
            .param("mbid", "00000000-0000-0000-0000-000000000000"),
        Spec::new("album.getInfo", "error-unknown-album")
            .param("artist", NONSENSE)
            .param("album", NONSENSE),
        Spec::new("track.getInfo", "error-unknown-track")
            .param("artist", NONSENSE)
            .param("track", NONSENSE),
        Spec::new("artist.getTags", "error-missing-user").param("artist", "Cher"),
        Spec::new("tag.getInfo", "unknown-tag").param("tag", NONSENSE),
    ]
}

fn unsigned_writes() -> Vec<Spec> {
    let track = |method: &str| {
        Spec::new(method, "error-unsigned")
            .param("artist", "Cher")
            .param("track", "Believe")
            .post()
    };
    vec![
        track("track.love"),
        track("track.unlove"),
        track("track.scrobble").num("timestamp", 1_700_000_000),
        track("track.updateNowPlaying"),
        Spec::new("auth.getToken", "error-unsigned"),
        Spec::new("auth.getSession", "error-unsigned").param("token", "abc"),
    ]
}
