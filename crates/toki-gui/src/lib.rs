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
use toki_store::{AppRow, DayHours, DomainRow, Span, Store, Total};

/// Hours 00:00-05:00 local. What "after midnight" means in the metric strips — late
/// enough to be worth noticing, early enough that a normal evening does not trip it.
const LATE_NIGHT_END_MS: i64 = 5 * 3_600_000;

/// The session in progress, for Today's "Now" card.
///
/// No domain: the resolver runs every five minutes against browser history, so the domain
/// of a session that has not closed yet is not known and guessing from an earlier one would
/// be a lie. The window title is not offered either — it is stored, but nothing in the GUI
/// has ever displayed it and this is not the place to start.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NowPlaying {
    /// Carried so Today's "Now" dot can take the same series colour the app has in the
    /// list below it, rather than being matched back by name.
    exe_path: String,
    app_name: String,
    /// Milliseconds after local midnight that the session started.
    start_ms: i64,
    elapsed_ms: i64,
}

/// The longest unbroken stretch in one app, within the day being shown.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Longest {
    ms: i64,
    app_name: String,
    start_ms: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DaySummary {
    date: String,
    /// "Tue 15 Sep" — formatted here so the webview does not have to agree with Rust about
    /// what a short date looks like.
    label: String,
    is_today: bool,
    /// Whether anything was recorded before this day, so the date stepper knows when to
    /// stop offering a previous page.
    has_earlier: bool,
    totals: Vec<Total>,
    total_ms: i64,
    /// Against the day before. Neutral in the UI, never red or green (DESIGN.md §9).
    delta_ms: i64,
    spans: Vec<Span>,
    /// This day predates the six-month rollup, so it has totals but no timing. The timeline
    /// has to say that rather than draw an empty strip — see [`toki_store::DayDetail`].
    rolled_up: bool,
    longest: Option<Longest>,
    /// Distinct apps with any time, and how many of those were browsers.
    app_count: usize,
    browser_count: usize,
    late_night_ms: i64,
    now: Option<NowPlaying>,
}

/// Everything Today draws, for one local date.
///
/// One command rather than six, because every figure on that page describes the same day and
/// splitting them would let the metric strip and the timeline disagree after a midnight
/// rollover mid-session.
///
/// `date` is an ISO string; `None` means today. A date in the future clamps to today —
/// there is nothing to show past it, and refusing is worse than showing what the caller can
/// already see. Same rule `week_totals` applies to a negative `weeks_ago`.
#[tauri::command]
fn day_summary(date: Option<String>) -> Result<DaySummary, String> {
    let store = Store::open_read_only(&toki_store::db_path()).map_err(|e| e.to_string())?;
    let now = Local::now();
    let offset_min = now.offset().local_minus_utc() / 60;
    let today = now.date_naive();

    let date = match date {
        Some(s) => s
            .parse::<NaiveDate>()
            .map_err(|e| e.to_string())?
            .min(today),
        None => today,
    };

    let totals = store
        .day_totals(date, offset_min)
        .map_err(|e| e.to_string())?;
    let total_ms: i64 = totals.iter().map(|t| t.ms).sum();

    let previous = store
        .day_totals(date - Duration::days(1), offset_min)
        .map_err(|e| e.to_string())?;
    let delta_ms = total_ms - previous.iter().map(|t| t.ms).sum::<i64>();

    let detail = store
        .day_spans(date, offset_min)
        .map_err(|e| e.to_string())?;

    let longest = detail
        .spans
        .iter()
        .max_by_key(|s| s.end_ms - s.start_ms)
        .map(|s| Longest {
            ms: s.end_ms - s.start_ms,
            app_name: s.app_name.clone(),
            start_ms: s.start_ms,
        });

    let late_night_ms = detail
        .spans
        .iter()
        .map(|s| (s.end_ms.min(LATE_NIGHT_END_MS) - s.start_ms).max(0))
        .sum();

    // `totals` carries one row per (app, domain), so counting rows would count a browser
    // once per domain. Apps and browsers are both counted over distinct executables.
    let app_count = totals
        .iter()
        .map(|t| t.exe_path.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let browser_count = totals
        .iter()
        .filter(|t| t.domain.is_some())
        .map(|t| t.exe_path.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len();

    let has_earlier = store
        .earliest_date(offset_min)
        .map_err(|e| e.to_string())?
        .is_some_and(|first| first < date);

    Ok(DaySummary {
        label: format!("{} {}", short_weekday(date), short_date(date)),
        date: date.to_string(),
        is_today: date == today,
        has_earlier,
        total_ms,
        totals,
        delta_ms,
        spans: detail.spans,
        rolled_up: detail.rolled_up,
        longest,
        app_count,
        browser_count,
        late_night_ms,
        now: if date == today {
            now_playing(&store, now.timestamp_millis(), offset_min)?
        } else {
            None
        },
    })
}

/// The open session from `meta`, named the way every list names an app.
///
/// Reads the mirror the daemon already keeps for crash recovery (DESIGN.md §7), so "what am
/// I in right now" costs a `meta` lookup rather than a new IPC message.
fn now_playing(store: &Store, now_ms: i64, offset_min: i32) -> Result<Option<NowPlaying>, String> {
    let Some(open) = store.open_session().map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let name = store
        .all_apps()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|a| a.exe_path == open.app.as_str())
        .map(|a| a.name)
        .unwrap_or_else(|| open.app.as_str().to_string());
    let (day_start, _) = toki_core::time::day_bounds(
        toki_core::time::local_date(open.start, offset_min),
        offset_min,
    );
    Ok(Some(NowPlaying {
        exe_path: open.app.as_str().to_string(),
        app_name: name,
        start_ms: open.start - day_start,
        elapsed_ms: (now_ms - open.start).max(0),
    }))
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
    /// Seven rows of 24 hourly totals, for the heatmap. Days past the rollup cutoff carry
    /// zeroes and `rolled_up: true`.
    hours: Vec<DayHours>,
    /// Across days that have actually happened, so a Wednesday check-in is not divided by
    /// seven.
    average_ms: i64,
    /// Time in hours 00:00-05:00 across the week.
    late_night_ms: i64,
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

    let hours = store
        .hour_histogram(week_start, 7, offset_min)
        .map_err(|e| e.to_string())?;
    // Hours 0..5 of every row. Summed from the histogram rather than from spans, because
    // the histogram is already the per-hour clamp and a second pass would be a second rule.
    let late_night_ms = hours
        .iter()
        .flat_map(|d| {
            d.hours
                .iter()
                .take((LATE_NIGHT_END_MS / 3_600_000) as usize)
        })
        .sum();

    Ok(WeekSummary {
        days,
        top,
        total_ms,
        delta_ms: total_ms - prev_total_ms,
        weeks_ago,
        has_earlier,
        hours,
        average_ms: if counted > 0 { total_ms / counted } else { 0 },
        late_night_ms,
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

/// "15 Sep", or "15 Sep 2025" outside the current year. Same rule the week range label uses:
/// the year is almost always redundant and reads as noise when it is.
fn short_date(d: NaiveDate) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = MONTHS[d.month0() as usize];
    if d.year() == Local::now().date_naive().year() {
        format!("{} {month}", d.day())
    } else {
        format!("{} {month} {}", d.day(), d.year())
    }
}

/// `"light"` or `"dark"`, read once at launch so the window opens in the right theme.
#[tauri::command]
fn system_theme() -> &'static str {
    theme::system_theme()
}

/// One row of the Apps table: an app, its alias and ignore state, and its time over the
/// selected range.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppEntry {
    exe_path: String,
    name: String,
    alias: Option<String>,
    ignored: bool,
    ms: i64,
}

/// Every app ever seen, with its time over the last `days` local days.
///
/// `days` of `None` means all time, and resolves to the first date with anything in it
/// rather than to some large number of days — a range that claims to start ten years ago is
/// a different answer to "all time" than one that starts where the data does.
///
/// The join is done here rather than in SQL because the two halves answer different
/// questions: `all_apps` is the full directory including apps with no recent time, and
/// `app_totals_between` is a range aggregate that includes ignored apps. An app with no
/// time in the range still belongs in the table — it is exactly the row someone goes looking
/// for when they want to ignore something they no longer use.
#[tauri::command]
fn app_directory(days: Option<i64>) -> Result<Vec<AppEntry>, String> {
    let store = Store::open_read_only(&toki_store::db_path()).map_err(|e| e.to_string())?;
    let offset_min = Local::now().offset().local_minus_utc() / 60;
    let today = Local::now().date_naive();
    let from = match days {
        Some(n) => today - Duration::days(n.max(1) - 1),
        None => store
            .earliest_date(offset_min)
            .map_err(|e| e.to_string())?
            .unwrap_or(today),
    };

    let by_exe: HashMap<String, i64> = store
        .app_totals_between(from, today, offset_min)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|t| (t.exe_path, t.ms))
        .collect();

    let mut rows: Vec<AppEntry> = store
        .all_apps()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|a| AppEntry {
            ms: by_exe.get(&a.exe_path).copied().unwrap_or(0),
            exe_path: a.exe_path,
            name: a.name,
            alias: a.alias,
            ignored: a.ignored,
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.ms));
    Ok(rows)
}

/// What the daemon is doing and how far the data goes back — the Settings diagnostics block.
///
/// Surfaces what DESIGN.md §11 already collects but the GUI never showed. `detail_since` and
/// `earliest_date` are deliberately both here: the gap between them is what retention did,
/// and it is the reason the timeline and heatmap stop where they do.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Diagnostics {
    app_version: &'static str,
    schema_version: i64,
    session_count: i64,
    pending_count: i64,
    /// Milliseconds since the daemon last wrote its heartbeat. `None` on a database that has
    /// never had one.
    heartbeat_age_ms: Option<i64>,
    /// The daemon's process id, for putting a name to the process in Task Manager. Stale on
    /// a database whose daemon has since died, which is why Settings prints it only beside
    /// a liveness check that came back "Running".
    daemon_pid: Option<u32>,
    earliest_date: Option<String>,
    detail_since: Option<String>,
    db_bytes: u64,
    /// Handed to the frontend so Settings' "open the folder" buttons do not have to rebuild
    /// `%LOCALAPPDATA%\Toki` themselves and get it subtly wrong.
    db_path: String,
    log_dir: String,
}

#[tauri::command]
fn diagnostics() -> Result<Diagnostics, String> {
    let path = toki_store::db_path();
    let store = Store::open_read_only(&path).map_err(|e| e.to_string())?;
    let offset_min = Local::now().offset().local_minus_utc() / 60;
    let now_ms = Local::now().timestamp_millis();

    Ok(Diagnostics {
        app_version: env!("CARGO_PKG_VERSION"),
        schema_version: store.schema_version().map_err(|e| e.to_string())?,
        session_count: store.rows_between(0, i64::MAX).map_err(|e| e.to_string())?,
        pending_count: store.pending_count().map_err(|e| e.to_string())?,
        heartbeat_age_ms: store
            .last_heartbeat()
            .map_err(|e| e.to_string())?
            .map(|at| (now_ms - at).max(0)),
        daemon_pid: store.daemon_pid().map_err(|e| e.to_string())?,
        earliest_date: store
            .earliest_date(offset_min)
            .map_err(|e| e.to_string())?
            .map(|d| d.to_string()),
        detail_since: store
            .earliest_detail_date(offset_min)
            .map_err(|e| e.to_string())?
            .map(|d| d.to_string()),
        db_bytes: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
        log_dir: path
            .parent()
            .map(|dir| dir.join("logs"))
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        db_path: path.to_string_lossy().into_owned(),
    })
}

/// What the rail's footer says underneath "Tracking": the open session, or `None` when
/// nothing is open. Same data as `day_summary().now`, but the rail lives outside any page
/// and polls on its own clock, so it should not pay for a whole day's totals and spans to
/// render two words.
#[tauri::command]
fn current_app() -> Result<Option<NowPlaying>, String> {
    let store = Store::open_read_only(&toki_store::db_path()).map_err(|e| e.to_string())?;
    let now = Local::now();
    now_playing(
        &store,
        now.timestamp_millis(),
        now.offset().local_minus_utc() / 60,
    )
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

/// Start the collector beside this executable if it isn't running — after "Quit Toki" in
/// the tray, opening Toki from the Start menu or a launcher is how anyone expects tracking
/// to come back.
///
/// Spawned unconditionally: the daemon's own single-instance mutex turns a second copy into
/// a process that exits at once, which is cheaper than asking first and can't race.
/// `CREATE_NO_WINDOW` because the daemon is a console program and would otherwise flash a
/// console window before letting go of it.
#[cfg(windows)]
fn start_daemon() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = std::process::Command::new(exe.with_file_name("toki-daemon.exe"))
        .arg("run")
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

/// On Linux the collector goes through systemd when autostart has installed its unit, so
/// it comes back under the same supervision it would have had at login. Otherwise it is
/// started beside this binary, in its own process group so a Ctrl+C aimed at a GUI run from
/// a terminal does not take tracking down with it.
#[cfg(unix)]
fn start_daemon() {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let via_systemd = Command::new("systemctl")
        .args(["--user", "start", "toki-daemon.service"])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if via_systemd {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = Command::new(exe.with_file_name("toki-daemon"))
        .arg("run")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
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
        .setup(|_| {
            start_daemon();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            day_summary,
            week_totals,
            current_app,
            system_theme,
            all_apps,
            all_domains,
            app_directory,
            diagnostics,
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
