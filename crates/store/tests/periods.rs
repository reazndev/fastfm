//! Per-period totals: building, ingest into a period, moving forward by days, and fallback.
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test the same way the tests do"
)]

mod common;

use common::{
    DAY, Rng, T0, TestDb, assert_aggregates_match, assert_periods_match, loaded, naive_top,
    random_scrobbles,
};
use lfm_store::{Kind, Listing, Page, Period, Ranked, Store, TimeRange, TopQuery, UserId};
use rusqlite::Connection;

async fn assert_lists_match(store: &Store, conn: &Connection, user: UserId, now: i64) {
    for period in Period::ALL {
        for kind in Kind::ALL {
            let want = naive_top(conn, user, kind, period.range(now));
            for page in [Page::new(1, 10), Page::new(3, 7), Page::new(1, 10_000)] {
                let query = TopQuery { kind, period, now };
                let got: Listing<Ranked> = store.top(user, query, page).await.unwrap();
                let got_ids: Vec<_> = got.items.iter().map(|r| (r.entity.id, r.plays)).collect();
                let skip = usize::try_from(page.offset()).unwrap_or(0);
                let limit = usize::try_from(page.limit).unwrap_or(0);
                let expected: Vec<_> = want.iter().copied().skip(skip).take(limit).collect();
                assert_eq!(got_ids, expected, "{period:?} {kind:?} {page:?}");
                assert_eq!(got.total, u64::try_from(want.len()).unwrap());
            }
        }
    }
}

#[tokio::test]
async fn period_lists_match_before_and_after_building() {
    let db = TestDb::new();
    let alice = loaded(&db).await;
    let conn = db.raw();
    let now = T0 + 700 * DAY + 3_600;
    assert_lists_match(&db.store, &conn, alice, now).await;

    let version = db.store.user(alice).await.unwrap().unwrap().version;
    db.store.advance_periods(now).await.unwrap();
    assert!(db.store.user(alice).await.unwrap().unwrap().version > version);
    assert_periods_match(&conn);
    assert_lists_match(&db.store, &conn, alice, now).await;
}

#[tokio::test]
async fn ingest_and_days_passing_keep_periods_exact() {
    let db = TestDb::new();
    let alice = loaded(&db).await;
    let conn = db.raw();
    let mut now = T0 + 600 * DAY + 50_000;
    db.store.advance_periods(now).await.unwrap();
    let mut rng = Rng(5);
    for step in 0..12 {
        let from = now - i64::try_from(rng.below(400)).unwrap() * DAY;
        let window = TimeRange::new(from, from + DAY / 2);
        let plays = random_scrobbles(&mut rng, window.from, window.to, 30);
        db.store.replace_window(alice, window, plays).await.unwrap();
        now += [0, DAY / 3, DAY, 3 * DAY, 40 * DAY][step % 5];
        db.store.advance_periods(now).await.unwrap();
        assert_periods_match(&conn);
    }
    assert_aggregates_match(&conn);
    assert_lists_match(&db.store, &conn, alice, now).await;

    now += 500 * DAY;
    db.store.advance_periods(now).await.unwrap();
    assert_periods_match(&conn);
    db.store.advance_periods(now - 20 * DAY).await.unwrap();
    assert_periods_match(&conn);
}
