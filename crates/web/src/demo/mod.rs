//! The speed comparison: the same API calls timed against Last.fm and against this mirror, both
//! from the server, one request at a time with one Last.fm key.

mod config;
mod measure;
mod plan;
mod race;

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use lfm_store::AccountId;
use lfm_sync::ImportProgress;
use lfm_upstream::TimedClient;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

pub use config::DemoConfig;
pub(crate) use measure::Outcome;
pub(crate) use plan::{Scenario, ScenarioView, facts, scenarios};
pub(crate) use race::{RacePage, Side};

use crate::error::WebError;
use crate::throttle::Throttle;

/// Comparisons one account may prepare, and scenarios it may run, per [`WINDOW`].
const PREPARES: u32 = 30;
const RUNS: u32 = 120;
/// Last.fm pages one account may fetch in the live race per [`WINDOW`], 200 scrobbles each.
const RACE_PAGES: u32 = 1000;
/// Demo imports one account may start per [`WINDOW`].
const IMPORTS: u32 = 5;
/// The rate-limit window in seconds.
const WINDOW: i64 = 3600;
/// A timed request that takes longer than this fails.
const TIMEOUT: Duration = Duration::from_secs(30);

/// A demo import running in the background.
type ImportTask = JoinHandle<lfm_sync::Result<ImportProgress>>;

/// The running demo: its HTTP client, pacing, imports and rate limits.
#[derive(Debug)]
pub(crate) struct Demo {
    pub(crate) config: DemoConfig,
    client: TimedClient,
    /// One comparison request runs at a time across the instance.
    turn: Semaphore,
    /// When the last request to Last.fm was sent.
    lastfm_sent: Mutex<Option<Instant>>,
    /// The newest demo import of each account, by account id.
    imports: Mutex<HashMap<AccountId, (String, ImportTask)>>,
    pub(crate) prepares: Throttle,
    pub(crate) runs: Throttle,
    pub(crate) race_pages: Throttle,
    pub(crate) import_starts: Throttle,
}

/// What the account's own demo import is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnImport {
    /// None for this user.
    None,
    /// Started and not finished.
    Running,
    /// Ended, successfully or not.
    Ended,
}

impl Demo {
    pub(crate) fn new(config: DemoConfig) -> Result<Self, WebError> {
        let agent = concat!("lfm-mirror-demo/", env!("CARGO_PKG_VERSION"));
        let client = TimedClient::new(agent, TIMEOUT).map_err(|e| WebError::internal(&e))?;
        Ok(Self {
            config,
            client,
            turn: Semaphore::new(1),
            lastfm_sent: Mutex::new(None),
            imports: Mutex::default(),
            prepares: Throttle::new(PREPARES, WINDOW),
            runs: Throttle::new(RUNS, WINDOW),
            race_pages: Throttle::new(RACE_PAGES, WINDOW),
            import_starts: Throttle::new(IMPORTS, WINDOW),
        })
    }

    /// Whether the account has a demo import that has not finished.
    pub(crate) fn importing(&self, account: AccountId) -> bool {
        self.lock_imports()
            .get(&account)
            .is_some_and(|(_, task)| !task.is_finished())
    }

    /// The account's demo import of `name`, if it started one.
    pub(crate) fn own_import(&self, account: AccountId, name: &str) -> OwnImport {
        let imports = self.lock_imports();
        match imports.get(&account) {
            Some((user, task)) if lfm_store::fold(user) == lfm_store::fold(name) => {
                if task.is_finished() {
                    OwnImport::Ended
                } else {
                    OwnImport::Running
                }
            }
            _ => OwnImport::None,
        }
    }

    /// Records the account's new demo import.
    pub(crate) fn started(&self, account: AccountId, name: &str, task: ImportTask) {
        self.lock_imports().insert(account, (name.to_owned(), task));
    }

    fn lock_imports(&self) -> MutexGuard<'_, HashMap<AccountId, (String, ImportTask)>> {
        self.imports.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
