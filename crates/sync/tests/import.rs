//! Full-history import against a fake Last.fm.

mod support;

use lfm_store::{JobStatus, JobUpdate, Page, TimeRange};
use lfm_sync::SyncConfig;
use support::{Harness, REGISTERED, USER, eventually};

const ALL: TimeRange = TimeRange {
    from: 0,
    to: 1 << 40,
};

/// A history where several scrobbles share seconds, some of them on page boundaries.
fn crowded_history() -> Vec<(i64, String)> {
    let copies = [1, 2, 3, 1, 4];
    (0..60)
        .zip(copies.into_iter().cycle())
        .flat_map(|(second, copies)| {
            (0..copies).map(move |copy| (REGISTERED + second * 100, format!("t{second}-{copy}")))
        })
        .collect()
}

fn add_all(h: &Harness, rows: &[(i64, String)]) {
    let refs: Vec<(i64, &str)> = rows.iter().map(|(t, n)| (*t, n.as_str())).collect();
    h.fake.add(&refs);
}

#[tokio::test]
async fn import_never_splits_a_second_across_windows() {
    for limit in [1, 2, 3, 4, 5, 7, 1000] {
        let h = Harness::new(limit).await;
        add_all(&h, &crowded_history());
        let progress = h.service.import(USER, None).await.unwrap();
        assert_eq!(progress.status, JobStatus::Done, "limit {limit}");
        h.assert_mirrors(ALL).await;
        assert_eq!(progress.estimated_total, 132);
        assert_eq!(progress.imported, 132);
    }
}

#[tokio::test]
async fn same_second_duplicates_larger_than_a_page_are_kept() {
    let h = Harness::new(3).await;
    let names: Vec<String> = (0..8).map(|i| format!("dup{i}")).collect();
    let mut rows: Vec<(i64, &str)> = names.iter().map(|n| (2_000_000, n.as_str())).collect();
    rows.extend([(1_999_999, "before"), (2_000_001, "after")]);
    h.fake.add(&rows);
    h.service.import(USER, None).await.unwrap();
    h.assert_mirrors(ALL).await;
    let user = h.user().await;
    let second = TimeRange::new(2_000_000, 2_000_001);
    assert_eq!(h.store.count_range(user, second).await.unwrap(), 8);
}

#[tokio::test]
async fn import_with_cutoff_stops_before_until() {
    let h = Harness::new(4).await;
    add_all(&h, &crowded_history());
    let until = REGISTERED + 3000;
    h.fake
        .with(|d| d.loved = vec![(until - 1, "t1-0".into()), (until, "t2-0".into())]);
    let progress = h.service.import(USER, Some(until)).await.unwrap();
    assert_eq!(progress.until, until);
    h.assert_mirrors(TimeRange::new(0, until)).await;
    let stored = h.stored(ALL).await;
    assert!(stored.iter().all(|(uts, _)| *uts < until));
    let user = h.user().await;
    let loved = h.store.loved_tracks(user, Page::new(1, 10)).await.unwrap();
    assert_eq!(loved.total, 1, "loves after the cutoff are left out");
    let mirrored = h.store.user(user).await.unwrap().unwrap();
    assert_eq!(mirrored.sync_cursor, until);
    assert_eq!(mirrored.profile.registered, REGISTERED);
    let charts = h.store.weekly_charts(user).await.unwrap();
    assert_eq!(
        charts,
        vec![TimeRange::new(1000, 605_800)],
        "weeks ending after it too"
    );
}

#[tokio::test]
async fn failed_import_resumes_where_it_stopped() {
    let config = SyncConfig::default().page_limit(5).import_concurrency(1);
    let h = Harness::with_config(config).await;
    add_all(&h, &crowded_history());
    // Planning takes one call with a single worker; let four windows through, then fail.
    h.fake.with(|d| d.fail_after = Some(5));
    let err = h.service.import(USER, None).await.unwrap_err();
    assert!(err.to_string().contains("Operation failed"), "{err}");
    let failed = h.service.progress(h.user().await).await.unwrap().unwrap();
    assert_eq!(failed.status, JobStatus::Failed);
    assert!(
        failed.imported > 0 && failed.imported < 132,
        "{}",
        failed.imported
    );
    assert!(failed.error.is_some());

    h.fake.with(|d| d.fail_after = None);
    let calls_before = h.fake.calls("user.getRecentTracks");
    let done = h.service.import(USER, None).await.unwrap();
    assert_eq!(done.job, failed.job, "the same job continues");
    assert_eq!(done.status, JobStatus::Done);
    assert_eq!(done.imported, 132);
    h.assert_mirrors(ALL).await;
    let resumed_calls = h.fake.calls("user.getRecentTracks") - calls_before;
    assert!(
        resumed_calls < 132 / 4,
        "resume skipped finished windows: {resumed_calls}"
    );
}

#[tokio::test]
async fn start_resumes_an_interrupted_import() {
    let config = SyncConfig::default().page_limit(5).import_concurrency(3);
    let h = Harness::with_config(config).await;
    add_all(&h, &crowded_history());
    let until = REGISTERED + 10_000;
    h.fake.with(|d| d.fail_after = Some(6));
    h.service.import(USER, Some(until)).await.unwrap_err();
    let user = h.user().await;
    let job = h
        .store
        .job(h.service.progress(user).await.unwrap().unwrap().job);
    let job = job.await.unwrap().unwrap();
    // Make the job look like the process died while it ran.
    let update = JobUpdate {
        status: JobStatus::Running,
        cursor: job.cursor,
        error: None,
    };
    h.store.update_job(job.id, update, 0).await.unwrap();

    h.fake.with(|d| d.fail_after = None);
    h.service.start().await.unwrap();
    eventually(|| async {
        let progress = h.service.progress(user).await.unwrap().unwrap();
        progress.status == JobStatus::Done
    })
    .await;
    h.service.shutdown().await;
    h.assert_mirrors(ALL).await;
}

#[tokio::test]
async fn a_private_profile_fails_the_import_with_a_clear_reason() {
    let h = Harness::new(5).await;
    add_all(&h, &crowded_history());
    h.fake.with(|data| data.private = true);
    let error = h.service.import(USER, None).await.unwrap_err();
    assert!(error.is_private_profile(), "{error}");
    let progress = h.service.progress(h.user().await).await.unwrap().unwrap();
    assert_eq!(progress.status, JobStatus::Failed);
    assert_eq!(progress.error.as_deref(), Some(lfm_sync::PRIVATE_MESSAGE));
    assert!(progress.is_private());
}

#[tokio::test]
async fn a_private_profile_is_retried_on_request_and_not_again_after_a_restart() {
    let h = Harness::new(5).await;
    add_all(&h, &crowded_history());
    h.fake.with(|data| data.private = true);
    let user = h.service.follow(USER).await.unwrap();
    eventually(|| async { h.service.is_private(user) }).await;
    let refused = h.fake.calls("user.getRecentTracks");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(h.fake.calls("user.getRecentTracks"), refused, "waits a day");
    h.service.shutdown().await;

    // A restart within the day does not ask Last.fm again.
    let unlimited = std::num::NonZeroU32::new(100_000).unwrap();
    let config = SyncConfig::default()
        .page_limit(5)
        .request_budgets(unlimited, unlimited);
    let again = lfm_sync::SyncService::new(
        h.store.clone(),
        std::sync::Arc::new(support::client(&h.server)),
        config,
    );
    again.follow(USER).await.unwrap();
    eventually(|| async { again.is_private(user) }).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(h.fake.calls("user.getRecentTracks"), refused);

    h.fake.with(|data| data.private = false);
    assert!(again.retry(user));
    eventually(|| async {
        let progress = again.progress(user).await.unwrap().unwrap();
        progress.status == JobStatus::Done
    })
    .await;
    assert!(!again.is_private(user));
    again.shutdown().await;
    h.assert_mirrors(ALL).await;
}
