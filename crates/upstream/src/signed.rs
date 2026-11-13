//! Signed calls: shared secrets, the credentials of a Last.fm application, and `api_sig`.

use std::fmt;

use lfm_core::{Params, api_signature};

use crate::config::ApiKey;
use crate::request::RawRequest;

/// A shared secret or session key. Its `Debug` output is redacted so it never reaches a log.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wraps a secret string.
    #[must_use]
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    /// The secret itself, for signing or storing.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// The `api_sig` of these wire parameters: MD5 of the sorted pairs plus this secret.
    pub(crate) fn sign(&self, params: &[(String, String)]) -> String {
        let params = params.iter().fold(Params::new(), |params, (name, value)| {
            params.with(name.as_str(), value.as_str())
        });
        api_signature(&params, &self.0)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(REDACTED)")
    }
}

/// A Last.fm API application: its key and the shared secret that signs its calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppCredentials {
    /// The application's API key.
    pub api_key: ApiKey,
    /// The application's shared secret.
    pub secret: Secret,
}

/// What a write on behalf of a user needs: the application and the user's session key, which
/// Last.fm issued to that same application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteCredentials {
    /// The application that obtained the session key.
    pub app: AppCredentials,
    /// The user's Last.fm session key.
    pub session_key: Secret,
}

impl RawRequest {
    /// Signs the request with the application's key and secret. `api_sig` is computed when the
    /// request is sent, over every parameter but `format`.
    #[must_use]
    pub fn signed(self, app: &AppCredentials) -> Self {
        Self {
            secret: Some(app.secret.clone()),
            ..self.api_key(app.api_key.clone())
        }
    }

    /// Sends the request once, with no retries. For writes: a retry after a lost answer could
    /// apply the write twice, and a waiting client should not sit through the backoff.
    #[must_use]
    pub fn single_attempt(self) -> Self {
        Self {
            single_attempt: true,
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_is_redacted() {
        let secret = Secret::new("hunter2");
        assert_eq!(format!("{secret:?}"), "Secret(REDACTED)");
        let app = AppCredentials {
            api_key: ApiKey::new("k"),
            secret,
        };
        assert!(!format!("{app:?}").contains("hunter2"));
    }

    #[test]
    fn signs_like_last_fm() {
        let params = vec![
            ("method".to_owned(), "auth.getSession".to_owned()),
            ("token".to_owned(), "t".to_owned()),
            ("api_key".to_owned(), "k".to_owned()),
            ("format".to_owned(), "json".to_owned()),
        ];
        // md5("api_keykmethodauth.getSessiontokents")
        let expected = lfm_core::api_signature(
            &Params::new()
                .with("api_key", "k")
                .with("method", "auth.getSession")
                .with("token", "t"),
            "s",
        );
        assert_eq!(Secret::new("s").sign(&params), expected);
    }
}
