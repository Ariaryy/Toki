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
use tauri::Manager;
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
    /// This week's total minus the same weekday-count of the week before, so a Wednesday
    /// check-in compares against last week's Mon-Wed rather than its full seven days.
    delta_ms: i64,
    /// Echoed back so the caller can tell a late response from a stale one after paging.
    weeks_ago: i64,
    /// Whether anything at all was recorded before this week — what tells the Week view to
    /// stop offering a previous page rather than walking back into empty weeks forever.
    has_earlier: bool,
}

/// Daily totals, top entities, and a week-over-week delta for one Monday-Sunday week —
/// the same boundary the weekly digest uses (DESIGN.md §10).
///
/// `weeks_ago` counts back from the current week: 0 is this one, 1 last week, and so on.
/// Negative values clamp to 0; there is nothing to show from the future, and refusing is
/// worse than showing the week the caller can already see.
///
/// Built from calls to `day_totals` rather than a new aggregate query: each is an indexed
/// range scan over a day with maybe a few hundred rows, so a dozen of them cost nothing and
/// every number here stays provably consistent with what Today shows for the same day —
/// including for weeks old enough to have rolled up, which `day_totals` reads too.
#[tauri::command]
fn week_totals(weeks_ago: i64) -> Result<WeekSummary, String> {
    let store = Store::open_read_only(&toki_store::db_path()).map_err(|e| e.to_string())?;
    let offset_min = Local::now().offset().local_minus_utc() / 60;
    let today = Local::now().date_naive();
    let weeks_ago = weeks_ago.max(0);
    let this_monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    let week_start = this_monday - Duration::days(7 * weeks_ago);

    let (days, top, total_ms) = week_rows(&store, week_start, today, offset_min)?;

    // Like for like: a week still in progress is measured against the same number of
    // weekdays of the week before, never against seven days that have not happened in
    // either of them. For a week already over this is simply all seven.
    let counted = days.iter().filter(|d| !d.is_future).count() as i64;
    let prev_total_ms = span_total_ms(&store, week_start - Duration::days(7), counted, offset_min)?;

    let has_earlier = store
        .earliest_date(offset_min)
        .map_err(|e| e.to_string())?
        .is_some_and(|first| first < week_start);

    Ok(WeekSummary {
        days,
        top,
        total_ms,
        delta_ms: total_ms - prev_total_ms,
        weeks_ago,
        has_earlier,
    })
}

/// The seven bars, the merged entity list and the grand total for the week beginning
/// `week_start`.
fn week_rows(
    store: &Store,
    week_start: NaiveDate,
    today: NaiveDate,
    offset_min: i32,
) -> Result<(Vec<DayTotal>, Vec<Total>, i64), String> {
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

    let mut top: Vec<Total> = merged.into_values().collect();
    top.sort_by_key(|t| std::cmp::Reverse(t.ms));
    top.truncate(8);
    Ok((days, top, total_ms))
}

/// Total recorded time over `days` consecutive local dates from `start`.
fn span_total_ms(
    store: &Store,
    start: NaiveDate,
    days: i64,
    offset_min: i32,
) -> Result<i64, String> {
    let mut total = 0;
    for offset in 0..days {
        let totals = store
            .day_totals(start + Duration::days(offset), offset_min)
            .map_err(|e| e.to_string())?;
        total += totals.iter().map(|t| t.ms).sum::<i64>();
    }
    Ok(total)
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
        // Registered first per the plugin's own requirement. The daemon spawns a new
        // `toki.exe` on every tray click with no idea whether one is already open — this is
        // what turns the second and later launches into a focus on the first instead of a
        // stack of windows.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
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

#[cfg(test)]
mod tests {
    use super::*;
    use toki_core::event::{AppKey, Presence};
    use toki_core::time::day_bounds;
    use toki_core::tracker::{Action, Session};

    const IST: i32 = 330;
    const HOUR: i64 = 3_600_000;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// An hour of `exe` on each of `dates`.
    fn store_with(dates: &[NaiveDate], exe: &str) -> Store {
        let store = Store::open_in_memory().unwrap();
        for d in dates {
            let (day_start, _) = day_bounds(*d, IST);
            store
                .apply(
                    &Action::Emit(Session {
                        start: day_start + 9 * HOUR,
                        end: day_start + 10 * HOUR,
                        app: AppKey::new(exe),
                        title: "main.rs".into(),
                        presence: Presence::Input,
                    }),
                    IST,
                    None,
                )
                .unwrap();
        }
        store
    }

    #[test]
    fn a_week_already_over_has_seven_real_days_and_no_today() {
        // Monday 2026-09-07 through Sunday the 13th, read from a later date.
        let monday = date(2026, 9, 7);
        let store = store_with(&[monday, date(2026, 9, 9)], "code.exe");

        let (days, top, total_ms) = week_rows(&store, monday, date(2026, 9, 20), IST).unwrap();

        assert_eq!(days.len(), 7);
        assert!(days.iter().all(|d| !d.is_future && !d.is_today));
        assert_eq!(days[0].ms, HOUR);
        assert_eq!(days[1].ms, 0);
        assert_eq!(days[2].ms, HOUR);
        assert_eq!(total_ms, 2 * HOUR);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].ms, 2 * HOUR);
    }

    #[test]
    fn a_week_in_progress_marks_today_and_leaves_the_rest_future() {
        let monday = date(2026, 9, 14);
        let store = store_with(&[monday], "code.exe");

        // Tuesday the 15th.
        let (days, _, _) = week_rows(&store, monday, date(2026, 9, 15), IST).unwrap();

        assert!(!days[0].is_future && !days[0].is_today);
        assert!(days[1].is_today);
        assert_eq!(days[2..].iter().filter(|d| d.is_future).count(), 5);
    }

    #[test]
    fn the_comparison_span_counts_only_the_days_asked_for() {
        let monday = date(2026, 9, 7);
        let store = store_with(&[monday, date(2026, 9, 8), date(2026, 9, 11)], "code.exe");

        // A Tuesday check-in compares two days, not the whole week.
        assert_eq!(span_total_ms(&store, monday, 2, IST).unwrap(), 2 * HOUR);
        assert_eq!(span_total_ms(&store, monday, 7, IST).unwrap(), 3 * HOUR);
        assert_eq!(span_total_ms(&store, monday, 0, IST).unwrap(), 0);
    }

    #[test]
    fn a_week_with_nothing_in_it_is_seven_empty_bars_rather_than_an_error() {
        let store = store_with(&[date(2026, 9, 7)], "code.exe");
        let (days, top, total_ms) =
            week_rows(&store, date(2026, 8, 3), date(2026, 9, 20), IST).unwrap();

        assert_eq!(days.len(), 7);
        assert!(days.iter().all(|d| d.ms == 0));
        assert!(top.is_empty());
        assert_eq!(total_ms, 0);
    }
}
