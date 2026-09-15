//! SQLite storage for Toki.
//!
//! Lives in its own crate because both binaries need it: `toki-daemon` is the sole writer,
//! and the GUI opens the same file read-only. Keeping the schema and the day-clamp query in
//! one place stops the two from drifting apart.
//!
//! `sessions` holds closed intervals only. The session currently in progress is kept in
//! `meta` instead, so a crash can be repaired from the heartbeat without leaving half-written
//! rows behind. See [`Store::recover`].

pub mod history;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Result, Transaction};
use serde::Serialize;
use toki_core::event::{AppKey, Millis, Presence};
use toki_core::resolve::Engine;
use toki_core::time::{day_bounds, local_date, overlap_ms};
use toki_core::tracker::{Action, Session, IDLE_THRESHOLD_BOUNDS_MS};

/// One entry per schema version after the first. `MIGRATIONS[0]` takes a version-1 database
/// to version 2, and so on, so [`SCHEMA_VERSION`] cannot drift from the list by construction.
///
/// Each entry runs in its own transaction and must be safe to apply exactly once. `SCHEMA`
/// always describes the *latest* shape — a fresh database gets it whole and skips every
/// migration, so a change to the schema means editing `SCHEMA` **and** appending the step
/// that brings an existing file to the same place.
const MIGRATIONS: &[&str] = &[
    // -> 2: reclassify resolution failures. Until now a caption that could never match —
    // a PWA window, Picture-in-Picture, the browser's own empty window — was recorded as
    // RESOLVE_FAILED, the same state as a page that genuinely missed. Putting the failures
    // back in the queue lets the next resolver pass sort them into the two states.
    "UPDATE sessions SET resolve_state = 0 WHERE resolve_state = 3;",
];

const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64 + 1;

/// `%LOCALAPPDATA%\Toki\toki.db` — defined once so the daemon, which writes it, and the
/// GUI, which only reads it, can never end up pointed at different files.
///
/// Deliberately not Documents or any synced folder: the database is unencrypted and holds
/// window titles.
pub fn db_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("Toki").join("toki.db")
}

/// A browser session whose domain has not been looked up yet.
pub const RESOLVE_PENDING: i64 = 0;
pub const RESOLVE_DONE: i64 = 1;
/// The foreground app was not a browser, so there is nothing to resolve.
pub const RESOLVE_NOT_BROWSER: i64 = 2;
/// Looked up and missed: a page caption that no history row matched. This is the one that
/// means something is wrong, and the one `toki reresolve` exists to retry.
pub const RESOLVE_FAILED: i64 = 3;
/// A browser window that was never showing a page — a PWA or app window, Picture-in-Picture,
/// the browser's own empty window, a workspace switcher. There is no URL behind it, so no
/// lookup can ever succeed and a retry is wasted. Kept apart from [`RESOLVE_FAILED`] so the
/// hit rate measures the thing it claims to.
pub const RESOLVE_NOT_A_PAGE: i64 = 4;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS apps (
  id            INTEGER PRIMARY KEY,
  exe_path      TEXT NOT NULL UNIQUE,
  detected_name TEXT,
  alias         TEXT,
  engine        INTEGER,
  ignored       INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS domains (
  id     INTEGER PRIMARY KEY,
  domain TEXT NOT NULL UNIQUE,
  alias  TEXT
);

CREATE TABLE IF NOT EXISTS titles (
  id    INTEGER PRIMARY KEY,
  title TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS sessions (
  id            INTEGER PRIMARY KEY,
  start_utc_ms  INTEGER NOT NULL,
  end_utc_ms    INTEGER NOT NULL,
  tz_offset_min INTEGER NOT NULL,
  app_id        INTEGER NOT NULL REFERENCES apps(id),
  title_id      INTEGER REFERENCES titles(id),
  domain_id     INTEGER REFERENCES domains(id),
  resolve_state INTEGER NOT NULL DEFAULT 0,
  presence      INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS sessions_start ON sessions(start_utc_ms);
CREATE INDEX IF NOT EXISTS sessions_pending ON sessions(resolve_state)
  WHERE resolve_state = 0;

CREATE TABLE IF NOT EXISTS daily_rollup (
  local_date TEXT NOT NULL,
  app_id     INTEGER NOT NULL,
  domain_id  INTEGER,
  seconds    INTEGER NOT NULL,
  PRIMARY KEY (local_date, app_id, domain_id)
);

CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT NOT NULL);
"#;

/// Bring an existing database up to [`SCHEMA_VERSION`], or stamp a fresh one.
///
/// `user_version` is the whole record of where a file stands: SQLite keeps it in the header,
/// it costs nothing to read, and a copy, a backup or a restore carries it along. Before this
/// existed the version was stamped unconditionally on every open, which would have marked
/// every database in the wild as already migrated the first time the schema changed.
///
/// A file from the future is refused rather than written to. An older build cannot know what
/// a newer one changed, and opening it anyway corrupts it for both.
fn migrate(conn: &Connection) -> Result<()> {
    let from: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    // Zero means the file did not exist a moment ago: `SCHEMA` has just created it at the
    // latest shape, so there is nothing to step through.
    if from == 0 {
        return conn.pragma_update(None, "user_version", SCHEMA_VERSION);
    }
    if from > SCHEMA_VERSION {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
            Some(format!(
                "database is schema version {from}; this build understands {SCHEMA_VERSION}"
            )),
        ));
    }
    // `MIGRATIONS[i]` moves a database from version `i + 1` to `i + 2`, so a file at `from`
    // resumes at index `from - 1`. Each step commits with its own stamp: an interrupted
    // upgrade leaves the file at the last version that fully applied, never between two.
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(from as usize - 1) {
        let to = i as i64 + 2;
        conn.execute_batch(&format!("BEGIN; {sql} PRAGMA user_version = {to}; COMMIT;"))?;
    }
    Ok(())
}

/// The session in progress, mirrored into `meta` so a crash is recoverable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenSession {
    pub start: Millis,
    pub app: AppKey,
    pub title: String,
    pub tz_offset_min: i32,
    pub engine: Option<Engine>,
}

/// One row of a day breakdown. `domain` is `None` for non-browser apps and for browser
/// sessions that never resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Total {
    pub exe_path: String,
    pub app_name: String,
    pub domain: Option<String>,
    pub ms: Millis,
}

/// One row of the Settings app list — every app ever seen, not just ones with time today.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRow {
    pub exe_path: String,
    /// `COALESCE(alias, detected_name, exe_path)` — the name shown everywhere else.
    pub name: String,
    pub alias: Option<String>,
    pub ignored: bool,
}

/// One row of the Settings domain list — every domain ever resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainRow {
    pub domain: String,
    /// `COALESCE(alias, domain)`.
    pub name: String,
    pub alias: Option<String>,
}

/// How one browser's sessions are split across the resolution states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveRow {
    /// The app's display name, the same one totals and Settings use.
    pub name: String,
    pub resolved: i64,
    pub missed: i64,
    pub not_a_page: i64,
    pub pending: i64,
}

/// A session awaiting domain resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub id: i64,
    pub title: String,
    pub engine: Engine,
}

/// What a lookup decided about one pending session. The three outcomes are the three
/// terminal `resolve_state`s a browser session can reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// A history row matched. [`RESOLVE_DONE`].
    Found(String),
    /// A page caption that nothing matched. Worth retrying — history is written when the
    /// visit is recorded, which can be after the caption changed. [`RESOLVE_FAILED`].
    Missed,
    /// Not a page window at all, so no retry can help. [`RESOLVE_NOT_A_PAGE`].
    NotAPage,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::from_conn(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::from_conn(Connection::open_in_memory()?)
    }

    /// The GUI's connection. Cannot write, so it cannot contend with the daemon.
    pub fn open_read_only(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(Self { conn })
    }

    fn from_conn(conn: Connection) -> Result<Self> {
        // WAL lets the GUI read while the daemon writes. It is a no-op on :memory:.
        let _: String = conn
            .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
            .unwrap_or_default();
        // SQLite's own recommendation for WAL mode: skip the fsync on most commits, since
        // the WAL file is what a crash recovers from anyway. The default, FULL, fsyncs every
        // commit — measured at ~700x write amplification against the actual row data for a
        // collector whose crash tolerance already lives in the 30s heartbeat, not in never
        // losing the last few seconds to an OS crash or power loss.
        conn.execute_batch("PRAGMA synchronous = NORMAL;")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
        Ok(Self { conn })
    }

    pub fn schema_version(&self) -> Result<i64> {
        self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))
    }

    // ---------------------------------------------------------------- writes

    /// Carry out what the tracker decided. Nothing else writes to `sessions`.
    pub fn apply(&self, action: &Action, tz_offset_min: i32, engine: Option<Engine>) -> Result<()> {
        match action {
            Action::Emit(session) => self.insert_session(session, tz_offset_min, engine),
            Action::ExtendLast { end } => self.extend_last(*end),
        }
    }

    fn insert_session(
        &self,
        session: &Session,
        tz_offset_min: i32,
        engine: Option<Engine>,
    ) -> Result<()> {
        let app_id = self.intern_app(session.app.as_str(), engine)?;
        let title_id = self.intern_title(&session.title)?;
        let resolve_state = if engine.is_some() {
            RESOLVE_PENDING
        } else {
            RESOLVE_NOT_BROWSER
        };
        self.conn.execute(
            "INSERT INTO sessions
               (start_utc_ms, end_utc_ms, tz_offset_min, app_id, title_id, resolve_state, presence)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session.start,
                session.end,
                tz_offset_min,
                app_id,
                title_id,
                resolve_state,
                presence_code(session.presence),
            ],
        )?;
        Ok(())
    }

    /// Absorb a sub-noise-floor blip into the previous row. Guarded so it can only ever move
    /// an end forward.
    fn extend_last(&self, end: Millis) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET end_utc_ms = ?1
             WHERE id = (SELECT max(id) FROM sessions) AND end_utc_ms < ?1",
            params![end],
        )?;
        Ok(())
    }

    /// Record the name Windows gives an executable, once.
    ///
    /// Only ever fills a NULL. The version resource is read the first time an app is seen,
    /// and re-reading it after an update should not silently rewrite history the user has
    /// been looking at. `alias` is the field that is meant to change.
    pub fn set_detected_name(&self, exe_path: &str, name: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO apps (exe_path) VALUES (?1)",
            params![exe_path],
        )?;
        self.conn.execute(
            "UPDATE apps SET detected_name = ?2 WHERE exe_path = ?1 AND detected_name IS NULL",
            params![exe_path, name],
        )?;
        Ok(())
    }

    /// Executables the user has marked as not worth recording.
    ///
    /// Separate from [`toki_core::ignore`]'s built-in list, which covers shell surfaces
    /// everyone wants gone. This one is personal and lives in the database.
    pub fn ignored_apps(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT exe_path FROM apps WHERE ignored = 1")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect()
    }

    /// Mark an executable ignored, or stop ignoring it.
    pub fn set_ignored(&self, exe_path: &str, ignored: bool) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO apps (exe_path) VALUES (?1)",
            params![exe_path],
        )?;
        self.conn.execute(
            "UPDATE apps SET ignored = ?2 WHERE exe_path = ?1",
            params![exe_path, i64::from(ignored)],
        )?;
        Ok(())
    }

    /// Rename an app for display, or clear the alias with `None` to fall back to the
    /// detected name or the exe path. Unlike `set_detected_name`, this always overwrites —
    /// the alias is the one field a user is meant to keep changing their mind about.
    pub fn set_app_alias(&self, exe_path: &str, alias: Option<&str>) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO apps (exe_path) VALUES (?1)",
            params![exe_path],
        )?;
        self.conn.execute(
            "UPDATE apps SET alias = ?2 WHERE exe_path = ?1",
            params![exe_path, alias],
        )?;
        Ok(())
    }

    /// Rename a domain for display, or clear the alias with `None`.
    pub fn set_domain_alias(&self, domain: &str, alias: Option<&str>) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO domains (domain) VALUES (?1)",
            params![domain],
        )?;
        self.conn.execute(
            "UPDATE domains SET alias = ?2 WHERE domain = ?1",
            params![domain, alias],
        )?;
        Ok(())
    }

    /// The idle threshold to use on the next daemon start, if Settings has ever overridden
    /// it. `None` means keep using `Config::default()` — same as before this existed.
    ///
    /// Filtered on the way out as well as on the way in. `meta` is a plain table in a file
    /// the user owns, and a value that predates the bounds, or one put there by hand, should
    /// not be able to reach a live `Tracker` just because it was already written.
    pub fn idle_threshold_ms(&self) -> Result<Option<Millis>> {
        Ok(self
            .meta_get("idle_threshold_ms")?
            .and_then(|v| v.parse().ok())
            .filter(|ms| IDLE_THRESHOLD_BOUNDS_MS.contains(ms)))
    }

    /// Persist a new idle threshold. Written here so it survives a restart; taking effect
    /// on the *running* daemon is a separate step the caller still has to do — the store
    /// has no way to reach a live `Tracker`. See `toki-daemon`'s `Raw::SetConfig`.
    ///
    /// Rejects anything outside [`IDLE_THRESHOLD_BOUNDS_MS`]. The check lives here because
    /// this is the one place every route to the setting meets: the named pipe, the GUI
    /// behind it, and anything later. A caller that gets an error must not go on to tell the
    /// collector about the value either, which falls out of returning one.
    pub fn set_idle_threshold_ms(&self, ms: Millis) -> Result<()> {
        if !IDLE_THRESHOLD_BOUNDS_MS.contains(&ms) {
            return Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
                Some(format!(
                    "idle threshold must be between {} and {} ms, got {ms}",
                    IDLE_THRESHOLD_BOUNDS_MS.start(),
                    IDLE_THRESHOLD_BOUNDS_MS.end()
                )),
            ));
        }
        self.meta_set("idle_threshold_ms", &ms.to_string())
    }

    /// The absorbed-idle histogram carried over from previous runs, as JSON.
    ///
    /// Opaque here on purpose: the shape belongs to `toki_core::IdleHistogram` and the
    /// daemon does the encoding, so adding a bucket doesn't touch the store. `None` means
    /// nothing has been recorded yet, which a fresh `IdleHistogram::default()` covers.
    pub fn idle_histogram_json(&self) -> Result<Option<String>> {
        self.meta_get("idle_histogram")
    }

    /// Persist the histogram. Overwrites rather than accumulates — the daemon holds the
    /// running totals in memory and writes the whole thing, so a failed write costs at most
    /// the interval since the last one rather than corrupting the count.
    pub fn set_idle_histogram_json(&self, json: &str) -> Result<()> {
        self.meta_set("idle_histogram", json)
    }

    /// Whether the one-time post-install GUI launch has already happened. See DESIGN.md
    /// §4 "First run": opens once so the tray icon is findable, never again after that.
    pub fn first_run_seen(&self) -> Result<bool> {
        Ok(self.meta_get("first_run_seen")?.is_some())
    }

    /// Record that the one-time launch has happened, so a later start never repeats it.
    pub fn mark_first_run_seen(&self) -> Result<()> {
        self.meta_set("first_run_seen", "1")
    }

    /// The Monday (local date) the weekly digest last fired for, if ever. See DESIGN.md §10
    /// "Weekly digest": once per week, on the first foreground activity of Monday.
    pub fn last_digest_monday(&self) -> Result<Option<NaiveDate>> {
        Ok(self
            .meta_get("last_digest_monday")?
            .and_then(|v| v.parse().ok()))
    }

    pub fn set_last_digest_monday(&self, monday: NaiveDate) -> Result<()> {
        self.meta_set("last_digest_monday", &monday.to_string())
    }

    pub fn heartbeat(&self, at: Millis) -> Result<()> {
        self.meta_set("last_seen", &at.to_string())
    }

    pub fn last_heartbeat(&self) -> Result<Option<Millis>> {
        Ok(self.meta_get("last_seen")?.and_then(|v| v.parse().ok()))
    }

    /// Mirror the in-progress session, or clear it on a clean close.
    pub fn set_open_session(&self, open: Option<&OpenSession>) -> Result<()> {
        match open {
            None => {
                for k in OPEN_KEYS {
                    self.meta_del(k)?;
                }
                Ok(())
            }
            Some(o) => {
                self.meta_set("open_start", &o.start.to_string())?;
                self.meta_set("open_app", o.app.as_str())?;
                self.meta_set("open_title", &o.title)?;
                self.meta_set("open_offset", &o.tz_offset_min.to_string())?;
                match o.engine {
                    Some(e) => self.meta_set("open_engine", &engine_code(e).to_string())?,
                    None => self.meta_del("open_engine")?,
                }
                Ok(())
            }
        }
    }

    pub fn open_session(&self) -> Result<Option<OpenSession>> {
        let (Some(start), Some(app), Some(offset)) = (
            self.meta_get("open_start")?,
            self.meta_get("open_app")?,
            self.meta_get("open_offset")?,
        ) else {
            return Ok(None);
        };
        let (Ok(start), Ok(tz_offset_min)) = (start.parse::<Millis>(), offset.parse::<i32>())
        else {
            return Ok(None);
        };
        Ok(Some(OpenSession {
            start,
            app: AppKey::new(app),
            title: self.meta_get("open_title")?.unwrap_or_default(),
            tz_offset_min,
            engine: self
                .meta_get("open_engine")?
                .and_then(|v| v.parse::<i64>().ok())
                .and_then(engine_from_code),
        }))
    }

    /// Close a session left dangling by a crash, at the last heartbeat.
    ///
    /// This is the mechanism that stops a machine which slept at 23:00 with an editor
    /// focused from logging nine hours. Suspend and lock events are precision on top of it;
    /// they do not fire on power loss or a killed process.
    pub fn recover(&self) -> Result<Option<Session>> {
        let Some(open) = self.open_session()? else {
            return Ok(None);
        };
        self.set_open_session(None)?;

        let Some(last_seen) = self.last_heartbeat()? else {
            return Ok(None);
        };
        if last_seen <= open.start {
            return Ok(None);
        }

        let session = Session {
            start: open.start,
            end: last_seen,
            app: open.app,
            title: open.title,
            // The tracker's presence state died with the process; assume input.
            presence: Presence::Input,
        };
        self.insert_session(&session, open.tz_offset_min, open.engine)?;
        Ok(Some(session))
    }

    /// Attach a resolved domain, or mark the lookup as having missed.
    pub fn set_domain(&self, session_id: i64, domain: Option<&str>) -> Result<()> {
        let outcome = match domain {
            Some(d) => Resolution::Found(d.to_string()),
            None => Resolution::Missed,
        };
        self.record_resolution(session_id, &outcome)
    }

    /// Write what one lookup decided. Private because the resolver goes through
    /// [`Store::record_resolutions`], which wraps a whole pass in one transaction.
    ///
    /// A non-match clears `domain_id` as well as setting the state. Only rows already at
    /// `RESOLVE_PENDING` reach here and those have no domain — `reset_resolution` nulls it
    /// on the way back into the queue — so this writes what is already there, and says so
    /// rather than relying on it.
    fn record_resolution(&self, session_id: i64, outcome: &Resolution) -> Result<()> {
        let (domain_id, state) = match outcome {
            Resolution::Found(domain) => (Some(self.intern_domain(domain)?), RESOLVE_DONE),
            Resolution::Missed => (None, RESOLVE_FAILED),
            Resolution::NotAPage => (None, RESOLVE_NOT_A_PAGE),
        };
        self.conn.execute(
            "UPDATE sessions SET domain_id = ?2, resolve_state = ?3 WHERE id = ?1",
            params![session_id, domain_id, state],
        )?;
        Ok(())
    }

    /// Write a whole resolver pass in one transaction.
    ///
    /// A pass drains up to `BATCH` rows, and one autocommit per row meant up to 5,000
    /// transactions — 5,000 WAL frame writes and 5,000 chances for the GUI's reader to see
    /// a half-drained queue. One commit is also the honest unit: a pass either happened or
    /// it did not.
    pub fn record_resolutions(&self, outcomes: &[(i64, Resolution)]) -> Result<()> {
        // `unchecked_transaction` rather than `transaction`, which needs `&mut self`: the
        // resolver holds a shared `Store`, and nothing else touches this connection.
        let tx = self.conn.unchecked_transaction()?;
        for (session_id, outcome) in outcomes {
            self.record_resolution(*session_id, outcome)?;
        }
        tx.commit()
    }

    /// Delete a time range, optionally limited to one app. The repair path for silently
    /// bad data; there is deliberately no GUI equivalent.
    pub fn purge(&self, from: Millis, to: Millis, exe_path: Option<&str>) -> Result<usize> {
        let n = match exe_path {
            Some(exe) => self.conn.execute(
                "DELETE FROM sessions
                 WHERE start_utc_ms < ?2 AND end_utc_ms > ?1
                   AND app_id = (SELECT id FROM apps WHERE exe_path = ?3)",
                params![from, to, exe],
            )?,
            None => self.conn.execute(
                "DELETE FROM sessions WHERE start_utc_ms < ?2 AND end_utc_ms > ?1",
                params![from, to],
            )?,
        };
        Ok(n)
    }

    /// Collapse sessions that started before `cutoff` into `daily_rollup`, then delete the
    /// raw rows — titles go with them, since `daily_rollup` has no title column. See
    /// DESIGN.md §7 "Retention": 6 months of raw sessions, older data as daily per-app,
    /// per-domain totals.
    ///
    /// A session straddling local midnight contributes its clamped portion to each date it
    /// touches, exactly like `day_totals`, run once per date instead of once per query.
    /// Runs in one transaction so a crash mid-rollup cannot double-count a session that was
    /// summed into `daily_rollup` but not yet deleted from `sessions`.
    pub fn rollup_before(&self, cutoff: Millis) -> Result<usize> {
        let candidates = self.sessions_before(cutoff)?;
        if candidates.is_empty() {
            return Ok(0);
        }
        let totals = rollup_totals(&candidates);

        // `unchecked_transaction`, not `transaction`, because every other `Store` method
        // takes `&self` — a `&mut self` here would be the one method callers have to treat
        // differently, for no benefit: SQLite already serializes writers on this connection.
        let tx = self.conn.unchecked_transaction()?;
        for ((date, app_id, domain_id), ms) in totals {
            upsert_rollup(&tx, date, app_id, domain_id, ms / 1_000)?;
        }
        tx.execute(
            "DELETE FROM sessions WHERE start_utc_ms < ?1",
            params![cutoff],
        )?;
        tx.commit()?;

        Ok(candidates.len())
    }

    /// Raw material for `rollup_before`: id is unused beyond the count, but reading it keeps
    /// the row shape identical to what the `DELETE` in the same transaction targets.
    fn sessions_before(&self, cutoff: Millis) -> Result<Vec<RollupCandidate>> {
        let mut stmt = self.conn.prepare(
            "SELECT start_utc_ms, end_utc_ms, tz_offset_min, app_id, domain_id
             FROM sessions WHERE start_utc_ms < ?1",
        )?;
        let rows = stmt.query_map(params![cutoff], |r| {
            Ok(RollupCandidate {
                start: r.get(0)?,
                end: r.get(1)?,
                offset_min: r.get(2)?,
                app_id: r.get(3)?,
                domain_id: r.get(4)?,
            })
        })?;
        rows.collect()
    }

    // ----------------------------------------------------------------- reads

    /// Per-app and per-domain totals for a local day.
    ///
    /// Sessions are stored whole, so a session straddling midnight is clamped into each day
    /// it touches. The `start_utc_ms > day_start - DAY` predicate is not redundant: without
    /// it the query cannot use `sessions_start` and degrades to a full scan, because
    /// `end_utc_ms > day_start` alone is not indexable. A session cannot outlive the audio
    /// cap plus the idle threshold, so a one-day bound is always safe.
    pub fn day_totals(&self, date: NaiveDate, offset_min: i32) -> Result<Vec<Total>> {
        let (day_start, day_end) = day_bounds(date, offset_min);
        let mut stmt = self.conn.prepare(
            "SELECT a.exe_path,
                    COALESCE(a.alias, a.detected_name, a.exe_path),
                    COALESCE(d.alias, d.domain),
                    SUM(min(s.end_utc_ms, ?2) - max(s.start_utc_ms, ?1)) AS ms
             FROM sessions s
             JOIN apps a ON a.id = s.app_id
             LEFT JOIN domains d ON d.id = s.domain_id
             WHERE s.start_utc_ms < ?2
               AND s.end_utc_ms   > ?1
               AND s.start_utc_ms > ?1 - 86400000
               AND a.ignored = 0
             GROUP BY a.exe_path, COALESCE(d.alias, d.domain)
             ORDER BY ms DESC",
        )?;
        let rows = stmt.query_map(params![day_start, day_end], |r| {
            Ok(Total {
                exe_path: r.get(0)?,
                app_name: r.get(1)?,
                domain: r.get(2)?,
                ms: r.get(3)?,
            })
        })?;
        rows.collect()
    }

    /// Every app ever seen, for the Settings alias editor and ignore list — not just ones
    /// with time in some particular window, unlike `day_totals`.
    pub fn all_apps(&self) -> Result<Vec<AppRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT exe_path, COALESCE(alias, detected_name, exe_path), alias, ignored
             FROM apps
             ORDER BY COALESCE(alias, detected_name, exe_path) COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(AppRow {
                exe_path: r.get(0)?,
                name: r.get(1)?,
                alias: r.get(2)?,
                ignored: r.get::<_, i64>(3)? != 0,
            })
        })?;
        rows.collect()
    }

    /// Every domain ever resolved, for the Settings alias editor.
    pub fn all_domains(&self) -> Result<Vec<DomainRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT domain, COALESCE(alias, domain), alias
             FROM domains
             ORDER BY COALESCE(alias, domain) COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(DomainRow {
                domain: r.get(0)?,
                name: r.get(1)?,
                alias: r.get(2)?,
            })
        })?;
        rows.collect()
    }

    /// Browser sessions still awaiting a domain, oldest first.
    pub fn pending(&self, limit: usize) -> Result<Vec<Pending>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, t.title, a.engine
             FROM sessions s
             JOIN apps a   ON a.id = s.app_id
             JOIN titles t ON t.id = s.title_id
             WHERE s.resolve_state = ?1 AND a.engine IS NOT NULL
             ORDER BY s.id
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![RESOLVE_PENDING, limit as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, title, code) = row?;
            if let Some(engine) = engine_from_code(code) {
                out.push(Pending { id, title, engine });
            }
        }
        Ok(out)
    }

    /// Put already-resolved browser sessions back in the queue, returning how many.
    ///
    /// A lookup that missed is not permanent: history is written when a page is visited, so
    /// a session recorded seconds earlier can miss and then match on the next pass. This is
    /// what `toki reresolve` runs after a browser has caught up, or after a fix to title
    /// normalization changes what would match.
    pub fn reset_resolution(&self, from: Millis, to: Millis) -> Result<usize> {
        let n = self.conn.execute(
            "UPDATE sessions SET resolve_state = ?3, domain_id = NULL
             WHERE start_utc_ms >= ?1 AND start_utc_ms < ?2
               AND app_id IN (SELECT id FROM apps WHERE engine IS NOT NULL)",
            params![from, to, RESOLVE_PENDING],
        )?;
        Ok(n)
    }

    pub fn rows_between(&self, from: Millis, to: Millis) -> Result<i64> {
        self.conn.query_row(
            "SELECT count(*) FROM sessions WHERE start_utc_ms >= ?1 AND start_utc_ms < ?2",
            params![from, to],
            |r| r.get(0),
        )
    }

    pub fn pending_count(&self) -> Result<i64> {
        self.conn.query_row(
            "SELECT count(*) FROM sessions WHERE resolve_state = ?1",
            params![RESOLVE_PENDING],
            |r| r.get(0),
        )
    }

    /// Where every browser session stands, one row per browser. What `doctor` prints.
    ///
    /// A single unresolved total says nothing about whether resolution is working: it mixes
    /// a queue that has not drained yet with captions that can never resolve and with real
    /// misses. Split per browser, because the browsers differ — one bad title format in one
    /// of them is invisible in the sum.
    ///
    /// Named the way every other list is, `COALESCE(alias, detected_name, exe_path)`, rather
    /// than by file name: Helium is a Chromium fork whose binary is also called `chrome.exe`,
    /// so the file name would merge it with Chrome and hide the difference this exists to
    /// show.
    pub fn resolution_by_app(&self) -> Result<Vec<ResolveRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(a.alias, a.detected_name, a.exe_path) AS name,
                    sum(s.resolve_state = ?1),
                    sum(s.resolve_state = ?2),
                    sum(s.resolve_state = ?3),
                    sum(s.resolve_state = ?4)
             FROM sessions s
             JOIN apps a ON a.id = s.app_id
             WHERE a.engine IS NOT NULL
             GROUP BY a.id
             ORDER BY name",
        )?;
        let rows = stmt.query_map(
            params![
                RESOLVE_DONE,
                RESOLVE_FAILED,
                RESOLVE_NOT_A_PAGE,
                RESOLVE_PENDING
            ],
            |r| {
                Ok(ResolveRow {
                    name: r.get(0)?,
                    resolved: r.get(1)?,
                    missed: r.get(2)?,
                    not_a_page: r.get(3)?,
                    pending: r.get(4)?,
                })
            },
        )?;
        rows.collect()
    }

    // ------------------------------------------------------------- interning

    fn intern_app(&self, exe_path: &str, engine: Option<Engine>) -> Result<i64> {
        self.conn.execute(
            "INSERT OR IGNORE INTO apps (exe_path, engine) VALUES (?1, ?2)",
            params![exe_path, engine.map(engine_code)],
        )?;
        if let Some(e) = engine {
            // An app may be seen before it is recognised as a browser.
            self.conn.execute(
                "UPDATE apps SET engine = ?2 WHERE exe_path = ?1 AND engine IS NULL",
                params![exe_path, engine_code(e)],
            )?;
        }
        self.conn.query_row(
            "SELECT id FROM apps WHERE exe_path = ?1",
            params![exe_path],
            |r| r.get(0),
        )
    }

    fn intern_title(&self, title: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT OR IGNORE INTO titles (title) VALUES (?1)",
            params![title],
        )?;
        self.conn.query_row(
            "SELECT id FROM titles WHERE title = ?1",
            params![title],
            |r| r.get(0),
        )
    }

    fn intern_domain(&self, domain: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT OR IGNORE INTO domains (domain) VALUES (?1)",
            params![domain],
        )?;
        self.conn.query_row(
            "SELECT id FROM domains WHERE domain = ?1",
            params![domain],
            |r| r.get(0),
        )
    }

    // ------------------------------------------------------------------ meta

    fn meta_set(&self, k: &str, v: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (k, v) VALUES (?1, ?2)
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            params![k, v],
        )?;
        Ok(())
    }

    fn meta_get(&self, k: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT v FROM meta WHERE k = ?1", params![k], |r| r.get(0))
            .optional()
    }

    fn meta_del(&self, k: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM meta WHERE k = ?1", params![k])?;
        Ok(())
    }
}

const OPEN_KEYS: &[&str] = &[
    "open_start",
    "open_app",
    "open_title",
    "open_offset",
    "open_engine",
];

fn presence_code(p: Presence) -> i64 {
    match p {
        Presence::Input => 0,
        Presence::AudioOnly => 1,
    }
}

fn engine_code(e: Engine) -> i64 {
    match e {
        Engine::Chromium => 0,
        Engine::Gecko => 1,
    }
}

fn engine_from_code(code: i64) -> Option<Engine> {
    match code {
        0 => Some(Engine::Chromium),
        1 => Some(Engine::Gecko),
        _ => None,
    }
}

/// One raw `sessions` row, as far as `rollup_before` needs it.
struct RollupCandidate {
    start: Millis,
    end: Millis,
    offset_min: i32,
    app_id: i64,
    domain_id: Option<i64>,
}

/// Sum each candidate's clamped duration into the local date(s) it touches.
///
/// A session is capped well under 24h (the audio cap plus the idle threshold), so it can
/// straddle at most one midnight — never more than two dates.
fn rollup_totals(candidates: &[RollupCandidate]) -> HashMap<(NaiveDate, i64, Option<i64>), Millis> {
    let mut totals = HashMap::new();
    for c in candidates {
        for date in touched_dates(c.start, c.end, c.offset_min) {
            let (day_start, day_end) = day_bounds(date, c.offset_min);
            let ms = overlap_ms(c.start, c.end, day_start, day_end);
            if ms > 0 {
                *totals.entry((date, c.app_id, c.domain_id)).or_insert(0) += ms;
            }
        }
    }
    totals
}

/// The one or two local dates a `[start, end)` interval overlaps.
fn touched_dates(start: Millis, end: Millis, offset_min: i32) -> Vec<NaiveDate> {
    let first = local_date(start, offset_min);
    let last = local_date(end - 1, offset_min);
    if first == last {
        vec![first]
    } else {
        vec![first, last]
    }
}

/// Add `seconds` to an existing rollup row, or start one. A plain `ON CONFLICT` upsert
/// can't be used here: `domain_id` is nullable, and SQLite treats every `NULL` as distinct
/// from every other `NULL` under a uniqueness constraint, so non-browser apps (`domain_id`
/// `NULL`) would never match an existing row and would insert a fresh duplicate on every
/// call. `IS` compares NULLs as equal, where `=` would not.
fn upsert_rollup(
    tx: &Transaction,
    date: NaiveDate,
    app_id: i64,
    domain_id: Option<i64>,
    seconds: i64,
) -> Result<()> {
    let date = date.to_string();
    let updated = tx.execute(
        "UPDATE daily_rollup SET seconds = seconds + ?4
         WHERE local_date = ?1 AND app_id = ?2 AND domain_id IS ?3",
        params![date, app_id, domain_id, seconds],
    )?;
    if updated == 0 {
        tx.execute(
            "INSERT INTO daily_rollup (local_date, app_id, domain_id, seconds)
             VALUES (?1, ?2, ?3, ?4)",
            params![date, app_id, domain_id, seconds],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Millis = 60 * 1_000;
    const HOUR: Millis = 60 * MINUTE;
    const IST: i32 = 330;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn session(start: Millis, end: Millis, exe: &str, title: &str) -> Session {
        Session {
            start,
            end,
            app: AppKey::new(exe),
            title: title.to_string(),
            presence: Presence::Input,
        }
    }

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    // ------------------------------------------------------------ migrations

    /// A helper that reproduces what every database in the wild looked like before the
    /// scaffold existed: schema v1, stamped, with resolution failures recorded in the one
    /// state that then existed for them.
    fn version_1_store_with_a_failure() -> Store {
        let s = store();
        let (day_start, _) = day_bounds(date(2026, 8, 27), IST);
        s.apply(
            &Action::Emit(session(
                day_start,
                day_start + MINUTE,
                "chrome.exe",
                "WhatsApp Web",
            )),
            IST,
            Some(Engine::Chromium),
        )
        .unwrap();
        let id = s.pending(10).unwrap()[0].id;
        s.record_resolution(id, &Resolution::Missed).unwrap();
        s.conn.pragma_update(None, "user_version", 1).unwrap();
        s
    }

    #[test]
    fn a_fresh_database_is_stamped_at_the_latest_version_and_skips_every_migration() {
        let s = store();
        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
        // Nothing to step through means nothing to undo: re-running is a no-op.
        migrate(&s.conn).unwrap();
        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn an_older_database_is_stepped_up_and_its_failures_requeued() {
        let s = version_1_store_with_a_failure();
        assert!(s.pending(10).unwrap().is_empty());

        migrate(&s.conn).unwrap();

        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
        // Back in the queue, where the next pass sorts it into missed or not-a-page.
        assert_eq!(s.pending(10).unwrap().len(), 1);
    }

    #[test]
    fn a_migration_does_not_run_twice() {
        let s = version_1_store_with_a_failure();
        migrate(&s.conn).unwrap();
        let id = s.pending(10).unwrap()[0].id;
        s.record_resolution(id, &Resolution::NotAPage).unwrap();

        // Already at the latest version, so the requeue must not happen again and undo the
        // classification the pass just made.
        migrate(&s.conn).unwrap();
        assert!(s.pending(10).unwrap().is_empty());
    }

    #[test]
    fn a_database_from_a_newer_build_is_refused_rather_than_written_to() {
        let s = store();
        s.conn
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        assert!(migrate(&s.conn).is_err());
        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION + 1);
    }

    // ------------------------------------------------- idle threshold bounds

    #[test]
    fn an_idle_threshold_outside_the_bounds_is_refused() {
        let s = store();
        for bad in [0, -1, 1_000, i64::MAX, 3 * HOUR] {
            assert!(s.set_idle_threshold_ms(bad).is_err(), "accepted {bad}");
        }
        // Refused means not written, so the daemon keeps the default rather than a value
        // that was rejected but persisted anyway.
        assert_eq!(s.idle_threshold_ms().unwrap(), None);

        s.set_idle_threshold_ms(2 * MINUTE).unwrap();
        assert_eq!(s.idle_threshold_ms().unwrap(), Some(2 * MINUTE));
    }

    #[test]
    fn an_out_of_range_threshold_already_in_the_file_does_not_reach_the_tracker() {
        let s = store();
        // As a hand-edited row, or one written before the bounds existed.
        s.meta_set("idle_threshold_ms", "0").unwrap();
        assert_eq!(s.idle_threshold_ms().unwrap(), None);
    }

    // -------------------------------------------------- resolution reporting

    #[test]
    fn a_caption_with_no_page_behind_it_is_not_counted_as_a_miss() {
        let s = store();
        let (day_start, _) = day_bounds(date(2026, 8, 27), IST);
        for (i, title) in [
            "Claude - Google Chrome",
            "WhatsApp Web",
            "Gmail - Google Chrome",
        ]
        .iter()
        .enumerate()
        {
            let at = day_start + i as Millis * HOUR;
            s.apply(
                &Action::Emit(session(at, at + MINUTE, "chrome.exe", title)),
                IST,
                Some(Engine::Chromium),
            )
            .unwrap();
        }
        let ids: Vec<i64> = s.pending(10).unwrap().iter().map(|p| p.id).collect();
        s.record_resolutions(&[
            (ids[0], Resolution::Found("claude.ai".into())),
            (ids[1], Resolution::NotAPage),
            (ids[2], Resolution::Missed),
        ])
        .unwrap();

        let rows = s.resolution_by_app().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "chrome.exe");
        assert_eq!(rows[0].resolved, 1);
        assert_eq!(rows[0].missed, 1);
        assert_eq!(rows[0].not_a_page, 1);
        assert_eq!(rows[0].pending, 0);
    }

    #[test]
    fn schema_applies_and_is_idempotent() {
        let s = store();
        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
        s.conn.execute_batch(SCHEMA).unwrap();
    }

    #[test]
    fn emitting_a_session_shows_up_in_the_day_total() {
        let s = store();
        let (day_start, _) = day_bounds(date(2026, 8, 27), IST);
        let sess = session(
            day_start + 9 * HOUR,
            day_start + 10 * HOUR,
            "code.exe",
            "main.rs",
        );
        s.apply(&Action::Emit(sess), IST, None).unwrap();

        let totals = s.day_totals(date(2026, 8, 27), IST).unwrap();
        assert_eq!(totals.len(), 1);
        assert_eq!(totals[0].exe_path, "code.exe");
        assert_eq!(totals[0].ms, HOUR);
        assert_eq!(totals[0].domain, None);
    }

    #[test]
    fn extend_last_moves_the_previous_end_forward_only() {
        let s = store();
        let (day_start, _) = day_bounds(date(2026, 8, 27), IST);
        let sess = session(day_start, day_start + 30 * 1_000, "code.exe", "main.rs");
        s.apply(&Action::Emit(sess), IST, None).unwrap();

        s.apply(
            &Action::ExtendLast {
                end: day_start + 31_000,
            },
            IST,
            None,
        )
        .unwrap();
        assert_eq!(s.day_totals(date(2026, 8, 27), IST).unwrap()[0].ms, 31_000);

        // A backwards extend must be ignored rather than shrinking the row.
        s.apply(
            &Action::ExtendLast {
                end: day_start + 5_000,
            },
            IST,
            None,
        )
        .unwrap();
        assert_eq!(s.day_totals(date(2026, 8, 27), IST).unwrap()[0].ms, 31_000);
    }

    #[test]
    fn a_session_across_midnight_is_credited_to_both_days() {
        let s = store();
        let (_, d27_end) = day_bounds(date(2026, 8, 27), IST);
        let sess = session(
            d27_end - 20 * MINUTE,
            d27_end + 20 * MINUTE,
            "code.exe",
            "late",
        );
        s.apply(&Action::Emit(sess), IST, None).unwrap();

        assert_eq!(
            s.day_totals(date(2026, 8, 27), IST).unwrap()[0].ms,
            20 * MINUTE
        );
        assert_eq!(
            s.day_totals(date(2026, 8, 28), IST).unwrap()[0].ms,
            20 * MINUTE
        );
    }

    #[test]
    fn ignored_apps_are_left_out_of_totals() {
        let s = store();
        let (day_start, _) = day_bounds(date(2026, 8, 27), IST);
        s.apply(
            &Action::Emit(session(
                day_start,
                day_start + HOUR,
                "TextInputHost.exe",
                "IME",
            )),
            IST,
            None,
        )
        .unwrap();
        s.conn
            .execute(
                "UPDATE apps SET ignored = 1 WHERE exe_path = 'TextInputHost.exe'",
                [],
            )
            .unwrap();

        assert!(s.day_totals(date(2026, 8, 27), IST).unwrap().is_empty());
    }

    #[test]
    fn only_browser_sessions_enter_the_resolve_queue() {
        let s = store();
        let (day_start, _) = day_bounds(date(2026, 8, 27), IST);
        s.apply(
            &Action::Emit(session(day_start, day_start + HOUR, "code.exe", "main.rs")),
            IST,
            None,
        )
        .unwrap();
        s.apply(
            &Action::Emit(session(
                day_start + HOUR,
                day_start + 2 * HOUR,
                "chrome.exe",
                "New chat - Claude - Google Chrome",
            )),
            IST,
            Some(Engine::Chromium),
        )
        .unwrap();

        let pending = s.pending(10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].engine, Engine::Chromium);
        assert_eq!(pending[0].title, "New chat - Claude - Google Chrome");
        assert_eq!(s.pending_count().unwrap(), 1);
    }

    #[test]
    fn resolving_attaches_a_domain_and_clears_the_queue() {
        let s = store();
        let (day_start, _) = day_bounds(date(2026, 8, 27), IST);
        s.apply(
            &Action::Emit(session(day_start, day_start + HOUR, "zen.exe", "Home / X")),
            IST,
            Some(Engine::Gecko),
        )
        .unwrap();

        let pending = s.pending(10).unwrap();
        s.set_domain(pending[0].id, Some("x.com")).unwrap();

        assert_eq!(s.pending_count().unwrap(), 0);
        let totals = s.day_totals(date(2026, 8, 27), IST).unwrap();
        assert_eq!(totals[0].domain.as_deref(), Some("x.com"));
    }

    #[test]
    fn a_missed_lookup_is_marked_failed_rather_than_retried_forever() {
        let s = store();
        let (day_start, _) = day_bounds(date(2026, 8, 27), IST);
        s.apply(
            &Action::Emit(session(
                day_start,
                day_start + HOUR,
                "zen.exe",
                "brand new tab",
            )),
            IST,
            Some(Engine::Gecko),
        )
        .unwrap();

        let pending = s.pending(10).unwrap();
        s.set_domain(pending[0].id, None).unwrap();
        assert_eq!(s.pending_count().unwrap(), 0);
        assert!(s.pending(10).unwrap().is_empty());
    }

    #[test]
    fn a_crash_closes_the_dangling_session_at_the_last_heartbeat() {
        let s = store();
        let (day_start, _) = day_bounds(date(2026, 8, 27), IST);

        s.set_open_session(Some(&OpenSession {
            start: day_start + 22 * HOUR,
            app: AppKey::new("code.exe"),
            title: "main.rs".into(),
            tz_offset_min: IST,
            engine: None,
        }))
        .unwrap();
        s.heartbeat(day_start + 23 * HOUR).unwrap();

        let recovered = s.recover().unwrap().expect("dangling session");
        assert_eq!(recovered.end, day_start + 23 * HOUR);
        assert_eq!(s.day_totals(date(2026, 8, 27), IST).unwrap()[0].ms, HOUR);

        // Recovery must not run twice against the same crash.
        assert!(s.recover().unwrap().is_none());
    }

    #[test]
    fn a_clean_shutdown_leaves_nothing_to_recover() {
        let s = store();
        s.set_open_session(Some(&OpenSession {
            start: 0,
            app: AppKey::new("code.exe"),
            title: "main.rs".into(),
            tz_offset_min: IST,
            engine: None,
        }))
        .unwrap();
        s.set_open_session(None).unwrap();
        assert!(s.open_session().unwrap().is_none());
        assert!(s.recover().unwrap().is_none());
    }

    #[test]
    fn purge_removes_a_range_and_can_be_limited_to_one_app() {
        let s = store();
        let (day_start, _) = day_bounds(date(2026, 8, 27), IST);
        s.apply(
            &Action::Emit(session(day_start, day_start + HOUR, "code.exe", "a")),
            IST,
            None,
        )
        .unwrap();
        s.apply(
            &Action::Emit(session(
                day_start + HOUR,
                day_start + 2 * HOUR,
                "chrome.exe",
                "b",
            )),
            IST,
            None,
        )
        .unwrap();

        let removed = s
            .purge(day_start, day_start + 3 * HOUR, Some("code.exe"))
            .unwrap();
        assert_eq!(removed, 1);
        let totals = s.day_totals(date(2026, 8, 27), IST).unwrap();
        assert_eq!(totals.len(), 1);
        assert_eq!(totals[0].exe_path, "chrome.exe");

        assert_eq!(s.purge(day_start, day_start + 3 * HOUR, None).unwrap(), 1);
        assert!(s.day_totals(date(2026, 8, 27), IST).unwrap().is_empty());
    }

    #[test]
    fn rollup_before_collapses_a_session_into_daily_rollup_and_drops_the_title() {
        let s = store();
        let (day_start, _) = day_bounds(date(2026, 8, 27), IST);
        s.apply(
            &Action::Emit(session(day_start, day_start + HOUR, "code.exe", "main.rs")),
            IST,
            None,
        )
        .unwrap();

        let rolled = s.rollup_before(day_start + 2 * HOUR).unwrap();
        assert_eq!(rolled, 1);
        assert!(s.day_totals(date(2026, 8, 27), IST).unwrap().is_empty());

        let seconds: i64 = s
            .conn
            .query_row(
                "SELECT r.seconds FROM daily_rollup r JOIN apps a ON a.id = r.app_id
                 WHERE a.exe_path = 'code.exe' AND r.local_date = '2026-08-27'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(seconds, 3_600);
    }

    #[test]
    fn rollup_before_leaves_sessions_at_or_after_the_cutoff() {
        let s = store();
        let (day_start, _) = day_bounds(date(2026, 8, 27), IST);
        s.apply(
            &Action::Emit(session(day_start, day_start + HOUR, "code.exe", "main.rs")),
            IST,
            None,
        )
        .unwrap();

        assert_eq!(s.rollup_before(day_start).unwrap(), 0);
        assert_eq!(s.day_totals(date(2026, 8, 27), IST).unwrap().len(), 1);
    }

    #[test]
    fn rollup_before_credits_a_straddling_session_to_both_dates() {
        let s = store();
        let (_, d27_end) = day_bounds(date(2026, 8, 27), IST);
        s.apply(
            &Action::Emit(session(
                d27_end - 20 * MINUTE,
                d27_end + 20 * MINUTE,
                "code.exe",
                "late",
            )),
            IST,
            None,
        )
        .unwrap();

        assert_eq!(s.rollup_before(d27_end + HOUR).unwrap(), 1);

        let seconds_27: i64 = s
            .conn
            .query_row(
                "SELECT seconds FROM daily_rollup WHERE local_date = '2026-08-27'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let seconds_28: i64 = s
            .conn
            .query_row(
                "SELECT seconds FROM daily_rollup WHERE local_date = '2026-08-28'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(seconds_27, 20 * 60);
        assert_eq!(seconds_28, 20 * 60);
    }

    #[test]
    fn rollup_before_sums_same_day_sessions_without_duplicating_null_domain_rows() {
        let s = store();
        let (day_start, _) = day_bounds(date(2026, 8, 27), IST);
        s.apply(
            &Action::Emit(session(day_start, day_start + HOUR, "code.exe", "a")),
            IST,
            None,
        )
        .unwrap();
        s.apply(
            &Action::Emit(session(
                day_start + HOUR,
                day_start + 2 * HOUR,
                "code.exe",
                "b",
            )),
            IST,
            None,
        )
        .unwrap();
        s.rollup_before(day_start + 3 * HOUR).unwrap();

        let rows: i64 = s
            .conn
            .query_row("SELECT count(*) FROM daily_rollup", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        let seconds: i64 = s
            .conn
            .query_row("SELECT seconds FROM daily_rollup", [], |r| r.get(0))
            .unwrap();
        assert_eq!(seconds, 2 * 3_600);

        // A second app with no domain, rolled up in a later call, must not collide with the
        // first NULL-domain row through a false `domain_id = domain_id` match — the bug an
        // `ON CONFLICT` upsert would have on a nullable key.
        s.apply(
            &Action::Emit(session(
                day_start + 3 * HOUR,
                day_start + 4 * HOUR,
                "notes.exe",
                "c",
            )),
            IST,
            None,
        )
        .unwrap();
        s.rollup_before(day_start + 5 * HOUR).unwrap();
        let rows: i64 = s
            .conn
            .query_row("SELECT count(*) FROM daily_rollup", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 2);
    }

    #[test]
    fn rollup_before_keeps_domain_identity_and_accumulates_across_runs() {
        let s = store();
        let (day_start, _) = day_bounds(date(2026, 8, 27), IST);
        s.apply(
            &Action::Emit(session(day_start, day_start + HOUR, "zen.exe", "Home / X")),
            IST,
            Some(Engine::Gecko),
        )
        .unwrap();
        let pending = s.pending(10).unwrap();
        s.set_domain(pending[0].id, Some("x.com")).unwrap();
        s.rollup_before(day_start + 2 * HOUR).unwrap();

        // A second x.com session appearing later accumulates onto the same rollup row
        // instead of a fresh duplicate for the same date.
        s.apply(
            &Action::Emit(session(
                day_start + HOUR,
                day_start + 2 * HOUR,
                "zen.exe",
                "Home / X",
            )),
            IST,
            Some(Engine::Gecko),
        )
        .unwrap();
        let pending = s.pending(10).unwrap();
        s.set_domain(pending[0].id, Some("x.com")).unwrap();
        s.rollup_before(day_start + 3 * HOUR).unwrap();

        let rows: i64 = s
            .conn
            .query_row("SELECT count(*) FROM daily_rollup", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        let seconds: i64 = s
            .conn
            .query_row(
                "SELECT r.seconds FROM daily_rollup r JOIN domains d ON d.id = r.domain_id
                 WHERE d.domain = 'x.com'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(seconds, 2 * 3_600);
    }

    #[test]
    fn interning_keeps_one_row_per_app_and_upgrades_a_late_engine() {
        let s = store();
        let (day_start, _) = day_bounds(date(2026, 8, 27), IST);
        s.apply(
            &Action::Emit(session(day_start, day_start + HOUR, "zen.exe", "a")),
            IST,
            None,
        )
        .unwrap();
        s.apply(
            &Action::Emit(session(
                day_start + HOUR,
                day_start + 2 * HOUR,
                "zen.exe",
                "b",
            )),
            IST,
            Some(Engine::Gecko),
        )
        .unwrap();

        let apps: i64 = s
            .conn
            .query_row("SELECT count(*) FROM apps", [], |r| r.get(0))
            .unwrap();
        assert_eq!(apps, 1);
        let engine: Option<i64> = s
            .conn
            .query_row(
                "SELECT engine FROM apps WHERE exe_path = 'zen.exe'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(engine, Some(engine_code(Engine::Gecko)));
    }

    #[test]
    fn aliases_take_precedence_in_totals() {
        let s = store();
        let (day_start, _) = day_bounds(date(2026, 8, 27), IST);
        s.apply(
            &Action::Emit(session(day_start, day_start + HOUR, "zen.exe", "Home / X")),
            IST,
            Some(Engine::Gecko),
        )
        .unwrap();
        let pending = s.pending(10).unwrap();
        s.set_domain(pending[0].id, Some("x.com")).unwrap();

        s.conn
            .execute(
                "UPDATE apps SET alias = 'Zen' WHERE exe_path = 'zen.exe'",
                [],
            )
            .unwrap();
        s.conn
            .execute("UPDATE domains SET alias = 'X' WHERE domain = 'x.com'", [])
            .unwrap();

        let totals = s.day_totals(date(2026, 8, 27), IST).unwrap();
        assert_eq!(totals[0].app_name, "Zen");
        assert_eq!(totals[0].domain.as_deref(), Some("X"));
    }
}
