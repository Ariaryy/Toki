//! Read-only Tauri shell for Toki.
//!
//! Owns no state and writes nothing: the daemon is the sole writer per DESIGN.md §7, so this
//! opens the same database read-only and turns queries into Tauri commands the Svelte side
//! calls directly. `resolve_now` is the one exception — a decision, not a read — sent over
//! the daemon's IPC pipe instead; see `ipc` here and `toki-daemon`'s `platform::ipc`.

mod ipc;
mod theme;

use std::collections::HashMap;

use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};
use serde::Serialize;
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

/// Kick the daemon's resolver on launch, per DESIGN.md §6. Failure is silent — a daemon
/// that isn't running, or a pipe that's briefly busy, just leaves the row for the next
/// scheduled pass, same as if this call had never been made.
#[tauri::command]
fn resolve_now() {
    let _ = ipc::resolve_now();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            today_totals,
            week_totals,
            system_theme,
            resolve_now
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
