//! GNOME's `color-scheme` key, which is what the XDG desktop portal reports to sandboxed
//! apps, and what Omarchy's theme switcher, GNOME and KDE all keep up to date.
//! `gsettings` ships with GLib, which a WebKitGTK app cannot run without.

use std::process::{Command, Stdio};

pub fn light() -> Option<bool> {
    let out = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    // `'default'` means the desktop expressed no preference, which reads as light.
    Some(!String::from_utf8_lossy(&out.stdout).contains("prefer-dark"))
}
