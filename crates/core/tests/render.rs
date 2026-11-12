//! Renderer output compared byte for byte with responses recorded from Last.fm on 2026-11-12.

use lfm_core::{
    ApiError, ErrorCode, Format, ImageSize, Node, Output, Params, Response, Text, date_node,
    images, track_url, user_url,
};

fn render(response: &Response, format: Format) -> String {
    String::from_utf8_lossy(&Output::plain(format).render(response)).into_owned()
}

fn error(error: &ApiError, format: Format) -> String {
    String::from_utf8_lossy(&Output::plain(format).render_error(error)).into_owned()
}

fn user_getinfo() -> Response {
    let user = Node::new("user")
        .json_order(&[
            "spotify_expiry_estimate",
            "playcount",
            "playlists",
            "album_count",
            "bootstrap",
            "url",
            "age",
            "name",
            "artist_count",
            "subscriber",
            "track_count",
            "realname",
            "image",
            "registered",
            "country",
            "gender",
            "corrections",
            "type",
        ])
        .child(Node::leaf("name", "Syntiiix"))
        .child(Node::leaf("realname", "Reazn"))
        .children(images(None, ImageSize::STANDARD))
        .child(Node::leaf("url", user_url("Syntiiix")))
        .child(Node::leaf("country", "Germany"))
        .child(Node::leaf("age", 0u32))
        .child(Node::leaf("gender", "n"))
        .child(Node::leaf("subscriber", 0u32))
        .child(Node::leaf("playcount", 160_643u64))
        .child(Node::leaf("playlists", 0u32))
        .child(Node::leaf("bootstrap", 0u32))
        .child(unixtime("registered", 1_628_951_940))
        .child(Node::leaf("type", "user"))
        .child(Node::leaf("artist_count", 4046u32))
        .child(Node::leaf("album_count", 6198u32))
        .child(Node::leaf("track_count", 15_477u32))
        .child(Node::leaf("corrections", 1u32))
        .child(unixtime("spotify_expiry_estimate", 1_802_563_200));
    Response::new(user).escape_slashes()
}

fn unixtime(name: &'static str, uts: i64) -> Node {
    Node::leaf(name, Text::number(uts)).attr("unixtime", uts)
}

#[test]
fn user_getinfo_xml() {
    let expected = include_str!("data/user_getinfo.xml");
    assert_eq!(render(&user_getinfo(), Format::Xml), expected);
}

#[test]
fn user_getinfo_json_with_scrambled_order_and_numeric_text() {
    let expected = include_str!("data/user_getinfo.json");
    assert_eq!(render(&user_getinfo(), Format::Json), expected);
}

struct Scrobble {
    artist: &'static str,
    artist_mbid: &'static str,
    name: &'static str,
    album: &'static str,
    image: &'static str,
    uts: u64,
}

fn recent_track(s: &Scrobble) -> Node {
    Node::new("track")
        .json_order(&[
            "artist",
            "streamable",
            "image",
            "mbid",
            "album",
            "name",
            "url",
            "date",
        ])
        .child(Node::leaf("artist", s.artist).attr("mbid", s.artist_mbid))
        .child(Node::leaf("name", s.name))
        .child(Node::leaf("streamable", "0"))
        .child(Node::leaf("mbid", ""))
        .child(Node::leaf("album", s.album).attr("mbid", ""))
        .child(Node::leaf("url", track_url(s.artist, s.name)))
        .children(images(Some(s.image), ImageSize::STANDARD))
        .child(date_node("date", s.uts))
}

fn recent_tracks() -> Response {
    let mbid = "da8fb855-1a7f-4648-8d97-dd832328b92b";
    let tracks = [
        Scrobble {
            artist: "Ledbyher",
            artist_mbid: mbid,
            name: "Remember, Remember",
            album: "The Elephant",
            image: "6415859101aff794f751adcc6fb55440.jpg",
            uts: 1_794_516_558,
        },
        Scrobble {
            artist: "Ledbyher",
            artist_mbid: mbid,
            name: "ABSURD",
            album: "ABSURD",
            image: "6d23449469dfdf1f173b4ffef91b1f58.jpg",
            uts: 1_794_516_399,
        },
    ];
    let root = Node::new("recenttracks")
        .json_order(&[
            "track",
            "@attr",
            "user",
            "totalPages",
            "page",
            "perPage",
            "total",
        ])
        .array("track")
        .attr("user", "Syntiiix")
        .attr("page", 1u32)
        .attr("perPage", 2u32)
        .attr("totalPages", 80_322u64)
        .attr("total", 160_643u64)
        .children(tracks.iter().map(recent_track));
    Response::new(root).escape_slashes()
}

#[test]
fn recent_tracks_xml() {
    let expected = include_str!("data/recenttracks.xml");
    assert_eq!(render(&recent_tracks(), Format::Xml), expected);
}

#[test]
fn recent_tracks_json() {
    let expected = include_str!("data/recenttracks.json");
    assert_eq!(render(&recent_tracks(), Format::Json), expected);
}

#[test]
fn single_item_list_stays_an_array_with_attr_in_the_middle() {
    let artist = Node::new("artist")
        .json_order(&[
            "streamable",
            "image",
            "mbid",
            "url",
            "playcount",
            "@attr",
            "name",
        ])
        .attr("rank", 1u32)
        .child(Node::leaf("name", "Juice WRLD"))
        .child(Node::leaf("playcount", 34_955u32))
        .child(Node::leaf("mbid", "4e4ebde4-0c56-4dec-844b-6c73adcdd92d"))
        .child(Node::leaf("url", lfm_core::artist_url("Juice WRLD")))
        .child(Node::leaf("streamable", "0"))
        .children(images(
            Some("2a96cbd8b46e442fc41c2b86b821562f.png"),
            ImageSize::WITH_MEGA,
        ));
    let root = Node::new("topartists")
        .json_order(&[
            "artist",
            "@attr",
            "user",
            "totalPages",
            "page",
            "perPage",
            "total",
        ])
        .array("artist")
        .attr("user", "Syntiiix")
        .attr("page", 1u32)
        .attr("perPage", 1u32)
        .attr("totalPages", 4046u32)
        .attr("total", 4046u32)
        .child(artist);
    let expected = include_str!("data/topartists_limit1.json");
    assert_eq!(
        render(&Response::new(root).escape_slashes(), Format::Json),
        expected
    );
}

#[test]
fn empty_declared_list_and_raw_unicode_without_slash_escaping() {
    let taggings = Node::new("taggings")
        .child(Node::new("artists").array("artist"))
        .attr("user", "Syntiiix")
        .attr("tag", "björk&<x>/")
        .attr("page", 1u32)
        .attr("perPage", 50u32)
        .attr("totalPages", 0u32)
        .attr("total", 0u32);
    let expected = include_str!("data/personaltags_empty.json");
    assert_eq!(render(&Response::new(taggings), Format::Json), expected);
}

#[test]
fn single_undeclared_child_collapses_to_an_object() {
    let root = Node::new("tracks").child(Node::new("track").child(Node::leaf("name", "A")));
    assert_eq!(
        render(&Response::new(root), Format::Json),
        r#"{"tracks":{"track":{"name":"A"}}}"#
    );
}

#[test]
fn text_first_order_and_empty_elements() {
    let node = Node::new("track")
        .child(
            Node::leaf("streamable", "0")
                .attr("fulltrack", "0")
                .json_order(&["#text"]),
        )
        .child(Node::new("mbid"));
    let response = Response::new(node);
    assert_eq!(
        render(&response, Format::Json),
        r##"{"track":{"streamable":{"#text":"0","fulltrack":"0"},"mbid":""}}"##
    );
    assert!(render(&response, Format::Xml).contains("\n    <mbid></mbid>\n"));
}

#[test]
fn xml_escapes_all_five_characters_and_json_escapes_controls() {
    let response = Response::new(Node::leaf("name", "a&b<c>\"d'e\n\u{1}").attr("x", "'"));
    assert!(
        render(&response, Format::Xml)
            .contains("<name x=\"&apos;\">a&amp;b&lt;c&gt;&quot;d&apos;e\n\u{1}</name>")
    );
    assert_eq!(
        render(&response, Format::Json),
        r##"{"name":{"x":"'","#text":"a&b<c>\"d'e\n\u0001"}}"##
    );
}

const ERRORS: [(ErrorCode, &str); 4] = [
    (
        ErrorCode::InvalidMethod,
        "Invalid Method - No method with that name in this package",
    ),
    (
        ErrorCode::InvalidParameters,
        "Invalid parameters - Your request is missing a required parameter",
    ),
    (
        ErrorCode::InvalidApiKey,
        "Invalid API key - You must be granted a valid key by last.fm",
    ),
    (
        ErrorCode::InvalidSignature,
        "Invalid method signature supplied",
    ),
];

#[test]
fn observed_errors_in_both_formats() {
    for (code, message) in ERRORS {
        let n = code.number();
        let err = ApiError::new(code);
        assert_eq!(
            error(&err, Format::Xml),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<lfm status=\"failed\">\n  \
                 <error code=\"{n}\">{message}</error>\n</lfm>\n"
            )
        );
        assert_eq!(
            error(&err, Format::Json),
            format!(r#"{{"message":"{message}","error":{n}}}"#)
        );
    }
}

#[test]
fn observed_error_statuses() {
    let status = |code| ApiError::new(code).http_status();
    assert_eq!(status(ErrorCode::InvalidMethod), 400);
    assert_eq!(status(ErrorCode::InvalidFormat), 400);
    assert_eq!(status(ErrorCode::InvalidParameters), 400);
    assert_eq!(status(ErrorCode::InvalidApiKey), 403);
    assert_eq!(status(ErrorCode::InvalidSignature), 400);
    let not_found = ApiError::not_found("User not found");
    assert_eq!(not_found.http_status(), 404);
    assert_eq!(
        error(&not_found, Format::Json),
        r#"{"message":"User not found","error":6}"#
    );
}

#[test]
fn format_error_escapes_the_apostrophe() {
    let err = ApiError::new(ErrorCode::InvalidFormat);
    assert!(error(&err, Format::Xml).contains(
        "<error code=\"5\">Invalid format - This service doesn&apos;t exist in that format</error>"
    ));
}

#[test]
fn jsonp_wraps_success_and_errors_without_newline() {
    let params = Params::new().with("format", "json").with("callback", "cb");
    let output = Output::from_params(&params).unwrap();
    assert_eq!(output.content_type(), "text/javascript; charset=utf-8");
    let body = output.render_error(&ApiError::new(ErrorCode::InvalidMethod));
    assert_eq!(
        String::from_utf8(body).unwrap(),
        r#"cb({"message":"Invalid Method - No method with that name in this package","error":3});"#
    );
    let body = output.render(&Response::new(Node::leaf("a", "b")));
    assert_eq!(String::from_utf8(body).unwrap(), r#"cb({"a":"b"});"#);
}

#[test]
fn callback_is_ignored_for_xml_and_for_unsafe_names() {
    let xml = Params::new().with("callback", "cb");
    assert_eq!(Output::from_params(&xml).unwrap().callback(), None);
    let unsafe_name = Params::new()
        .with("format", "json")
        .with("callback", "alert(1)//");
    let output = Output::from_params(&unsafe_name).unwrap();
    assert_eq!(output.callback(), None);
    assert_eq!(output.content_type(), "application/json");
}

#[test]
fn every_error_code_has_a_distinct_number() {
    let codes = [
        ErrorCode::InvalidService,
        ErrorCode::InvalidMethod,
        ErrorCode::AuthenticationFailed,
        ErrorCode::InvalidFormat,
        ErrorCode::InvalidParameters,
        ErrorCode::InvalidResource,
        ErrorCode::OperationFailed,
        ErrorCode::InvalidSessionKey,
        ErrorCode::InvalidApiKey,
        ErrorCode::ServiceOffline,
        ErrorCode::SubscribersOnly,
        ErrorCode::InvalidSignature,
        ErrorCode::UnauthorizedToken,
        ErrorCode::NotStreamable,
        ErrorCode::TemporarilyUnavailable,
        ErrorCode::LoginRequired,
        ErrorCode::TrialExpired,
        ErrorCode::NotEnoughContent,
        ErrorCode::NotEnoughMembers,
        ErrorCode::NotEnoughFans,
        ErrorCode::NotEnoughNeighbours,
        ErrorCode::NoPeakRadio,
        ErrorCode::RadioNotFound,
        ErrorCode::ApiKeySuspended,
        ErrorCode::Deprecated,
        ErrorCode::RateLimitExceeded,
    ];
    let numbers: Vec<u8> = codes.iter().map(|c| c.number()).collect();
    let expected: Vec<u8> = (2..=29).filter(|n| ![19, 28].contains(n)).collect();
    assert_eq!(numbers, expected);
}
