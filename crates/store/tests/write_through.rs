//! The outbox, settling locally written scrobbles, and what revoking a key ends.
#![allow(clippy::unwrap_used, reason = "tests")]

use lfm_store::{
    AccountId, ApiSession, AuthToken, KeyScope, Named, NewApiKey, NewOutboxEntry, OutboxStatus,
    Page, Scrobble, Store, TimeRange, UserId,
};

fn scrobble(uts: i64, artist: &str, track: &str) -> Scrobble {
    Scrobble {
        uts,
        artist: Named::new(artist),
        album: None,
        track: Named::new(track),
        image: None,
    }
}

async fn names(store: &Store, user: UserId) -> Vec<(i64, String)> {
    let page = store
        .recent_tracks(user, TimeRange::ALL, Page::new(1, 50))
        .await
        .unwrap();
    page.items
        .iter()
        .map(|t| (t.uts, t.track.name.clone()))
        .collect()
}

async fn playcount(store: &Store, user: UserId) -> u64 {
    store.user(user).await.unwrap().unwrap().stats.playcount
}

#[tokio::test]
async fn settling_removes_ignored_replaces_corrected_and_never_doubles() {
    let store = Store::open_in_memory().unwrap();
    let user = store.ensure_user("someone").await.unwrap();
    let sent = vec![
        scrobble(100, "Cher", "Believe"),
        scrobble(200, "cher", "belive"),
        scrobble(300, "Nobody", "Nothing"),
    ];
    store.append_scrobbles(user, sent.clone()).await.unwrap();
    let version = store.user(user).await.unwrap().unwrap().version;

    // Last.fm accepted the first, corrected the second and ignored the third.
    let remove = vec![sent[1].clone(), sent[2].clone()];
    let keep = vec![sent[0].clone(), scrobble(200, "Cher", "Believe")];
    store
        .settle_scrobbles(user, remove, keep.clone())
        .await
        .unwrap();
    assert_eq!(
        names(&store, user).await,
        vec![(200, "Believe".to_owned()), (100, "Believe".to_owned())]
    );
    assert_eq!(playcount(&store, user).await, 2);
    assert!(store.user(user).await.unwrap().unwrap().version > version);

    // Settling again changes nothing, and a scrobble a poll dropped comes back once.
    store
        .settle_scrobbles(user, vec![], keep.clone())
        .await
        .unwrap();
    assert_eq!(playcount(&store, user).await, 2);
    store
        .replace_window(user, TimeRange::new(0, 1000), vec![])
        .await
        .unwrap();
    store.settle_scrobbles(user, vec![], keep).await.unwrap();
    assert_eq!(playcount(&store, user).await, 2);
}

#[tokio::test]
async fn settling_keeps_real_same_second_duplicates() {
    let store = Store::open_in_memory().unwrap();
    let user = store.ensure_user("someone").await.unwrap();
    let twice = vec![scrobble(100, "A", "a"), scrobble(100, "A", "a")];
    store
        .append_scrobbles(user, twice[..1].to_vec())
        .await
        .unwrap();
    store.settle_scrobbles(user, vec![], twice).await.unwrap();
    assert_eq!(playcount(&store, user).await, 2);
}

async fn account(store: &Store) -> AccountId {
    store.create_account("owner", "hash", 0).await.unwrap().id
}

fn entry(account: AccountId, status: OutboxStatus) -> NewOutboxEntry {
    NewOutboxEntry {
        account,
        method: "track.love".into(),
        payload: "{}".into(),
        status,
        next_at: 10,
        error: None,
    }
}

#[tokio::test]
async fn outbox_retries_fails_and_summarises() {
    let store = Store::open_in_memory().unwrap();
    let owner = account(&store).await;
    store
        .enqueue_write(entry(owner, OutboxStatus::Queued), 0)
        .await
        .unwrap();
    store
        .enqueue_write(entry(owner, OutboxStatus::Held), 0)
        .await
        .unwrap();
    assert_eq!(store.due_writes(9, 10).await.unwrap().len(), 0);
    let due = store.due_writes(10, 10).await.unwrap();
    assert_eq!((due.len(), due[0].attempts), (1, 0));

    store.retry_write(due[0].id, 50, "HTTP 503").await.unwrap();
    assert_eq!(store.due_writes(49, 10).await.unwrap().len(), 0);
    assert_eq!(store.due_writes(50, 10).await.unwrap()[0].attempts, 1);
    let summary = store.outbox_summary(owner).await.unwrap();
    assert_eq!((summary.queued, summary.held, summary.failed), (1, 1, 0));
    assert_eq!(summary.last_error.as_deref(), Some("HTTP 503"));

    store
        .fail_write(due[0].id, "Invalid session key")
        .await
        .unwrap();
    let summary = store.outbox_summary(owner).await.unwrap();
    assert_eq!((summary.queued, summary.failed), (0, 1));
    store.finish_write(due[0].id).await.unwrap();
    assert_eq!(store.outbox_summary(owner).await.unwrap().failed, 0);
}

#[tokio::test]
async fn outbox_keeps_a_bounded_number_of_rows() {
    let store = Store::open_in_memory().unwrap();
    let owner = account(&store).await;
    for _ in 0..1005 {
        store
            .enqueue_write(entry(owner, OutboxStatus::Held), 0)
            .await
            .unwrap();
    }
    assert_eq!(store.outbox_summary(owner).await.unwrap().held, 1000);
}

#[tokio::test]
async fn revoking_a_key_ends_its_sessions_and_tokens() {
    let store = Store::open_in_memory().unwrap();
    let owner = account(&store).await;
    let key = NewApiKey {
        key: "k".repeat(32),
        secret: "s".repeat(32),
        label: "app".into(),
        scope: KeyScope::Write,
    };
    store.create_api_key(owner, key.clone(), 0).await.unwrap();
    let session = ApiSession {
        session_key: "sk".into(),
        account: owner,
        api_key: key.key.clone(),
        created_at: 0,
    };
    store.create_api_session(session).await.unwrap();
    let token = AuthToken {
        token: "t".into(),
        api_key: key.key.clone(),
        account: None,
        expires_at: 5,
    };
    store.create_auth_token(token.clone(), 0).await.unwrap();
    // An expired token is still found, so auth.getSession can answer 15 rather than 4.
    assert_eq!(store.find_auth_token("t").await.unwrap(), Some(token));
    assert!(store.auth_token("t", 10).await.unwrap().is_none());

    store.revoke_api_key(owner, &key.key).await.unwrap();
    assert!(store.api_session("sk").await.unwrap().is_none());
    assert!(store.find_auth_token("t").await.unwrap().is_none());
}
