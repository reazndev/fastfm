//! The import job's persisted cursor and the progress read from it.

use lfm_store::{Job, JobId, JobStatus, JobUpdate, Store};
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// The `kind` of import jobs in the jobs table.
pub(crate) const IMPORT: &str = "import";

/// One time range that a worker walks backwards from `cursor` down to `from`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Segment {
    pub(crate) from: i64,
    pub(crate) cursor: i64,
}

impl Segment {
    pub(crate) fn done(self) -> bool {
        self.cursor <= self.from
    }
}

/// Everything an import needs to continue after a restart, stored as the job's cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ImportState {
    /// Upper bound of the import, exclusive: the start time or the `until` cutoff.
    pub(crate) t0: i64,
    /// Whether `t0` is a cutoff the caller asked for, rather than the start time.
    pub(crate) cutoff: bool,
    /// Last.fm's scrobble count below `t0` when the import was planned.
    pub(crate) total: u64,
    /// Scrobbles written so far.
    pub(crate) imported: u64,
    /// The planned ranges; empty until planning ran.
    pub(crate) segments: Vec<Segment>,
}

impl ImportState {
    pub(crate) fn new(t0: i64, cutoff: bool) -> Self {
        Self {
            t0,
            cutoff,
            total: 0,
            imported: 0,
            segments: Vec::new(),
        }
    }

    pub(crate) fn planned(&self) -> bool {
        !self.segments.is_empty()
    }

    pub(crate) fn parse(job: &Job) -> Result<Self> {
        Ok(serde_json::from_str(&job.cursor)?)
    }

    /// Saves the state with the given status and error.
    pub(crate) async fn save(
        &self,
        store: &Store,
        job: JobId,
        (status, error): (JobStatus, Option<String>),
    ) -> Result<()> {
        let cursor = serde_json::to_string(self)?;
        let update = JobUpdate {
            status,
            cursor,
            error,
        };
        Ok(store.update_job(job, update, crate::clock::now()).await?)
    }
}

/// Where an import stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportProgress {
    /// The job's id in the store.
    pub job: JobId,
    /// Queued, running, done, or failed.
    pub status: JobStatus,
    /// Scrobbles written so far.
    pub imported: u64,
    /// Last.fm's count of scrobbles to import, known once planning ran; 0 before.
    pub estimated_total: u64,
    /// The import's upper bound in Unix seconds, exclusive.
    pub until: i64,
    /// The last error, for a failed job.
    pub error: Option<String>,
}

impl ImportProgress {
    pub(crate) fn from_job(job: &Job) -> Result<Self> {
        let state = ImportState::parse(job)?;
        Ok(Self {
            job: job.id,
            status: job.status,
            imported: state.imported,
            estimated_total: state.total,
            until: state.t0,
            error: job.error.clone(),
        })
    }
}
