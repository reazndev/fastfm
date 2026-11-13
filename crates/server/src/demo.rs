//! Wiring of the speed comparison: an internal API key, and this server's own loopback URL.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use lfm_api::methods::auth::random_hex;
use lfm_api::{ApiKey, ResponseCache, Scope};
use lfm_web::DemoConfig;

use crate::Config;

/// A fresh read-only key the comparison sends to this server's `/2.0/`. It lives in memory
/// only and changes with every start.
pub(crate) fn internal_key() -> anyhow::Result<(String, ApiKey)> {
    let key = random_hex(16)?;
    let record = ApiKey {
        account_id: 0,
        secret: random_hex(16)?,
        scope: Scope::Read,
        active: true,
    };
    Ok((key, record))
}

/// The comparison's settings, when the instance has a Last.fm key to time Last.fm with.
pub(crate) fn config(
    config: &Config,
    internal_key: &str,
    cache: Arc<dyn ResponseCache>,
) -> Option<DemoConfig> {
    let lastfm_key = config.lastfm_keys.first()?.clone();
    let demo = DemoConfig::new(lastfm_key, loopback_url(config.bind), internal_key, cache);
    Some(match config.demo_max_scrobbles {
        Some(max) => demo.max_scrobbles(max),
        None => demo,
    })
}

/// `http://<addr>/2.0/` for reaching this server from itself; a wildcard bind address becomes
/// the loopback address of the same family.
fn loopback_url(bind: SocketAddr) -> String {
    let ip = match bind.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    format!("http://{}/2.0/", SocketAddr::new(ip, bind.port()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_binds_are_reached_over_loopback() {
        let url = |bind: &str| loopback_url(bind.parse().unwrap());
        assert_eq!(url("0.0.0.0:3055"), "http://127.0.0.1:3055/2.0/");
        assert_eq!(url("[::]:9000"), "http://[::1]:9000/2.0/");
        assert_eq!(url("10.0.0.2:80"), "http://10.0.0.2:80/2.0/");
    }
}
