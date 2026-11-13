//! The Last.fm link: the own-key rule, web auth's state binding, claims, private profiles, and
//! credentials that no longer open.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "tests")]

mod support;

use lfm_store::{AccountId, LastfmCredentials, StoreError};
use serde_json::{Value, json};
use support::mock::{mount_get, mount_profile, ok};
use support::{Harness, callback_state};
use wiremock::Mock;
use wiremock::matchers::query_param;

async fn account_id(h: &Harness, cookie: &str) -> AccountId {
    let me = h.call("GET", "/app/api/me").cookie(cookie).send().await;
    AccountId(me.json["account"]["id"].as_i64().unwrap())
}

async fn link(h: &Harness, cookie: &str, name: &str) -> support::Reply {
    h.call("PUT", "/app/api/lastfm")
        .cookie(cookie)
        .json(json!({ "username": name }))
        .send()
        .await
}

async fn status(h: &Harness, cookie: &str) -> Value {
    h.call("GET", "/app/api/lastfm")
        .cookie(cookie)
        .send()
        .await
        .json
}

/// Starts web auth and returns the state in the callback URL.
async fn start_web_auth(h: &Harness, cookie: &str) -> String {
    let start = h
        .call("GET", "/app/api/lastfm/auth/start")
        .cookie(cookie)
        .header("host", "fm.test")
        .send()
        .await;
    assert_eq!(start.status, 200, "{}", start.json);
    callback_state(start.json["url"].as_str().unwrap())
}

async fn callback(h: &Harness, cookie: &str, query: &str) -> support::Reply {
    h.call("GET", &format!("/app/api/lastfm/auth/callback?{query}"))
        .cookie(cookie)
        .send()
        .await
}

async fn mount_session(h: &Harness) {
    let session =
        json!({ "session": { "subscriber": 0, "name": "LfmAlice", "key": "real-session-key" } });
    mount_get(h, "auth.getSession", &session.to_string()).await;
}

#[tokio::test]
async fn the_own_key_rule_blocks_linking_until_a_key_is_saved() {
    let h = Harness::new().await;
    mount_profile(&h).await;
    let admin = h.signup("admin").await;
    let settings = h
        .call("PUT", "/app/api/admin/settings")
        .cookie(&admin)
        .json(json!({ "requireOwnKey": true }))
        .send()
        .await;
    assert_eq!(settings.json["requireOwnKey"], true, "{}", settings.json);
    assert_eq!(settings.json["signupMode"], "approval", "unchanged");
    assert_eq!(
        status(&h, &admin).await["credentials"]["ownKeyRequired"],
        true
    );

    let refused = link(&h, &admin, "LfmAlice").await;
    assert_eq!((refused.status, refused.code()), (409, "own_key_required"));

    mount_get(&h, "auth.getToken", r#"{"token":"t"}"#).await;
    let key = json!({ "apiKey": "a".repeat(32), "secret": "b".repeat(32) });
    let saved = h
        .call("PUT", "/app/api/lastfm/key")
        .cookie(&admin)
        .json(key)
        .send()
        .await;
    assert_eq!(saved.status, 204, "{}", saved.json);
    let linked = link(&h, &admin, "LfmAlice").await;
    assert_eq!(linked.status, 200, "{}", linked.json);
    assert_eq!(linked.json["ownKey"], true);
}

#[tokio::test]
async fn caller_keys_answer_an_accounts_own_lastfm_key() {
    let h = Harness::new().await;
    let admin = h.signup("admin").await;
    let caller_keys = h.web.caller_keys();
    let id = account_id(&h, &admin).await;
    assert!(caller_keys.lastfm_key(id.0).await.is_none());

    mount_get(&h, "auth.getToken", r#"{"token":"t"}"#).await;
    let own = "a".repeat(32);
    let key = json!({ "apiKey": own, "secret": "b".repeat(32) });
    let saved = h
        .call("PUT", "/app/api/lastfm/key")
        .cookie(&admin)
        .json(key)
        .send()
        .await;
    assert_eq!(saved.status, 204, "{}", saved.json);
    let found = caller_keys.lastfm_key(id.0).await.unwrap();
    assert_eq!(found.expose(), own);
    assert!(caller_keys.lastfm_key(id.0 + 1).await.is_none());
}

#[tokio::test]
async fn without_the_rule_the_instance_key_is_used() {
    let h = Harness::new().await;
    mount_profile(&h).await;
    let cookie = h.signup("admin").await;
    let linked = link(&h, &cookie, "LfmAlice").await;
    assert_eq!(linked.status, 200, "{}", linked.json);
    assert_eq!(
        (
            linked.json["ownKey"].as_bool(),
            linked.json["credentials"]["ownKeyRequired"].as_bool()
        ),
        (Some(false), Some(false))
    );
}

#[tokio::test]
async fn the_callback_needs_the_state_its_own_session_started() {
    let h = Harness::new().await;
    mount_profile(&h).await;
    mount_session(&h).await;
    let alice = h.signup("alice").await;
    h.call("PUT", "/app/api/admin/settings")
        .cookie(&alice)
        .json(json!({ "signupMode": "open" }))
        .send()
        .await;
    let mallory = h.signup("mallory").await;

    let missing = callback(&h, &alice, "token=t").await;
    assert_eq!((missing.status, missing.code()), (400, "invalid_state"));
    let state = start_web_auth(&h, &mallory).await;
    let forged = callback(&h, &alice, &format!("state={state}&token=t")).await;
    assert_eq!(forged.code(), "invalid_state", "another session's state");
    assert_eq!(h.lastfm_calls("auth.getSession").await.len(), 0);

    let state = start_web_auth(&h, &alice).await;
    let linked = callback(&h, &alice, &format!("state={state}&token=t")).await;
    assert_eq!(linked.status, 200, "{}", linked.json);
    assert_eq!(linked.json["authorized"], true);
    let reused = callback(&h, &alice, &format!("state={state}&token=t")).await;
    assert_eq!(reused.code(), "invalid_state");
}

#[tokio::test]
async fn confirming_ownership_takes_over_an_unconfirmed_claim() {
    let h = Harness::new().await;
    mount_profile(&h).await;
    mount_session(&h).await;
    let alice = h.signup("alice").await;
    h.call("PUT", "/app/api/admin/settings")
        .cookie(&alice)
        .json(json!({ "signupMode": "open" }))
        .send()
        .await;
    let squatter = h.signup("squatter").await;
    assert_eq!(link(&h, &squatter, "lfmalice").await.status, 200);
    let taken = link(&h, &alice, "LfmAlice").await;
    assert_eq!((taken.status, taken.code()), (409, "lastfm_user_taken"));

    let state = start_web_auth(&h, &alice).await;
    let confirmed = callback(&h, &alice, &format!("state={state}&token=t")).await;
    assert_eq!(confirmed.status, 200, "{}", confirmed.json);
    assert_eq!(confirmed.json["username"], "LfmAlice");
    assert_eq!(status(&h, &squatter).await["username"], Value::Null);

    // A confirmed link stays: the squatter cannot claim it back.
    let again = link(&h, &squatter, "LfmAlice").await;
    assert_eq!(again.code(), "lastfm_user_taken");
}

#[tokio::test]
async fn the_database_refuses_a_second_claim_in_any_case() {
    let h = Harness::new().await;
    let alice = h.signup("alice").await;
    h.call("PUT", "/app/api/admin/settings")
        .cookie(&alice)
        .json(json!({ "signupMode": "open" }))
        .send()
        .await;
    let bob = h.signup("bob").await;
    let (alice, bob) = (account_id(&h, &alice).await, account_id(&h, &bob).await);
    h.store
        .link_lastfm(alice, Some("LfmAlice".into()))
        .await
        .unwrap();
    let second = h.store.link_lastfm(bob, Some("LFMALICE".into())).await;
    assert!(matches!(second, Err(StoreError::NameTaken)), "{second:?}");
    h.store.link_lastfm(alice, None).await.unwrap();
    h.store
        .link_lastfm(bob, Some("LFMALICE".into()))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_private_profile_shows_why_and_can_be_retried() {
    let h = Harness::new().await;
    let private = json!({ "error": 17, "message": "Login: User required to be logged in" });
    Mock::given(query_param("method", "user.getRecentTracks"))
        .respond_with(ok(&private.to_string()))
        .with_priority(1)
        .mount(&h.lastfm)
        .await;
    mount_profile(&h).await;
    let cookie = h.signup("alice").await;
    assert_eq!(link(&h, &cookie, "LfmAlice").await.status, 200);
    support::eventually(|| async { status(&h, &cookie).await["import"]["status"] == "private" })
        .await;
    let shown = status(&h, &cookie).await;
    assert_eq!(shown["import"]["error"], lfm_sync::PRIVATE_MESSAGE);
    let refused = h.lastfm_calls("user.getRecentTracks").await.len();

    let retried = h
        .call("POST", "/app/api/lastfm/import/retry")
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(retried.status, 200, "{}", retried.json);
    support::eventually(|| async { h.lastfm_calls("user.getRecentTracks").await.len() > refused })
        .await;
}

#[tokio::test]
async fn credentials_that_no_longer_open_can_be_replaced() {
    let h = Harness::new().await;
    let cookie = h.signup("alice").await;
    let id = account_id(&h, &cookie).await;
    let garbage = LastfmCredentials {
        api_key: Some(vec![1; 40]),
        secret: Some(vec![2; 40]),
        session_key: Some(vec![3; 40]),
    };
    h.store.set_lastfm_credentials(id, garbage).await.unwrap();
    let shown = h
        .call("GET", "/app/api/lastfm")
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(shown.status, 200, "{}", shown.json);
    assert_eq!(
        (
            shown.json["credentials"]["error"].as_str().is_some(),
            shown.json["ownKey"].as_bool()
        ),
        (true, Some(false))
    );
    let cleared = h
        .call("DELETE", "/app/api/lastfm/key")
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(cleared.status, 204, "{}", cleared.json);
    assert_eq!(
        status(&h, &cookie).await["credentials"]["error"],
        Value::Null
    );
}
