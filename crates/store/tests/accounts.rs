//! Accounts, sign-up modes, API keys, sessions, auth tokens, and jobs.

mod common;

use common::{TestDb, user};
use lfm_store::{
    AccountStatus, ApiSession, AuthToken, JobStatus, JobUpdate, KeyScope, NewApiKey, Role,
    SignupMode, Store, StoreError, WebSession,
};

fn key(n: u8) -> NewApiKey {
    NewApiKey {
        key: format!("{n:032x}"),
        secret: format!("{:032x}", u32::from(n) + 1000),
        label: format!("key {n}"),
        scope: KeyScope::Read,
    }
}

#[tokio::test]
async fn first_account_is_admin_even_when_closed() {
    let db = TestDb::new();
    db.store.set_signup_mode(SignupMode::Closed).await.unwrap();
    let first = db.store.create_account("Root", "hash", 1).await.unwrap();
    assert_eq!(
        (first.role, first.status),
        (Role::Admin, AccountStatus::Active)
    );
    let second = db.store.create_account("other", "hash", 2).await;
    assert!(matches!(second, Err(StoreError::SignupClosed)));
}

#[tokio::test]
async fn concurrent_first_sign_ups_make_one_admin() {
    let db = TestDb::new();
    let attempts: Vec<_> = (0..8)
        .map(|n| {
            let store: Store = db.store.clone();
            tokio::spawn(async move { store.create_account(&format!("user{n}"), "h", n).await })
        })
        .collect();
    let mut admins = 0;
    for attempt in attempts {
        let account = attempt.await.unwrap().unwrap();
        admins += usize::from(account.role == Role::Admin);
    }
    assert_eq!(admins, 1);
}

#[tokio::test]
async fn sign_up_modes() {
    let db = TestDb::new();
    assert_eq!(db.store.signup_mode().await.unwrap(), SignupMode::Approval);
    db.store.create_account("admin", "h", 0).await.unwrap();
    let pending = db.store.create_account("pending", "h", 1).await.unwrap();
    assert_eq!(
        (pending.role, pending.status),
        (Role::User, AccountStatus::Pending)
    );
    db.store.set_signup_mode(SignupMode::Open).await.unwrap();
    let open = db.store.create_account("open", "h", 2).await.unwrap();
    assert_eq!(open.status, AccountStatus::Active);
    let taken = db.store.create_account("OPEN", "h", 3).await;
    assert!(matches!(taken, Err(StoreError::NameTaken)));
    db.store.set_signup_mode(SignupMode::Closed).await.unwrap();
    assert!(matches!(
        db.store.create_account("x", "h", 4).await,
        Err(StoreError::SignupClosed)
    ));
    assert_eq!(
        db.store.setting("signup_mode").await.unwrap().as_deref(),
        Some("closed")
    );
}

#[tokio::test]
async fn admin_actions() {
    let db = TestDb::new();
    let admin = db.store.create_account("admin", "h", 0).await.unwrap().id;
    let a = db.store.create_account("a", "h", 1).await.unwrap().id;
    let b = db.store.create_account("b", "h", 2).await.unwrap().id;
    let pending = db
        .store
        .list_accounts(Some(AccountStatus::Pending))
        .await
        .unwrap();
    assert_eq!(pending.iter().map(|x| x.id).collect::<Vec<_>>(), [a, b]);

    db.store.approve_account(a).await.unwrap();
    assert!(matches!(
        db.store.approve_account(a).await,
        Err(StoreError::NotFound)
    ));
    db.store.reject_account(b).await.unwrap();
    assert_eq!(db.store.account(b).await.unwrap(), None);
    assert!(matches!(
        db.store.reject_account(a).await,
        Err(StoreError::NotFound)
    ));

    db.store.disable_account(a).await.unwrap();
    assert_eq!(
        db.store.account(a).await.unwrap().unwrap().status,
        AccountStatus::Disabled
    );
    db.store.enable_account(a).await.unwrap();
    assert!(matches!(
        db.store.disable_account(admin).await,
        Err(StoreError::LastAdmin)
    ));
    assert!(matches!(
        db.store.set_role(admin, Role::User).await,
        Err(StoreError::LastAdmin)
    ));
    db.store.set_role(a, Role::Admin).await.unwrap();
    db.store.set_role(admin, Role::User).await.unwrap();
    assert_eq!(db.store.list_accounts(None).await.unwrap().len(), 2);

    db.store.link_lastfm(a, Some("lfm".into())).await.unwrap();
    let creds = lfm_store::LastfmCredentials {
        api_key: Some(vec![1, 2]),
        secret: Some(vec![3]),
        session_key: None,
    };
    db.store
        .set_lastfm_credentials(a, creds.clone())
        .await
        .unwrap();
    db.store.set_password_hash(a, "new").await.unwrap();
    let account = db.store.account_by_name("A").await.unwrap().unwrap();
    assert_eq!(account.lastfm_user.as_deref(), Some("lfm"));
    assert_eq!(
        (account.credentials, account.password_hash.as_str()),
        (creds, "new")
    );
}

#[tokio::test]
async fn api_keys_follow_account_status() {
    let db = TestDb::new();
    let admin = db.store.create_account("admin", "h", 0).await.unwrap().id;
    let user = db.store.create_account("user", "h", 0).await.unwrap().id;
    let refused = db.store.create_api_key(user, key(1), 5).await;
    assert!(matches!(refused, Err(StoreError::AccountNotActive)));

    db.store.approve_account(user).await.unwrap();
    db.store.create_api_key(user, key(1), 5).await.unwrap();
    db.store.create_api_key(user, key(2), 6).await.unwrap();
    db.store.create_api_key(admin, key(3), 7).await.unwrap();
    assert_eq!(db.store.active_api_keys().await.unwrap().len(), 3);

    db.store.disable_account(user).await.unwrap();
    let active = db.store.active_api_keys().await.unwrap();
    assert_eq!(
        active.iter().map(|k| k.key.clone()).collect::<Vec<_>>(),
        [key(3).key]
    );

    db.store.revoke_api_key(user, &key(2).key).await.unwrap();
    assert!(matches!(
        db.store.revoke_api_key(user, &key(2).key).await,
        Err(StoreError::NotFound)
    ));
    let left = db.store.api_keys(user).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left.first().unwrap().secret, key(1).secret);
}

#[tokio::test]
async fn key_usage_is_written_in_batches() {
    let db = TestDb::new();
    let admin = db.store.create_account("admin", "h", 0).await.unwrap().id;
    db.store.create_api_key(admin, key(1), 5).await.unwrap();
    db.store.mark_key_used(&key(1).key, 100);
    db.store.mark_key_used(&key(1).key, 120);
    let before = db.store.api_keys(admin).await.unwrap();
    assert_eq!(
        before.first().unwrap().last_used,
        None,
        "nothing written before the flush"
    );
    db.store.flush_key_usage().await.unwrap();
    db.store.flush_key_usage().await.unwrap();
    let after = db.store.api_keys(admin).await.unwrap();
    assert_eq!(after.first().unwrap().last_used, Some(120));
}

#[tokio::test]
async fn web_sessions_expire() {
    let db = TestDb::new();
    let admin = db.store.create_account("admin", "h", 0).await.unwrap().id;
    let session = WebSession {
        id: "s1".into(),
        account: admin,
        created_at: 10,
        expires_at: 20,
    };
    db.store.create_session(session.clone()).await.unwrap();
    assert_eq!(db.store.session("s1", 19).await.unwrap(), Some(session));
    assert_eq!(db.store.session("s1", 20).await.unwrap(), None);
    db.store.purge_expired_sessions(30).await.unwrap();
    db.store.delete_session("s1").await.unwrap();
    assert_eq!(db.store.session("s1", 0).await.unwrap(), None);
}

#[tokio::test]
async fn auth_token_trades_for_a_session_key_once() {
    let db = TestDb::new();
    let admin = db.store.create_account("admin", "h", 0).await.unwrap().id;
    let token = AuthToken {
        token: "t".into(),
        api_key: "k".into(),
        account: None,
        expires_at: 100,
    };
    db.store.create_auth_token(token.clone(), 0).await.unwrap();
    assert_eq!(
        db.store.exchange_auth_token("t", "sk", 10).await.unwrap(),
        None,
        "not approved"
    );
    assert!(matches!(
        db.store.authorize_auth_token("t", admin, 100).await,
        Err(StoreError::NotFound)
    ));
    db.store.authorize_auth_token("t", admin, 10).await.unwrap();
    assert_eq!(
        db.store.auth_token("t", 10).await.unwrap().unwrap().account,
        Some(admin)
    );
    let session = db
        .store
        .exchange_auth_token("t", "sk", 11)
        .await
        .unwrap()
        .unwrap();
    let want = ApiSession {
        session_key: "sk".into(),
        account: admin,
        api_key: "k".into(),
        created_at: 11,
    };
    assert_eq!(session, want);
    assert_eq!(
        db.store.exchange_auth_token("t", "sk2", 12).await.unwrap(),
        None,
        "used up"
    );
    assert_eq!(db.store.api_session("sk").await.unwrap(), Some(want));
    let mobile = ApiSession {
        session_key: "m".into(),
        account: admin,
        api_key: "k".into(),
        created_at: 1,
    };
    db.store.create_api_session(mobile.clone()).await.unwrap();
    assert_eq!(db.store.api_session("m").await.unwrap(), Some(mobile));
}

#[tokio::test]
async fn jobs_resume_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jobs.db");
    let (job, alice) = {
        let store = Store::open(&path).unwrap();
        let alice = user(&store, "alice").await;
        let job = store
            .create_job("import", Some(alice), "{}", 1)
            .await
            .unwrap();
        let update = JobUpdate {
            status: JobStatus::Running,
            cursor: r#"{"page":7}"#.into(),
            error: None,
        };
        store.update_job(job, update, 2).await.unwrap();
        store.create_job("prewarm", None, "{}", 3).await.unwrap();
        (job, alice)
    };
    let store = Store::open(&path).unwrap();
    let unfinished = store.unfinished_jobs().await.unwrap();
    assert_eq!(unfinished.len(), 2);
    let first = unfinished.first().unwrap();
    assert_eq!((first.id, first.cursor.as_str()), (job, r#"{"page":7}"#));
    let failed = JobUpdate {
        status: JobStatus::Failed,
        cursor: "{}".into(),
        error: Some("x".into()),
    };
    store.update_job(job, failed, 4).await.unwrap();
    let mine = store.user_jobs(alice).await.unwrap();
    assert_eq!(mine.first().unwrap().error.as_deref(), Some("x"));
    assert_eq!(store.unfinished_jobs().await.unwrap().len(), 1);
    assert_eq!(store.job(job).await.unwrap().unwrap().updated_at, 4);
}
