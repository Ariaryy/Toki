//! Everything that knows Linux exists. The Windows side is `super::win`.
//!
//! Linux has no one answer to "which window is in front" — under Wayland that belongs to
//! the compositor, and each one exposes it differently. This collector speaks Hyprland's
//! IPC, which is what [`hyprland`] wraps. The rest leans on freedesktop standards that hold
//! on any distribution: idle time from the `ext-idle-notify` Wayland protocol, sleep and
//! lock from logind, the tray from StatusNotifierItem, audio from PipeWire's Pulse server.
//!
//! A second compositor means a second module beside [`hyprland`] answering the same two
//! questions — the focused window, and when it changes — and nothing else here moving.

pub mod audio;
pub mod autostart;
pub mod idle;
pub mod ipc;
pub mod memory;
pub mod pump;
pub mod single;
pub mod tray;
pub mod window;

mod hyprland;
mod logind;

/// The terminal, and signals, so a daemon run by hand or by systemd closes its session
/// cleanly instead of leaving it for crash recovery.
pub mod console {
    use std::io::IsTerminal;

    use chrono::Utc;
    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
    use signal_hook::iterator::Signals;

    use crate::platform::Raw;

    /// Whether anyone is reading stderr. There is nothing to let go of: unlike Windows, a
    /// daemon started by systemd or a login script is never handed a terminal of its own.
    pub fn detach_if_ours() -> bool {
        std::io::stderr().is_terminal()
    }

    /// Never. A bare `toki-daemon` from a launcher should print usage rather than guess,
    /// because autostart always passes `run`.
    pub fn is_ours() -> bool {
        false
    }

    /// Ctrl+C, `systemctl stop`, and a closed terminal all mean the same thing: stop now.
    /// The signal is turned into the same `Raw::Stop` a session ending sends, so the run
    /// loop closes the open session at the moment it was asked to.
    pub fn trap() {
        let Ok(mut signals) = Signals::new([SIGINT, SIGTERM, SIGHUP]) else {
            return;
        };
        let _ = std::thread::Builder::new()
            .name("toki-signals".into())
            .spawn(move || {
                if signals.forever().next().is_some() {
                    super::pump::send(Raw::Stop {
                        at: Utc::now().timestamp_millis(),
                    });
                }
            });
    }
}
