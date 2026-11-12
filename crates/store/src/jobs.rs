//! Persisted job state, so import and sync resume after a restart.

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::error::{Result, changed};
use crate::pool::Store;
use crate::text::text_enum;
use crate::types::{JobId, UserId};

text_enum!(
    /// Where a job stands.
    JobStatus {
        /// Waiting to run.
        Queued => "queued",
        /// Running, or was running when the process stopped.
        Running => "running",
        /// Finished.
        Done => "done",
        /// Stopped with an error.
        Failed => "failed",
    }
);

/// A persisted job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    /// Store id.
    pub id: JobId,
    /// Job kind, defined by the caller, such as `import`.
    pub kind: String,
    /// User the job works on, if any.
    pub user: Option<UserId>,
    /// Where to continue, as JSON defined by the caller.
    pub cursor: String,
    /// Where the job stands.
    pub status: JobStatus,
    /// Last error message.
    pub error: Option<String>,
    /// Creation time in unix seconds.
    pub created_at: i64,
    /// Last update in unix seconds.
    pub updated_at: i64,
}

/// New state for a job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobUpdate {
    /// New status.
    pub status: JobStatus,
    /// New cursor JSON.
    pub cursor: String,
    /// Error message, or `None` to clear it.
    pub error: Option<String>,
}

const COLUMNS: &str = "id, kind, user_id, cursor, status, error, created_at, updated_at";

impl Store {
    /// Creates a queued job.
    pub async fn create_job(
        &self,
        kind: &str,
        user: Option<UserId>,
        cursor: &str,
        now: i64,
    ) -> Result<JobId> {
        let (kind, cursor) = (kind.to_owned(), cursor.to_owned());
        self.write(move |tx| {
            let sql = "INSERT INTO jobs (kind, user_id, cursor, status, created_at, updated_at) \
                       VALUES (?1, ?2, ?3, 'queued', ?4, ?4) RETURNING id";
            let values = params![kind, user, cursor, now];
            Ok(tx
                .prepare_cached(sql)?
                .query_row(values, |row| row.get(0))?)
        })
        .await
    }

    /// Saves a job's status, cursor, and error.
    pub async fn update_job(&self, id: JobId, update: JobUpdate, now: i64) -> Result<()> {
        self.write(move |tx| {
            let sql = "UPDATE jobs SET status = ?2, cursor = ?3, error = ?4, updated_at = ?5 \
                       WHERE id = ?1";
            let u = &update;
            changed(
                tx.prepare_cached(sql)?
                    .execute(params![id, u.status, u.cursor, u.error, now])?,
            )
        })
        .await
    }

    /// Looks up a job.
    pub async fn job(&self, id: JobId) -> Result<Option<Job>> {
        self.read(move |conn| {
            let sql = format!("SELECT {COLUMNS} FROM jobs WHERE id = ?1");
            Ok(conn
                .prepare_cached(&sql)?
                .query_row([id], job_from_row)
                .optional()?)
        })
        .await
    }

    /// Jobs that are queued or were running, oldest first, to resume after a restart.
    pub async fn unfinished_jobs(&self) -> Result<Vec<Job>> {
        self.read(|conn| select(conn, "status IN ('queued', 'running') ORDER BY id", None))
            .await
    }

    /// All jobs of one user, newest first.
    pub async fn user_jobs(&self, user: UserId) -> Result<Vec<Job>> {
        self.read(move |conn| select(conn, "user_id = ?1 ORDER BY id DESC", Some(user)))
            .await
    }
}

fn select(conn: &Connection, filter: &str, user: Option<UserId>) -> Result<Vec<Job>> {
    let sql = format!("SELECT {COLUMNS} FROM jobs WHERE {filter}");
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows: rusqlite::Result<Vec<Job>> = match user {
        Some(user) => stmt.query_map([user], job_from_row)?.collect(),
        None => stmt.query_map([], job_from_row)?.collect(),
    };
    Ok(rows?)
}

fn job_from_row(row: &Row<'_>) -> rusqlite::Result<Job> {
    Ok(Job {
        id: row.get(0)?,
        kind: row.get(1)?,
        user: row.get(2)?,
        cursor: row.get(3)?,
        status: row.get(4)?,
        error: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}
