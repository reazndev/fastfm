//! Signed calls and writes against a mock Last.fm.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "tests")]

#[allow(dead_code, reason = "shared helpers; this binary uses some of them")]
mod support;

use lfm_upstream::{ApiKey, AppCredentials, ScrobbleItem, Secret, TrackItem, WriteCredentials};
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn creds() -> WriteCredentials {
    WriteCredentials {
        app: AppCredentials {
            api_key: ApiKey::new("app-key"),
            secret: Secret::new("app-secret"),
        },
        session_key: Secret::new("user-sk"),
    }
}

fn item(artist: &str, at: i64) -> ScrobbleItem {
    ScrobbleItem {
        track: TrackItem {
            artist: artist.into(),
            track: "Song".into(),
            album: Some("Album".into()),
            ..TrackItem::default()
        },
        timestamp: at,
        chosen_by_user: None,
    }
}

#[tokio::test]
async fn scrobbles_are_posted_signed_and_parsed_per_item() {
    let server = MockServer::start().await;
    let body = r##"{"scrobbles":{"scrobble":[
        {"artist":{"corrected":"1","#text":"Fixed"},"track":{"corrected":"0","#text":"Song"},
         "ignoredMessage":{"code":"0","#text":""}},
        {"artist":{"corrected":"0","#text":"B"},"track":{"corrected":"0","#text":"Song"},
         "ignoredMessage":{"code":"3","#text":"Timestamp was too old"}}],
        "@attr":{"accepted":1,"ignored":1}}}"##;
    Mock::given(method("POST"))
        .and(body_string_contains("method=track.scrobble"))
        .respond_with(support::json(200, body))
        .mount(&server)
        .await;
    let client = support::fast_client(&server);
    let outcomes = client
        .scrobble(&[item("fixd", 10), item("B", 20)], &creds())
        .await
        .unwrap();
    assert_eq!(
        (
            outcomes[0].artist.text.as_str(),
            outcomes[0].artist.corrected
        ),
        ("Fixed", true)
    );
    assert_eq!(outcomes[0].album.text, "Album");
    assert!(outcomes[0].ignored.is_none());
    assert_eq!(outcomes[1].ignored.as_ref().unwrap().code, 3);

    let sent = &server.received_requests().await.unwrap()[0];
    let form: Vec<(String, String)> = form_pairs(&sent.body);
    let get = |name: &str| {
        form.iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(get("artist[1]"), Some("B"));
    assert_eq!(get("timestamp[0]"), Some("10"));
    assert_eq!(get("sk"), Some("user-sk"));
    assert_eq!(get("api_key"), Some("app-key"));
    let unsigned = form
        .iter()
        .filter(|(n, _)| n != "api_sig" && n != "format")
        .fold(lfm_core::Params::new(), |p, (n, v)| {
            p.with(n.as_str(), v.as_str())
        });
    assert_eq!(
        get("api_sig"),
        Some(lfm_core::api_signature(&unsigned, "app-secret").as_str())
    );
}

#[tokio::test]
async fn writes_are_sent_once_and_classified() {
    let server = MockServer::start().await;
    Mock::given(body_string_contains("method=track.love"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    Mock::given(body_string_contains("method=track.unlove"))
        .respond_with(support::json(
            403,
            r#"{"error":9,"message":"Invalid session key"}"#,
        ))
        .mount(&server)
        .await;
    let client = support::fast_client(&server);
    let error = client.set_love("A", "B", true, &creds()).await.unwrap_err();
    assert!(error.is_transient());
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        1,
        "no retry for a write"
    );
    let error = client
        .set_love("A", "B", false, &creds())
        .await
        .unwrap_err();
    assert!(!error.is_transient(), "{error}");
}

fn form_pairs(body: &[u8]) -> Vec<(String, String)> {
    String::from_utf8_lossy(body)
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(n, v)| (decode(n), decode(v)))
        .collect()
}

fn decode(text: &str) -> String {
    let bytes = text.replace('+', " ");
    let mut out = Vec::new();
    let mut iter = bytes.bytes();
    while let Some(b) = iter.next() {
        if b == b'%' {
            let hex: String = iter.by_ref().take(2).map(char::from).collect();
            out.push(u8::from_str_radix(&hex, 16).unwrap());
        } else {
            out.push(b);
        }
    }
    String::from_utf8(out).unwrap()
}
