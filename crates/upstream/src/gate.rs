use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::oneshot;

use crate::request::Priority;

/// A priority queue of waiting requests. One request at a time holds the turn and waits for
/// a rate-limit token; when it lets go, the oldest waiter in the highest lane gets the turn.
#[derive(Debug, Default)]
pub(crate) struct Gate {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    busy: bool,
    lanes: [VecDeque<oneshot::Sender<Turn>>; Priority::ALL.len()],
}

/// The right to take the next token. Dropping it hands the turn on.
#[derive(Debug)]
pub(crate) struct Turn {
    gate: Arc<Gate>,
    armed: bool,
}

impl Gate {
    /// Waits until this request is the highest-priority waiter.
    pub(crate) async fn enter(self: &Arc<Self>, priority: Priority) -> Turn {
        let receiver = {
            let mut state = self.lock();
            if !state.busy {
                state.busy = true;
                return Turn::new(Arc::clone(self));
            }
            let (sender, receiver) = oneshot::channel();
            if let Some(lane) = state.lanes.get_mut(lane_index(priority)) {
                lane.push_back(sender);
            }
            receiver
        };
        match receiver.await {
            Ok(turn) => turn,
            // The gate drops a sender only while handing the turn to this waiter, so this arm
            // means the gate itself is gone; carrying on unordered is the safe fallback.
            Err(_) => Turn::new(Arc::clone(self)).disarmed(),
        }
    }

    fn release(self: &Arc<Self>) {
        let mut state = self.lock();
        loop {
            let next = state.lanes.iter_mut().find_map(VecDeque::pop_front);
            let Some(sender) = next else {
                state.busy = false;
                return;
            };
            // A waiter whose future was dropped returns the turn; try the next one.
            match sender.send(Turn::new(Arc::clone(self))) {
                Ok(()) => return,
                Err(turn) => drop(turn.disarmed()),
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Turn {
    fn new(gate: Arc<Gate>) -> Self {
        Self { gate, armed: true }
    }

    fn disarmed(mut self) -> Self {
        self.armed = false;
        self
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        if self.armed {
            self.gate.release();
        }
    }
}

fn lane_index(priority: Priority) -> usize {
    Priority::ALL
        .iter()
        .position(|p| *p == priority)
        .unwrap_or(Priority::ALL.len() - 1)
}
