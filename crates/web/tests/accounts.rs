//! Sign-up modes, the setup token, approval, admin actions, API keys, cookies, CSRF and the
//! login rate limit.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "tests")]

mod support;

use serde_json::json;
use support::Harness;

const PASSWORD: &str = "a long password";

async fn key(h: &Harness, cookie: &str, scope: &str) -> (String, String) {
    let reply = h
        .call("POST", "/app/api/keys")
        .cookie(cookie)
        .json(json!({ "label": "app", "scope": scope }))
        .send()
        .await;
    assert_eq!(reply.status, 201, "{}", reply.json);
    let key = &reply.json["key"];
    (
        key["key"].as_str().unwrap().to_owned(),
        key["secret"].as_str().unwrap().to_owned(),
    )
}

/// Whether the API dispatcher accepts `key`.
async fn key_works(h: &Harness, key: &str) -> bool {
    let (status, body) = h
        .api(&[("method", "auth.getToken"), ("api_key", key)])
        .await;
    assert!(status == 200 || body["error"] == 10, "{status} {body}");
    status == 200
}

async fn id_of(h: &Harness, admin: &str, name: &str) -> i64 {
    let list = h
        .call("GET", "/app/api/admin/accounts")
        .cookie(admin)
        .send()
        .await;
    let accounts = list.json["accounts"].as_array().unwrap();
    let account = accounts.iter().find(|a| a["username"] == name).unwrap();
    account["id"].as_i64().unwrap()
}

async fn admin_post(h: &Harness, admin: &str, id: i64, action: &str) -> u16 {
    let path = format!("/app/api/admin/accounts/{id}/{action}");
    h.call("POST", &path).cookie(admin).send().await.status
}

#[tokio::test]
async fn first_account_is_admin_and_pending_accounts_are_refused_everywhere() {
    let h = Harness::new().await;
    let admin = h.signup("alice").await;
    let me = h.call("GET", "/app/api/me").cookie(&admin).send().await;
    assert_eq!(
        (
            me.json["account"]["role"].as_str(),
            me.json["account"]["status"].as_str()
        ),
        (Some("admin"), Some("active"))
    );

    let bob = h.signup("bob").await;
    let me = h.call("GET", "/app/api/me").cookie(&bob).send().await;
    assert_eq!(me.status, 200);
    assert_eq!(me.json["account"]["status"], "pending");
    let refused = [
        ("GET", "/app/api/keys"),
        ("POST", "/app/api/keys"),
        ("GET", "/app/api/lastfm"),
        ("PUT", "/app/api/lastfm"),
        ("DELETE", "/app/api/lastfm"),
        ("PUT", "/app/api/lastfm/key"),
        ("GET", "/app/api/lastfm/auth/start"),
        ("GET", "/app/api/lastfm/auth/callback?token=t"),
        ("GET", "/app/api/auth/request?token=t"),
        ("POST", "/app/api/auth/approve"),
        ("GET", "/app/api/admin/accounts"),
        ("GET", "/app/api/admin/settings"),
    ];
    for (method, path) in refused {
        let reply = h
            .call(method, path)
            .cookie(&bob)
            .json(json!({}))
            .send()
            .await;
        assert_eq!(
            (reply.status, reply.code()),
            (403, "account_pending"),
            "{method} {path}"
        );
    }
    let anonymous = h.call("GET", "/app/api/keys").send().await;
    assert_eq!((anonymous.status, anonymous.code()), (401, "not_logged_in"));
}

#[tokio::test]
async fn sign_up_modes() {
    let h = Harness::new().await;
    let admin = h.signup("alice").await;
    let settings = h
        .call("GET", "/app/api/admin/settings")
        .cookie(&admin)
        .send()
        .await;
    assert_eq!(settings.json["signupMode"], "approval");
    assert_eq!(
        settings.json["lastfmKeyPool"],
        json!({ "keys": 1, "canSign": true })
    );
    assert!(!settings.json.to_string().contains(support::LASTFM_KEY));

    let set = |mode: &'static str| {
        h.call("PUT", "/app/api/admin/settings")
            .cookie(&admin)
            .json(json!({ "signupMode": mode }))
    };
    assert_eq!(set("open").send().await.status, 200);
    let carol = h.signup("carol").await;
    let me = h.call("GET", "/app/api/me").cookie(&carol).send().await;
    assert_eq!(me.json["account"]["status"], "active");

    assert_eq!(set("closed").send().await.status, 200);
    let refused = h
        .call("POST", "/app/api/signup")
        .json(json!({ "username": "dave", "password": PASSWORD }))
        .send()
        .await;
    assert_eq!((refused.status, refused.code()), (403, "signup_closed"));
    assert_eq!(set("sometimes").send().await.status, 400);
    let not_admin = h
        .call("GET", "/app/api/admin/settings")
        .cookie(&carol)
        .send()
        .await;
    assert_eq!((not_admin.status, not_admin.code()), (403, "admin_only"));
}

#[tokio::test]
async fn sign_up_rules() {
    let h = Harness::new().await;
    let signup = |name: &str, password: &str| {
        h.call("POST", "/app/api/signup")
            .json(json!({ "username": name, "password": password }))
    };
    assert_eq!(
        signup("x", PASSWORD).send().await.code(),
        "invalid_username"
    );
    assert_eq!(
        signup("alice", "short").send().await.code(),
        "weak_password"
    );
    h.signup("alice").await;
    let taken = signup("ALICE", PASSWORD).send().await;
    assert_eq!((taken.status, taken.code()), (409, "name_taken"));
}

#[tokio::test]
async fn approve_reject_disable_enable_and_the_last_admin() {
    let h = Harness::new().await;
    let admin = h.signup("alice").await;
    let bob = h.signup("bob").await;
    h.signup("eve").await;
    let (bob_id, eve_id) = (
        id_of(&h, &admin, "bob").await,
        id_of(&h, &admin, "eve").await,
    );
    let pending = h
        .call("GET", "/app/api/admin/accounts?status=pending")
        .cookie(&admin)
        .send()
        .await;
    assert_eq!(pending.json["accounts"].as_array().unwrap().len(), 2);

    assert_eq!(admin_post(&h, &admin, bob_id, "approve").await, 204);
    let (bob_key, _) = key(&h, &bob, "read").await;
    assert!(key_works(&h, &bob_key).await);

    assert_eq!(admin_post(&h, &admin, eve_id, "reject").await, 204);
    let login = h
        .call("POST", "/app/api/login")
        .json(json!({ "username": "eve", "password": PASSWORD }))
        .send()
        .await;
    assert_eq!(login.code(), "invalid_credentials");

    assert_eq!(admin_post(&h, &admin, bob_id, "disable").await, 204);
    assert!(
        !key_works(&h, &bob_key).await,
        "a disabled account's key must stop working"
    );
    let refused = h.call("GET", "/app/api/me").cookie(&bob).send().await;
    assert_eq!((refused.status, refused.code()), (403, "account_disabled"));
    assert_eq!(admin_post(&h, &admin, bob_id, "enable").await, 204);
    assert!(key_works(&h, &bob_key).await);

    let alice_id = id_of(&h, &admin, "alice").await;
    let last = h
        .call(
            "POST",
            &format!("/app/api/admin/accounts/{alice_id}/disable"),
        )
        .cookie(&admin)
        .send()
        .await;
    assert_eq!((last.status, last.code()), (409, "last_admin"));
    let role = |id: i64, role: &'static str| {
        h.call("PUT", &format!("/app/api/admin/accounts/{id}/role"))
            .cookie(&admin)
            .json(json!({ "role": role }))
    };
    assert_eq!(role(alice_id, "user").send().await.code(), "last_admin");
    assert_eq!(role(bob_id, "admin").send().await.status, 204);
    assert_eq!(role(alice_id, "user").send().await.status, 204);
}

#[tokio::test]
async fn keys_are_created_listed_and_revoked() {
    let h = Harness::new().await;
    let admin = h.signup("alice").await;
    let (key, secret) = key(&h, &admin, "read_write").await;
    assert_eq!((key.len(), secret.len()), (32, 32));
    assert!(
        key.bytes()
            .chain(secret.bytes())
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    );
    let list = h.call("GET", "/app/api/keys").cookie(&admin).send().await;
    let listed = &list.json["keys"][0];
    assert_eq!(
        (listed["secret"].as_str(), listed["scope"].as_str()),
        (Some(secret.as_str()), Some("read_write"))
    );
    assert!(key_works(&h, &key).await);

    let revoke = h
        .call("DELETE", &format!("/app/api/keys/{key}"))
        .cookie(&admin)
        .send()
        .await;
    assert_eq!(revoke.status, 204);
    assert!(
        !key_works(&h, &key).await,
        "a revoked key must stop working"
    );
    let bad_label = h
        .call("POST", "/app/api/keys")
        .cookie(&admin)
        .json(json!({ "label": " ", "scope": "read" }))
        .send()
        .await;
    assert_eq!(bad_label.code(), "invalid_label");
}

#[tokio::test]
async fn cookie_flags_rotation_and_logout() {
    let h = Harness::new().await;
    let signup = h
        .call("POST", "/app/api/signup")
        .json(json!({ "username": "alice", "password": PASSWORD }))
        .send()
        .await;
    let flags = signup.set_cookie().unwrap();
    for flag in ["HttpOnly", "SameSite=Strict", "Path=/", "Max-Age=2592000"] {
        assert!(flags.contains(flag), "{flags}");
    }
    assert!(
        !flags.contains("Secure"),
        "plain HTTP gets no Secure flag: {flags}"
    );
    let first = signup.cookie().unwrap();

    let login = h
        .call("POST", "https://mirror.example/app/api/login")
        .cookie(&first)
        .json(json!({ "username": "alice", "password": PASSWORD }))
        .send()
        .await;
    assert_eq!(login.status, 200);
    assert!(login.set_cookie().unwrap().contains("; Secure"));
    let second = login.cookie().unwrap();
    assert_ne!(first, second);
    let old = h.call("GET", "/app/api/me").cookie(&first).send().await;
    assert_eq!(old.status, 401, "login must end the session it replaces");

    let logout = h
        .call("POST", "/app/api/logout")
        .cookie(&second)
        .send()
        .await;
    assert_eq!(logout.status, 204);
    assert!(logout.set_cookie().unwrap().contains("Max-Age=0"));
    assert_eq!(
        h.call("GET", "/app/api/me")
            .cookie(&second)
            .send()
            .await
            .status,
        401
    );
}

#[tokio::test]
async fn state_changes_need_the_csrf_header() {
    let h = Harness::new().await;
    let admin = h.signup("alice").await;
    let login = h
        .call("POST", "/app/api/login")
        .without_csrf()
        .json(json!({ "username": "alice", "password": PASSWORD }))
        .send()
        .await;
    assert_eq!((login.status, login.code()), (403, "csrf_header_missing"));
    let create = h
        .call("POST", "/app/api/keys")
        .cookie(&admin)
        .without_csrf()
        .json(json!({ "label": "x", "scope": "read" }))
        .send()
        .await;
    assert_eq!(create.code(), "csrf_header_missing");
    let callback = h
        .call("GET", "/app/api/lastfm/auth/callback?token=t")
        .cookie(&admin)
        .without_csrf()
        .send()
        .await;
    assert_eq!(callback.code(), "csrf_header_missing");
    let read = h
        .call("GET", "/app/api/keys")
        .cookie(&admin)
        .without_csrf()
        .send()
        .await;
    assert_eq!(read.status, 200);
}

#[tokio::test]
async fn login_failures_look_alike_and_are_rate_limited() {
    let h = Harness::new().await;
    h.signup("alice").await;
    let login = |name: &'static str, password: &'static str| {
        h.call("POST", "/app/api/login")
            .json(json!({ "username": name, "password": password }))
    };
    let unknown = login("nobody", PASSWORD).send().await;
    let wrong = login("alice", "wrong password").send().await;
    assert_eq!((unknown.status, &unknown.json), (wrong.status, &wrong.json));
    for _ in 1..10 {
        assert_eq!(login("alice", "wrong password").send().await.status, 401);
    }
    let limited = login("alice", PASSWORD).send().await;
    assert_eq!((limited.status, limited.code()), (429, "rate_limited"));
    // Another name from the same client is still allowed until the per-IP limit.
    assert_eq!(login("nobody", PASSWORD).send().await.status, 401);
}

#[tokio::test]
async fn last_fm_clients_are_sent_to_the_approval_route() {
    let h = Harness::new().await;
    let reply = h.call("GET", "/api/auth/?api_key=k&token=t").send().await;
    assert_eq!(reply.status, 303);
    let location = reply.headers.get("location").unwrap().to_str().unwrap();
    assert_eq!(location, "/authorize?api_key=k&token=t");
}

#[tokio::test]
async fn the_sign_up_mode_is_public() {
    let h = Harness::new().await;
    let fresh = h.call("GET", "/app/api/signup-mode").send().await;
    assert_eq!(
        fresh.json,
        json!({ "mode": "approval", "hasAccounts": false, "setupTokenRequired": false })
    );
    let admin = h.signup("alice").await;
    h.call("PUT", "/app/api/admin/settings")
        .cookie(&admin)
        .json(json!({ "signupMode": "closed" }))
        .send()
        .await;
    let later = h.call("GET", "/app/api/signup-mode").send().await;
    assert_eq!(
        (
            later.json["mode"].as_str(),
            later.json["hasAccounts"].as_bool()
        ),
        (Some("closed"), Some(true))
    );
}

#[tokio::test]
async fn the_first_sign_up_needs_the_setup_token_when_one_is_set() {
    let h = Harness::with_setup_token("s3cret-setup").await;
    let mode = h.call("GET", "/app/api/signup-mode").send().await;
    assert_eq!(mode.json["setupTokenRequired"], true);
    let signup = |token: Option<&str>| {
        h.call("POST", "/app/api/signup")
            .json(json!({ "username": "alice", "password": PASSWORD, "setupToken": token }))
            .send()
    };
    let without = signup(None).await;
    assert_eq!(
        (without.status, without.code()),
        (403, "setup_token_invalid")
    );
    let wrong = signup(Some("s3cret-setuq")).await;
    assert_eq!(wrong.code(), "setup_token_invalid");
    let right = signup(Some("s3cret-setup")).await;
    assert_eq!(right.status, 201, "{}", right.json);
    assert_eq!(right.json["account"]["role"], "admin");
    let mode = h.call("GET", "/app/api/signup-mode").send().await;
    assert_eq!(mode.json["setupTokenRequired"], false);
    let second = h
        .call("POST", "/app/api/signup")
        .json(json!({ "username": "bob", "password": PASSWORD }))
        .send()
        .await;
    assert_eq!(second.status, 201, "later sign-ups need no token");
}

#[tokio::test]
async fn sessions_are_stored_as_hashes() {
    let h = Harness::new().await;
    let cookie = h.signup("alice").await;
    let raw = cookie.split_once('=').unwrap().1;
    assert!(h.store.session(raw, 0).await.unwrap().is_none());
    let me = h.call("GET", "/app/api/me").cookie(&cookie).send().await;
    assert_eq!(me.status, 200);
}

#[tokio::test]
async fn expired_sessions_are_swept() {
    let h = Harness::new().await;
    let cookie = h.signup("alice").await;
    let id = account_id(&h, &cookie).await;
    let expired = lfm_store::WebSession {
        id: "old".into(),
        account: id,
        created_at: 0,
        expires_at: 1,
    };
    h.store.create_session(expired).await.unwrap();
    assert!(h.store.session("old", 0).await.unwrap().is_some());
    h.web.sweep().await;
    assert!(h.store.session("old", 0).await.unwrap().is_none());
    let me = h.call("GET", "/app/api/me").cookie(&cookie).send().await;
    assert_eq!(me.status, 200, "a live session stays");
}

#[tokio::test]
async fn an_approved_token_cannot_be_taken_over_by_another_account() {
    let h = Harness::new().await;
    let alice = h.signup("alice").await;
    h.call("PUT", "/app/api/admin/settings")
        .cookie(&alice)
        .json(json!({ "signupMode": "open" }))
        .send()
        .await;
    let bob = h.signup("bob").await;
    let (api_key, _) = key(&h, &alice, "read_write").await;
    let token = lfm_store::AuthToken {
        token: "t".repeat(32),
        api_key,
        account: None,
        expires_at: i64::MAX,
    };
    h.store.create_auth_token(token.clone(), 0).await.unwrap();
    let (alice, bob) = (account_id(&h, &alice).await, account_id(&h, &bob).await);
    h.store
        .authorize_auth_token(&token.token, alice, 1)
        .await
        .unwrap();
    // The store refuses even a racing approval that passed the handler's check.
    let raced = h.store.authorize_auth_token(&token.token, bob, 1).await;
    assert!(
        matches!(raced, Err(lfm_store::StoreError::NotFound)),
        "{raced:?}"
    );
    let found = h
        .store
        .find_auth_token(&token.token)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.account, Some(alice));
}

async fn account_id(h: &Harness, cookie: &str) -> lfm_store::AccountId {
    let me = h.call("GET", "/app/api/me").cookie(cookie).send().await;
    lfm_store::AccountId(me.json["account"]["id"].as_i64().unwrap())
}
