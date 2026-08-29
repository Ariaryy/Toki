//! Pause state shared between the collector and the IPC server.
//!
//! The collector is the only writer — it already decides when a pause starts, ends, or
//! times out on its own tick — so this is a read side for `get_state`, not a second copy of
//! the pause logic.

use std::sync::Mutex;

use toki_core::event::Millis;

#[derive(Debug, Clone, Copy, Default)]
pub struct PauseState {
    pub paused: bool,
    /// When a timed pause ends. `None` while running, and also while paused indefinitely.
    pub until: Option<Millis>,
}

#[derive(Default)]
pub struct Shared(Mutex<PauseState>);

impl Shared {
    pub fn get(&self) -> PauseState {
        *self.0.lock().unwrap()
    }

    pub fn set(&self, state: PauseState) {
        *self.0.lock().unwrap() = state;
    }
}
