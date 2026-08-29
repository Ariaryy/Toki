use serde::{Deserialize, Serialize};

/// Milliseconds since the Unix epoch, UTC.
pub type Millis = i64;

/// Stable identity for an application: the full executable path.
///
/// The platform layer is responsible for handing over the *real* process, not the host —
/// every UWP app reports as `ApplicationFrameHost.exe` until its child windows are walked.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AppKey(pub String);

impl AppKey {
    pub fn new(exe_path: impl Into<String>) -> Self {
        Self(exe_path.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Which rule kept a session alive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Presence {
    /// Kept alive by keyboard or mouse input throughout.
    Input,
    /// Spent part of its life idle, with the foreground process rendering audio.
    ///
    /// Recorded so the audio exemption can be audited when a total looks wrong.
    AudioOnly,
}

/// Discrete events from the platform.
///
/// Presence sampling is deliberately *not* here — see [`Tracker::on_tick`], which takes a
/// probe closure so the WASAPI check only runs at the moment it can change the outcome.
///
/// [`Tracker::on_tick`]: crate::tracker::Tracker::on_tick
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Foreground window changed, or its title changed. Both arrive here; the tracker
    /// treats them identically because a browser tab switch is only ever a title change.
    Focus {
        at: Millis,
        app: AppKey,
        title: String,
    },
    /// `PBT_APMSUSPEND`. Fires before sleep, so `at` is trustworthy.
    Suspend {
        at: Millis,
    },
    /// `WTS_SESSION_LOCK`, screensaver, or fast user switch.
    Lock {
        at: Millis,
    },
    PauseStart {
        at: Millis,
    },
    PauseEnd {
        at: Millis,
    },
    /// Clean daemon shutdown. Crash recovery is the store's job, using the heartbeat.
    Shutdown {
        at: Millis,
    },
}
