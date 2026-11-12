//! The time methods see as "now".

use std::time::{SystemTime, UNIX_EPOCH};

/// The request clock: the system time, or a fixed time for comparing against fixtures that were
/// recorded at a known moment (`LFM_NOW`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Clock {
    /// The system time.
    #[default]
    System,
    /// Always this Unix time.
    Fixed(u64),
}

impl Clock {
    /// The current Unix time in seconds.
    #[must_use]
    pub fn now(self) -> u64 {
        match self {
            Self::System => system_now(),
            Self::Fixed(now) => now,
        }
    }
}

/// The system time in Unix seconds, whatever the request clock says.
#[must_use]
pub fn system_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
