//! The mirror's HTTP server: configuration, routes and service wiring.

pub mod config;
mod demo;
pub mod jobs;
pub mod routes;
pub mod serve;
mod ui;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use lfm_api::methods::registry;
use lfm_api::{
    ApiKey, Clock, Dispatcher, MemoryCache, NowPlayingSource, Passthrough, ResponseCache, Scope,
    Services, StoreKeys, StoreSessions, recent_track_of,
};
use lfm_store::{RecentTrack, Store, UserId};
use lfm_sync::{NowPlaying, SyncConfig, SyncService};
use lfm_upstream::{Client, ClientConfig};
use lfm_web::{DemoConfig, Web, WebConfig, WebDeps};

pub use config::{Config, DevKey};
pub use jobs::Jobs;
pub use routes::{app_router, router};

/// How long a rendered response is kept at most; the budget is `LFM_CACHE_MB`.
const CACHE_TTL: Duration = Duration::from_secs(86_400);

/// How long an expired Last.fm answer is kept to serve while Last.fm is down, in seconds.
const STALE_KEPT: i64 = 30 * 86_400;

/// How often key use is written, keys are reloaded and period totals move on.
const MAINTENANCE_EVERY: Duration = Duration::from_secs(60);

/// The running service: the dispatcher, the web API and what background maintenance needs.
#[derive(Debug, Clone)]
pub struct App {
    /// Routes API calls.
    pub dispatcher: Arc<Dispatcher>,
    /// The JSON API behind the web UI.
    pub web: Web,
    /// The in-memory key map, reloaded by [`App::maintain`].
    pub keys: Arc<StoreKeys>,
    /// The read-through cache in front of Last.fm, when `LASTFM_API_KEYS` is set.
    pub passthrough: Option<Passthrough>,
    /// Import and live sync, unless the clock is frozen.
    pub sync: Option<Arc<SyncService>>,
}

impl App {
    /// Wires the dispatcher and the web API to the store and to Last.fm. Without
    /// `LASTFM_API_KEYS` only accounts that bring their own Last.fm key can sync.
    ///
    /// # Errors
    /// When the API keys cannot be loaded, the Last.fm client cannot be built or the web API's
    /// secrets key file cannot be opened.
    pub async fn new(store: Store, config: &Config) -> anyhow::Result<Self> {
        let (internal_key, internal_record) = demo::internal_key()?;
        let keys = load_keys(&store, config, (&internal_key, internal_record)).await?;
        let client = ClientConfig::new(config.lastfm_keys.iter().cloned());
        let client = Arc::new(Client::new(client).context("building the Last.fm client")?);
        let service = Arc::new(SyncService::new(
            store.clone(),
            Arc::clone(&client),
            SyncConfig::default().request_budgets(pool_budget(config), SYNC_BUDGET),
        ));
        let deps = WebDeps {
            store: store.clone(),
            sync: Arc::clone(&service),
            upstream: Arc::clone(&client),
            keys: Arc::clone(&keys),
        };
        let cache: Arc<dyn ResponseCache> =
            Arc::new(MemoryCache::new(config.cache_bytes, CACHE_TTL));
        let demo = demo::config(config, &internal_key, Arc::clone(&cache));
        let web = Web::start(web_config(config, demo), deps)
            .await
            .context("starting the web API")?;
        let registry = lfm_api::methods::with_accounts(registry(), &web.accounts());
        let services = Services {
            store: store.clone(),
            clock: config.now.map_or(Clock::System, Clock::Fixed),
            api_keys: Arc::clone(&keys) as Arc<dyn lfm_api::ApiKeys>,
            sessions: Arc::new(StoreSessions::new(store.clone())),
            cache,
        };
        let mut dispatcher = Dispatcher::new(registry, services);
        let passthrough = Passthrough::new(store, Arc::clone(&client))
            .miss_rate(f64::from(config.upstream_misses));
        let pooled = !config.lastfm_keys.is_empty();
        dispatcher = dispatcher.with_passthrough(if pooled {
            passthrough.clone()
        } else {
            passthrough.clone().without_pool(Arc::new(NoOwnKeys))
        });
        // Pre-warming needs the pool: it acts for no account.
        let passthrough = pooled.then_some(passthrough);
        // A frozen clock is for comparing with fixtures; syncing would change the data.
        let sync = config.now.is_none().then_some(service);
        if let Some(service) = &sync {
            let playing = Arc::new(LiveNowPlaying(service.now_playing()));
            dispatcher = dispatcher.with_now_playing(playing);
        }
        Ok(Self {
            dispatcher: Arc::new(dispatcher),
            web,
            keys,
            passthrough,
            sync,
        })
    }

    /// Runs forever: every minute, writes key use, reloads keys, moves period totals to the
    /// current day and drops Last.fm answers expired for a month. Failures are logged and
    /// retried on the next round.
    pub async fn maintain(self) {
        let services = self.dispatcher.services().clone();
        let mut ticks = tokio::time::interval(MAINTENANCE_EVERY);
        loop {
            ticks.tick().await;
            let now = i64::try_from(services.clock.now()).unwrap_or(i64::MAX);
            if let Err(error) = services.store.advance_periods(now).await {
                tracing::warn!(%error, "moving period totals failed");
            }
            if let Err(error) = services.store.flush_key_usage().await {
                tracing::warn!(%error, "writing key use failed");
            }
            if let Err(error) = self.keys.refresh().await {
                tracing::warn!(%error, "reloading API keys failed");
            }
            let wall = i64::try_from(lfm_api::clock::system_now()).unwrap_or(i64::MAX);
            if let Err(error) = services.store.cache_purge(wall - STALE_KEPT).await {
                tracing::warn!(%error, "purging the upstream cache failed");
            }
        }
    }
}

/// Own Last.fm keys of accounts, for an instance without a key pool. The web crate holds the
/// decrypted keys and is to implement [`lfm_api::CallerKeys`]; until it does, a pass-through
/// miss on such an instance answers error 16 and logs why.
#[derive(Debug)]
struct NoOwnKeys;

#[async_trait::async_trait]
impl lfm_api::CallerKeys for NoOwnKeys {
    async fn lastfm_key(&self, _account_id: i64) -> Option<lfm_upstream::ApiKey> {
        None
    }
}

/// Live sync's now-playing map, as the dispatcher reads it.
#[derive(Debug)]
struct LiveNowPlaying(NowPlaying);

impl NowPlayingSource for LiveNowPlaying {
    fn now_playing(&self, user: UserId) -> Option<RecentTrack> {
        self.0
            .get(user)
            .map(|playing| recent_track_of(&playing.track))
    }
}

async fn load_keys(
    store: &Store,
    config: &Config,
    (internal_key, internal_record): (&str, ApiKey),
) -> anyhow::Result<Arc<StoreKeys>> {
    let fixed: HashMap<String, ApiKey> = config
        .dev_key
        .iter()
        .map(|dev| (dev.key.clone(), dev_key(dev)))
        .chain([(internal_key.to_owned(), internal_record)])
        .collect();
    let keys = StoreKeys::load(store.clone(), fixed)
        .await
        .context("loading API keys")?;
    Ok(Arc::new(keys))
}

/// Background Last.fm requests per second for each key: below the client's 4 per key, so
/// pass-through reads keep headroom.
const SYNC_BUDGET: std::num::NonZeroU32 = std::num::NonZeroU32::MIN.saturating_add(2);

/// The instance pool's background budget: [`SYNC_BUDGET`] for each of its keys.
fn pool_budget(config: &Config) -> std::num::NonZeroU32 {
    let keys = u32::try_from(config.lastfm_keys.len())
        .unwrap_or(u32::MAX)
        .max(1);
    SYNC_BUDGET.saturating_mul(std::num::NonZeroU32::MIN.saturating_add(keys - 1))
}

fn web_config(config: &Config, demo: Option<DemoConfig>) -> WebConfig {
    let web = WebConfig::new(&config.data_dir)
        .cookies(config.cookies)
        .trust_proxy(config.trust_proxy)
        .key_pool_size(config.lastfm_keys.len());
    let web = match &config.setup_token {
        Some(token) => web.setup_token(token.as_str()),
        None => web,
    };
    let web = match demo {
        Some(demo) => web.demo(demo),
        None => web,
    };
    let web = match &config.public_url {
        Some(url) => web.public_url(url.as_str()),
        None => web,
    };
    match config.lastfm_app() {
        Some(app) => web.lastfm_app(app),
        None => web,
    }
}

fn dev_key(dev: &DevKey) -> ApiKey {
    ApiKey {
        account_id: 0,
        secret: dev.secret.clone(),
        scope: Scope::ReadWrite,
        active: true,
    }
}
