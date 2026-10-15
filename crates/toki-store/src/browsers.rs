//! Where the browsers keep their per-profile databases.
//!
//! Shared by the daemon, which reads history, and the GUI, which reads favicons, so the list
//! of browsers and the way profiles are found live in one place.

use std::path::{Path, PathBuf};

use toki_core::resolve::Engine;

/// One browser database found on this machine: a profile's history, or its favicons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub label: &'static str,
    pub engine: Engine,
    pub path: PathBuf,
}

/// Where each installed browser keeps its profiles, on Windows.
///
/// Helium is a Chromium fork by imput, so its user data sits under a vendor directory
/// rather than beside the product name. Zen, Chrome and Helium verified on this machine
/// 2026-08-27; Firefox and Chromium are their documented default locations.
#[cfg(windows)]
fn roots() -> Vec<(&'static str, Engine, PathBuf)> {
    let env = |name| std::env::var_os(name).map(PathBuf::from);
    let mut roots = Vec::new();
    if let Some(appdata) = env("APPDATA") {
        roots.push(("Zen", Engine::Gecko, appdata.join("zen/Profiles")));
        roots.push((
            "Firefox",
            Engine::Gecko,
            appdata.join("Mozilla/Firefox/Profiles"),
        ));
    }
    if let Some(local) = env("LOCALAPPDATA") {
        roots.push((
            "Chrome",
            Engine::Chromium,
            local.join("Google/Chrome/User Data"),
        ));
        roots.push((
            "Helium",
            Engine::Chromium,
            local.join("imput/Helium/User Data"),
        ));
        roots.push((
            "Chromium",
            Engine::Chromium,
            local.join("Chromium/User Data"),
        ));
    }
    roots
}

/// Where each installed browser keeps its profiles, on Linux.
///
/// Newer Gecko builds keep profiles under `~/.config`, older ones in a dot-directory in the
/// home folder, so both spellings are checked. Zen in `~/.config/zen` and Chromium verified
/// on Omarchy 2026-10-05; the Helium path is its app id, not yet seen on a real install.
#[cfg(not(windows))]
fn roots() -> Vec<(&'static str, Engine, PathBuf)> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    vec![
        ("Zen", Engine::Gecko, config.join("zen")),
        ("Zen", Engine::Gecko, home.join(".zen")),
        ("Firefox", Engine::Gecko, config.join("mozilla/firefox")),
        ("Firefox", Engine::Gecko, home.join(".mozilla/firefox")),
        ("Chrome", Engine::Chromium, config.join("google-chrome")),
        ("Chromium", Engine::Chromium, config.join("chromium")),
        ("Helium", Engine::Chromium, config.join("net.imput.helium")),
    ]
}

/// Expand each browser's root into the `file` of every profile under it.
fn sources(file_of: fn(Engine) -> &'static str) -> Vec<Source> {
    let mut found = Vec::new();
    for (label, engine, root) in roots() {
        for path in profiles(&root, file_of(engine)) {
            found.push(Source {
                label,
                engine,
                path,
            });
        }
    }
    found
}

/// Every history database: the one the resolver reads visits from.
pub fn history_sources() -> Vec<Source> {
    sources(|engine| match engine {
        Engine::Gecko => "places.sqlite",
        Engine::Chromium => "History",
    })
}

/// Every favicon database, which sits beside each profile's history. Read only to draw a
/// site's icon in the GUI; see `favicons`.
pub fn favicon_sources() -> Vec<Source> {
    sources(|engine| match engine {
        Engine::Gecko => "favicons.sqlite",
        Engine::Chromium => "Favicons",
    })
}

/// Both engines scatter profiles across sibling directories — Gecko under random names,
/// Chromium as `Default`, `Profile 1`, and so on — so every child is checked rather than
/// assuming one well-known name.
fn profiles(root: &Path, file: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path().join(file))
        .filter(|p| p.is_file())
        .collect();
    out.sort();
    out
}
