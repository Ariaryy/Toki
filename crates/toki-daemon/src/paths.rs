//! Where Toki's own files live, and where the browsers keep the history it reads.

use std::path::PathBuf;

/// Diagnostics, kept apart from the database so the whole directory can be handed to
/// someone reading a bug report without handing over the window titles too.
pub fn log_dir() -> PathBuf {
    toki_store::data_dir().join("logs")
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

pub use toki_store::browsers::history_sources;
