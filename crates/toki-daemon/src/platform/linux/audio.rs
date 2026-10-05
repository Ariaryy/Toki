//! Whether the app you are looking at is playing sound.
//!
//! Asked only once input silence has crossed the idle threshold, a handful of times a day,
//! so shelling out to `pactl` costs nothing worth a PulseAudio binding. It is present
//! wherever PipeWire's Pulse server is, which is every desktop distribution's default.
//!
//! The comparison is by executable, not process id, for the same reason as on Windows:
//! Chromium plays audio from a utility process that is not the window's. That also means a
//! Chromium web app is credited with any Chromium tab's sound — an over-count bounded by the
//! audio cap, and the right way to fail for the YouTube web app this exists for.

use std::process::{Command, Stdio};

use serde::Deserialize;

use super::window::exe_path_of;
use crate::platform::Window;

/// Nothing to initialise. Exists so the run loop reads the same on both platforms.
pub struct Com;

impl Com {
    pub fn init() -> Self {
        Com
    }
}

#[derive(Deserialize)]
struct SinkInput {
    corked: bool,
    properties: Properties,
}

#[derive(Deserialize)]
struct Properties {
    #[serde(rename = "application.process.id")]
    pid: Option<String>,
}

/// Is any uncorked playback stream owned by the window's executable?
///
/// Every failure answers "no" — no `pactl`, no sound server, unparseable output — which
/// ends an idle session at the last input, the behaviour without the exemption at all.
pub fn is_rendering(window: &Window) -> bool {
    let Some(target) = exe_path_of(window.pid) else {
        return false;
    };
    let Ok(out) = Command::new("pactl")
        .args(["-f", "json", "list", "sink-inputs"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    let Ok(streams) = serde_json::from_slice::<Vec<SinkInput>>(&out.stdout) else {
        return false;
    };
    streams
        .iter()
        .filter(|s| !s.corked)
        .filter_map(|s| s.properties.pid.as_deref()?.parse::<u32>().ok())
        .any(|pid| exe_path_of(pid).is_some_and(|exe| exe == target))
}
