//! Shared parameter and formatting helpers. Expected values come from Last.fm responses
//! recorded on 2026-11-12 unless a test says otherwise.

use lfm_core::{
    ApiError, Format, ImageSize, PageSpec, Paging, Params, Period, TimeRange, album_url,
    api_signature, artist_url, date_text, image_url, track_url, verify_signature,
};

#[test]
fn date_text_is_utc_with_zero_padded_day() {
    assert_eq!(date_text(1_793_990_654), "06 Nov 2026, 18:44");
    assert_eq!(date_text(1_794_516_558), "12 Nov 2026, 20:49");
    assert_eq!(date_text(0), "01 Jan 1970, 00:00");
    assert_eq!(date_text(951_782_400), "29 Feb 2000, 00:00");
}

#[test]
fn music_urls() {
    assert_eq!(
        track_url("Ledbyher", "Remember, Remember"),
        "https://www.last.fm/music/Ledbyher/_/Remember,+Remember"
    );
    assert_eq!(
        track_url(
            "AC/DC",
            "It's a Long Way to the Top (If You Wanna Rock 'N' Roll)"
        ),
        "https://www.last.fm/music/AC%2FDC/_/It%27s+a+Long+Way+to+the+Top+(If+You+Wanna+Rock+%27N%27+Roll)"
    );
    assert_eq!(
        album_url("Björk", "Vespertine"),
        "https://www.last.fm/music/Bj%C3%B6rk/Vespertine"
    );
    assert_eq!(
        artist_url("Juice WRLD"),
        "https://www.last.fm/music/Juice+WRLD"
    );
}

#[test]
fn image_urls() {
    let file = "6415859101aff794f751adcc6fb55440.jpg";
    let urls: Vec<String> = ImageSize::WITH_MEGA
        .iter()
        .map(|s| image_url(file, *s))
        .collect();
    let base = "https://lastfm-img.freetls.fastly.net/i/u";
    assert_eq!(
        urls,
        ["34s", "64s", "174s", "300x300", "300x300"].map(|p| format!("{base}/{p}/{file}"))
    );
}

#[test]
fn paging_defaults_clamps_and_counts() {
    let spec = PageSpec::new(50, 200);
    let parse = |p: Params| Paging::parse(&p, spec);
    assert_eq!(parse(Params::new()), Paging { page: 1, limit: 50 });
    let p = parse(Params::new().with("page", "3").with("limit", "1000"));
    assert_eq!(
        p,
        Paging {
            page: 3,
            limit: 200
        }
    );
    assert_eq!(p.offset(), 400);
    let bad = parse(Params::new().with("page", "0").with("limit", "x"));
    assert_eq!(bad, Paging { page: 1, limit: 50 });
    let two = Paging { page: 1, limit: 2 };
    assert_eq!(two.total_pages(160_643), 80_322);
    assert_eq!(two.total_pages(0), 0);
}

#[test]
fn period_ranges() {
    let now = 1_000_000_000;
    let period = |v: &str| Period::parse(&Params::new().with("period", v));
    assert_eq!(period("7day"), Period::Week);
    assert_eq!(period("12month").as_str(), "12month");
    assert_eq!(period("bogus"), Period::Overall);
    assert_eq!(Period::Overall.range(now), TimeRange::default());
    let week = Period::Week.range(now);
    assert_eq!(week.from, Some(now - 7 * 86_400));
    assert!(week.contains(now) && !week.contains(now - 7 * 86_400 - 1));
    let range = TimeRange::parse(&Params::new().with("from", "10").with("to", "x"));
    assert_eq!(
        range,
        TimeRange {
            from: Some(10),
            to: None
        }
    );
}

#[test]
fn params_merge_flags_and_required() {
    let params = Params::new()
        .with_urlencoded(b"user=a&extended=1&empty=")
        .with_urlencoded(b"user=b&autocorrect=true");
    assert_eq!(params.get("user"), Some("b"));
    assert!(params.flag("extended") && params.flag("autocorrect") && !params.flag("nope"));
    assert_eq!(params.get("empty"), Some(""));
    assert_eq!(params.required("empty"), Err(ApiError::missing_parameter()));
}

#[test]
fn format_parsing() {
    assert_eq!(Format::parse(None), Ok(Format::Xml));
    assert_eq!(Format::parse(Some("json")), Ok(Format::Json));
    assert_eq!(Format::parse(Some("bogus")).unwrap_err().code().number(), 5);
}

#[test]
fn signature_skips_format_callback_and_api_sig() {
    // Expected value computed with Python's hashlib over
    // "api_keyxxxartistChermethodtrack.loveskyyytrackBelieve" + "s3cret".
    let expected = "b82430273207d440cfdd24e21622908e";
    let params = Params::new()
        .with("method", "track.love")
        .with("track", "Believe")
        .with("artist", "Cher")
        .with("api_key", "xxx")
        .with("sk", "yyy")
        .with("format", "json")
        .with("callback", "cb");
    assert_eq!(api_signature(&params, "s3cret"), expected);
    let signed = params.clone().with("api_sig", expected.to_uppercase());
    assert!(verify_signature(&signed, "s3cret"));
    assert!(!verify_signature(&signed, "other"));
    assert!(!verify_signature(&params, "s3cret"));
}
