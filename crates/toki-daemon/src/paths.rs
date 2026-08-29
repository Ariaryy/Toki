//! Where Toki's own files live, and where the browsers keep the history it reads.

use std::path::{Path, PathBuf};

use toki_core::resolve::Engine;

/// `%LOCALAPPDATA%\Toki`. Deliberately not Documents or any synced folder: the database is
/// unencrypted and holds window titles.
pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("Toki")
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
    let gui = exe.parent()?.join("toki.exe");
    gui.is_file().then_some(gui)
}

/// A browser history database the resolver can build a title index from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistorySource {
    pub label: &'static str,
    pub engine: Engine,
    pub path: PathBuf,
}

/// Every browser history database present on this machine.
///
/// Helium is a Chromium fork by imput, so its user data sits under a vendor directory
/// rather than beside the product name. Verified on this machine 2026-08-27.
pub fn history_sources() -> Vec<HistorySource> {
    let mut found = Vec::new();

    if let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) {
        for path in profiles(&appdata.join("zen").join("Profiles"), "places.sqlite") {
            found.push(HistorySource {
                label: "Zen",
                engine: Engine::Gecko,
                path,
            });
        }
    }

    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        let roots = [
            ("Chrome", local.join("Google/Chrome/User Data")),
            ("Helium", local.join("imput/Helium/User Data")),
        ];
        for (label, user_data) in roots {
            for path in profiles(&user_data, "History") {
                found.push(HistorySource {
                    label,
                    engine: Engine::Chromium,
                    path,
                });
            }
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
