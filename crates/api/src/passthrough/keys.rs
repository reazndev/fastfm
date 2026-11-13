//! Which Last.fm key a pass-through miss uses: the instance's key pool, or, on an instance
//! without one, the own Last.fm key of the account that owns the calling API key.

use async_trait::async_trait;
use lfm_upstream::ApiKey;

/// The own Last.fm keys of local accounts. The web crate, which holds the decrypted keys,
/// implements it: `lastfm_key` answers the key an account entered in its settings, if any.
#[async_trait]
pub trait CallerKeys: Send + Sync + std::fmt::Debug {
    /// The Last.fm API key of account `account_id`, or `None` when it has none.
    async fn lastfm_key(&self, account_id: i64) -> Option<ApiKey>;
}

/// Who asks for a forwarded answer.
#[derive(Debug, Clone, Copy)]
pub struct Caller<'a> {
    /// The mirror API key of the request, which the miss limit counts by.
    pub api_key: &'a str,
    /// The account that owns that key; 0 for keys registered in code.
    pub account_id: i64,
}

impl Caller<'_> {
    /// Whether the miss limit applies. Keys registered in code (the development key and the
    /// web UI's internal key, account 0) are the operator's own and are not limited.
    #[must_use]
    pub fn limited(&self) -> bool {
        self.account_id != 0
    }
}

#[cfg(test)]
mod tests {
    use super::Caller;

    #[test]
    fn only_keys_of_accounts_are_limited() {
        let caller = |account_id| Caller {
            api_key: "k",
            account_id,
        };
        assert!(caller(7).limited());
        assert!(!caller(0).limited());
    }
}
