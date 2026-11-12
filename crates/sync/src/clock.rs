//! Wall-clock time in Unix seconds.

use std::time::{SystemTime, UNIX_EPOCH};

/// The current Unix time; 0 if the clock is before 1970.
pub(crate) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
