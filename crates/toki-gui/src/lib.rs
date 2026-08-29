//! Read-only Tauri shell for Toki.
//!
//! Owns no state and writes nothing itself: the daemon is the sole writer per DESIGN.md §7.
//! Queries go straight at the database, read-only, and turn into Tauri commands the Svelte
//! side calls directly. Anything that changes something — pause, `resolve_now`, aliases, the
//! ignore list, the idle threshold — goes over the daemon's IPC pipe instead, through the one
//! generic `ipc_call`; see `ipc` here and `toki-daemon`'s `platform::ipc` for the other end.

mod ipc;
mod theme;

use std::collections::HashMap;

use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};
use serde::Serialize;
use serde_json::Value;
use toki_store::{AppRow, DomainRow, Store, Total};

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

/// One day's total inside a week, for the daily-totals strip.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DayTotal {
    date: String,
    label: &'static str,
    ms: i64,
    is_today: bool,
    is_future: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WeekSummary {
    /// Monday through Sunday, in order. Days after today carry `ms: 0` and `is_future: true`
    /// rather than being omitted, so the strip always has seven bars.
    days: Vec<DayTotal>,
    /// Per-app and per-domain totals summed across the week, most first.
    top: Vec<Total>,
    total_ms: i64,
    /// This week's total minus the same weekday-count of last week, so a Wednesday check-in
    /// compares against last week's Mon–Wed rather than its full seven days.
    delta_ms: i64,
}

/// Daily totals, top entities, and a week-over-week delta for the current Monday–Sunday
/// week — the same boundary the weekly digest uses (DESIGN.md §10).
///
/// Built from seven-plus-seven calls to `day_totals` rather than a new aggregate query: each
/// call is already an indexed range scan over a day with maybe a few hundred rows, so
/// fourteen of them costs nothing and every number here stays provably consistent with what
/// Today shows for the same day.
#[tauri::command]
fn week_totals() -> Result<WeekSummary, String> {
    let store = Store::open_read_only(&toki_store::db_path()).map_err(|e| e.to_string())?;
    let offset_min = Local::now().offset().local_minus_utc() / 60;
    let today = Local::now().date_naive();
    let week_start = today - Duration::days(today.weekday().num_days_from_monday() as i64);

    let mut days = Vec::with_capacity(7);
    let mut merged: HashMap<(String, Option<String>), Total> = HashMap::new();
    let mut total_ms: i64 = 0;

    for offset in 0..7 {
        let date = week_start + Duration::days(offset);
        if date > today {
            days.push(DayTotal {
                date: date.to_string(),
                label: short_weekday(date),
                ms: 0,
                is_today: false,
                is_future: true,
            });
            continue;
        }
        let totals = store
            .day_totals(date, offset_min)
            .map_err(|e| e.to_string())?;
        let day_ms: i64 = totals.iter().map(|t| t.ms).sum();
        total_ms += day_ms;
        days.push(DayTotal {
            date: date.to_string(),
            label: short_weekday(date),
            ms: day_ms,
            is_today: date == today,
            is_future: false,
        });
        for t in totals {
            let key = (t.exe_path.clone(), t.domain.clone());
            let ms = t.ms;
            merged.entry(key).and_modify(|e| e.ms += ms).or_insert(t);
        }
    }

    // Compared against the same number of weekdays last week — a Wednesday check-in isn't
    // measured against a full seven days that hasn't happened yet this week either.
    let days_elapsed = today.weekday().num_days_from_monday() as i64 + 1;
    let prev_week_start = week_start - Duration::days(7);
    let mut prev_total_ms: i64 = 0;
    for offset in 0..days_elapsed {
        let date = prev_week_start + Duration::days(offset);
        let totals = store
            .day_totals(date, offset_min)
            .map_err(|e| e.to_string())?;
        prev_total_ms += totals.iter().map(|t| t.ms).sum::<i64>();
    }

    let mut top: Vec<Total> = merged.into_values().collect();
    top.sort_by_key(|t| std::cmp::Reverse(t.ms));
    top.truncate(8);

    Ok(WeekSummary {
        days,
        top,
        total_ms,
        delta_ms: total_ms - prev_total_ms,
    })
}

fn short_weekday(d: NaiveDate) -> &'static str {
    match d.weekday() {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    }
}

/// `"light"` or `"dark"`, read once at launch so the window opens in the right theme.
#[tauri::command]
fn system_theme() -> &'static str {
    theme::system_theme()
}

/// Every app ever seen, for the Settings alias editor and ignore list.
#[tauri::command]
fn all_apps() -> Result<Vec<AppRow>, String> {
    let store = Store::open_read_only(&toki_store::db_path()).map_err(|e| e.to_string())?;
    store.all_apps().map_err(|e| e.to_string())
}

/// Every domain ever resolved, for the Settings alias editor.
#[tauri::command]
fn all_domains() -> Result<Vec<DomainRow>, String> {
    let store = Store::open_read_only(&toki_store::db_path()).map_err(|e| e.to_string())?;
    store.all_domains().map_err(|e| e.to_string())
}

/// The idle threshold in force, in minutes — Settings' own override if there is one,
/// otherwise `Config::default()`'s. A read, so it goes straight at the database rather than
/// through IPC; only *changing* it needs the daemon.
#[tauri::command]
fn idle_threshold_minutes() -> Result<i64, String> {
    let store = Store::open_read_only(&toki_store::db_path()).map_err(|e| e.to_string())?;
    let ms = store
        .idle_threshold_ms()
        .map_err(|e| e.to_string())?
        .unwrap_or(toki_core::Config::default().idle_threshold_ms);
    Ok(ms / 60_000)
}

/// The one write path out of the GUI: a raw request forwarded to the daemon's IPC pipe
/// verbatim, and its raw JSON response handed back. See `ipc` and `toki-daemon`'s
/// `platform::ipc` for what `request.type` can be.
#[tauri::command]
fn ipc_call(request: Value) -> Result<Value, String> {
    ipc::call(&request).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            today_totals,
            week_totals,
            system_theme,
            all_apps,
            all_domains,
            idle_threshold_minutes,
            ipc_call
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
