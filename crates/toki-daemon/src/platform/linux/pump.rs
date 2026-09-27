//! The one thread that listens to the compositor.
//!
//! It reads Hyprland's event stream and turns focus and title changes into [`Raw::Focus`].
//! The tray, logind and signal threads send through [`send`] too, so everything the run
//! loop hears still arrives on one channel. Nothing is decided here.
//!
//! As on Windows, events are timestamped where they are seen, and a title change counts only
//! for the focused window: a background tab retitling itself every second is not a switch.

use std::io::{BufRead, BufReader};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Mutex;

use chrono::Utc;
use tracing::{error, warn};

use toki_core::event::Millis;

use super::hyprland::{self, Event};
use crate::platform::{Raw, Window};

/// The run loop's channel, for every thread that has something to report.
static TX: Mutex<Option<Sender<Raw>>> = Mutex::new(None);
/// A second handle on the event stream, so [`stop`] can unblock the read.
static EVENTS: Mutex<Option<UnixStream>> = Mutex::new(None);
static STOPPING: AtomicBool = AtomicBool::new(false);

fn now() -> Millis {
    Utc::now().timestamp_millis()
}

/// Hand a fact to the run loop. A no-op before the pump starts and after it stops.
pub fn send(raw: Raw) {
    if let Some(tx) = TX.lock().ok().and_then(|tx| tx.clone()) {
        let _ = tx.send(raw);
    }
}

/// Ask the pump to shut down. Safe to call before it has started.
pub fn stop() {
    STOPPING.store(true, Ordering::Relaxed);
    if let Some(stream) = EVENTS.lock().ok().and_then(|mut s| s.take()) {
        let _ = stream.shutdown(Shutdown::Both);
    }
}

/// Follow the compositor until [`stop`] is called or the session ends.
///
/// Blocks. Intended as the body of a dedicated thread.
pub fn run(tx: Sender<Raw>) {
    if let Ok(mut slot) = TX.lock() {
        *slot = Some(tx);
    }

    let Some(stream) = hyprland::events() else {
        // Recording nothing while looking alive is the one failure a tracker must not have.
        error!("no Hyprland session found; Toki on Linux needs Hyprland for now");
        send(Raw::Stop { at: now() });
        return;
    };
    if let Ok(mut slot) = EVENTS.lock() {
        *slot = stream.try_clone().ok();
    }

    super::logind::watch();
    super::tray::spawn();

    let mut focus = Focus::default();
    // Whatever is in front when we start is the first session.
    focus.refresh();

    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else {
            break;
        };
        match hyprland::parse(&line) {
            Some(Event::Focus(_)) => focus.refresh(),
            Some(Event::Title(addr)) if hyprland::same_address(addr, &focus.address) => {
                focus.refresh()
            }
            _ => {}
        }
    }

    // The stream only ends on its own when the compositor has gone, which is the session
    // ending — the Linux `WM_ENDSESSION`.
    if !STOPPING.load(Ordering::Relaxed) {
        warn!("Hyprland closed its event stream; stopping");
        send(Raw::Stop { at: now() });
    }
    super::tray::remove();
}

/// The pump's memory of what it last reported.
#[derive(Default)]
struct Focus {
    /// The focused window's address, to tell its title changes from everyone else's.
    address: String,
    /// The last reported app and title, so a repeated event is not a new session.
    last: Option<(String, String)>,
}

impl Focus {
    /// Ask what is in front and report it if it changed.
    ///
    /// Focus on nothing — an empty workspace — reports nothing, so the session in progress
    /// runs on until idle closes it, as the desktop does on Windows.
    fn refresh(&mut self) {
        let Some(client) = hyprland::active_window() else {
            self.address.clear();
            return;
        };
        self.address = client.address.clone();
        let Some(window) = super::window::describe(&client) else {
            return;
        };
        if toki_core::ignore::is_default_ignored(&window.exe_path) {
            return;
        }
        let key = (window.exe_path.clone(), window.title.clone());
        if self.last.as_ref() == Some(&key) {
            return;
        }
        self.last = Some(key);
        send(Raw::Focus { at: now(), window });
    }
}

/// Launch the GUI, if it is installed. Spawned and forgotten: it owns its own lifetime.
///
/// Under systemd it goes into a scope of its own. Spawned plainly it would join the
/// service's cgroup, and stopping or restarting the collector would close a window the
/// user has open. `INVOCATION_ID` is how systemd tells a service it is one.
///
/// Returns whether it started, because the first-run caller gets one chance and should not
/// spend it on a failure.
pub fn open_gui() -> bool {
    let Some(path) = crate::paths::gui_path() else {
        return false;
    };
    let mut cmd = if std::env::var_os("INVOCATION_ID").is_some() {
        let mut c = Command::new("systemd-run");
        c.args(["--user", "--scope", "--quiet", "--collect"])
            .arg(&path);
        c
    } else {
        Command::new(&path)
    };
    let spawned = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        // Reaped on a thread of its own, or every closed window would leave a zombie
        // behind for the life of the daemon.
        Ok(mut child) => {
            let _ = std::thread::Builder::new()
                .name("toki-gui-reaper".into())
                .spawn(move || child.wait());
            true
        }
        Err(e) => {
            warn!(%e, path = %path.display(), "failed to spawn the GUI");
            false
        }
    }
}

/// The focused window right now. Used after a resume, when no event fires on its own.
pub fn current_foreground() -> Option<Window> {
    super::window::describe(&hyprland::active_window()?)
}
