//! The account's Last.fm link: which user it mirrors, the import, and write forwarding.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use lfm_store::{Account, OutboxSummary};
use lfm_sync::{ImportProgress, PRIVATE_MESSAGE};
use lfm_upstream::{ApiKey, AppCredentials, Secret};
use serde::{Deserialize, Serialize};

use crate::error::WebError;
use crate::link::{self, Linked};
use crate::session::Member;
use crate::state::AppState;

/// Where the account's link stands.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LinkStatus {
    /// The linked Last.fm username.
    username: Option<String>,
    /// Whether Last.fm web auth confirmed the user, which forwarding writes needs.
    authorized: bool,
    /// Whether the account uses its own Last.fm API key.
    own_key: bool,
    /// The instance's key rule and the state of the saved credentials.
    credentials: CredentialsView,
    /// The newest import.
    import: Option<ImportView>,
    /// Live sync: everything before `cursor` (Unix time) is mirrored.
    sync: Option<SyncView>,
    /// What happens to writes from clients.
    writes: WritesView,
}

/// `{"ownKeyRequired": bool, "error": "…" | null}`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CredentialsView {
    /// Whether this instance requires an own key before linking.
    own_key_required: bool,
    /// Set when saved credentials could not be read and must be entered again.
    error: Option<&'static str>,
}

/// Where an import stands, as the UI shows it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportView {
    pub(crate) status: &'static str,
    pub(crate) imported: u64,
    pub(crate) estimated_total: u64,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncView {
    cursor: i64,
    scrobbles: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WritesView {
    /// False: writes are applied here only, because Last.fm web auth was not done.
    forwarding: bool,
    queued: u64,
    failed: u64,
    held_locally: u64,
    last_error: Option<String>,
}

/// `GET /lastfm`.
pub(crate) async fn status(
    State(state): State<AppState>,
    Member(account): Member,
) -> Result<Json<LinkStatus>, WebError> {
    link_status(&state, &account).await.map(Json)
}

/// The status of `account`, read fresh.
pub(crate) async fn link_status(
    state: &AppState,
    account: &Account,
) -> Result<LinkStatus, WebError> {
    let linked = Linked::open(&state.vault, account);
    let user = match &account.lastfm_user {
        Some(name) => state.store.user_by_name(name).await?,
        None => None,
    };
    let import = match &user {
        Some(user) => state
            .sync
            .progress(user.id)
            .await?
            .map(|progress| import_view(progress, state.sync.is_private(user.id))),
        None => None,
    };
    let outbox: OutboxSummary = state.store.outbox_summary(account.id).await?;
    Ok(LinkStatus {
        username: account.lastfm_user.clone(),
        authorized: linked.session_key.is_some(),
        own_key: linked.own_app.is_some(),
        credentials: CredentialsView {
            own_key_required: own_key_required(&state.store).await?,
            error: linked
                .unreadable
                .then_some("Saved Last.fm credentials could not be read; enter them again."),
        },
        import,
        sync: user.map(|user| SyncView {
            cursor: user.sync_cursor,
            scrobbles: user.stats.playcount,
        }),
        writes: WritesView {
            forwarding: linked.write_credentials(&state.config).is_some(),
            queued: outbox.queued,
            failed: outbox.failed,
            held_locally: outbox.held,
            last_error: outbox.last_error,
        },
    })
}

/// The import as the UI shows it. `private` is live sync's latest verdict, which also covers a
/// profile that hid its history after the import finished.
pub(crate) fn import_view(progress: ImportProgress, private: bool) -> ImportView {
    if private || progress.is_private() {
        return ImportView {
            status: "private",
            imported: progress.imported,
            estimated_total: progress.estimated_total,
            error: Some(PRIVATE_MESSAGE.to_owned()),
        };
    }
    ImportView {
        status: progress.status.as_str(),
        imported: progress.imported,
        estimated_total: progress.estimated_total,
        error: progress.error,
    }
}

/// `{"username": "…"}`.
#[derive(Debug, Deserialize)]
pub(crate) struct LinkBody {
    username: String,
}

/// `PUT /lastfm`: links a Last.fm user and starts mirroring them; the import runs in the
/// background. Changing the user drops the Last.fm session key, which belonged to the old one.
pub(crate) async fn link(
    State(state): State<AppState>,
    Member(account): Member,
    Json(body): Json<LinkBody>,
) -> Result<Json<LinkStatus>, WebError> {
    let name = lastfm_username(&body.username)?;
    let account = relink(&state, account, name).await?;
    link_status(&state, &account).await.map(Json)
}

/// The trimmed Last.fm username from a request body.
pub(crate) fn lastfm_username(raw: &str) -> Result<&str, WebError> {
    let name = raw.trim();
    if name.is_empty() || name.len() > 64 {
        return Err(WebError::invalid(
            "invalid_username",
            "Enter a Last.fm username.",
        ));
    }
    Ok(name)
}

/// Points `account` at `name`: follows the new user, stops the old one, and returns the
/// account as stored afterwards.
pub(crate) async fn relink(
    state: &AppState,
    account: Account,
    name: &str,
) -> Result<Account, WebError> {
    let own_key = Linked::open(&state.vault, &account)
        .own_app
        .map(|app| app.api_key);
    if own_key.is_none() && own_key_required(&state.store).await? {
        return Err(WebError::conflict(
            "own_key_required",
            "Add your own Last.fm API key in settings first.",
        ));
    }
    link::ensure_unclaimed(&state.store, account.id, name).await?;
    state.sync.use_key(name, own_key);
    let user = state.sync.follow(name).await?;
    let canonical = state
        .store
        .user(user)
        .await?
        .map_or_else(|| name.to_owned(), |u| u.name);
    let same = account
        .lastfm_user
        .as_deref()
        .is_some_and(|old| lfm_store::fold(old) == lfm_store::fold(&canonical));
    if !same {
        stop_following(state, &account).await?;
        link::update(&state.store, &state.vault, &account, |l| {
            l.session_key = None;
        })
        .await?;
        state
            .store
            .link_lastfm(account.id, Some(canonical))
            .await
            .map_err(|error| match error {
                lfm_store::StoreError::NameTaken => link::taken(),
                other => other.into(),
            })?;
    }
    state
        .store
        .account(account.id)
        .await?
        .ok_or_else(WebError::not_found)
}

/// `DELETE /lastfm`: unlinks, stops the sync, and forgets the session key. Mirrored data stays.
pub(crate) async fn unlink(
    State(state): State<AppState>,
    Member(account): Member,
) -> Result<StatusCode, WebError> {
    stop_following(&state, &account).await?;
    link::update(&state.store, &state.vault, &account, |l| {
        l.session_key = None;
    })
    .await?;
    state.store.link_lastfm(account.id, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn stop_following(state: &AppState, account: &Account) -> Result<(), WebError> {
    if let Some(name) = &account.lastfm_user
        && let Some(user) = state.store.user_by_name(name).await?
    {
        state.sync.unfollow(user.id).await;
        state.sync.use_key(name, None);
    }
    Ok(())
}

/// `POST /lastfm/import/retry`: tries the import or live poll again now, after the owner made
/// a private profile public.
pub(crate) async fn retry(
    State(state): State<AppState>,
    Member(account): Member,
) -> Result<Json<LinkStatus>, WebError> {
    let name = account
        .lastfm_user
        .as_deref()
        .ok_or_else(|| WebError::invalid("not_linked", "Link a Last.fm user first."))?;
    let user = state.sync.follow(name).await?;
    state.sync.retry(user);
    link_status(&state, &account).await.map(Json)
}

/// The setting key of the own-key rule.
pub(crate) const OWN_KEY_REQUIRED: &str = "require_own_key";

/// Whether accounts must save their own Last.fm API key before linking a user.
pub(crate) async fn own_key_required(store: &lfm_store::Store) -> Result<bool, WebError> {
    Ok(store.setting(OWN_KEY_REQUIRED).await?.as_deref() == Some("true"))
}

/// `{"apiKey": "…", "secret": "…"}`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OwnKeyBody {
    api_key: String,
    secret: String,
}

/// `PUT /lastfm/key`: the account's own Last.fm application, checked with one signed call.
pub(crate) async fn set_own_key(
    State(state): State<AppState>,
    Member(account): Member,
    Json(body): Json<OwnKeyBody>,
) -> Result<StatusCode, WebError> {
    let (key, secret) = (body.api_key.trim(), body.secret.trim());
    let valid =
        |s: &str| (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric());
    if !valid(key) || !valid(secret) {
        return Err(WebError::invalid(
            "invalid_key",
            "Enter the API key and shared secret.",
        ));
    }
    let app = AppCredentials {
        api_key: ApiKey::new(key),
        secret: Secret::new(secret),
    };
    state
        .upstream
        .check_app(&app)
        .await
        .map_err(|e| WebError::lastfm(&e))?;
    let api_key = app.api_key.clone();
    link::update(&state.store, &state.vault, &account, |l| {
        l.own_app = Some(app);
        l.session_key = None;
    })
    .await?;
    if let Some(name) = &account.lastfm_user {
        state.sync.use_key(name, Some(api_key));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /lastfm/key`: back to the instance's key. Drops the session key too.
pub(crate) async fn clear_own_key(
    State(state): State<AppState>,
    Member(account): Member,
) -> Result<StatusCode, WebError> {
    link::update(&state.store, &state.vault, &account, |l| {
        *l = Linked::default();
    })
    .await?;
    if let Some(name) = &account.lastfm_user {
        state.sync.use_key(name, None);
    }
    Ok(StatusCode::NO_CONTENT)
}
