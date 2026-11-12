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
}

impl Config {
    /// Reads the settings from the process environment.
    ///
    /// # Errors
    /// When `LFM_BIND` is not a socket address.
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Reads the settings through a lookup function, so tests need not touch the environment.
    ///
    /// # Errors
    /// When `LFM_BIND` is not a socket address.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let bind = lookup("LFM_BIND").unwrap_or_else(|| "0.0.0.0:8080".to_owned());
        let bind = bind
            .parse()
            .with_context(|| format!("LFM_BIND is not a socket address: {bind}"))?;
        let data_dir = lookup("LFM_DATA_DIR").map_or_else(|| PathBuf::from("/data"), PathBuf::from);
        Ok(Self { bind, data_dir })
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
        let config = Config::from_lookup(|name| match name {
            "LFM_BIND" => Some("127.0.0.1:9000".into()),
            _ => Some("/tmp/x".into()),
        })
        .unwrap();
        assert_eq!(config.bind.to_string(), "127.0.0.1:9000");
        assert_eq!(config.data_dir, PathBuf::from("/tmp/x"));
        assert!(Config::from_lookup(|_| Some("nope".into())).is_err());
    }
}
