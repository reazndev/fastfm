//! Users looked up by name, remembered until the next write.
//!
//! Every API request names a user, and reading the row costs a round trip to a reader thread.
//! The memo answers repeats from memory. Each answer remembers the write generation it was read
//! at; any committed write moves the generation on, so the next lookup reads again. Another
//! process writing the same file is not seen by the generation, so an answer also expires after
//! a second.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{PoisonError, RwLock};
use std::time::{Duration, Instant};

use crate::users::MirroredUser;

/// How long an answer may be served without a write in this process.
const MAX_AGE: Duration = Duration::from_secs(1);

/// More names than this and the memo starts over, so odd names cannot grow it without bound.
const MAX_NAMES: usize = 4096;

/// A valid answer from the memo: the user, or `None` when no such user is mirrored.
#[derive(Debug)]
pub(crate) struct Remembered(pub(crate) Option<MirroredUser>);

/// One remembered answer: `None` when no such user is mirrored.
#[derive(Debug, Clone)]
struct Answer {
    generation: u64,
    read_at: Instant,
    user: Option<MirroredUser>,
}

/// The write generation and the remembered answers.
#[derive(Debug, Default)]
pub(crate) struct UserMemo {
    generation: AtomicU64,
    answers: RwLock<HashMap<String, Answer>>,
}

impl UserMemo {
    /// The current write generation. Read it before the query whose answer is remembered, so
    /// a write that commits in between makes the answer stale rather than wrongly fresh.
    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Called after every committed write.
    pub(crate) fn written(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    /// The remembered answer for `name` exactly as requested, if still valid.
    pub(crate) fn get(&self, name: &str) -> Option<Remembered> {
        let answers = self.answers.read().unwrap_or_else(PoisonError::into_inner);
        let answer = answers.get(name)?;
        let valid = answer.generation == self.generation() && answer.read_at.elapsed() < MAX_AGE;
        valid.then(|| Remembered(answer.user.clone()))
    }

    /// Remembers the answer for `name`, read at `generation`.
    pub(crate) fn put(&self, name: &str, generation: u64, user: Option<MirroredUser>) {
        let mut answers = self.answers.write().unwrap_or_else(PoisonError::into_inner);
        if answers.len() >= MAX_NAMES {
            answers.clear();
        }
        let answer = Answer {
            generation,
            read_at: Instant::now(),
            user,
        };
        answers.insert(name.to_owned(), answer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_or_a_later_generation_hides_the_answer() {
        let memo = UserMemo::default();
        let read_at = memo.generation();
        memo.put("a", read_at, None);
        assert!(memo.get("a").is_some_and(|answer| answer.0.is_none()));
        assert!(memo.get("A").is_none(), "names are remembered as requested");
        memo.written();
        assert!(memo.get("a").is_none());
        // An answer read before a write that committed meanwhile is never served.
        memo.put("b", read_at, None);
        assert!(memo.get("b").is_none());
    }
}
