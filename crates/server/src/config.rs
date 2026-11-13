//! Configuration from environment variables.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context as _;
use lfm_api::prewarm::PrewarmConfig;
use lfm_upstream::{ApiKey, AppCredentials, Secret};
use lfm_web::CookiePolicy;

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
    /// `LASTFM_API_KEYS`, comma separated: the instance's Last.fm key pool. Without keys the
    /// mirror serves only what it computes locally, and runs no sync.
    pub lastfm_keys: Vec<ApiKey>,
    /// `LASTFM_API_SECRET`: the shared secret of the pool's first key, which signs Last.fm web
    /// auth and forwarded writes for accounts without their own key.
    pub lastfm_secret: Option<Secret>,
    /// `LFM_FOLLOW`, comma separated: Last.fm users to import and keep in sync besides the ones
    /// that accounts link. A development switch.
    pub follow: Vec<String>,
    /// `LFM_PUBLIC_URL`, such as `https://fm.example.org`: where browsers reach the instance,
    /// for the Last.fm web auth callback. Without it the request's `Host` header is used.
    pub public_url: Option<String>,
    /// `LFM_SECURE_COOKIES`: `auto` (default), `always` or `never`. `auto` means `always` when
    /// the public URL is HTTPS, else whether the request came over HTTPS.
    pub cookies: CookiePolicy,
    /// `LFM_TRUST_PROXY=true`: trust `X-Forwarded-For` and `X-Forwarded-Proto` from a reverse
    /// proxy. Leave it off when clients reach the server directly.
    pub trust_proxy: bool,
    /// `LFM_PREWARM_RANKS` (default 2000) and `LFM_PREWARM_INTERVAL_MS` (default 1000): how many
    /// of a followed user's top artists, albums and tracks get `getInfo` pre-warmed, and the
    /// pause after each call.
    pub prewarm: PrewarmConfig,
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
    /// Reads the settings from the process environment, then from `.env` in the working
    /// directory, as `lfm-sync` and the recorder do.
    ///
    /// # Errors
    /// When a value does not parse.
    pub fn from_env() -> anyhow::Result<Self> {
        let file = std::fs::read_to_string(".env").unwrap_or_default();
        Self::from_lookup(|name| std::env::var(name).ok().or_else(|| file_value(&file, name)))
    }

    /// Reads the settings through a lookup function, so tests need not touch the environment.
    ///
    /// # Errors
    /// When a value does not parse.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let bind = lookup("LFM_BIND").unwrap_or_else(|| "0.0.0.0:8080".to_owned());
        let bind = bind
            .parse()
            .with_context(|| format!("LFM_BIND is not a socket address: {bind}"))?;
        let data_dir = lookup("LFM_DATA_DIR").map_or_else(|| PathBuf::from("/data"), PathBuf::from);
        let db = lookup("LFM_DB").map_or_else(|| data_dir.join("lfm.db"), PathBuf::from);
        let now = number(&lookup, "LFM_NOW")?;
        let dev_key = lookup("LFM_DEV_API_KEY")
            .filter(|key| !key.is_empty())
            .map(|key| DevKey {
                secret: lookup("LFM_DEV_API_SECRET").unwrap_or_else(|| key.clone()),
                key,
            });
        let list = |name| lookup(name).map(|value| list(&value)).unwrap_or_default();
        let lastfm_keys = list("LASTFM_API_KEYS")
            .into_iter()
            .map(ApiKey::new)
            .collect();
        let public_url = lookup("LFM_PUBLIC_URL").filter(|url| !url.trim().is_empty());
        let cookies = cookie_policy(
            lookup("LFM_SECURE_COOKIES").as_deref(),
            public_url.as_deref(),
        )?;
        let mut prewarm = PrewarmConfig::default();
        if let Some(ranks) = number(&lookup, "LFM_PREWARM_RANKS")? {
            prewarm = prewarm.ranks(ranks);
        }
        if let Some(millis) = number(&lookup, "LFM_PREWARM_INTERVAL_MS")? {
            prewarm = prewarm.interval(Duration::from_millis(millis));
        }
        Ok(Self {
            bind,
            data_dir,
            db,
            now,
            dev_key,
            lastfm_keys,
            lastfm_secret: lookup("LASTFM_API_SECRET")
                .filter(|secret| !secret.is_empty())
                .map(Secret::new),
            follow: list("LFM_FOLLOW"),
            public_url,
            cookies,
            trust_proxy: flag(&lookup, "LFM_TRUST_PROXY")?,
            prewarm,
        })
    }

    /// The instance's Last.fm application: the first pool key with `LASTFM_API_SECRET`.
    #[must_use]
    pub fn lastfm_app(&self) -> Option<AppCredentials> {
        Some(AppCredentials {
            api_key: self.lastfm_keys.first()?.clone(),
            secret: self.lastfm_secret.clone()?,
        })
    }
}

fn cookie_policy(value: Option<&str>, public_url: Option<&str>) -> anyhow::Result<CookiePolicy> {
    let https = public_url.is_some_and(|url| url.starts_with("https://"));
    match value.map_or("auto", str::trim) {
        "auto" if https => Ok(CookiePolicy::Always),
        "auto" => Ok(CookiePolicy::Auto),
        "always" => Ok(CookiePolicy::Always),
        "never" => Ok(CookiePolicy::Never),
        other => anyhow::bail!("LFM_SECURE_COOKIES must be auto, always or never: {other}"),
    }
}

fn flag(lookup: impl Fn(&str) -> Option<String>, name: &str) -> anyhow::Result<bool> {
    match lookup(name).as_deref().map(str::trim) {
        None | Some("" | "0" | "false") => Ok(false),
        Some("1" | "true") => Ok(true),
        Some(other) => anyhow::bail!("{name} must be true or false: {other}"),
    }
}

fn number(lookup: impl Fn(&str) -> Option<String>, name: &str) -> anyhow::Result<Option<u64>> {
    lookup(name)
        .map(|value| {
            value
                .trim()
                .parse()
                .with_context(|| format!("{name} is not a number: {value}"))
        })
        .transpose()
}

fn list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `NAME=value` from a `.env` file, without quotes; comments and other lines are skipped.
fn file_value(text: &str, name: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().trim_matches(['"', '\'']).to_owned())
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
        assert_eq!(config.lastfm_app(), None);
        assert_eq!((config.now, config.dev_key), (None, None));
        assert!(config.lastfm_keys.is_empty() && config.follow.is_empty());
        assert_eq!(
            (config.cookies, config.trust_proxy),
            (CookiePolicy::Auto, false)
        );
        let config = Config::from_lookup(|name| match name {
            "LFM_BIND" => Some("127.0.0.1:9000".into()),
            "LFM_NOW" => Some("1794518885".into()),
            "LFM_DEV_API_KEY" => Some("k".into()),
            "LFM_DB" | "LFM_DEV_API_SECRET" | "LFM_PREWARM_RANKS" | "LFM_SECURE_COOKIES" => None,
            "LFM_PUBLIC_URL" => Some("https://fm.example.org".into()),
            "LFM_TRUST_PROXY" => Some("true".into()),
            "LASTFM_API_SECRET" => Some("s".into()),
            "LASTFM_API_KEYS" => Some("a, b,".into()),
            "LFM_FOLLOW" => Some("alice".into()),
            "LFM_PREWARM_INTERVAL_MS" => Some("250".into()),
            _ => Some("/tmp/x".into()),
        })
        .unwrap();
        assert_eq!(config.bind.to_string(), "127.0.0.1:9000");
        assert_eq!(config.data_dir, PathBuf::from("/tmp/x"));
        assert_eq!(config.db, PathBuf::from("/tmp/x/lfm.db"));
        assert_eq!(config.now, Some(1_794_518_885));
        let app = config.lastfm_app().unwrap();
        assert_eq!(
            (app.api_key, app.secret),
            (ApiKey::new("a"), Secret::new("s"))
        );
        let key = config.dev_key.unwrap();
        assert_eq!((key.key.as_str(), key.secret.as_str()), ("k", "k"));
        assert_eq!(config.lastfm_keys, [ApiKey::new("a"), ApiKey::new("b")]);
        assert_eq!(config.follow, ["alice"]);
        assert_eq!(
            (config.cookies, config.trust_proxy),
            (CookiePolicy::Always, true)
        );
        let interval = PrewarmConfig::default().interval(Duration::from_millis(250));
        assert_eq!(config.prewarm, interval);
        assert!(Config::from_lookup(|_| Some("nope".into())).is_err());
    }

    #[test]
    fn cookie_policy_follows_the_public_url_unless_set() {
        let policy = cookie_policy;
        assert_eq!(policy(None, None).unwrap(), CookiePolicy::Auto);
        assert_eq!(policy(None, Some("http://x")).unwrap(), CookiePolicy::Auto);
        assert_eq!(
            policy(None, Some("https://x")).unwrap(),
            CookiePolicy::Always
        );
        assert_eq!(
            policy(Some("never"), Some("https://x")).unwrap(),
            CookiePolicy::Never
        );
        assert!(policy(Some("yes"), None).is_err());
    }

    #[test]
    fn reads_plain_and_quoted_env_file_values() {
        let text = "# comment\nA=1\nB=\"two\"\n";
        assert_eq!(file_value(text, "A").as_deref(), Some("1"));
        assert_eq!(file_value(text, "B").as_deref(), Some("two"));
        assert_eq!(file_value(text, "C"), None);
    }
}
