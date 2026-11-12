//! Configuration from environment variables.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context as _;

/// Server settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// `LFM_BIND`, default `0.0.0.0:8080`.
    pub bind: SocketAddr,
    /// `LFM_DATA_DIR`, default `/data`.
    pub data_dir: PathBuf,
    /// The database file: `LFM_DB`, default `lfm.db` in the data directory.
    pub db: PathBuf,
    /// Development and test only: `LFM_NOW` freezes the time methods see, in Unix seconds, so
    /// period charts can be compared with fixtures recorded at a known time.
    pub now: Option<u64>,
    /// Development and test only: `LFM_DEV_API_KEY` (with `LFM_DEV_API_SECRET`) registers one
    /// read-write key in memory at start.
    pub dev_key: Option<DevKey>,
}

/// An API key registered from the environment, for development and tests.
#[derive(Clone, PartialEq, Eq)]
pub struct DevKey {
    /// The key.
    pub key: String,
    /// Its shared secret: `LFM_DEV_API_SECRET`, or the key itself when that is unset.
    pub secret: String,
}

impl std::fmt::Debug for DevKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DevKey(redacted)")
    }
}

impl Config {
    /// Reads the settings from the process environment.
    ///
    /// # Errors
    /// When `LFM_BIND` is not a socket address or `LFM_NOW` is not a number.
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Reads the settings through a lookup function, so tests need not touch the environment.
    ///
    /// # Errors
    /// When `LFM_BIND` is not a socket address or `LFM_NOW` is not a number.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let bind = lookup("LFM_BIND").unwrap_or_else(|| "0.0.0.0:8080".to_owned());
        let bind = bind
            .parse()
            .with_context(|| format!("LFM_BIND is not a socket address: {bind}"))?;
        let data_dir = lookup("LFM_DATA_DIR").map_or_else(|| PathBuf::from("/data"), PathBuf::from);
        let db = lookup("LFM_DB").map_or_else(|| data_dir.join("lfm.db"), PathBuf::from);
        let now = lookup("LFM_NOW")
            .map(|now| {
                now.trim()
                    .parse()
                    .with_context(|| format!("LFM_NOW is not a Unix time: {now}"))
            })
            .transpose()?;
        let dev_key = lookup("LFM_DEV_API_KEY")
            .filter(|key| !key.is_empty())
            .map(|key| DevKey {
                secret: lookup("LFM_DEV_API_SECRET").unwrap_or_else(|| key.clone()),
                key,
            });
        Ok(Self {
            bind,
            data_dir,
            db,
            now,
            dev_key,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides() {
        let config = Config::from_lookup(|_| None).unwrap();
        assert_eq!(config.bind.to_string(), "0.0.0.0:8080");
        assert_eq!(config.data_dir, PathBuf::from("/data"));
        assert_eq!(config.db, PathBuf::from("/data/lfm.db"));
        assert_eq!((config.now, config.dev_key), (None, None));
        let config = Config::from_lookup(|name| match name {
            "LFM_BIND" => Some("127.0.0.1:9000".into()),
            "LFM_NOW" => Some("1794518885".into()),
            "LFM_DEV_API_KEY" => Some("k".into()),
            "LFM_DB" | "LFM_DEV_API_SECRET" => None,
            _ => Some("/tmp/x".into()),
        })
        .unwrap();
        assert_eq!(config.bind.to_string(), "127.0.0.1:9000");
        assert_eq!(config.data_dir, PathBuf::from("/tmp/x"));
        assert_eq!(config.db, PathBuf::from("/tmp/x/lfm.db"));
        assert_eq!(config.now, Some(1_794_518_885));
        let key = config.dev_key.unwrap();
        assert_eq!((key.key.as_str(), key.secret.as_str()), ("k", "k"));
        assert!(Config::from_lookup(|_| Some("nope".into())).is_err());
    }
}
