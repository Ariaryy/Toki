//! Turning a Hyprland client into the thing a person would name.
//!
//! The Windows problems have Linux twins:
//!
//! 1. Shared runtimes. Every system-Electron app on Arch reports `/usr/lib/electron*/electron`,
//!    every Omarchy web app reports Chromium, and an AppImage reports a fresh `/tmp/.mount_*`
//!    path each launch. For those the window's app id is the stable identity, not the exe.
//! 2. No version resource. The name a person knows is in the app's `.desktop` file.

use std::collections::HashMap;
use std::path::PathBuf;

use toki_core::ignore::exe_name;

use super::hyprland::Client;
use crate::platform::Window;

/// Describe a Hyprland client, or `None` if its process is already gone.
pub fn describe(client: &Client) -> Option<Window> {
    let pid = u32::try_from(client.pid).ok()?;
    let exe = exe_path_of(pid)?;
    Some(Window {
        pid,
        exe_path: app_key(&exe, &client.class),
        title: client.title.clone(),
        class: client.class.clone(),
    })
}

/// The executable behind a process.
///
/// A binary replaced by a package upgrade while running reads back with ` (deleted)` on the
/// end, which would make one app into two the day it was updated.
pub fn exe_path_of(pid: u32) -> Option<String> {
    let path = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let path = path.to_string_lossy();
    Some(path.strip_suffix(" (deleted)").unwrap_or(&path).to_string())
}

/// What a window's time is recorded against: its executable, unless that is shared with
/// unrelated apps, in which case its app id.
fn app_key(exe: &str, class: &str) -> String {
    if class.is_empty() || !(is_shared_runtime(exe) || web_app_host(class).is_some()) {
        exe.to_string()
    } else {
        class.to_string()
    }
}

/// Executables that host many different apps.
fn is_shared_runtime(exe: &str) -> bool {
    let name = exe_name(exe);
    exe.starts_with("/tmp/.mount_")
        || name.starts_with("electron")
        || name.starts_with("python")
        || matches!(name.as_str(), "java" | "node" | "gjs" | "bwrap")
}

/// The site behind a Chromium `--app` window, whose app id Chromium spells
/// `chrome-<host>__<path>-<profile>`. This is how every Omarchy web app is launched.
fn web_app_host(class: &str) -> Option<&str> {
    let rest = class.strip_prefix("chrome-")?;
    let host = rest.split("__").next()?;
    (host.contains('.') && !host.contains('/')).then_some(host)
}

/// The app's name from its `.desktop` file, found by — in order of how sure the match is —
/// file name equal to the app id, `StartupWMClass`, the web app's URL, or the executable.
///
/// Read once per app per run and cached in the database, because it reads a directory of
/// small files.
pub fn display_name(window: &Window) -> Option<String> {
    let entries = desktop_entries();
    let class = window.class.to_ascii_lowercase();
    let exe = exe_path_of(window.pid).unwrap_or_else(|| window.exe_path.clone());
    let host = web_app_host(&window.class);

    let found = entries
        .iter()
        .find(|e| !class.is_empty() && e.id.to_ascii_lowercase() == class)
        .or_else(|| {
            entries
                .iter()
                .find(|e| !class.is_empty() && e.wm_class.to_ascii_lowercase() == class)
        })
        .or_else(|| {
            let host = host?;
            entries
                .iter()
                .find(|e| e.exec.contains(&format!("://{host}")))
        })
        .or_else(|| {
            let program = exe_name(&exe);
            entries.iter().find(|e| {
                let first = e.exec.split_whitespace().next().unwrap_or("");
                first == exe || exe_name(first.trim_matches('"')) == program
            })
        });
    found
        .map(|e| e.name.clone())
        .or_else(|| host.map(str::to_string))
}

#[derive(Debug, Default)]
struct DesktopEntry {
    /// File name without `.desktop`.
    id: String,
    name: String,
    exec: String,
    wm_class: String,
}

/// Every application entry, user ones first so they shadow the system's — the same order
/// launchers use. Hidden and `NoDisplay` entries still count: they are often exactly the
/// ones that name a helper window.
fn desktop_entries() -> Vec<DesktopEntry> {
    let mut dirs = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match std::env::var_os("XDG_DATA_HOME") {
        Some(d) => dirs.push(PathBuf::from(d)),
        None => dirs.extend(home.map(|h| h.join(".local/share"))),
    }
    let system = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.extend(system.split(':').map(PathBuf::from));

    let mut seen = HashMap::new();
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(files) = std::fs::read_dir(dir.join("applications")) else {
            continue;
        };
        for path in files.flatten().map(|f| f.path()) {
            let Some(id) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".desktop"))
            else {
                continue;
            };
            if seen.insert(id.to_string(), ()).is_some() {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Some(entry) = parse_entry(id, &text) {
                    out.push(entry);
                }
            }
        }
    }
    out
}

/// The `[Desktop Entry]` group's unlocalised keys. Actions and other groups come after it
/// and carry their own `Name=`, which must not overwrite the app's.
fn parse_entry(id: &str, text: &str) -> Option<DesktopEntry> {
    let mut entry = DesktopEntry {
        id: id.to_string(),
        ..Default::default()
    };
    let mut in_main = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let value = value.trim().to_string();
            match key.trim() {
                "Name" => entry.name = value,
                "Exec" => entry.exec = value,
                "StartupWMClass" => entry.wm_class = value,
                _ => {}
            }
        }
    }
    (!entry.name.is_empty()).then_some(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ordinary_app_is_its_executable() {
        assert_eq!(
            app_key("/opt/zen-browser-bin/zen-bin", "zen"),
            "/opt/zen-browser-bin/zen-bin"
        );
    }

    #[test]
    fn apps_sharing_a_runtime_are_told_apart_by_app_id() {
        assert_eq!(app_key("/usr/lib/electron37/electron", "code"), "code");
        assert_eq!(
            app_key("/tmp/.mount_Obsidabc/obsidian", "obsidian"),
            "obsidian"
        );
        // Without an app id there is nothing better than the executable.
        assert_eq!(
            app_key("/usr/lib/electron37/electron", ""),
            "/usr/lib/electron37/electron"
        );
    }

    #[test]
    fn web_apps_are_their_own_app_but_the_browser_is_not() {
        let class = "chrome-web.whatsapp.com__-Default";
        assert_eq!(web_app_host(class), Some("web.whatsapp.com"));
        assert_eq!(app_key("/usr/lib/chromium/chromium", class), class);
        assert_eq!(web_app_host("chromium"), None);
        assert_eq!(
            app_key("/usr/lib/chromium/chromium", "chromium"),
            "/usr/lib/chromium/chromium"
        );
    }

    #[test]
    fn only_the_main_group_names_the_app() {
        let text = "[Desktop Entry]\nName=Zen Browser\nName[de]=Zen\nExec=/opt/zen-browser-bin/zen-bin %u\nStartupWMClass=zen\n\n[Desktop Action new-window]\nName=New Window\n";
        let e = parse_entry("zen", text).unwrap();
        assert_eq!(e.name, "Zen Browser");
        assert_eq!(e.wm_class, "zen");
        assert!(e.exec.starts_with("/opt/zen-browser-bin/zen-bin"));
    }
}
