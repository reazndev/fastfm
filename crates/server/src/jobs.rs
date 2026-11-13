//! Background jobs that run next to the API: import and live sync of the Last.fm users that
//! accounts link (and of `LFM_FOLLOW`), and pre-warming their libraries once the import is done.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use lfm_api::Passthrough;
use lfm_api::prewarm::{self, PrewarmConfig};
use lfm_store::{JobStatus, Store, UserId, fold};
use lfm_sync::SyncService;
use lfm_web::Web;
use tokio::task::{AbortHandle, JoinHandle, JoinSet};

use crate::{App, Config};

/// How often to check whether a followed user's import has finished.
const IMPORT_CHECK: Duration = Duration::from_secs(30);

/// How often to look for links that changed. The web API follows a new link at once; this
/// round queues its pre-warming, drops it for unlinked users, and retries failed follows.
const LINK_CHECK: Duration = Duration::from_secs(30);

/// The running jobs. [`Jobs::shutdown`] stops them; their progress is in the store.
#[derive(Debug)]
pub struct Jobs {
    sync: Arc<SyncService>,
    watcher: JoinHandle<()>,
}

impl Jobs {
    /// Resumes unfinished imports, then keeps the linked and configured users followed.
    /// Returns `None` when the app runs without sync.
    ///
    /// # Errors
    /// When the store cannot list unfinished jobs.
    pub async fn start(app: &App, config: &Config) -> anyhow::Result<Option<Self>> {
        let Some(sync) = &app.sync else {
            return Ok(None);
        };
        sync.start().await?;
        let watcher = Watcher {
            web: app.web.clone(),
            fixed: config.follow.clone(),
            warm: Warm {
                sync: Arc::clone(sync),
                store: app.dispatcher.services().store.clone(),
                passthrough: app.passthrough.clone(),
                config: config.prewarm,
            },
            warming: HashMap::new(),
            tasks: JoinSet::new(),
        };
        Ok(Some(Self {
            sync: Arc::clone(sync),
            watcher: tokio::spawn(watcher.run()),
        }))
    }

    /// Stops pre-warming and sync, and waits for sync to save its state.
    pub async fn shutdown(self) {
        self.watcher.abort();
        // Aborting drops the watcher, whose task set aborts every pre-warm.
        let _ = self.watcher.await;
        self.sync.shutdown().await;
    }
}

/// Keeps the wanted users followed, with one pre-warm task each.
struct Watcher {
    web: Web,
    fixed: Vec<String>,
    warm: Warm,
    /// Pre-warm tasks by folded user name.
    warming: HashMap<String, AbortHandle>,
    tasks: JoinSet<()>,
}

impl Watcher {
    async fn run(mut self) {
        let mut ticks = tokio::time::interval(LINK_CHECK);
        loop {
            ticks.tick().await;
            self.round().await;
            // Reap finished pre-warms so the set does not grow.
            while self.tasks.try_join_next().is_some() {}
        }
    }

    async fn round(&mut self) {
        let linked = match self.web.linked_users().await {
            Ok(linked) => linked,
            Err(error) => {
                tracing::warn!(%error, "listing linked Last.fm users failed");
                return;
            }
        };
        let wanted: HashMap<String, String> = self
            .fixed
            .iter()
            .chain(&linked)
            .map(|name| (fold(name), name.clone()))
            .collect();
        self.warming.retain(|key, task| {
            let keep = wanted.contains_key(key);
            if !keep {
                task.abort();
            }
            keep
        });
        for (key, name) in wanted {
            if self.warming.contains_key(&key) {
                continue;
            }
            match self.warm.sync.follow(&name).await {
                Ok(user) => {
                    let task = self.tasks.spawn(self.warm.clone().after_import(user));
                    self.warming.insert(key, task);
                }
                Err(error) => tracing::warn!(user = %name, %error, "cannot follow"),
            }
        }
    }
}

#[derive(Clone)]
struct Warm {
    sync: Arc<SyncService>,
    store: Store,
    passthrough: Option<Passthrough>,
    config: PrewarmConfig,
}

impl Warm {
    /// Waits until `user`'s import is done, then warms their library through the read-through
    /// cache, when there is one.
    async fn after_import(self, user: UserId) {
        let Some(passthrough) = self.passthrough else {
            return;
        };
        loop {
            match self.sync.progress(user).await {
                Ok(Some(progress)) if progress.status == JobStatus::Done => break,
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "reading import progress failed"),
            }
            tokio::time::sleep(IMPORT_CHECK).await;
        }
        let task = prewarm::start(self.store, passthrough, user, self.config);
        let abort = task.abort_handle();
        // Dropping this future (on shutdown or unlink) must stop the spawned task too.
        let _guard = AbortOnDrop(abort);
        if let Err(error) = task.await
            && error.is_panic()
        {
            tracing::error!(user = user.0, "pre-warming panicked");
        }
    }
}

struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
