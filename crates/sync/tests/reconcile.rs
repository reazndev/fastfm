//! Reconciliation against a fake Last.fm.

mod support;

use lfm_store::TimeRange;
use support::{Harness, REGISTERED};

const UNTIL: i64 = REGISTERED + 10_000;

/// Imports a history of 200 scrobbles, two per 50 seconds, up to `UNTIL`.
async fn imported(page_limit: u32) -> Harness {
    let h = Harness::new(page_limit).await;
    let names: Vec<(i64, String)> = (0..200)
        .map(|i| (REGISTERED + 50 * (i / 2), format!("s{i}")))
        .collect();
    let rows: Vec<(i64, &str)> = names.iter().map(|(t, n)| (*t, n.as_str())).collect();
    h.fake.add(&rows);
    h.import(Some(UNTIL)).await;
    h
}

#[tokio::test]
async fn a_clean_pass_compares_slices_only() {
    let h = imported(2).await;
    let user = h.user().await;
    let report = h.service.reconcile(user, None).await.unwrap();
    assert_eq!(report.range, TimeRange::new(0, UNTIL));
    assert_eq!(report.repaired, []);
    assert_eq!(report.written, 0);
    // One range before registration plus 20 slices of 512 seconds.
    assert_eq!(report.compared, 21);
    assert_eq!(report.requests, 21);
}

/// Last.fm reported 20,894 for a range whose halves reported 14,361 and 6,526, what it lists.
#[tokio::test]
async fn a_range_whose_halves_match_is_recorded_and_left_alone() {
    let h = imported(2).await;
    let user = h.user().await;
    let point = REGISTERED + 3000;
    h.fake.with(|d| d.inflated = Some((point, 500, 7)));
    let report = h.service.reconcile(user, None).await.unwrap();
    assert_eq!(report.repaired, []);
    assert_eq!(report.inconsistent.len(), 1);
    assert!((report.inconsistent[0].from..report.inconsistent[0].to).contains(&point));
    let found = h.store.upstream_mismatches(user, 0).await.unwrap();
    assert_eq!(found[0].remote, found[0].local + 7);

    let again = h.service.reconcile(user, None).await.unwrap();
    assert_eq!((again.inconsistent, again.repaired), (vec![], vec![]));
    assert_eq!(again.requests, 21, "the known range costs nothing more");
    h.assert_mirrors(TimeRange::new(0, UNTIL)).await;
}

#[tokio::test]
async fn bisects_and_repairs_upstream_deletes_and_additions() {
    let h = imported(2).await;
    let user = h.user().await;
    h.fake.with(|d| {
        d.scrobbles
            .retain(|(_, name)| name != "s40" && name != "s41");
        d.scrobbles.push((REGISTERED + 3001, "late".into()));
        d.scrobbles.push((REGISTERED + 3001, "late twin".into()));
        d.scrobbles
            .push((REGISTERED - 10, "before registration".into()));
    });
    let report = h.service.reconcile(user, None).await.unwrap();
    h.assert_mirrors(TimeRange::new(0, UNTIL)).await;
    assert_ne!(report.repaired, []);
    assert!(
        report
            .repaired
            .iter()
            .all(|r| r.to - r.from < 400 || r.from == 0),
        "repairs stay small: {:?}",
        report.repaired
    );

    let again = h.service.reconcile(user, None).await.unwrap();
    assert_eq!(again.repaired, []);
}

#[tokio::test]
async fn repairs_a_window_lost_locally() {
    let h = imported(1000).await;
    let user = h.user().await;
    let lost = TimeRange::new(REGISTERED + 2000, REGISTERED + 2600);
    h.store
        .replace_window(user, lost, Vec::new())
        .await
        .unwrap();
    assert_eq!(h.store.count_range(user, lost).await.unwrap(), 0);

    let report = h.service.reconcile(user, Some(UNTIL)).await.unwrap();
    // The repair rewrites the whole slices that differ, which hold the 24 lost scrobbles.
    assert!(report.written >= 24, "{}", report.written);
    assert!(report.repaired.iter().all(|r| r.to - r.from <= 512));
    h.assert_mirrors(TimeRange::new(0, UNTIL)).await;
}
