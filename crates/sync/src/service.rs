//! The supervisor that owns every sync job of the process.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use lfm_store::{Job, Store, UserId};
use lfm_upstream::{ApiKey, Client};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{info, warn};

use crate::budget::Budgets;
use crate::config::SyncConfig;
use crate::context::Context;
use crate::enrich::{self, EnrichReport};
use crate::error::Result;
use crate::import::{self, IMPORT, ImportProgress, ImportState};
use crate::now_playing::NowPlaying;
use crate::reconcile::{self, ReconcileReport};
use crate::tasks::{self, Signal};
use crate::upstream::Upstream;

/// Runs imports, live sync, reconciliation, and the period timer for the whole process.
///
/// Background work starts with [`SyncService::start`] and [`SyncService::follow`] and ends with
/// [`SyncService::unfollow`] and [`SyncService::shutdown`]. [`SyncService::import`] and
/// [`SyncService::reconcile`] run in the caller's task. Every job saves its progress in the
/// store, so work cut short by a shutdown or a crash continues on the next start.
#[derive(Debug)]
pub struct SyncService {
    ctx: Context,
    shutdown: CancellationToken,
    tracker: TaskTracker,
    followed: Mutex<HashMap<UserId, Follow>>,
}

#[derive(Debug)]
struct Follow {
    stop: CancellationToken,
    task: JoinHandle<()>,
    signal: Arc<Signal>,
}

impl SyncService {
    /// A service over `store` that calls Last.fm through `client`.
    #[must_use]
    pub fn new(store: Store, client: Arc<Client>, config: SyncConfig) -> Self {
        let budgets = Budgets::new(config.pool_budget, config.key_budget);
        let ctx = Context {
            store,
            upstream: Upstream::new(client, budgets),
            config,
            now_playing: NowPlaying::default(),
            private: Arc::default(),
        };
        Self {
            ctx,
            shutdown: CancellationToken::new(),
            tracker: TaskTracker::new(),
            followed: Mutex::default(),
        }
    }

    /// The now-playing state that live sync keeps up to date.
    #[must_use]
    pub fn now_playing(&self) -> NowPlaying {
        self.ctx.now_playing.clone()
    }

    /// Makes `name`'s import, live sync, reconciliation and enrichment call Last.fm with the
    /// account's own key, or with the shared pool again when `key` is `None`.
    pub fn use_key(&self, name: &str, key: Option<ApiKey>) {
        self.ctx.upstream.use_key(name, key);
    }

    /// Last.fm requests made through this service so far.
    #[must_use]
    pub fn requests(&self) -> u64 {
        self.ctx.upstream.requests()
    }

    /// Starts the period timer and resumes every unfinished import in the store. Users with an
    /// unfinished full import are followed; imports with a cutoff just run to their end.
    pub async fn start(&self) -> Result<()> {
        let ctx = self.ctx.clone();
        let shutdown = self.shutdown.clone();
        self.tracker
            .spawn(async move { tasks::advance_periods(&ctx, &shutdown).await });
        for job in self.ctx.store.unfinished_jobs().await? {
            if job.kind == IMPORT {
                self.resume(job).await?;
            }
        }
        Ok(())
    }

    async fn resume(&self, job: Job) -> Result<()> {
        if ImportState::parse(&job)?.cutoff {
            let (ctx, stop) = (self.ctx.clone(), self.shutdown.clone());
            self.tracker.spawn(async move {
                if let Err(err) = import::run(&ctx, job.id, &stop).await {
                    warn!(job = job.id.0, error = %err, "resumed import ended");
                }
            });
        } else if let Some(user) = job.user {
            let name = self.ctx.user_name(user).await?;
            self.follow(&name).await?;
        }
        Ok(())
    }

    /// Follows a Last.fm user in the background: imports their history unless a full import
    /// already finished, then polls for new scrobbles and reconciles once per interval.
    /// Does nothing for a user that is already followed.
    pub async fn follow(&self, name: &str) -> Result<UserId> {
        let user = match self.ctx.store.user_by_name(name).await? {
            Some(user) => user.id,
            None => import::profile(&self.ctx, name).await?,
        };
        let mut followed = self.followed.lock().unwrap_or_else(PoisonError::into_inner);
        if followed.get(&user).is_some_and(|f| !f.task.is_finished()) {
            return Ok(user);
        }
        let stop = self.shutdown.child_token();
        let signal = Arc::new(Signal::default());
        let (ctx, token, wake) = (self.ctx.clone(), stop.clone(), Arc::clone(&signal));
        let task = self
            .tracker
            .spawn(async move { tasks::follow(&ctx, user, &wake, &token).await });
        followed.insert(user, Follow { stop, task, signal });
        info!(user = user.0, "following");
        Ok(user)
    }

    /// Asks for an early poll of a followed user whose last poll is older than
    /// `nudge_after`, for when their data is read. Returns whether a poll was asked for.
    #[must_use]
    pub fn nudge(&self, user: UserId) -> bool {
        self.with_signal(user, |signal| {
            let stale = signal.stale(self.ctx.config.nudge_after);
            if stale {
                signal.wake();
            }
            stale
        })
    }

    /// Tries a followed user's import or poll again now, after the owner made a private
    /// profile public. Returns whether the user is followed.
    pub fn retry(&self, user: UserId) -> bool {
        self.with_signal(user, |signal| {
            self.ctx.set_private(user, false);
            signal.wake();
            true
        })
    }

    /// Whether Last.fm last refused the user's history because the profile hides it.
    #[must_use]
    pub fn is_private(&self, user: UserId) -> bool {
        self.ctx.is_private(user)
    }

    fn with_signal(&self, user: UserId, act: impl FnOnce(&Signal) -> bool) -> bool {
        let followed = self.followed.lock().unwrap_or_else(PoisonError::into_inner);
        followed
            .get(&user)
            .filter(|f| !f.task.is_finished())
            .is_some_and(|f| act(&f.signal))
    }

    /// Stops following a user and waits for their task to end.
    pub async fn unfollow(&self, user: UserId) {
        let follow = self
            .followed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&user);
        if let Some(follow) = follow {
            follow.stop.cancel();
            // A task that panicked has nothing left to clean up.
            let _ = follow.task.await;
        }
        self.ctx.now_playing.set(user, None, 0);
        self.ctx.set_private(user, false);
    }

    /// Imports `name`'s history in the caller's task, or continues an unfinished import with
    /// the same cutoff. With `until`, only scrobbles before that Unix time are imported.
    pub async fn import(&self, name: &str, until: Option<i64>) -> Result<ImportProgress> {
        import::start(&self.ctx, name, until, &self.shutdown).await
    }

    /// Runs [`SyncService::import`] with the cutoff `until` in the background, tracked like
    /// the other jobs so a shutdown stops it and the next start resumes it to its end. The
    /// user is not followed afterwards.
    pub fn spawn_import(&self, name: &str, until: i64) -> JoinHandle<Result<ImportProgress>> {
        let (ctx, stop, name) = (self.ctx.clone(), self.shutdown.clone(), name.to_owned());
        self.tracker
            .spawn(async move { import::start(&ctx, &name, Some(until), &stop).await })
    }

    /// Where the user's newest import stands.
    pub async fn progress(&self, user: UserId) -> Result<Option<ImportProgress>> {
        let jobs = self.ctx.store.user_jobs(user).await?;
        jobs.iter()
            .find(|job| job.kind == IMPORT)
            .map(ImportProgress::from_job)
            .transpose()
    }

    /// Compares `[0, until)` with Last.fm and repairs the ranges that differ. `until` defaults
    /// to the user's sync cursor, below which the store is complete.
    pub async fn reconcile(&self, user: UserId, until: Option<i64>) -> Result<ReconcileReport> {
        let until = match until {
            Some(until) => until,
            None => tasks::sync_cursor(&self.ctx, user).await?,
        };
        reconcile::reconcile(&self.ctx, user, until).await
    }

    /// Reads the user's all-time top lists and stores the library entities their scrobbles
    /// count under, in the caller's task. Imports already do this once at their end.
    pub async fn enrich(&self, user: UserId) -> Result<EnrichReport> {
        enrich::enrich(&self.ctx, user).await
    }

    /// Stops every job and waits for them. Unfinished imports stay `running` in the store and
    /// resume on the next [`SyncService::start`].
    pub async fn shutdown(&self) {
        self.shutdown.cancel();
        self.tracker.close();
        self.tracker.wait().await;
    }
}
