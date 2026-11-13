//! JSON shapes the web API returns. Field names are camelCase for the Angular UI.

use lfm_store::{Account, ApiKey, KeyScope};
use serde::{Deserialize, Serialize};

/// An account as the UI sees it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountView {
    id: i64,
    username: String,
    role: &'static str,
    status: &'static str,
    lastfm_user: Option<String>,
    created_at: i64,
}

impl From<&Account> for AccountView {
    fn from(account: &Account) -> Self {
        Self {
            id: account.id.0,
            username: account.name.clone(),
            role: account.role.as_str(),
            status: account.status.as_str(),
            lastfm_user: account.lastfm_user.clone(),
            created_at: account.created_at,
        }
    }
}

/// `{"account": …}`.
#[derive(Debug, Serialize)]
pub(crate) struct AccountBody {
    pub(crate) account: AccountView,
}

impl AccountBody {
    pub(crate) fn of(account: &Account) -> Self {
        Self {
            account: account.into(),
        }
    }
}

/// An API key with its secret, which clients need to sign calls.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KeyView {
    key: String,
    secret: String,
    label: String,
    scope: Scope,
    created_at: i64,
    last_used: Option<i64>,
}

impl From<ApiKey> for KeyView {
    fn from(key: ApiKey) -> Self {
        Self {
            key: key.key,
            secret: key.secret,
            label: key.label,
            scope: key.scope.into(),
            created_at: key.created_at,
            last_used: key.last_used,
        }
    }
}

/// A key's scope as the UI names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Scope {
    /// Read methods only.
    Read,
    /// Reads and writes.
    ReadWrite,
}

impl From<KeyScope> for Scope {
    fn from(scope: KeyScope) -> Self {
        match scope {
            KeyScope::Read => Self::Read,
            KeyScope::Write => Self::ReadWrite,
        }
    }
}

impl From<Scope> for KeyScope {
    fn from(scope: Scope) -> Self {
        match scope {
            Scope::Read => Self::Read,
            Scope::ReadWrite => Self::Write,
        }
    }
}
