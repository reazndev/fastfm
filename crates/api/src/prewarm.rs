//! Pre-warming: fetching `getInfo` for everything in a user's library before anyone asks.
//!
//! Ranks are walked most-played first; each rank warms that artist, album and track as JSON
//! with the user's name, which is what the user's own `getInfo` requests look up (see
//! `passthrough::overlay`). Calls go on the pre-warm lane, so client requests always go first,
//! and are spaced by the configured interval on top of that. The next rank is saved in a
//! `prewarm` job after each step, so a restart resumes where it stopped.

use std::time::Duration;

use lfm_core::Format;
use lfm_store::{
    JobId, JobStatus, JobUpdate, Kind, Page, Period, Ranked, Store, StoreError, TopQuery, UserId,
};
use tokio::task::JoinHandle;

use crate::clock::system_now;
use crate::passthrough::{Forward, Passthrough, forwarded};

/// The job kind in the store.
pub const PREWARM: &str = "prewarm";

/// Ranks read from the store at a time.
const BATCH: u32 = 100;
/// The wait after Last.fm failed, before trying the same rank again.
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// How much to warm and how fast, built by chaining setters on [`PrewarmConfig::default`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrewarmConfig {
    ranks: u64,
    interval: Duration,
}

impl Default for PrewarmConfig {
    fn default() -> Self {
        Self {
            ranks: 2_000,
            interval: Duration::from_secs(1),
        }
    }
}

impl PrewarmConfig {
    /// Warms the top `ranks` artists, albums and tracks, so at most three times as many calls.
    /// Defaults to 2,000.
    #[must_use]
    pub fn ranks(self, ranks: u64) -> Self {
        Self { ranks, ..self }
    }

    /// The pause after each upstream call. Defaults to one second.
    #[must_use]
    pub fn interval(self, interval: Duration) -> Self {
        Self { interval, ..self }
    }
}

/// Warms `user`'s library in the background, continuing an earlier unfinished run. Abort the
/// handle to stop; the job keeps its place.
#[must_use = "abort the handle to stop pre-warming"]
pub fn start(
    store: Store,
    passthrough: Passthrough,
    user: UserId,
    config: PrewarmConfig,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let warmer = Warmer {
            store,
            passthrough,
            user,
            config,
        };
        if let Err(error) = warmer.run().await {
            tracing::warn!(%error, user = user.0, "pre-warming stopped");
        }
    })
}

struct Warmer {
    store: Store,
    passthrough: Passthrough,
    user: UserId,
    config: PrewarmConfig,
}

impl Warmer {
    async fn run(&self) -> Result<(), StoreError> {
        let Some(name) = self.store.user(self.user).await?.map(|user| user.name) else {
            return Ok(());
        };
        let Some((job, mut next)) = self.job().await? else {
            return Ok(());
        };
        tracing::info!(user = %name, from = next, "pre-warming");
        while next < self.config.ranks {
            let batch = self.batch(next, &name).await?;
            if batch.is_empty() {
                break;
            }
            for requests in batch {
                self.warm_all(&requests).await;
                next += 1;
                self.save(job, next, JobStatus::Running).await?;
            }
        }
        self.save(job, next, JobStatus::Done).await?;
        tracing::info!(user = %name, ranks = next, "pre-warming done");
        Ok(())
    }

    /// The job and the next rank to warm (counted from 0), or `None` when it is done.
    async fn job(&self) -> Result<Option<(JobId, u64)>, StoreError> {
        let jobs = self.store.user_jobs(self.user).await?;
        let Some(job) = jobs.into_iter().find(|job| job.kind == PREWARM) else {
            let id = self
                .store
                .create_job(PREWARM, Some(self.user), "0", now())
                .await?;
            return Ok(Some((id, 0)));
        };
        if job.status == JobStatus::Done {
            return Ok(None);
        }
        Ok(Some((job.id, job.cursor.trim().parse().unwrap_or(0))))
    }

    async fn save(&self, job: JobId, next: u64, status: JobStatus) -> Result<(), StoreError> {
        let update = JobUpdate {
            status,
            cursor: next.to_string(),
            error: None,
        };
        self.store.update_job(job, update, now()).await
    }

    /// The requests of the ranks from `next` to the end of its store page, one list per rank.
    async fn batch(&self, next: u64, name: &str) -> Result<Vec<Vec<Forward>>, StoreError> {
        let page = Page::new(
            u32::try_from(next / u64::from(BATCH) + 1).unwrap_or(u32::MAX),
            BATCH,
        );
        let skip = usize::try_from(next % u64::from(BATCH)).unwrap_or(usize::MAX);
        let [artists, albums, tracks] = [Kind::Artist, Kind::Album, Kind::Track].map(|kind| {
            let query = TopQuery {
                kind,
                period: Period::Overall,
                now: now(),
            };
            self.store.top(self.user, query, page)
        });
        let (artists, albums, tracks) = (artists.await?, albums.await?, tracks.await?);
        let rows = artists
            .items
            .len()
            .max(albums.items.len())
            .max(tracks.items.len());
        Ok((skip..rows)
            .map(|row| {
                [
                    artists
                        .items
                        .get(row)
                        .and_then(|r| request(Kind::Artist, r, name)),
                    albums
                        .items
                        .get(row)
                        .and_then(|r| request(Kind::Album, r, name)),
                    tracks
                        .items
                        .get(row)
                        .and_then(|r| request(Kind::Track, r, name)),
                ]
                .into_iter()
                .flatten()
                .collect()
            })
            .collect())
    }

    async fn warm_all(&self, requests: &[Forward]) {
        for request in requests {
            loop {
                match self.passthrough.warm(request).await {
                    Ok(true) => tokio::time::sleep(self.config.interval).await,
                    Ok(false) => {}
                    Err(error) => {
                        tracing::debug!(%error, key = request.key(), "pre-warm call failed");
                        tokio::time::sleep(RETRY_AFTER).await;
                        continue;
                    }
                }
                break;
            }
        }
    }
}

/// The `getInfo` request for one ranked entity, as the user's own client would send it.
fn request(kind: Kind, ranked: &Ranked, user: &str) -> Option<Forward> {
    let entity = &ranked.entity;
    let artist = entity.artist.as_ref().map(|artist| artist.name.as_str());
    let params = match kind {
        Kind::Artist => vec![("artist", entity.name.as_str())],
        Kind::Album => vec![("artist", artist?), ("album", entity.name.as_str())],
        Kind::Track => vec![("artist", artist?), ("track", entity.name.as_str())],
    };
    let method = forwarded(match kind {
        Kind::Artist => "artist.getInfo",
        Kind::Album => "album.getInfo",
        Kind::Track => "track.getInfo",
    })?;
    let params = params.into_iter().chain([("username", user)]);
    Some(Forward::of(method, params, Format::Json))
}

fn now() -> i64 {
    i64::try_from(system_now()).unwrap_or(i64::MAX)
}
