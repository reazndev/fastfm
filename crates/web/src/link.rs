//! An account's Last.fm credentials: sealed in the store, opened only to sign calls.
//!
//! A Last.fm session key belongs to the application that obtained it, so changing the
//! account's own key drops the session key and the account authorises again.

use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

use lfm_store::{Account, AccountId, AccountStatus, LastfmCredentials, Store, fold};
use lfm_upstream::{ApiKey, AppCredentials, Secret, WriteCredentials};

use crate::config::WebConfig;
use crate::crypto::{Vault, VaultError};
use crate::error::WebError;
use crate::state::AppState;

/// An account's Last.fm credentials in the clear.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Linked {
    /// The account's own application, if it brought one.
    pub(crate) own_app: Option<AppCredentials>,
    /// The session key from Last.fm web auth, issued to [`Linked::app`]. Its presence also
    /// means Last.fm confirmed the account owns the linked user.
    pub(crate) session_key: Option<Secret>,
    /// Whether stored credentials could not be opened, as after losing the secrets key file.
    /// They count as absent, and saving new ones overwrites them.
    pub(crate) unreadable: bool,
}

/// Accounts already warned about unreadable credentials, so the log says it once.
static WARNED: Mutex<BTreeSet<i64>> = Mutex::new(BTreeSet::new());

impl Linked {
    /// Opens the account's sealed credentials. A value that does not open counts as absent.
    pub(crate) fn open(vault: &Vault, account: &Account) -> Self {
        let creds = &account.credentials;
        let mut unreadable = false;
        let mut open = |field: &str, sealed: Option<&Vec<u8>>| {
            let opened = sealed.map(|sealed| vault.open(&context(account.id, field), sealed));
            match opened {
                Some(Ok(plain)) => Some(plain),
                Some(Err(_)) => {
                    unreadable = true;
                    None
                }
                None => None,
            }
        };
        let api_key = open("own_key", creds.api_key.as_ref());
        let secret = open("own_secret", creds.secret.as_ref());
        let session_key = open("lastfm_sk", creds.session_key.as_ref()).map(Secret::new);
        let own_app = api_key.zip(secret).map(|(key, secret)| AppCredentials {
            api_key: ApiKey::new(key),
            secret: Secret::new(secret),
        });
        if unreadable
            && WARNED
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(account.id.0)
        {
            tracing::warn!(
                account = account.id.0,
                "stored Last.fm credentials do not open with the secrets key; ignoring them"
            );
        }
        Self {
            own_app,
            session_key,
            unreadable,
        }
    }

    /// Seals the credentials for the account's row.
    pub(crate) fn seal(
        &self,
        vault: &Vault,
        account: AccountId,
    ) -> Result<LastfmCredentials, VaultError> {
        let seal = |field: &str, plain: Option<&str>| {
            plain
                .map(|plain| vault.seal(&context(account, field), plain))
                .transpose()
        };
        let app = self.own_app.as_ref();
        Ok(LastfmCredentials {
            api_key: seal("own_key", app.map(|a| a.api_key.expose()))?,
            secret: seal("own_secret", app.map(|a| a.secret.expose()))?,
            session_key: seal("lastfm_sk", self.session_key.as_ref().map(Secret::expose))?,
        })
    }

    /// The application that signs this account's calls: its own, else the instance's.
    pub(crate) fn app(&self, config: &WebConfig) -> Option<AppCredentials> {
        self.own_app.clone().or_else(|| config.lastfm_app.clone())
    }

    /// What forwarding a write needs, when the account has authorised Last.fm.
    pub(crate) fn write_credentials(&self, config: &WebConfig) -> Option<WriteCredentials> {
        Some(WriteCredentials {
            app: self.app(config)?,
            session_key: self.session_key.clone()?,
        })
    }
}

/// Reads, changes and writes back an account's credentials.
pub(crate) async fn update(
    store: &Store,
    vault: &Vault,
    account: &Account,
    change: impl FnOnce(&mut Linked),
) -> Result<(), WebError> {
    let mut linked = Linked::open(vault, account);
    linked.unreadable = false;
    change(&mut linked);
    let sealed = linked.seal(vault, account.id)?;
    store.set_lastfm_credentials(account.id, sealed).await?;
    Ok(())
}

/// The Last.fm users that active accounts link, with each account's own key registered.
pub(crate) async fn linked_users(state: &AppState) -> Result<Vec<String>, WebError> {
    let accounts = state
        .store
        .list_accounts(Some(AccountStatus::Active))
        .await?;
    Ok(accounts
        .iter()
        .filter_map(|account| Some((account, account.lastfm_user.clone()?)))
        .map(|(account, name)| {
            let own_key = Linked::open(&state.vault, account)
                .own_app
                .map(|app| app.api_key);
            state.sync.use_key(&name, own_key);
            name
        })
        .collect())
}

/// The account other than `account` that links `lastfm_user`, if any.
async fn claimant(
    store: &Store,
    account: AccountId,
    lastfm_user: &str,
) -> Result<Option<Account>, WebError> {
    let wanted = fold(lastfm_user);
    Ok(store.list_accounts(None).await?.into_iter().find(|other| {
        other.id != account
            && other
                .lastfm_user
                .as_deref()
                .is_some_and(|name| fold(name) == wanted)
    }))
}

/// Fails when another account already links this Last.fm user. The database enforces the
/// same rule; checking first keeps a refused link from following the user.
pub(crate) async fn ensure_unclaimed(
    store: &Store,
    account: AccountId,
    lastfm_user: &str,
) -> Result<(), WebError> {
    match claimant(store, account, lastfm_user).await? {
        Some(_) => Err(taken()),
        None => Ok(()),
    }
}

/// Unlinks `lastfm_user` from another account that claimed it without Last.fm confirming
/// ownership, after Last.fm confirmed `account` owns it. A confirmed claim stays.
pub(crate) async fn release_unconfirmed(
    state: &AppState,
    account: AccountId,
    lastfm_user: &str,
) -> Result<(), WebError> {
    let Some(other) = claimant(&state.store, account, lastfm_user).await? else {
        return Ok(());
    };
    if Linked::open(&state.vault, &other).session_key.is_some() {
        return Err(taken());
    }
    tracing::info!(
        account = account.0,
        from = other.id.0,
        "Last.fm confirmed the owner; taking over an unconfirmed link"
    );
    state.store.link_lastfm(other.id, None).await?;
    Ok(())
}

/// The answer when another account links the Last.fm user.
pub(crate) fn taken() -> WebError {
    WebError::conflict(
        "lastfm_user_taken",
        "Another account already links this Last.fm user.",
    )
}

/// The associated data that binds a sealed value to its account and column.
fn context(account: AccountId, field: &str) -> String {
    format!("account:{}:{field}", account.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_and_open_round_trip() {
        let vault = Vault::from_key([1; 32]);
        let linked = Linked {
            own_app: Some(AppCredentials {
                api_key: ApiKey::new("key"),
                secret: Secret::new("secret"),
            }),
            session_key: Some(Secret::new("sk")),
            unreadable: false,
        };
        let sealed = linked.seal(&vault, AccountId(4)).unwrap();
        let account = Account {
            id: AccountId(4),
            name: "a".into(),
            password_hash: String::new(),
            role: lfm_store::Role::User,
            status: lfm_store::AccountStatus::Active,
            lastfm_user: None,
            credentials: sealed.clone(),
            created_at: 0,
        };
        assert_eq!(Linked::open(&vault, &account), linked);
        let elsewhere = Account {
            id: AccountId(5),
            ..account
        };
        let lost = Linked::open(&vault, &elsewhere);
        assert!(lost.unreadable && lost.own_app.is_none() && lost.session_key.is_none());
        assert!(!format!("{linked:?}").contains("secret\""));
    }
}
