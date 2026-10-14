//! Windows collector for Toki.
//!
//! This crate owns everything platform-specific and makes no decisions: it produces
//! [`toki_core::Event`]s, hands them to [`toki_core::Tracker`], and writes the resulting
//! actions through [`toki_store::Store`].
//!
//! The hook callback must only push to a channel and return. Windows silently unhooks
//! callbacks that block, and a tracker that stops recording without saying so is the
//! failure this design most wants to avoid.

mod digest;
mod ipc;
mod logging;
mod paths;
mod platform;
mod resolver;
mod run;
mod state;

use std::process::ExitCode;
use std::time::Instant;

use chrono::{Local, NaiveDate, Utc};
use toki_core::time::day_bounds;
use toki_core::{Config, IdleHistogram, IDLE_BUCKET_BOUNDS_MS};
use toki_store::Store;

const USAGE: &str = "\
toki-daemon — Toki's headless collector

USAGE:
    toki-daemon run                              collect until stopped
    toki-daemon doctor                           health check
    toki-daemon purge --from <YYYY-MM-DD> --to <YYYY-MM-DD> [--app <exe>]
    toki-daemon reresolve --from <YYYY-MM-DD> [--to <YYYY-MM-DD>]
    toki-daemon autostart [on|off]               start at login; no argument reports
    toki-daemon config                           show effective thresholds
    toki-daemon bench [--hours N] [--label L] [--out FILE]
                                                 time a resolver pass on a copy of the data
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Double-clicked in Explorer, there is no argument and no terminal to print usage to —
    // the console would flash and close. Starting the collector is the only thing that
    // launch can have meant.
    let command = match args.first() {
        None if platform::console::is_ours() => Some("run"),
        first => first.map(String::as_str),
    };
    match command {
        Some("run") => match run::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("run: {e}");
                ExitCode::FAILURE
            }
        },
        Some("reresolve") => reresolve(&args[1..]),
        Some("doctor") => doctor(),
        Some("purge") => purge(&args[1..]),
        Some("autostart") => autostart(args.get(1).map(String::as_str)),
        Some("config") => config(),
        Some("bench") => bench(&args[1..]),
        _ => {
            print!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

/// The thresholds actually in force, plus what the idle one has been absorbing.
///
/// Reads the store rather than printing `Config::default()` outright: Settings can override
/// the idle threshold, and a command that says "effective" while ignoring that override is
/// worse than not having one.
fn config() -> ExitCode {
    let db = paths::db_path();
    let mut cfg = Config::default();
    let store = Store::open_read_only(&db).ok();

    let mut source = "default";
    if let Some(s) = &store {
        if let Ok(Some(ms)) = s.idle_threshold_ms() {
            cfg.idle_threshold_ms = ms;
            source = "Settings";
        }
    }

    println!("noise floor     {} ms", cfg.noise_floor_ms);
    println!("idle threshold  {} ms ({source})", cfg.idle_threshold_ms);
    println!("audio cap       {} ms", cfg.audio_cap_ms);
    println!("database        {}", db.display());

    // Pauses shorter than the idle threshold never close a session, so their time is
    // counted as active and leaves no trace in `sessions`. This is the only readout of it,
    // and the evidence the threshold was always meant to be settled against.
    let hist = store
        .as_ref()
        .and_then(|s| s.idle_histogram_json().ok().flatten())
        .and_then(|j| serde_json::from_str::<IdleHistogram>(&j).ok());

    let Some(hist) = hist else {
        println!("\nabsorbed idle   nothing recorded yet");
        return ExitCode::SUCCESS;
    };

    println!("\nabsorbed idle — input silence counted as active, by pause length");
    let mut cum_count = 0u32;
    let mut cum_ms = 0i64;
    for (i, b) in hist.buckets.iter().enumerate().rev() {
        cum_count += b.count;
        cum_ms += b.total_ms;
        let label = match IDLE_BUCKET_BOUNDS_MS.get(i) {
            Some(hi) => {
                let lo = if i == 0 {
                    0
                } else {
                    IDLE_BUCKET_BOUNDS_MS[i - 1]
                };
                format!("{}-{}s", lo / 1_000, hi / 1_000)
            }
            None => format!("{}s+", IDLE_BUCKET_BOUNDS_MS[i - 1] / 1_000),
        };
        // The cumulative columns are the answer to "what if the threshold were lower":
        // everything at or above a row would have closed its session instead.
        println!(
            "  {label:>10}  {:>6} pauses  {:>8.2} h    at or above: {:>6} / {:>8.2} h",
            b.count,
            b.total_ms as f64 / 3_600_000.0,
            cum_count,
            cum_ms as f64 / 3_600_000.0,
        );
    }
    ExitCode::SUCCESS
}

/// What to run when the numbers look wrong. The failure this checks for is the daemon
/// having quietly stopped days ago.
/// Per-browser resolution, the only place the hit rate is visible.
///
/// The raw unresolved count above cannot distinguish a queue that has not drained from a
/// caption that can never resolve, and it hides a browser whose titles stopped matching
/// behind the ones whose still do.
fn print_resolution(store: &Store) {
    let rows = match store.resolution_by_app() {
        Ok(rows) => rows,
        Err(e) => {
            println!("resolution unreadable: {e}");
            return;
        }
    };
    println!("resolution");
    if rows.is_empty() {
        println!("  no browser sessions yet");
    }
    for r in &rows {
        let hit = resolver::hit_rate(r.resolved as usize, r.missed as usize);
        println!(
            "  {:<14} hit {:>3.0}%   resolved {:<6} missed {:<6} not a page {:<6} pending {}",
            r.name,
            hit * 100.0,
            r.resolved,
            r.missed,
            r.not_a_page,
            r.pending
        );
    }
}

fn doctor() -> ExitCode {
    let db = paths::db_path();
    println!("database   {}", db.display());

    // Read-only, so a health check never creates the database it is reporting on.
    match Store::open_read_only(&db) {
        Ok(store) => print_store_health(&store),
        // A missing file is a fact about the daemon, not a failure of the check.
        Err(e) if !db.is_file() => println!("           absent — the daemon has never run ({e})"),
        Err(e) => {
            println!("           unreadable: {e}");
            return ExitCode::FAILURE;
        }
    }

    println!("offset     {:+} min", local_offset_min());
    match platform::autostart::current() {
        Some(cmd) => println!("autostart  on — {cmd}"),
        None => println!("autostart  off"),
    }
    println!("logs       {}", paths::log_dir().display());
    print_history_sources();
    ExitCode::SUCCESS
}

/// What the database says about the daemon: is it running, what is it doing, and is
/// resolution keeping up.
///
/// Every line is reported independently. One unreadable table should not take the rest of
/// the health check down with it — the whole point is to run when something is wrong.
fn print_store_health(store: &Store) {
    let now = Utc::now().timestamp_millis();
    match store.last_heartbeat() {
        Ok(Some(last)) => {
            let age_s = (now - last) / 1_000;
            let state = if age_s < 120 { "alive" } else { "STALE" };
            println!("heartbeat  {age_s}s ago ({state})");
        }
        Ok(None) => println!("heartbeat  never written"),
        Err(e) => println!("heartbeat  unreadable: {e}"),
    }

    match store.open_session() {
        Ok(Some(open)) => println!("open       {} since {}", open.app.as_str(), open.start),
        Ok(None) => println!("open       none"),
        Err(e) => println!("open       unreadable: {e}"),
    }

    let (day_start, day_end) = day_bounds(Local::now().date_naive(), local_offset_min());
    match store.rows_between(day_start, day_end) {
        Ok(n) => println!("rows today {n}"),
        Err(e) => println!("rows today unreadable: {e}"),
    }
    match store.pending_count() {
        Ok(n) => println!("unresolved {n}"),
        Err(e) => println!("unresolved unreadable: {e}"),
    }
    print_resolution(store);
}

fn print_history_sources() {
    println!("history sources");
    let sources = paths::history_sources();
    if sources.is_empty() {
        println!("  none found");
    }
    for s in &sources {
        let size = std::fs::metadata(&s.path)
            .map(|m| format!("{} KB", m.len() / 1024))
            .unwrap_or_else(|e| format!("unreadable: {e}"));
        println!(
            "  {:<7} {:?}  {}  {}",
            s.label,
            s.engine,
            size,
            s.path.display()
        );
    }
}

/// Measure the resolver on a copy of the live database: requeue the browser sessions of
/// the last `--hours` (default 24, about what one pass sees) and drain them.
///
/// The live file is only read, through `VACUUM INTO`. Reports wall time, bytes the history
/// copies wrote, working set before and at peak, and the hit rate; `--out` appends the same
/// as one JSON line, so results from successive versions can be compared.
fn bench(args: &[String]) -> ExitCode {
    let hours: i64 = flag(args, "--hours")
        .and_then(|v| v.parse().ok())
        .unwrap_or(24);
    let label = flag(args, "--label").unwrap_or_else(|| "unlabelled".into());
    let scratch = std::env::temp_dir().join(format!("toki-bench-{}.db", std::process::id()));

    let result = bench_on_copy(&scratch, hours);
    for ext in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{ext}", scratch.display()));
    }
    let mut report = match result {
        Ok(r) => r,
        Err(e) => {
            eprintln!("bench: {e}");
            return ExitCode::FAILURE;
        }
    };
    report["label"] = label.into();
    report["taken"] = Local::now().to_rfc3339().into();
    println!(
        "{}",
        serde_json::to_string_pretty(&report).unwrap_or_default()
    );

    if let Some(out) = flag(args, "--out") {
        use std::io::Write;
        let line = serde_json::to_string(&report).unwrap_or_default();
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&out)
            .and_then(|mut f| writeln!(f, "{line}"));
        if let Err(e) = written {
            eprintln!("bench: cannot write {out}: {e}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn bench_on_copy(scratch: &std::path::Path, hours: i64) -> Result<serde_json::Value, String> {
    use std::sync::atomic::Ordering;
    let _ = std::fs::remove_file(scratch);
    Store::open_read_only(&paths::db_path())
        .and_then(|live| live.snapshot_into(scratch))
        .map_err(|e| format!("cannot copy the database: {e}"))?;
    let store = Store::open(scratch).map_err(|e| e.to_string())?;

    let now = Utc::now().timestamp_millis();
    let queued = store
        .reset_resolution(now - hours * 3_600_000, now + 1)
        .map_err(|e| e.to_string())?;

    platform::memory::trim();
    let (ws_before, private_before) = platform::memory::usage().unwrap_or_default();
    let sampler = MemorySampler::start();
    let bytes_before = toki_store::history::BYTES_COPIED.load(Ordering::Relaxed);
    let started = Instant::now();
    let mut total = resolver::Stats::default();
    let mut passes = 0u32;
    loop {
        let s = resolver::pass(&store).map_err(|e| e.to_string())?;
        if s.attempted == 0 {
            break;
        }
        passes += 1;
        total.attempted += s.attempted;
        total.resolved += s.resolved;
        total.missed += s.missed;
        total.not_a_page += s.not_a_page;
        total.visits += s.visits;
    }
    let ms = started.elapsed().as_millis() as u64;
    let bytes = toki_store::history::BYTES_COPIED.load(Ordering::Relaxed) - bytes_before;
    let (ws_peak, private_peak) = sampler.stop();

    Ok(serde_json::json!({
        "hours": hours,
        "queued": queued,
        "passes": passes,
        "attempted": total.attempted,
        "resolved": total.resolved,
        "missed": total.missed,
        "not_a_page": total.not_a_page,
        "hit_rate": total.hit_rate(),
        "visits": total.visits,
        "ms_total": ms,
        "ms_per_pass": ms / u64::from(passes.max(1)),
        "bytes_written": bytes,
        "bytes_written_per_pass": bytes / u64::from(passes.max(1)),
        "working_set_before": ws_before,
        "working_set_peak": ws_peak,
        "private_before": private_before,
        "private_peak": private_peak,
    }))
}

/// Polls this process's memory every millisecond on its own thread and keeps the maximum.
/// The OS's own peak counters cover the whole process lifetime, which here includes taking
/// the database copy, so they cannot isolate a pass.
struct MemorySampler {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: std::thread::JoinHandle<(u64, u64)>,
}

impl MemorySampler {
    fn start() -> Self {
        use std::sync::atomic::{AtomicBool, Ordering};
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut peak = (0, 0);
            while !flag.load(Ordering::Relaxed) {
                if let Some((ws, private)) = platform::memory::usage() {
                    peak = (peak.0.max(ws), peak.1.max(private));
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            peak
        });
        Self { stop, thread }
    }

    fn stop(self) -> (u64, u64) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.thread.join().unwrap_or_default()
    }
}

/// Try the domain lookup again over a date range.
///
/// Worth running after a browser has caught up on writing history, or after a change to
/// title normalization. Rows that already resolved are redone too: if normalization changed,
/// the old answer was produced by the old rules.
fn reresolve(args: &[String]) -> ExitCode {
    let Some(from) = flag(args, "--from").and_then(|v| parse_date(&v)) else {
        eprintln!("reresolve: --from is required, as YYYY-MM-DD");
        return ExitCode::FAILURE;
    };
    let to = flag(args, "--to")
        .and_then(|v| parse_date(&v))
        .unwrap_or_else(|| Local::now().date_naive());
    if to < from {
        eprintln!("reresolve: --to is before --from");
        return ExitCode::FAILURE;
    }

    let offset_min = local_offset_min();
    let (start, _) = day_bounds(from, offset_min);
    let (_, end) = day_bounds(to, offset_min);

    let store = match Store::open(&paths::db_path()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("reresolve: cannot open database: {e}");
            return ExitCode::FAILURE;
        }
    };
    let queued = match store.reset_resolution(start, end) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("reresolve: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("requeued {queued} browser sessions from {from} to {to} inclusive");

    // The queue can exceed one batch, so keep going until a pass finds nothing.
    let mut total = resolver::Stats::default();
    loop {
        match resolver::pass(&store) {
            Ok(s) if s.attempted == 0 => break,
            Ok(s) => {
                total.attempted += s.attempted;
                total.resolved += s.resolved;
                total.missed += s.missed;
                total.visits += s.visits;
            }
            Err(e) => {
                eprintln!("reresolve: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    println!(
        "resolved {} missed {} of {} ({:.1}% hit) from {} history visits",
        total.resolved,
        total.missed,
        total.attempted,
        total.hit_rate() * 100.0,
        total.visits
    );
    ExitCode::SUCCESS
}

fn purge(args: &[String]) -> ExitCode {
    let from = flag(args, "--from").and_then(|v| parse_date(&v));
    let to = flag(args, "--to").and_then(|v| parse_date(&v));
    let app = flag(args, "--app");

    let (Some(from), Some(to)) = (from, to) else {
        eprintln!("purge: --from and --to are required, as YYYY-MM-DD");
        return ExitCode::FAILURE;
    };
    if to < from {
        eprintln!("purge: --to is before --from");
        return ExitCode::FAILURE;
    }

    let store = match Store::open(&paths::db_path()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("purge: cannot open database: {e}");
            return ExitCode::FAILURE;
        }
    };
    match store.purge(from, to, local_offset_min(), app.as_deref()) {
        Ok((sessions, totals)) => {
            println!(
                "deleted {sessions} sessions and {totals} daily totals from {from} to {to} inclusive"
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("purge: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Turn starting-with-Windows on or off, or say what it is now.
///
/// Reporting the value rather than just "on" is the point: the common failure is a Run entry
/// left pointing at where the executable used to be.
fn autostart(arg: Option<&str>) -> ExitCode {
    use platform::autostart;

    let result = match arg {
        Some("on") => autostart::enable(),
        Some("off") => autostart::disable(),
        None | Some("status") => {
            match autostart::current() {
                Some(cmd) => {
                    println!("on   {cmd}");
                    // A stale path is silently a daemon that stops starting, so the check
                    // is worth the two lines.
                    if autostart::command().is_some_and(|want| want != cmd) {
                        println!("     does not match this executable — run `autostart on`");
                    }
                }
                None => println!("off"),
            }
            return ExitCode::SUCCESS;
        }
        Some(other) => {
            eprintln!("autostart: expected on, off, or nothing, not {other}");
            return ExitCode::FAILURE;
        }
    };

    match result {
        Ok(()) => {
            println!("autostart {}", arg.unwrap_or("status"));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("autostart: {e}");
            ExitCode::FAILURE
        }
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).cloned()
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// The offset in force right now, which is what gets stored on rows written now.
fn local_offset_min() -> i32 {
    Local::now().offset().local_minus_utc() / 60
}
