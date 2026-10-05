//! Hyprland's two IPC sockets.
//!
//! `.socket2.sock` streams one line per event, `NAME>>DATA`, to anyone connected — the
//! equivalent of the WinEvent hooks, minus any filtering, so it is filtered here.
//! `.socket.sock` answers one request per connection, which is how the window an event is
//! about gets described: events carry an address and sometimes a title, but never the pid.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

/// The focused window, as `j/activewindow` reports it.
#[derive(Debug, Clone, Deserialize)]
pub struct Client {
    pub address: String,
    pub class: String,
    pub title: String,
    pub pid: i32,
}

/// What an event line means to the collector. Everything else Hyprland says is ignored.
#[derive(Debug, PartialEq, Eq)]
pub enum Event<'a> {
    /// Focus moved to the window at this address, or to nothing when it is empty.
    Focus(&'a str),
    /// The window at this address changed its title.
    Title(&'a str),
}

/// Parse one line of `.socket2.sock`.
///
/// Only the `v2` events: they carry the window address, which is the one thing that tells
/// a title change on the focused window apart from one on a window in the background.
/// Addresses arrive without the `0x` that `j/activewindow` puts on them, so both are
/// compared through [`same_address`].
pub fn parse(line: &str) -> Option<Event<'_>> {
    let (name, data) = line.split_once(">>")?;
    match name {
        "activewindowv2" => Some(Event::Focus(data.trim())),
        "windowtitlev2" => Some(Event::Title(data.split_once(',')?.0)),
        _ => None,
    }
}

/// Whether two spellings of a window address name the same window.
pub fn same_address(a: &str, b: &str) -> bool {
    let strip = |s: &str| s.trim().trim_start_matches("0x").to_ascii_lowercase();
    !a.trim().is_empty() && strip(a) == strip(b)
}

/// The running Hyprland's socket directory.
///
/// From `HYPRLAND_INSTANCE_SIGNATURE` when the environment has it. A daemon started some
/// other way may not — so failing that, the most recently started instance under
/// `$XDG_RUNTIME_DIR/hypr`, which on a single-seat machine is the only live one.
pub fn socket_dir() -> Option<PathBuf> {
    let base = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?).join("hypr");
    if let Some(sig) = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE") {
        return Some(base.join(sig));
    }
    std::fs::read_dir(&base)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join(".socket2.sock").exists())
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
}

/// Subscribe to the event stream.
pub fn events() -> Option<UnixStream> {
    UnixStream::connect(socket_dir()?.join(".socket2.sock")).ok()
}

/// The focused window right now, or `None` when nothing has focus — Hyprland answers `{}`
/// for an empty workspace, which fails to parse into a [`Client`] and lands here too.
pub fn active_window() -> Option<Client> {
    let mut socket = UnixStream::connect(socket_dir()?.join(".socket.sock")).ok()?;
    // A wedged compositor must not take the pump down with it.
    socket.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    socket.write_all(b"j/activewindow").ok()?;
    let mut reply = Vec::new();
    socket.read_to_end(&mut reply).ok()?;
    let client: Client = serde_json::from_slice(&reply).ok()?;
    (client.pid > 0).then_some(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_and_title_events_are_recognised() {
        assert_eq!(
            parse("activewindowv2>>615815d8c1f0"),
            Some(Event::Focus("615815d8c1f0"))
        );
        // Titles can contain commas; only the first one separates the address.
        assert_eq!(
            parse("windowtitlev2>>615815d8c1f0,Inbox (3), Mail — Zen Browser"),
            Some(Event::Title("615815d8c1f0"))
        );
    }

    #[test]
    fn focus_on_nothing_is_an_empty_address() {
        assert_eq!(parse("activewindowv2>>"), Some(Event::Focus("")));
        assert!(!same_address("", ""));
    }

    #[test]
    fn other_events_and_v1_duplicates_are_ignored() {
        assert_eq!(parse("activewindow>>zen,Home — Zen Browser"), None);
        assert_eq!(parse("workspace>>2"), None);
        assert_eq!(parse("garbage"), None);
    }

    #[test]
    fn addresses_compare_with_or_without_the_prefix() {
        assert!(same_address("0x615815d8c1f0", "615815d8c1f0"));
        assert!(!same_address("0x615815d8c1f0", "615815d8c1f1"));
    }

    #[test]
    fn an_empty_workspace_is_not_a_window() {
        assert!(serde_json::from_str::<Client>("{}").is_err());
    }
}
