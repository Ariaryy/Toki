//! The collector loop.
//!
//! Three threads. The pump talks to Windows and sends facts. The resolver reads browser
//! history on its own schedule. This one is the only thing that decides anything, and the
//! only thing that writes sessions.
//!
//! It owns the `Tracker`, so all session assembly happens on one thread with no locking,
//! and the store connection lives here too — SQLite in WAL mode lets the resolver hold a
//! second connection for its updates.

use std::collections::HashMap;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use chrono::{Datelike, Local, Months, Utc};
use tracing::{error, info, warn};

use toki_core::event::{AppKey, Millis};
use toki_core::resolve::Engine;
use toki_core::{Action, Config, Event, Tracker};
use toki_store::{OpenSession, Store};

use crate::platform::{audio, console, idle, ipc, pump, single, tray, window, Raw, Window};
use crate::resolver::ResolverMsg;
use crate::state::{PauseState, Shared};
use crate::{digest, logging, paths, resolver};

/// How often presence is sampled. Fifteen seconds bounds how far past the idle threshold a
/// session can run before it is closed. The close is backdated to the last input, so the
/// sampling rate costs accuracy only in when the row appears, not in what it says.
const TICK: Duration = Duration::from_secs(15);
/// How often `last_seen` is written. Crash recovery closes a dangling session there, so
/// this is the worst-case over-count from a power cut.
const HEARTBEAT_MS: Millis = 30_000;
/// How often the resolver drains the pending queue.
const RESOLVE_EVERY: Duration = Duration::from_secs(5 * 60);
/// How far wall time may move between one pass of the loop and the next before the loop is
/// taken not to have run across the gap. An arriving event only makes a pass happen sooner
/// than `TICK`, never later, so four missed ticks is well past anything scheduling jitter
/// on a loaded machine explains.
const MAX_LOOP_GAP_MS: Millis = 60_000;

pub fn run() -> Result<(), String> {
    // Before anything is written, because whether there is a console decides where the
    // first line goes.
    let console_attached = console::detach_if_ours();
    logging::init(console_attached);

    let _instance = single::Instance::acquire()
        .ok_or_else(|| "another toki-daemon is already running".to_string())?;

    let store = Store::open(&paths::db_path()).map_err(|e| format!("cannot open database: {e}"))?;
    info!(db = %paths::db_path().display(), "starting");

    // Anything left open by a crash is closed at the last heartbeat before this run writes
    // a single row, so the two can never overlap.
    match store.recover() {
        Ok(Some(s)) => info!(
            app = s.app.as_str(),
            seconds = s.duration_ms() / 1_000,
            "recovered a session left open by the last run"
        ),
        Ok(None) => {}
        Err(e) => warn!(%e, "crash recovery failed"),
    }

    // Retention: 6 months of raw sessions, older data collapsed into daily_rollup — see
    // DESIGN.md §7. Run once per start rather than on its own timer; the daemon restarts
    // routinely (autostart on login, updates), which is a fine cadence for housekeeping
    // this coarse-grained.
    match store.rollup_before(retention_cutoff()) {
        Ok(0) => {}
        Ok(n) => info!(sessions = n, "rolled up sessions older than 6 months"),
        Err(e) => warn!(%e, "retention rollup failed"),
    }

    let resolver_tx = spawn_resolver();
    let shared = Arc::new(Shared::default());

    let (tx, rx) = channel::<Raw>();
    let ipc_thread = {
        let tx = tx.clone();
        let resolver_tx = resolver_tx.clone();
        let shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("toki-ipc".into())
            .spawn(move || ipc::run(tx, resolver_tx, shared))
            .map_err(|e| format!("cannot start the IPC server: {e}"))?
    };
    let pump_thread = std::thread::Builder::new()
        .name("toki-pump".into())
        .spawn(move || pump::run(tx))
        .map_err(|e| format!("cannot start the event pump: {e}"))?;
    console::trap();

    // Opens the GUI once so there is visible evidence Toki works and the tray icon is
    // findable — see DESIGN.md §4 "First run". The flag lives in `meta`, so it survives
    // restarts and this fires exactly once for the life of the database, not once per boot.
    match store.first_run_seen() {
        Ok(false) => {
            pump::open_gui();
            if let Err(e) = store.mark_first_run_seen() {
                warn!(%e, "failed to record first run");
            }
        }
        Ok(true) => {}
        Err(e) => warn!(%e, "failed to read first-run state"),
    }

    // Held for the life of the loop so the audio probe is not entering and leaving a COM
    // apartment on every call.
    let _com = audio::Com::init();

    let mut collector = Collector::new(store, shared);
    loop {
        match rx.recv_timeout(TICK) {
            Ok(raw) => {
                // Before the event, not only on the tick below. Every `Raw` carries its own
                // wall-clock stamp, so one that arrived after the clock moved would close
                // the open session at the post-jump time and write the jump down as time
                // spent — the tick that would have caught it never gets to run first.
                collector.check_continuity(now());
                if collector.handle(raw) {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            // The pump has gone. Continuing would record time nobody is watching.
            Err(RecvTimeoutError::Disconnected) => break,
        }
        collector.tick(now());
    }

    collector.shutdown(now());
    let _ = resolver_tx.send(ResolverMsg::Stop);
    ipc::stop();
    pump::stop();
    let _ = pump_thread.join();
    let _ = ipc_thread.join();
    info!("stopped");
    Ok(())
}

fn now() -> Millis {
    Utc::now().timestamp_millis()
}

/// The offset in force right now, which is what gets stored on rows written now.
fn local_offset_min() -> i32 {
    Local::now().offset().local_minus_utc() / 60
}

/// Raw `sessions` rows older than this are eligible for `daily_rollup`. A calendar-month
/// subtraction, not a fixed day count, so "6 months" tracks what a person means by it
/// across months of different lengths.
fn retention_cutoff() -> Millis {
    Utc::now()
        .checked_sub_months(Months::new(6))
        .unwrap_or_else(|| Utc::now() - chrono::Duration::days(180))
        .timestamp_millis()
}

/// What Windows can tell us about an app beyond its path, read once.
struct AppInfo {
    engine: Option<Engine>,
}

struct Collector {
    store: Store,
    tracker: Tracker,
    apps: HashMap<String, AppInfo>,
    /// The window the open session belongs to, kept for the audio probe and for reopening
    /// after idle.
    current: Option<Window>,
    last_input: Millis,
    /// Wall clock as of the last pass of the run loop, so the next one can tell whether
    /// time moved further than a pass could account for — see
    /// [`Collector::check_continuity`].
    last_pass: Millis,
    next_heartbeat: Millis,
    /// When a timed pause ends. `None` while running, and also while paused indefinitely —
    /// the tracker is what knows which, so the two are never confused.
    ///
    /// Deliberately not persisted. A pause is a decision about the next hour, not about the
    /// installation, and one that survived a reboot would be a tracker that had silently
    /// stopped. Restarting the daemon resumes.
    paused_until: Option<Millis>,
    /// Mirrors `tracker.is_open()` as of the last write, so the `meta` row is only touched
    /// when it actually changes.
    mirrored: bool,
    /// Whether the last auxiliary `meta` write failed. Exists so a failing database is
    /// reported once rather than every 30 seconds — see [`Collector::aux_write`].
    aux_failing: bool,
    /// The IPC server's read side of the pause state. Updated alongside `paused_until`
    /// rather than derived from it on demand, since `get_state` runs on another thread.
    shared: Arc<Shared>,
}

impl Collector {
    fn new(store: Store, shared: Arc<Shared>) -> Self {
        let at = now();
        // Settings may have overridden the idle threshold on a previous run. Anything it
        // never touched keeps the default — this only ever narrows `Config::default()` by
        // one field, never invents the other two.
        let mut cfg = Config::default();
        if let Ok(Some(ms)) = store.idle_threshold_ms() {
            cfg.idle_threshold_ms = ms;
        }
        // Carry the absorbed-idle sample across restarts, so a week of it survives the
        // daemon stopping and starting rather than resetting to nothing each time. A
        // missing or unreadable value just starts the count over — this is measurement,
        // not data anyone would miss.
        let mut tracker = Tracker::new(cfg);
        if let Ok(Some(json)) = store.idle_histogram_json() {
            match serde_json::from_str(&json) {
                Ok(h) => tracker.set_idle_histogram(h),
                Err(e) => warn!(%e, "discarding unreadable idle histogram"),
            }
        }

        Self {
            store,
            tracker,
            apps: HashMap::new(),
            current: None,
            last_input: idle::last_input_at(at),
            last_pass: at,
            aux_failing: false,
            next_heartbeat: at,
            paused_until: None,
            mirrored: false,
            shared,
        }
    }

    /// Handle one fact from the pump. Returns true when the loop should end.
    fn handle(&mut self, raw: Raw) -> bool {
        match raw {
            Raw::Focus { at, window } => {
                self.focus(at, window);
                false
            }
            Raw::Suspend { at } | Raw::Lock { at } => {
                let actions = self.tracker.on_event(Event::Suspend { at });
                self.emit(actions);
                self.current = None;
                self.mirror();
                false
            }
            // Windows does not reliably re-announce the foreground window after a wake or
            // an unlock, so it is asked directly rather than waited for.
            Raw::Resume { at } => {
                if let Some(w) = pump::current_foreground() {
                    if !toki_core::ignore::is_default_ignored(&w.exe_path)
                        && !toki_core::ignore::is_shell_chrome(&w.exe_path, &w.class)
                    {
                        self.focus(at, w);
                    }
                }
                false
            }
            Raw::Pause { at, until } => {
                self.pause(at, until);
                false
            }
            Raw::Unpause { at } => {
                self.unpause(at);
                false
            }
            Raw::SetConfig { idle_threshold_ms } => {
                let mut cfg = *self.tracker.config();
                cfg.idle_threshold_ms = idle_threshold_ms;
                self.tracker.set_config(cfg);
                info!(
                    minutes = idle_threshold_ms / 60_000,
                    "idle threshold changed"
                );
                false
            }
            Raw::Stop { .. } => true,
        }
    }

    /// Stop recording. The open session is closed at `at`, so the paused span is absent from
    /// the day rather than credited to whatever was in front when it began.
    fn pause(&mut self, at: Millis, until: Option<Millis>) {
        let actions = self.tracker.on_event(Event::PauseStart { at });
        self.emit(actions);
        self.paused_until = until;
        self.mirror();
        tray::set_paused(true);
        self.shared.set(PauseState {
            paused: true,
            until,
        });
        match until {
            Some(u) => info!(minutes = (u - at) / 60_000, "paused"),
            None => info!("paused until resumed"),
        }
    }

    /// Start recording again, and pick up whatever is in front.
    ///
    /// Resuming changes no window and renames nothing, so — exactly as with a return from
    /// idle — no event arrives on its own and the foreground has to be asked for.
    fn unpause(&mut self, at: Millis) {
        if !self.tracker.is_paused() {
            return;
        }
        let actions = self.tracker.on_event(Event::PauseEnd { at });
        self.emit(actions);
        self.paused_until = None;
        tray::set_paused(false);
        self.shared.set(PauseState {
            paused: false,
            until: None,
        });
        info!("resumed");
        if let Some(w) = pump::current_foreground() {
            if !toki_core::ignore::is_default_ignored(&w.exe_path)
                && !toki_core::ignore::is_shell_chrome(&w.exe_path, &w.class)
            {
                self.focus(at, w);
            }
        }
    }

    /// Record a change of foreground window or caption.
    fn focus(&mut self, at: Millis, window: Window) {
        self.learn(&window);
        let actions = self.tracker.on_event(Event::Focus {
            at,
            app: AppKey::new(window.exe_path.clone()),
            title: window.title.clone(),
        });
        self.emit(actions);
        self.current = Some(window);
        self.mirror();
        self.maybe_show_weekly_digest(at);
    }

    /// Pop the weekly digest balloon on the first foreground activity of Monday — see
    /// DESIGN.md §10. `focus` fires on the very first foreground change of a session,
    /// including one right after boot or unlock, which is what "first foreground activity"
    /// means here; there is no separate timer to miss while asleep.
    fn maybe_show_weekly_digest(&mut self, at: Millis) {
        let offset = local_offset_min();
        let today = toki_core::time::local_date(at, offset);
        if today.weekday() != chrono::Weekday::Mon {
            return;
        }
        match self.store.last_digest_monday() {
            Ok(Some(last)) if last == today => return,
            Err(e) => {
                warn!(%e, "failed to read last digest date");
                return;
            }
            Ok(_) => {}
        }
        let digest = match digest::for_week_ending(&self.store, today, offset) {
            Ok(d) => d,
            Err(e) => {
                warn!(%e, "weekly digest failed");
                return;
            }
        };

        // Not marked done when this fails: the tray icon didn't exist yet to show it on, so
        // the next focus event today gets another try instead of the digest being spent
        // silently.
        let (title, body) = digest::format(&digest);
        if !tray::show_digest(&title, &body) {
            return;
        }
        if let Err(e) = self.store.set_last_digest_monday(today) {
            warn!(%e, "failed to record digest date");
        }
    }

    /// Close the open session when wall time has moved further than one pass of the loop
    /// could account for.
    ///
    /// Every timestamp in this daemon comes from the wall clock, `last_input_at` included —
    /// it is reported as `now - idle`. So a clock correction carries the input sample along
    /// with it and nothing downstream can tell the jump from ordinary activity: the open
    /// session simply absorbs the whole of it as time spent. This is not hypothetical. A
    /// boot with the RTC hours behind, corrected by NTP once the network came up, left a
    /// ten-hour Discord session in this machine's database on 2026-09-11.
    ///
    /// A machine that slept without telling us looks exactly the same from in here — modern
    /// standby gets no `PBT_APMSUSPEND` — and so does a process starved off the CPU long
    /// enough to miss its ticks. None of the three needs to be identified, because they
    /// want the same answer: the last pass the daemon actually made bounds what it can
    /// vouch for, so the session ends no later than that — earlier if input had already
    /// stopped before it, which [`Tracker::close_unaccounted`] works out. Whatever comes
    /// back afterwards reopens through the ordinary idle-return path.
    ///
    /// [`Tracker::on_input_return`] states the same rule where it belongs, in terms of the
    /// idle threshold rather than the loop. This is the half that can see a gap at all.
    fn check_continuity(&mut self, at: Millis) {
        let last = std::mem::replace(&mut self.last_pass, at);
        // Backwards as well as forwards: a clock moved back leaves `at - open.last_input`
        // shrinking instead of growing, so the session never reaches the threshold and runs
        // until the clock catches up with itself.
        if (0..=MAX_LOOP_GAP_MS).contains(&(at - last)) {
            return;
        }

        let actions = self.tracker.close_unaccounted(last);
        let closed = !actions.is_empty();
        self.emit(actions);
        self.mirror();
        // The mirror just changed and the next start recovers from it, so do not sit on
        // that for another half minute. Also unsticks a heartbeat scheduled past a clock
        // that has since moved back.
        self.next_heartbeat = at;
        warn!(
            seconds = (at - last) / 1_000,
            closed, "wall clock moved further than the loop could have"
        );
    }

    /// Presence sampling, the heartbeat, and the reopen that has to follow an idle close.
    fn tick(&mut self, at: Millis) {
        self.check_continuity(at);

        // A timed pause ends here rather than on a timer of its own. Fifteen seconds late
        // costs nothing when the shortest pause on offer is fifteen minutes.
        if self.paused_until.is_some_and(|until| at >= until) {
            self.unpause(at);
        }

        let seen = idle::last_input_at(at);

        let actions = {
            let current = self.current.clone();
            self.tracker.on_tick(at, seen, || match &current {
                Some(w) => audio::is_rendering(&w.exe_path),
                None => false,
            })
        };
        self.emit(actions);

        // Coming back from idle changes no window and renames nothing, so no event fires.
        // Without this, the session that idle closed never reopens and the rest of the day
        // goes unrecorded.
        if !self.tracker.is_open() && !self.tracker.is_paused() && seen > self.last_input {
            if let Some(w) = self.current.clone() {
                self.focus(seen, w);
            }
        }
        self.last_input = seen;
        self.mirror();

        if at >= self.next_heartbeat {
            let r = self.store.heartbeat(at);
            self.aux_write("heartbeat", r);
            self.persist_idle_histogram();
            self.next_heartbeat = at + HEARTBEAT_MS;
        }
    }

    /// Write the absorbed-idle counts out. Rides the heartbeat rather than firing per tick:
    /// it is one small `meta` row either way, and at heartbeat cadence a kill costs at most
    /// the last 30 seconds of counting.
    fn persist_idle_histogram(&mut self) {
        match serde_json::to_string(&self.tracker.idle_histogram()) {
            Ok(json) => {
                let r = self.store.set_idle_histogram_json(&json);
                self.aux_write("idle histogram", r);
            }
            Err(e) => warn!(%e, "failed to encode idle histogram"),
        }
    }

    /// Close cleanly: at the moment we were told to stop, not at the last heartbeat, and
    /// with the mirror cleared so the next start finds nothing to recover.
    fn shutdown(&mut self, at: Millis) {
        let actions = self.tracker.on_event(Event::Shutdown { at });
        self.emit(actions);
        let r = self.store.set_open_session(None);
        self.aux_write("clear open session", r);
        let r = self.store.heartbeat(at);
        self.aux_write("heartbeat", r);
        // Not just on the heartbeat: a clean stop should not throw away the counting done
        // since the last one.
        self.persist_idle_histogram();
    }

    /// Read what Windows can tell us about an app, once, the first time it is seen.
    fn learn(&mut self, w: &Window) {
        if self.apps.contains_key(&w.exe_path) {
            return;
        }
        // Touches the disk, which is why it happens here rather than in the hook callback.
        if let Some(name) = window::display_name(&w.exe_path) {
            let r = self.store.set_detected_name(&w.exe_path, &name);
            self.aux_write("detected name", r);
        }
        self.apps.insert(
            w.exe_path.clone(),
            AppInfo {
                engine: Engine::of_exe(&w.exe_path),
            },
        );
    }

    /// Write what the tracker decided.
    ///
    /// The engine comes from the session's own app, not from whatever is in front now: a
    /// focus change hands back the action that closes the *previous* session.
    fn emit(&mut self, actions: Vec<Action>) {
        for action in &actions {
            let engine = match action {
                Action::Emit(s) => self.apps.get(s.app.as_str()).and_then(|a| a.engine),
                Action::ExtendLast { .. } => None,
            };
            if let Err(e) = self.store.apply(action, local_offset_min(), engine) {
                error!(%e, "write failed");
            }
        }
    }

    /// Report a failed auxiliary `meta` write — the heartbeat, the open-session mirror, and
    /// the two measurement rows.
    ///
    /// These deliberately don't abort anything the way a failed session write would: losing
    /// one is survivable. Losing them *silently* is not, which is what this fixes. The
    /// heartbeat and the open-session mirror together are the entire crash-recovery
    /// mechanism, so if they stop landing, the next start recovers a dangling session to a
    /// stale timestamp — or misses it altogether — and nothing anywhere says why.
    ///
    /// Logged on the transition rather than per call. All four writes go to one database
    /// and fail together, so a broken one would otherwise emit a line every 30 seconds,
    /// burying the first and most useful report under thousands of copies — and writing
    /// hard to a disk that may be full, which is a plausible reason it is failing at all.
    fn aux_write(&mut self, what: &'static str, r: rusqlite::Result<()>) {
        match (r, self.aux_failing) {
            (Err(e), false) => {
                warn!(%e, what, "auxiliary write failed; crash recovery may be stale");
                self.aux_failing = true;
            }
            (Ok(()), true) => {
                info!(what, "auxiliary writes recovered");
                self.aux_failing = false;
            }
            _ => {}
        }
    }

    /// Keep `meta`'s copy of the in-progress session in step, writing only on a change.
    fn mirror(&mut self) {
        let open = self.tracker.is_open();
        match (open, &self.current) {
            (true, Some(w)) => {
                let r = self.store.set_open_session(Some(&OpenSession {
                    start: self.tracker.open_start().unwrap_or_else(now),
                    app: AppKey::new(w.exe_path.clone()),
                    title: w.title.clone(),
                    tz_offset_min: local_offset_min(),
                    engine: self.apps.get(&w.exe_path).and_then(|a| a.engine),
                }));
                self.aux_write("open session mirror", r);
                self.mirrored = true;
            }
            _ => {
                if self.mirrored {
                    let r = self.store.set_open_session(None);
                    self.aux_write("clear open session", r);
                    self.mirrored = false;
                }
            }
        }
    }
}

/// The resolver runs on its own connection and its own clock.
///
/// Separate because building the title index copies five SQLite files and takes about
/// 140 ms. On the collector thread that would be 140 ms of foreground changes queued behind
/// it every five minutes.
///
/// Returns the sender side of its wake channel, so the IPC layer's `resolve_now` — and, once
/// it exists, the GUI's launch-time backfill — can run a pass without waiting out the rest of
/// the current interval.
fn spawn_resolver() -> Sender<ResolverMsg> {
    let (tx, rx) = channel::<ResolverMsg>();
    let _ = std::thread::Builder::new()
        .name("toki-resolver".into())
        .spawn(move || {
            let Ok(store) = Store::open(&paths::db_path()) else {
                return;
            };
            loop {
                match resolver::pass(&store) {
                    Ok(s) if s.attempted > 0 => info!(
                        resolved = s.resolved,
                        missed = s.missed,
                        not_a_page = s.not_a_page,
                        attempted = s.attempted,
                        hit = format!("{:.0}%", s.hit_rate() * 100.0),
                        "resolver pass"
                    ),
                    Ok(_) => {}
                    Err(e) => warn!(%e, "resolver"),
                }
                match rx.recv_timeout(RESOLVE_EVERY) {
                    Ok(ResolverMsg::RunNow) | Err(RecvTimeoutError::Timeout) => {}
                    Ok(ResolverMsg::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        });
    tx
}
