//! When live sync polls a user next: a tier picked by how recently they scrobbled, with jitter.

use std::hash::{BuildHasher, RandomState};
use std::time::Duration;

use crate::config::PollTiers;

/// The pause before the next poll. `active` means a track is playing or the last poll found a
/// new scrobble; `last_scrobble` is the newest known scrobble in Unix seconds.
pub(crate) fn interval(
    tiers: &PollTiers,
    active: bool,
    last_scrobble: Option<i64>,
    now: i64,
) -> Duration {
    if active {
        return tiers.playing;
    }
    let Some(last) = last_scrobble else {
        return tiers.idle;
    };
    let age = Duration::from_secs(u64::try_from(now.saturating_sub(last)).unwrap_or(0));
    if age <= tiers.recent_window {
        tiers.recent
    } else if age <= tiers.day_window {
        tiers.day
    } else {
        tiers.idle
    }
}

/// `duration` moved by a random amount of up to `percent` of it either way.
pub(crate) fn jitter(duration: Duration, percent: u32) -> Duration {
    let millis = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
    let span = millis / 100 * u64::from(percent) + millis % 100 * u64::from(percent) / 100;
    if span == 0 {
        return duration;
    }
    let offset = random() % (span * 2 + 1);
    Duration::from_millis(millis - span + offset)
}

/// A random number; `RandomState` is seeded per instance from the OS, which is plenty for
/// spreading timers apart.
fn random() -> u64 {
    RandomState::new().hash_one(std::time::Instant::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SyncConfig;

    const NOW: i64 = 1_000_000;

    #[test]
    fn tiers_follow_the_last_scrobble() {
        let tiers = SyncConfig::default().tiers;
        let at = |active, ago: Option<i64>| interval(&tiers, active, ago.map(|a| NOW - a), NOW);
        assert_eq!(at(true, Some(100_000)), Duration::from_secs(10));
        assert_eq!(at(false, Some(0)), Duration::from_secs(60));
        assert_eq!(at(false, Some(1800)), Duration::from_secs(60));
        assert_eq!(at(false, Some(1801)), Duration::from_secs(300));
        assert_eq!(at(false, Some(86_400)), Duration::from_secs(300));
        assert_eq!(at(false, Some(86_401)), Duration::from_secs(900));
        assert_eq!(at(false, None), Duration::from_secs(900));
    }

    #[test]
    fn jitter_stays_within_its_span_and_varies() {
        let base = Duration::from_secs(60);
        let samples: Vec<Duration> = (0..200).map(|_| jitter(base, 10)).collect();
        assert!(
            samples
                .iter()
                .all(|d| (Duration::from_secs(54)..=Duration::from_secs(66)).contains(d))
        );
        assert!(samples.iter().any(|d| *d != samples[0]), "not all equal");
        assert_eq!(jitter(base, 0), base);
    }
}
