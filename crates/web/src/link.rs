//! An account's Last.fm credentials: sealed in the store, opened only to sign calls.
//!
//! A Last.fm session key belongs to the application that obtained it, so changing the
//! account's own key drops the session key and the account authorises again.

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
    /// The session key from Last.fm web auth, issued to [`Linked::app`].
    pub(crate) session_key: Option<Secret>,
}

impl Linked {
    /// Opens the account's sealed credentials.
    pub(crate) fn open(vault: &Vault, account: &Account) -> Result<Self, VaultError> {
        let creds = &account.credentials;
        let open = |field: &str, sealed: Option<&Vec<u8>>| {
            sealed
                .map(|sealed| vault.open(&context(account.id, field), sealed))
                .transpose()
        };
        let api_key = open("own_key", creds.api_key.as_ref())?;
        let secret = open("own_secret", creds.secret.as_ref())?;
        let own_app = api_key.zip(secret).map(|(key, secret)| AppCredentials {
            api_key: ApiKey::new(key),
            secret: Secret::new(secret),
        });
        Ok(Self {
            own_app,
            session_key: open("lastfm_sk", creds.session_key.as_ref())?.map(Secret::new),
        })
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
    let mut linked = Linked::open(vault, account)?;
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
    accounts
        .iter()
        .filter_map(|account| Some((account, account.lastfm_user.clone()?)))
        .map(|(account, name)| {
            let own_key = Linked::open(&state.vault, account)?
                .own_app
                .map(|app| app.api_key);
            state.sync.use_key(&name, own_key);
            Ok(name)
        })
        .collect()
}

/// Fails when another account already links this Last.fm user.
pub(crate) async fn ensure_unclaimed(
    store: &Store,
    account: AccountId,
    lastfm_user: &str,
) -> Result<(), WebError> {
    let wanted = fold(lastfm_user);
    let taken = store.list_accounts(None).await?.iter().any(|other| {
        other.id != account
            && other
                .lastfm_user
                .as_deref()
                .is_some_and(|name| fold(name) == wanted)
    });
    if taken {
        Err(WebError::conflict(
            "lastfm_user_taken",
            "Another account already links this Last.fm user.",
        ))
    } else {
        Ok(())
    }
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
        assert_eq!(Linked::open(&vault, &account).unwrap(), linked);
        let elsewhere = Account {
            id: AccountId(5),
            ..account
        };
        assert!(Linked::open(&vault, &elsewhere).is_err());
        assert!(!format!("{linked:?}").contains("secret\""));
    }
}
