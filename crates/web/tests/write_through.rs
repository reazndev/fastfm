//! The emulated Last.fm auth flow and write-through, end to end through the dispatcher, with
//! Last.fm mocked.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "tests")]

mod support;

use std::time::{SystemTime, UNIX_EPOCH};

use lfm_store::{AccountId, AuthToken, Page, TimeRange, UserId};
use serde_json::{Value, json};
use support::mock::scrobbles;
use support::mock::{accepted, assert_signed, entry, mount_get, mount_post, mount_profile, ok};
use support::{Harness, callback_state};
use wiremock::matchers::body_string_contains;
use wiremock::{Mock, ResponseTemplate};

const PASSWORD: &str = "a long password";

/// What a client holds after authorising: its key, secret and session key.
struct Client {
    cookie: String,
    key: String,
    secret: String,
    sk: String,
}

fn now() -> i64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    i64::try_from(secs).unwrap()
}

async fn create_key(h: &Harness, cookie: &str, scope: &str) -> (String, String) {
    let reply = h
        .call("POST", "/app/api/keys")
        .cookie(cookie)
        .json(json!({ "label": "scrobbler", "scope": scope }))
        .send()
        .await;
    let key = &reply.json["key"];
    (
        key["key"].as_str().unwrap().into(),
        key["secret"].as_str().unwrap().into(),
    )
}

/// Alice links Last.fm through web auth, creates a key, and authorises it the Last.fm way.
async fn authorised(h: &Harness) -> Client {
    let cookie = h.signup("alice").await;
    link_with_web_auth(h, &cookie).await;
    let (key, secret) = create_key(h, &cookie, "read_write").await;
    let sk = approve_the_last_fm_way(h, &cookie, &key, &secret).await;
    Client {
        cookie,
        key,
        secret,
        sk,
    }
}

/// Last.fm web auth: start, then the callback trades Last.fm's token for its session key.
async fn link_with_web_auth(h: &Harness, cookie: &str) {
    mount_profile(h).await;
    let session =
        json!({ "session": { "subscriber": 0, "name": "LfmAlice", "key": "real-session-key" } });
    mount_get(h, "auth.getSession", &session.to_string()).await;
    let start = h
        .call("GET", "/app/api/lastfm/auth/start")
        .cookie(cookie)
        .header("host", "fm.test")
        .send()
        .await;
    let url = start.json["url"].as_str().unwrap();
    assert!(
        url.starts_with("https://www.last.fm/api/auth/?api_key=1111"),
        "{url}"
    );
    let state = callback_state(url);
    assert!(
        url.ends_with(&format!(
            "&cb=http%3A%2F%2Ffm.test%2Flastfm%2Fcallback%3Fstate%3D{state}"
        )),
        "{url}"
    );
    let linked = h
        .call(
            "GET",
            &format!("/app/api/lastfm/auth/callback?state={state}&token=lfm-token"),
        )
        .cookie(cookie)
        .send()
        .await;
    assert_eq!(linked.status, 200, "{}", linked.json);
    assert_eq!(linked.json["username"], "LfmAlice");
    assert_eq!(
        (
            linked.json["authorized"].as_bool(),
            linked.json["writes"]["forwarding"].as_bool()
        ),
        (Some(true), Some(true))
    );
    let get_session = h.lastfm_calls("auth.getSession").await;
    assert_eq!(get_session[0]["token"], "lfm-token");
    assert_signed(&get_session[0]);
}

/// `auth.getToken`, approval on the web page, then `auth.getSession`. Returns the session key.
async fn approve_the_last_fm_way(h: &Harness, cookie: &str, key: &str, secret: &str) -> String {
    let (status, body) = h
        .api(&[("method", "auth.getToken"), ("api_key", key)])
        .await;
    assert_eq!(status, 200);
    let token = body["token"].as_str().unwrap().to_owned();
    assert_eq!(token.len(), 32);
    let get_session = [
        ("method", "auth.getSession"),
        ("api_key", key),
        ("token", token.as_str()),
    ];
    let (status, early) = h.signed(&get_session, secret).await;
    assert_eq!(
        (status, early["error"].as_i64()),
        (403, Some(14)),
        "{early}"
    );

    let asked = h
        .call("GET", &format!("/app/api/auth/request?token={token}"))
        .cookie(cookie)
        .send()
        .await;
    assert_eq!(
        (asked.json["label"].as_str(), asked.json["owner"].as_str()),
        (Some("scrobbler"), Some("alice"))
    );
    let approve = h
        .call("POST", "/app/api/auth/approve")
        .cookie(cookie)
        .json(json!({ "token": token }))
        .send()
        .await;
    assert_eq!(approve.status, 204);

    let (status, session) = h.signed(&get_session, secret).await;
    assert_eq!(status, 200, "{session}");
    assert_eq!(session["session"]["name"], "LfmAlice");
    assert_eq!(session["session"]["subscriber"], 0);
    let sk = session["session"]["key"].as_str().unwrap().to_owned();
    assert_eq!(sk.len(), 32);
    let (_, again) = h.signed(&get_session, secret).await;
    assert_eq!(again["error"], 4, "a token is spent once");
    sk
}

impl Client {
    async fn write(&self, h: &Harness, method: &str, extra: &[(&str, &str)]) -> (u16, Value) {
        let base = [
            ("method", method),
            ("api_key", self.key.as_str()),
            ("sk", self.sk.as_str()),
        ];
        let pairs: Vec<(&str, &str)> = base.iter().chain(extra).copied().collect();
        h.signed(&pairs, &self.secret).await
    }
}

async fn user(h: &Harness) -> UserId {
    h.store.user_by_name("LfmAlice").await.unwrap().unwrap().id
}

async fn stored(h: &Harness) -> Vec<(i64, String, String)> {
    let page = h
        .store
        .recent_tracks(user(h).await, TimeRange::ALL, Page::new(1, 50))
        .await
        .unwrap();
    page.items
        .iter()
        .map(|t| {
            (
                t.uts,
                t.track.artist.as_ref().unwrap().name.clone(),
                t.track.name.clone(),
            )
        })
        .collect()
}

#[tokio::test]
async fn signed_scrobble_lands_locally_and_is_forwarded_re_signed() {
    let h = Harness::new().await;
    let client = authorised(&h).await;
    mount_post(&h, "track.scrobble", accepted(), 5).await;
    let at = (now() + 30).to_string();
    let (status, body) = client
        .write(
            &h,
            "track.scrobble",
            &[("artist", "Cher"), ("track", "Believe"), ("timestamp", &at)],
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let scrobble = &body["scrobbles"]["scrobble"];
    assert_eq!(
        body["scrobbles"]["@attr"],
        json!({ "accepted": 1, "ignored": 0 })
    );
    assert_eq!(
        scrobble["artist"],
        json!({ "corrected": "0", "#text": "Cher" })
    );
    assert_eq!(scrobble["timestamp"], at.as_str());
    assert_eq!(
        stored(&h).await,
        vec![(now_parse(&at), "Cher".into(), "Believe".into())]
    );

    let forwarded = h.lastfm_calls("track.scrobble").await;
    assert_eq!(forwarded.len(), 1);
    assert_eq!(forwarded[0]["sk"], "real-session-key");
    assert_eq!(
        (
            forwarded[0]["artist[0]"].as_str(),
            forwarded[0]["timestamp[0]"].as_str()
        ),
        ("Cher", at.as_str())
    );
    assert_signed(&forwarded[0]);
}

fn now_parse(text: &str) -> i64 {
    text.parse().unwrap()
}

#[tokio::test]
async fn bad_signature_unknown_and_expired_tokens() {
    let h = Harness::new().await;
    let client = authorised(&h).await;
    let base = [
        ("method", "track.love"),
        ("api_key", client.key.as_str()),
        ("sk", client.sk.as_str()),
        ("artist", "A"),
        ("track", "B"),
    ];
    let (status, body) = h.signed(&base, "not-the-secret").await;
    assert_eq!((status, body["error"].as_i64()), (400, Some(13)));

    let get = |token: &'static str| {
        [
            ("method", "auth.getSession"),
            ("api_key", client.key.as_str()),
            ("token", token),
        ]
    };
    let (_, unknown) = h.signed(&get("no-such-token"), &client.secret).await;
    assert_eq!(unknown["error"], 4);
    let me = h
        .call("GET", "/app/api/me")
        .cookie(&client.cookie)
        .send()
        .await;
    let account = AccountId(me.json["account"]["id"].as_i64().unwrap());
    let expired = AuthToken {
        token: "old-token".into(),
        api_key: client.key.clone(),
        account: Some(account),
        expires_at: now() - 1,
    };
    h.store
        .create_auth_token(expired, now() - 3600)
        .await
        .unwrap();
    let (_, body) = h.signed(&get("old-token"), &client.secret).await;
    assert_eq!(body["error"], 15);
}

#[tokio::test]
async fn read_only_keys_cannot_write_and_sessions_stay_with_their_key() {
    let h = Harness::new().await;
    let client = authorised(&h).await;
    let (read_key, read_secret) = create_key(&h, &client.cookie, "read").await;
    let mobile = [
        ("method", "auth.getMobileSession"),
        ("api_key", read_key.as_str()),
        ("username", "alice"),
        ("password", PASSWORD),
    ];
    let (status, session) = h.signed(&mobile, &read_secret).await;
    assert_eq!(status, 200, "{session}");
    let read_sk = session["session"]["key"].as_str().unwrap();
    let love = [
        ("method", "track.love"),
        ("api_key", read_key.as_str()),
        ("sk", read_sk),
        ("artist", "A"),
        ("track", "B"),
    ];
    let (_, body) = h.signed(&love, &read_secret).await;
    assert_eq!(body["error"], 4, "{body}");

    let wrong = [
        ("method", "auth.getMobileSession"),
        ("api_key", read_key.as_str()),
        ("username", "alice"),
        ("password", "wrong password"),
    ];
    assert_eq!(h.signed(&wrong, &read_secret).await.1["error"], 4);

    // The read-write key's session key used with another key is not a session of that key.
    let (other_key, other_secret) = create_key(&h, &client.cookie, "read_write").await;
    let borrowed = [
        ("method", "track.love"),
        ("api_key", other_key.as_str()),
        ("sk", client.sk.as_str()),
        ("artist", "A"),
        ("track", "B"),
    ];
    assert_eq!(h.signed(&borrowed, &other_secret).await.1["error"], 9);

    // Revoking the key ends its sessions.
    h.call("DELETE", &format!("/app/api/keys/{}", client.key))
        .cookie(&client.cookie)
        .send()
        .await;
    let (_, body) = client
        .write(&h, "track.love", &[("artist", "A"), ("track", "B")])
        .await;
    assert_eq!(body["error"], 10);
    assert!(h.store.api_session(&client.sk).await.unwrap().is_none());
}

#[tokio::test]
async fn batches_keep_order_and_apply_last_fm_corrections() {
    let h = Harness::new().await;
    let client = authorised(&h).await;
    let corrected = json!([
        entry(("Cher", "1"), ("Believe", "1"), ("0", "")),
        entry(("Spam", "0"), ("Spam", "0"), ("1", "Artist was ignored")),
    ]);
    mount_post(&h, "track.scrobble", scrobbles(&corrected, 1, 1), 5).await;
    let (t0, t1) = ((now() + 30).to_string(), (now() + 40).to_string());
    let old = (now() - 30 * 86_400).to_string();
    let batch = [
        ("artist[0]", "cher"),
        ("track[0]", "belive"),
        ("timestamp[0]", t0.as_str()),
        ("artist[1]", "Old"),
        ("track[1]", "Song"),
        ("timestamp[1]", old.as_str()),
        ("artist[2]", "Spam"),
        ("track[2]", "Spam"),
        ("timestamp[2]", t1.as_str()),
    ];
    let (status, body) = client.write(&h, "track.scrobble", &batch).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["scrobbles"]["@attr"],
        json!({ "accepted": 1, "ignored": 2 })
    );
    let items = body["scrobbles"]["scrobble"].as_array().unwrap();
    assert_eq!(
        items[0]["track"],
        json!({ "corrected": "1", "#text": "Believe" })
    );
    assert_eq!(
        items[1]["ignoredMessage"],
        json!({ "code": "3", "#text": "Timestamp was too old" })
    );
    assert_eq!(items[2]["ignoredMessage"]["code"], "1");
    assert_eq!(
        stored(&h).await,
        vec![(now_parse(&t0), "Cher".into(), "Believe".into())]
    );
    let forwarded = h.lastfm_calls("track.scrobble").await;
    assert!(
        !forwarded[0].contains_key("artist[2]"),
        "locally ignored items are not sent"
    );
}

#[tokio::test]
async fn transient_forward_failures_are_retried_without_double_counting() {
    let h = Harness::new().await;
    let client = authorised(&h).await;
    Mock::given(body_string_contains("method=track.scrobble&"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&h.lastfm)
        .await;
    mount_post(&h, "track.scrobble", accepted(), 5).await;
    let at = (now() + 30).to_string();
    let (status, body) = client
        .write(
            &h,
            "track.scrobble",
            &[("artist", "Cher"), ("track", "Believe"), ("timestamp", &at)],
        )
        .await;
    assert_eq!(
        (status, body["scrobbles"]["@attr"]["accepted"].as_i64()),
        (200, Some(1))
    );
    let link = h
        .call("GET", "/app/api/lastfm")
        .cookie(&client.cookie)
        .send()
        .await;
    assert_eq!(link.json["writes"]["queued"], 1, "{}", link.json);

    assert_eq!(h.web.retry_writes().await, 1);
    let link = h
        .call("GET", "/app/api/lastfm")
        .cookie(&client.cookie)
        .send()
        .await;
    assert_eq!(link.json["writes"]["queued"], 0);
    assert_eq!(h.lastfm_calls("track.scrobble").await.len(), 2);
    assert_eq!(
        stored(&h).await.len(),
        1,
        "the retried scrobble is stored once"
    );
}

#[tokio::test]
async fn love_unlove_and_now_playing() {
    let h = Harness::new().await;
    let client = authorised(&h).await;
    mount_post(&h, "track.love", ok("{}"), 5).await;
    mount_post(&h, "track.unlove", ok("{}"), 5).await;
    mount_post(&h, "track.updateNowPlaying", ok("{}"), 5).await;
    let track = [("artist", "Cher"), ("track", "Believe")];
    let (status, body) = client.write(&h, "track.love", &track).await;
    assert_eq!((status, body), (200, json!({})));
    let loved = h
        .store
        .loved_tracks(user(&h).await, Page::new(1, 10))
        .await
        .unwrap();
    assert_eq!(loved.items[0].track.name, "Believe");
    assert_signed(&h.lastfm_calls("track.love").await[0]);
    client.write(&h, "track.unlove", &track).await;
    assert_eq!(
        h.store
            .loved_tracks(user(&h).await, Page::new(1, 10))
            .await
            .unwrap()
            .total,
        0
    );
    assert_eq!(h.lastfm_calls("track.unlove").await.len(), 1);

    let version = h.store.user(user(&h).await).await.unwrap().unwrap().version;
    let (status, body) = client
        .write(
            &h,
            "track.updateNowPlaying",
            &[
                ("artist", "Cher"),
                ("track", "Believe"),
                ("album", "Believe"),
            ],
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        body["nowplaying"]["track"],
        json!({ "corrected": "0", "#text": "Believe" })
    );
    assert_eq!(
        body["nowplaying"]["ignoredMessage"],
        json!({ "code": "0", "#text": "" })
    );
    let playing = h.sync.now_playing().get(user(&h).await).unwrap();
    assert_eq!(
        (
            playing.track.name.as_str(),
            playing.track.album.name.as_str()
        ),
        ("Believe", "Believe")
    );
    assert!(h.store.user(user(&h).await).await.unwrap().unwrap().version > version);
    assert_eq!(h.lastfm_calls("track.updateNowPlaying").await.len(), 1);
}

#[tokio::test]
async fn an_unconfirmed_link_cannot_write() {
    let h = Harness::new().await;
    let cookie = h.signup("alice").await;
    let me = h.call("GET", "/app/api/me").cookie(&cookie).send().await;
    let account = AccountId(me.json["account"]["id"].as_i64().unwrap());
    h.store
        .link_lastfm(account, Some("LfmAlice".into()))
        .await
        .unwrap();
    let (key, secret) = create_key(&h, &cookie, "read_write").await;
    let mobile = [
        ("method", "auth.getMobileSession"),
        ("api_key", key.as_str()),
        ("username", "alice"),
        ("password", PASSWORD),
    ];
    let (_, session) = h.signed(&mobile, &secret).await;
    let client = Client {
        cookie,
        key,
        secret,
        sk: session["session"]["key"].as_str().unwrap().into(),
    };
    let at = now().to_string();
    let scrobble = [
        ("artist[0]", "Cher"),
        ("track[0]", "Believe"),
        ("timestamp[0]", at.as_str()),
    ];
    let (status, body) = client.write(&h, "track.scrobble", &scrobble).await;
    assert_eq!((status, body["error"].as_i64()), (403, Some(4)), "{body}");
    assert!(body["message"].as_str().unwrap().contains("Connect"));
    let (_, body) = client
        .write(
            &h,
            "track.love",
            &[("artist", "Cher"), ("track", "Believe")],
        )
        .await;
    assert_eq!(body["error"], 4);
    let mirrored = h.store.user_by_name("LfmAlice").await.unwrap();
    assert!(mirrored.is_none(), "nothing was written");
    assert_eq!(h.lastfm_calls("track.scrobble").await.len(), 0);
}
