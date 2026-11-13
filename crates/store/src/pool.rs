//! The `Store` handle and its worker threads.
//!
//! One thread owns the writer connection. A pool of threads, each owning one read-only connection
//! with its own prepared-statement cache, takes read jobs from a shared queue. Async methods send
//! a closure to a thread and await its answer on a oneshot channel, so the tokio runtime never
//! blocks on SQLite.

use std::collections::HashMap;
use std::num::NonZero;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use crossbeam_channel::{Receiver, Sender};
use rusqlite::{Connection, Transaction, TransactionBehavior};
use tokio::sync::oneshot;

use crate::db::{self, Tuning};
use crate::error::{Result, StoreError};
use crate::memo::UserMemo;

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

/// Handle to the database. Cheap to clone; every clone shares the same threads.
#[derive(Clone, Debug)]
pub struct Store {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    writer: Sender<Job>,
    readers: Sender<Job>,
    /// Last-used times of API keys, written in one batch by `flush_key_usage`.
    key_usage: Mutex<HashMap<String, i64>>,
    /// Users by name, valid until the next write.
    users: UserMemo,
}

impl Store {
    /// Opens or creates the database file at `path`, with one reader per CPU and the default
    /// [`Tuning`].
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with(path, default_readers(), Tuning::default())
    }

    /// Opens or creates the database file at `path` with `readers` read connections.
    pub fn open_with_readers(path: &Path, readers: usize) -> Result<Self> {
        Self::open_with(path, readers, Tuning::default())
    }

    /// Opens or creates the database file at `path` with `readers` read connections, each
    /// connection set up by `tuning`.
    pub fn open_with(path: &Path, readers: usize, tuning: Tuning) -> Result<Self> {
        let writer = spawn("lfm-store-write", db::open_writer(path, tuning)?)?;
        let (sender, receiver) = crossbeam_channel::unbounded();
        for index in 0..readers.max(1) {
            let conn = db::open_reader(path, tuning)?;
            run(&format!("lfm-store-read-{index}"), conn, receiver.clone())?;
        }
        Ok(Self::from_senders(writer, sender))
    }

    /// Opens a private in-memory database for tests. Reads share the writer connection.
    pub fn open_in_memory() -> Result<Self> {
        let writer = spawn("lfm-store-memory", db::open_memory()?)?;
        Ok(Self::from_senders(writer.clone(), writer))
    }

    fn from_senders(writer: Sender<Job>, readers: Sender<Job>) -> Self {
        let key_usage = Mutex::default();
        Self {
            inner: Arc::new(Inner {
                writer,
                readers,
                key_usage,
                users: UserMemo::default(),
            }),
        }
    }

    pub(crate) fn key_usage(&self) -> &Mutex<HashMap<String, i64>> {
        &self.inner.key_usage
    }

    pub(crate) fn user_memo(&self) -> &UserMemo {
        &self.inner.users
    }

    /// Runs `f` on a read connection inside one read transaction, so every query in `f` sees
    /// the same snapshot. One transaction also saves the WAL lock round trips that each
    /// statement would otherwise make on its own.
    pub(crate) async fn read<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        submit(&self.inner.readers, move |conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
            let value = f(&tx)?;
            tx.commit()?;
            Ok(value)
        })
        .await
    }

    /// Runs `f` inside one immediate transaction on the writer, and commits if it succeeds.
    /// A commit moves the user memo's generation on before the caller hears back.
    pub(crate) async fn write<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> Result<T> + Send + 'static,
    {
        let inner = Arc::clone(&self.inner);
        submit(&self.inner.writer, move |conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let value = f(&tx)?;
            tx.commit()?;
            inner.users.written();
            Ok(value)
        })
        .await
    }
}

/// One reader per CPU.
#[must_use]
pub fn default_readers() -> usize {
    thread::available_parallelism().map_or(4, NonZero::get)
}

async fn submit<T, F>(queue: &Sender<Job>, f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
{
    let (reply, answer) = oneshot::channel();
    let job: Job = Box::new(move |conn| {
        // The caller may have gone away; its answer is then not needed.
        let _ = reply.send(f(conn));
    });
    queue.send(job).map_err(|_| StoreError::Closed)?;
    answer.await.map_err(|_| StoreError::Closed)?
}

fn spawn(name: &str, conn: Connection) -> Result<Sender<Job>> {
    let (sender, receiver) = crossbeam_channel::unbounded();
    run(name, conn, receiver)?;
    Ok(sender)
}

fn run(name: &str, mut conn: Connection, jobs: Receiver<Job>) -> Result<()> {
    thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            while let Ok(job) = jobs.recv() {
                job(&mut conn);
            }
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Store;

    #[test]
    fn store_is_shareable() {
        fn shareable<T: Clone + Send + Sync + 'static>() {}
        shareable::<Store>();
    }
}
