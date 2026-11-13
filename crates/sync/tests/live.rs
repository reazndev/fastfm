//! Live sync and the now-playing state, against a fake Last.fm.

mod support;

use std::time::Duration;

use lfm_store::TimeRange;
use lfm_sync::SyncConfig;
use support::{Harness, REGISTERED, USER, eventually, now};

const ALL: TimeRange = TimeRange {
    from: 0,
    to: 1 << 40,
};

fn fast(page_limit: u32) -> SyncConfig {
    SyncConfig::default()
        .page_limit(page_limit)
        .poll_intervals(
            Duration::from_millis(10),
            Duration::from_millis(30),
            Duration::from_millis(30),
        )
        .idle_interval(Duration::from_millis(30))
        .periods_interval(Duration::from_millis(50))
}

async fn mirrors(h: &Harness) -> bool {
    h.stored(ALL).await == h.fake.listing(ALL)
}

/// Waits until the store mirrors the fake, then for a few more polls, and checks it still does.
async fn settle(h: &Harness) {
    eventually(|| mirrors(h)).await;
    let polls = h.fake.calls("user.getRecentTracks");
    eventually(|| async { h.fake.calls("user.getRecentTracks") >= polls + 3 }).await;
    h.assert_mirrors(ALL).await;
}

#[tokio::test]
async fn live_sync_appends_without_loss_or_doubling() {
    let h = Harness::with_config(&fast(1000)).await;
    h.fake
        .add(&[(REGISTERED, "old"), (REGISTERED + 1, "older")]);
    h.service.follow(USER).await.unwrap();
    settle(&h).await;

    let base = now() + 100;
    h.fake.add(&[(base, "a"), (base + 5, "b")]);
    settle(&h).await;
    // Another scrobble in the newest second already synced: it must arrive, once.
    h.fake.add(&[(base + 5, "c"), (base + 6, "d")]);
    settle(&h).await;
    h.fake.add(&[(base + 6, "e")]);
    settle(&h).await;

    let user = h.user().await;
    let cursor = h.store.user(user).await.unwrap().unwrap().sync_cursor;
    assert_eq!(cursor, base + 6);
    assert_eq!(h.stored(ALL).await.len(), 7);
    h.service.shutdown().await;
}

#[tokio::test]
async fn live_sync_catches_up_on_more_than_one_page() {
    let h = Harness::with_config(&fast(3)).await;
    h.fake.add(&[(REGISTERED, "old")]);
    h.service.follow(USER).await.unwrap();
    settle(&h).await;

    let base = now() + 100;
    let names: Vec<String> = (0..11).map(|i| format!("n{i}")).collect();
    let rows: Vec<(i64, &str)> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (base + i64::try_from(i / 2).unwrap(), n.as_str()))
        .collect();
    h.fake.add(&rows);
    settle(&h).await;
    assert_eq!(h.stored(ALL).await.len(), 12);
    h.service.shutdown().await;
}

#[tokio::test]
async fn now_playing_follows_the_last_poll() {
    let h = Harness::with_config(&fast(1000)).await;
    h.fake.add(&[(REGISTERED, "old")]);
    let user = h.service.follow(USER).await.unwrap();
    let now_playing = h.service.now_playing();
    settle(&h).await;
    assert!(now_playing.get(user).is_none());

    h.fake.with(|d| d.now_playing = Some("Believe".into()));
    eventually(|| async { now_playing.get(user).is_some() }).await;
    let playing = now_playing.get(user).unwrap();
    assert_eq!(playing.track.name, "Believe");
    assert_eq!(playing.track.artist.name, "Cher");
    assert!(playing.seen_at > 0);

    h.fake.with(|d| d.now_playing = None);
    eventually(|| async { now_playing.get(user).is_none() }).await;

    h.fake.with(|d| d.now_playing = Some("Strong".into()));
    eventually(|| async { now_playing.get(user).is_some() }).await;
    h.service.unfollow(user).await;
    assert!(now_playing.get(user).is_none(), "unfollow clears the state");
    h.service.shutdown().await;
}

#[tokio::test]
async fn follow_skips_the_import_when_one_finished() {
    let h = Harness::with_config(&fast(1000)).await;
    h.fake.add(&[(REGISTERED, "old")]);
    h.import(None).await;
    let planned = h.fake.calls("user.getInfo");
    h.service.follow(USER).await.unwrap();
    settle(&h).await;
    assert_eq!(h.fake.calls("user.getInfo"), planned, "no second import");
    h.service.shutdown().await;
}

/// Fast polls while active, an hour otherwise, so only nudges and new activity poll.
fn tiered() -> SyncConfig {
    let hour = Duration::from_secs(3600);
    SyncConfig::default()
        .poll_intervals(Duration::from_millis(10), hour, hour)
        .idle_interval(hour)
        .poll_jitter(0)
}

#[tokio::test]
async fn an_idle_user_is_polled_rarely_until_nudged() {
    let h = Harness::with_config(&tiered().nudge_after(Duration::ZERO)).await;
    h.fake.add(&[(REGISTERED, "old")]);
    let user = h.service.follow(USER).await.unwrap();
    eventually(|| mirrors(&h)).await;
    eventually(|| async { h.fake.calls("user.getRecentTracks") >= 3 }).await;
    let polls = h.fake.calls("user.getRecentTracks");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(h.fake.calls("user.getRecentTracks"), polls, "idle tier");

    assert!(h.service.nudge(user));
    eventually(|| async { h.fake.calls("user.getRecentTracks") > polls }).await;
    h.service.shutdown().await;
}

#[tokio::test]
async fn a_recent_poll_ignores_nudges() {
    let h = Harness::with_config(&tiered()).await;
    h.fake.add(&[(REGISTERED, "old")]);
    let user = h.service.follow(USER).await.unwrap();
    eventually(|| mirrors(&h)).await;
    eventually(|| async { h.fake.calls("user.getRecentTracks") >= 3 }).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!h.service.nudge(user), "polled less than a minute ago");
    assert!(!h.service.nudge(lfm_store::UserId(999)), "not followed");
    h.service.shutdown().await;
}

#[tokio::test]
async fn now_playing_moves_an_idle_user_to_the_fast_tier() {
    let h = Harness::with_config(&tiered().nudge_after(Duration::ZERO)).await;
    h.fake.add(&[(REGISTERED, "old")]);
    let user = h.service.follow(USER).await.unwrap();
    eventually(|| async { h.fake.calls("user.getRecentTracks") >= 3 }).await;
    h.fake.with(|d| d.now_playing = Some("Believe".into()));
    assert!(h.service.nudge(user));
    let polls = h.fake.calls("user.getRecentTracks");
    eventually(|| async { h.fake.calls("user.getRecentTracks") >= polls + 5 }).await;
    h.service.shutdown().await;
}

#[tokio::test]
async fn a_profile_that_turns_private_is_reported_and_retried_on_request() {
    let h = Harness::with_config(&fast(1000)).await;
    h.fake.add(&[(REGISTERED, "old")]);
    let user = h.service.follow(USER).await.unwrap();
    settle(&h).await;
    h.fake.with(|d| d.private = true);
    eventually(|| async { h.service.is_private(user) }).await;
    let refused = h.fake.calls("user.getRecentTracks");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(h.fake.calls("user.getRecentTracks"), refused, "waits a day");
    h.fake.with(|d| d.private = false);
    assert!(h.service.retry(user));
    settle(&h).await;
    assert!(!h.service.is_private(user));
    h.service.shutdown().await;
}

/// A scrobble sent late, with a time below the sync cursor, as offline scrobbling does: syntiiix
/// had a `13 Nov 16:50` scrobble on Last.fm that the mirror, cursor at 20:10, never fetched.
#[tokio::test]
async fn the_trailing_check_finds_late_scrobbles_below_the_cursor() {
    let quick = Duration::from_millis(30);
    let config = fast(1000).trailing_check(Duration::from_secs(14 * 86_400), quick, quick);
    let h = Harness::with_config(&config).await;
    let base = now() - 3_600;
    h.fake.add(&[(base, "a"), (base + 600, "b")]);
    h.service.follow(USER).await.unwrap();
    settle(&h).await;

    h.fake.add(&[(base + 300, "sent late")]);
    eventually(|| mirrors(&h)).await;
    h.service.shutdown().await;
}

#[tokio::test]
async fn reconcile_runs_on_schedule_across_restarts() {
    let config = fast(1000).reconcile_interval(Duration::from_secs(3_600));
    let h = Harness::with_config(&config).await;
    h.fake.add(&[(REGISTERED, "old")]);
    let user = h.store.ensure_user(USER).await.unwrap();
    let last = now() - 7_200;
    h.store.set_reconciled_at(user, last).await.unwrap();
    h.service.follow(USER).await.unwrap();
    eventually(|| async { h.store.reconciled_at(user).await.unwrap() > Some(last) }).await;
    h.service.shutdown().await;
}
