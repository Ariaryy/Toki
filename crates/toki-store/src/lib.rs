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

use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Result};
use serde::Serialize;
use toki_core::event::{AppKey, Millis, Presence};
use toki_core::resolve::Engine;
use toki_core::time::day_bounds;
use toki_core::tracker::{Action, Session};

const SCHEMA_VERSION: i64 = 1;

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
/// Looked up and missed. The title was not in any history database.
pub const RESOLVE_FAILED: i64 = 3;

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

/// A session awaiting domain resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub id: i64,
    pub title: String,
    pub engine: Engine,
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
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        conn.execute_batch(SCHEMA)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
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
        match domain {
            Some(d) => {
                let domain_id = self.intern_domain(d)?;
                self.conn.execute(
                    "UPDATE sessions SET domain_id = ?2, resolve_state = ?3 WHERE id = ?1",
                    params![session_id, domain_id, RESOLVE_DONE],
                )?;
            }
            None => {
                self.conn.execute(
                    "UPDATE sessions SET resolve_state = ?2 WHERE id = ?1",
                    params![session_id, RESOLVE_FAILED],
                )?;
            }
        }
        Ok(())
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
