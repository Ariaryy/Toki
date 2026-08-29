//! Read-only Tauri shell for Toki.
//!
//! Owns no state and writes nothing: the daemon is the sole writer per DESIGN.md §7, so this
//! opens the same database read-only and turns queries into Tauri commands the Svelte side
//! calls directly. Pause and liveness go over the IPC pipe instead — see `toki-daemon`'s
//! `platform::ipc` — because those are decisions, not reads.

mod theme;

use chrono::Local;
use toki_store::{Store, Total};

/// Per-app and per-domain totals for today, local time.
#[tauri::command]
fn today_totals() -> Result<Vec<Total>, String> {
    let store = Store::open_read_only(&toki_store::db_path()).map_err(|e| e.to_string())?;
    let today = Local::now().date_naive();
    let offset_min = Local::now().offset().local_minus_utc() / 60;
    store
        .day_totals(today, offset_min)
        .map_err(|e| e.to_string())
}

/// `"light"` or `"dark"`, read once at launch so the window opens in the right theme.
#[tauri::command]
fn system_theme() -> &'static str {
    theme::system_theme()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![today_totals, system_theme])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
