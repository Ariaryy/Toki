//! Where Toki's own files live, and where the browsers keep the history it reads.

use std::path::{Path, PathBuf};

use toki_core::resolve::Engine;

/// Defined in `toki-store`, beside [`db_path`], so the two cannot drift apart.
pub fn data_dir() -> PathBuf {
    toki_store::data_dir()
}

/// Defined in `toki-store`, not here, so the daemon and the GUI can never disagree about
/// which file they mean.
pub fn db_path() -> PathBuf {
    toki_store::db_path()
}

/// Diagnostics, kept apart from the database so the whole directory can be handed to
/// someone reading a bug report without handing over the window titles too.
pub fn log_dir() -> PathBuf {
    data_dir().join("logs")
}

/// The GUI, if it is installed beside us.
///
/// The tray offers to open it, and the honest answer before Phase 6 is that it is not there
/// yet — which is a greyed-out menu entry rather than a silent no-op.
pub fn gui_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let gui = exe
        .parent()?
        .join(if cfg!(windows) { "toki.exe" } else { "toki" });
    gui.is_file().then_some(gui)
}

/// A browser history database the resolver reads visits from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistorySource {
    pub label: &'static str,
    pub engine: Engine,
    pub path: PathBuf,
}

/// Every browser history database present on this machine.
///
/// Helium is a Chromium fork by imput, so its user data sits under a vendor directory
/// rather than beside the product name. Zen, Chrome and Helium verified on this machine
/// 2026-08-27; Firefox and Chromium are their documented default locations.
#[cfg(windows)]
pub fn history_sources() -> Vec<HistorySource> {
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
    sources(roots)
}

/// Every browser history database present on this machine.
///
/// Newer Gecko builds keep profiles under `~/.config`, older ones in a dot-directory in the
/// home folder, so both spellings are checked. Zen in `~/.config/zen` and Chromium verified
/// on Omarchy 2026-10-05; the Helium path is its app id, not yet seen on a real install.
#[cfg(not(windows))]
pub fn history_sources() -> Vec<HistorySource> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    sources(vec![
        ("Zen", Engine::Gecko, config.join("zen")),
        ("Zen", Engine::Gecko, home.join(".zen")),
        ("Firefox", Engine::Gecko, config.join("mozilla/firefox")),
        ("Firefox", Engine::Gecko, home.join(".mozilla/firefox")),
        ("Chrome", Engine::Chromium, config.join("google-chrome")),
        ("Chromium", Engine::Chromium, config.join("chromium")),
        ("Helium", Engine::Chromium, config.join("net.imput.helium")),
    ])
}

/// Expand each browser's root into the history file of every profile under it.
fn sources(roots: Vec<(&'static str, Engine, PathBuf)>) -> Vec<HistorySource> {
    let mut found = Vec::new();
    for (label, engine, root) in roots {
        let file = match engine {
            Engine::Gecko => "places.sqlite",
            Engine::Chromium => "History",
        };
        for path in profiles(&root, file) {
            found.push(HistorySource {
                label,
                engine,
                path,
            });
        }
    }
    found
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
