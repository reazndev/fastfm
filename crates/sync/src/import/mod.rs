//! Full-history import of one user.
//!
//! The import fixes its upper bound `t0` when it starts, splits `[0, t0)` into ranges of about
//! equal size, and walks each range backwards in parallel. One coordinator writes every window
//! with `replace_window` and saves the cursors after each one, so a restart repeats at most the
//! windows that were in flight. Profile, loved tracks, the chart list and enrichment follow, and
//! the user's sync cursor is set to `t0` last.

mod extras;
mod plan;
mod state;

use std::time::Duration;

use lfm_store::{JobId, JobStatus, UserId};
use lfm_upstream::Priority;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::clock;
use crate::context::Context;
use crate::enrich;
use crate::error::{Result, SyncError};
use crate::walk::Window;

pub(crate) use extras::profile;
pub use state::ImportProgress;
pub(crate) use state::{IMPORT, ImportState, Segment};

/// Starts an import of `name`, or continues an unfinished one with the same cutoff.
pub(crate) async fn start(
    ctx: &Context,
    name: &str,
    until: Option<i64>,
    cancel: &CancellationToken,
) -> Result<ImportProgress> {
    let user = extras::profile(ctx, name).await?;
    let job = if let Some(job) = resumable(ctx, user, until).await? {
        job
    } else {
        let state = ImportState::new(until.unwrap_or_else(clock::now), until.is_some());
        let cursor = serde_json::to_string(&state)?;
        ctx.store
            .create_job(IMPORT, Some(user), &cursor, clock::now())
            .await?
    };
    run(ctx, job, cancel).await
}

/// The newest import of `user` with this cutoff, if it has not finished.
async fn resumable(ctx: &Context, user: UserId, until: Option<i64>) -> Result<Option<JobId>> {
    let newest = newest_import(ctx, user, until).await?;
    Ok(newest
        .filter(|(_, status)| *status != JobStatus::Done)
        .map(|(job, _)| job))
}

/// The newest import of `user` with this cutoff, `None` meaning a full import.
async fn newest_import(
    ctx: &Context,
    user: UserId,
    until: Option<i64>,
) -> Result<Option<(JobId, JobStatus)>> {
    for job in ctx.store.user_jobs(user).await? {
        if job.kind != IMPORT {
            continue;
        }
        let state = ImportState::parse(&job)?;
        let same = match until {
            Some(until) => state.cutoff && state.t0 == until,
            None => !state.cutoff,
        };
        if same {
            return Ok(Some((job.id, job.status)));
        }
    }
    Ok(None)
}

/// The job error shown for a private profile, in the web UI's words.
pub const PRIVATE_MESSAGE: &str = "This Last.fm profile hides its listening history. Make recent \
     listening public in Last.fm's privacy settings, then retry.";

/// How long until the next automatic attempt, when `user`'s newest full import failed on a
/// private profile less than `private_interval` ago.
pub(crate) async fn private_wait(ctx: &Context, user: UserId) -> Result<Option<Duration>> {
    for job in ctx.store.user_jobs(user).await? {
        if job.kind != IMPORT || ImportState::parse(&job)?.cutoff {
            continue;
        }
        if job.status != JobStatus::Failed || job.error.as_deref() != Some(PRIVATE_MESSAGE) {
            return Ok(None);
        }
        let since = u64::try_from(clock::now() - job.updated_at).unwrap_or(0);
        let wait = ctx
            .config
            .private_interval
            .saturating_sub(Duration::from_secs(since));
        return Ok((!wait.is_zero()).then_some(wait));
    }
    Ok(None)
}

/// Makes sure `name` has a finished full import, running or resuming one if not.
pub(crate) async fn ensure(ctx: &Context, name: &str, cancel: &CancellationToken) -> Result<()> {
    if let Some(user) = ctx.store.user_by_name(name).await? {
        let newest = newest_import(ctx, user.id, None).await?;
        if newest.is_some_and(|(_, status)| status == JobStatus::Done) {
            return Ok(());
        }
    }
    start(ctx, name, None, cancel).await.map(drop)
}

/// Runs or resumes the import job `job` to its end, recording success or failure in the job.
pub(crate) async fn run(
    ctx: &Context,
    job: JobId,
    cancel: &CancellationToken,
) -> Result<ImportProgress> {
    let record = ctx
        .store
        .job(job)
        .await?
        .ok_or(SyncError::Inconsistent(format!(
            "import job {} is missing",
            job.0
        )))?;
    let user = record
        .user
        .ok_or_else(|| SyncError::UnknownUser(format!("job {}", job.0)))?;
    let mut state = ImportState::parse(&record)?;
    state
        .save(&ctx.store, job, (JobStatus::Running, None))
        .await?;
    let outcome = steps(ctx, (job, user), &mut state, cancel).await;
    match &outcome {
        Ok(()) => state.save(&ctx.store, job, (JobStatus::Done, None)).await?,
        Err(SyncError::Stopped) => info!(job = job.0, "import stopped; it resumes on next start"),
        Err(err) if err.is_private_profile() => {
            info!(
                job = job.0,
                "the Last.fm profile hides its history; retrying daily"
            );
            let failed = (JobStatus::Failed, Some(PRIVATE_MESSAGE.to_owned()));
            state.save(&ctx.store, job, failed).await?;
        }
        Err(err) => {
            warn!(job = job.0, error = %err, "import failed");
            let failed = (JobStatus::Failed, Some(err.to_string()));
            state.save(&ctx.store, job, failed).await?;
        }
    }
    outcome?;
    let done = ctx.store.job(job).await?;
    done.as_ref()
        .map(ImportProgress::from_job)
        .transpose()?
        .ok_or(SyncError::Inconsistent("import job vanished".to_owned()))
}

async fn steps(
    ctx: &Context,
    (job, user): (JobId, UserId),
    state: &mut ImportState,
    cancel: &CancellationToken,
) -> Result<()> {
    let name = ctx.user_name(user).await?;
    if !state.planned() {
        plan::plan(ctx, &name, state).await?;
        state
            .save(&ctx.store, job, (JobStatus::Running, None))
            .await?;
        info!(user = %name, total = state.total, t0 = state.t0, ranges = state.segments.len(), "import planned");
    }
    fetch(ctx, (job, user), &name, state, cancel).await?;
    extras::loved_and_charts(ctx, user, &name, state.t0).await?;
    enrich::enrich(ctx, user).await?;
    ctx.store.set_sync_cursor(user, state.t0).await?;
    ctx.store.advance_periods(clock::now()).await?;
    info!(user = %name, imported = state.imported, "import done");
    Ok(())
}

type Fetched = Result<(usize, Window)>;

/// Walks every unfinished segment in parallel and writes the windows in arrival order.
async fn fetch(
    ctx: &Context,
    (job, user): (JobId, UserId),
    name: &str,
    state: &mut ImportState,
    cancel: &CancellationToken,
) -> Result<()> {
    let (sender, mut windows) = mpsc::channel::<Fetched>(state.segments.len().max(1) * 2);
    let mut workers = JoinSet::new();
    for (index, segment) in state.segments.iter().copied().enumerate() {
        if !segment.done() {
            let (ctx, name, sender) = (ctx.clone(), name.to_owned(), sender.clone());
            workers.spawn(async move { walk(&ctx, &name, (index, segment), &sender).await });
        }
    }
    drop(sender);
    loop {
        let next = tokio::select! {
            () = cancel.cancelled() => return Err(SyncError::Stopped),
            next = windows.recv() => next,
        };
        let Some(fetched) = next else {
            return Ok(());
        };
        let (index, window) = fetched?;
        let written = u64::try_from(window.scrobbles.len()).unwrap_or_default();
        let from = window.range.from;
        ctx.store
            .replace_window(user, window.range, window.scrobbles)
            .await?;
        if let Some(segment) = state.segments.get_mut(index) {
            segment.cursor = from;
        }
        state.imported += written;
        state
            .save(&ctx.store, job, (JobStatus::Running, None))
            .await?;
    }
}

/// One worker: walks its segment backwards and sends each window, or the first error.
async fn walk(
    ctx: &Context,
    name: &str,
    (index, segment): (usize, Segment),
    out: &mpsc::Sender<Fetched>,
) {
    let walker = ctx.walker(name, Priority::Import);
    let mut cursor = segment.cursor;
    while cursor > segment.from {
        let step = walker.step(segment.from, cursor).await;
        let failed = step.is_err();
        if let Ok(window) = &step {
            cursor = window.range.from;
        }
        if out.send(step.map(|w| (index, w))).await.is_err() || failed {
            return;
        }
    }
}
