//! Reading the *browsers'* SQLite, as opposed to Toki's own.
//!
//! Both engines keep every visit with its time, and the page's URL and title. This module
//! streams those visits into a [`VisitMatcher`]; deciding which visit names which session
//! is [`toki_core::resolve`]'s job.
//!
//! The file is copied before it is opened. Opening the live file with `immutable=1` would
//! be faster but skips the WAL, so it returns whatever was true at the last checkpoint —
//! exactly the recent rows the resolver needs most.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, Result};
use toki_core::event::Millis;
use toki_core::resolve::{Engine, VisitMatcher};

/// Microseconds between 1601-01-01 and 1970-01-01. Chromium counts from the former.
const WEBKIT_EPOCH_OFFSET_US: i64 = 11_644_473_600_000_000;

/// Stream one history database's visits into `matcher`, returning how many were read.
///
/// Failure is not fatal to the caller: a browser mid-upgrade, a profile that has never been
/// opened, or a schema change all just mean this source contributes nothing this pass.
pub fn match_visits(path: &Path, engine: Engine, matcher: &mut VisitMatcher) -> Result<usize> {
    let copy = TempCopy::of(path)?;
    // Writable because the copy is ours: a copy taken mid-transaction carries a hot rollback
    // journal, and SQLite can only undo it, getting back the last committed state, on a
    // connection that may write.
    let conn = Connection::open_with_flags(&copy.path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    read_visits(&conn, engine, matcher)
}

/// The query half, separated so it can be tested against a synthetic database.
///
/// Visits past the matcher's horizon stay in the database, since none can name a pending
/// session. The title is borrowed from SQLite's buffer and checked before the URL is parsed,
/// so the visits that are dropped, nearly all of them, cost no allocation.
pub fn read_visits(conn: &Connection, engine: Engine, matcher: &mut VisitMatcher) -> Result<usize> {
    let sql = match engine {
        Engine::Gecko => {
            "SELECT v.visit_date, p.url, p.title
             FROM moz_historyvisits v JOIN moz_places p ON p.id = v.place_id
             WHERE p.title <> '' AND v.visit_date <= ?1"
        }
        Engine::Chromium => {
            "SELECT v.visit_time, u.url, u.title
             FROM visits v JOIN urls u ON u.id = v.url
             WHERE u.title <> '' AND v.visit_time <= ?1"
        }
    };

    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query([from_millis(matcher.horizon(), engine)])?;
    let mut read = 0;
    while let Some(row) = rows.next()? {
        read += 1;
        let (Ok(title), Ok(url)) = (row.get_ref(2)?.as_str(), row.get_ref(1)?.as_str()) else {
            continue;
        };
        if !matcher.wants(title) {
            continue;
        }
        if let Some(host) = host_of(url) {
            matcher.offer(to_millis(row.get(0)?, engine), title, host);
        }
    }
    Ok(read)
}

/// Both engines store microseconds, but from different epochs.
fn to_millis(stamp: i64, engine: Engine) -> Millis {
    if stamp <= 0 {
        return 0;
    }
    match engine {
        Engine::Gecko => stamp / 1_000,
        Engine::Chromium => (stamp - WEBKIT_EPOCH_OFFSET_US) / 1_000,
    }
}

/// The inverse of [`to_millis`].
fn from_millis(ms: Millis, engine: Engine) -> i64 {
    match engine {
        Engine::Gecko => ms.saturating_mul(1_000),
        Engine::Chromium => ms
            .saturating_mul(1_000)
            .saturating_add(WEBKIT_EPOCH_OFFSET_US),
    }
}

/// Host of an `http`/`https` URL.
///
/// Returning `None` for every other scheme is the filter that keeps `chrome://`,
/// `about:`, `file://` and extension pages out of the index.
fn host_of(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    // Anything before '@' is userinfo, not the host.
    let authority = authority.rsplit('@').next()?;

    let host = if let Some(after_bracket) = authority.strip_prefix('[') {
        // IPv6 literal: the colons inside are not a port separator.
        let end = after_bracket.find(']')?;
        &authority[..end + 2]
    } else {
        authority.split(':').next()?
    };

    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

/// Files SQLite keeps beside a database: the WAL pair, or the rollback journal Chromium uses.
const SIDECARS: [&str; 3] = ["-wal", "-shm", "-journal"];

/// A copy of a database file, plus its sidecars, deleted on drop.
struct TempCopy {
    path: PathBuf,
}

impl TempCopy {
    fn of(source: &Path) -> Result<Self> {
        let stem = source
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "history".to_string());
        let unique = format!(
            "toki-{}-{}-{stem}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);

        let copied = std::fs::copy(source, &path).map_err(to_sqlite_err)?;
        BYTES_COPIED.fetch_add(copied, std::sync::atomic::Ordering::Relaxed);
        // A WAL holds everything written since the last checkpoint; a journal holds what an
        // unfinished transaction overwrote. Missing ones are normal: the browser may not be
        // running, and each engine uses only one kind.
        for ext in SIDECARS {
            let from = append_ext(source, ext);
            if from.is_file() {
                if let Ok(copied) = std::fs::copy(&from, append_ext(&path, ext)) {
                    BYTES_COPIED.fetch_add(copied, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        Ok(Self { path })
    }
}

impl Drop for TempCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        for ext in SIDECARS {
            let _ = std::fs::remove_file(append_ext(&self.path, ext));
        }
    }
}

static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Bytes written by history copies since the process started. What `toki-daemon bench`
/// reports as the SSD cost of a pass.
pub static BYTES_COPIED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn append_ext(path: &Path, ext: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(ext);
    PathBuf::from(s)
}

fn to_sqlite_err(e: std::io::Error) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
        Some(e.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Millis = 60_000;
    /// 2026-08-27T00:00:00Z, so stamps look like the real thing.
    const T0: Millis = 1_787_788_800_000;

    const CHROMIUM_SCHEMA: &str = "
        CREATE TABLE urls (id INTEGER PRIMARY KEY, url TEXT, title TEXT);
        CREATE TABLE visits (id INTEGER PRIMARY KEY, url INTEGER, visit_time INTEGER);";

    /// One URL per row, each with one visit at the given time.
    fn chromium_db(conn: &Connection, rows: &[(&str, &str, Millis)]) {
        conn.execute_batch(CHROMIUM_SCHEMA).unwrap();
        for (url, title, at) in rows {
            conn.execute(
                "INSERT INTO urls (url, title) VALUES (?1, ?2)",
                rusqlite::params![url, title],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO visits (url, visit_time) VALUES (?1, ?2)",
                rusqlite::params![conn.last_insert_rowid(), from_millis(*at, Engine::Chromium)],
            )
            .unwrap();
        }
    }

    fn gecko_db(rows: &[(&str, &str, Millis)]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE moz_places (id INTEGER PRIMARY KEY, url TEXT, title TEXT);
             CREATE TABLE moz_historyvisits (id INTEGER PRIMARY KEY, place_id INTEGER,
                                             visit_date INTEGER);",
        )
        .unwrap();
        for (url, title, at) in rows {
            conn.execute(
                "INSERT INTO moz_places (url, title) VALUES (?1, ?2)",
                rusqlite::params![url, title],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO moz_historyvisits (place_id, visit_date) VALUES (?1, ?2)",
                rusqlite::params![conn.last_insert_rowid(), from_millis(*at, Engine::Gecko)],
            )
            .unwrap();
        }
        conn
    }

    fn session(caption: &str, engine: Engine, end: Millis) -> VisitMatcher {
        VisitMatcher::new([(caption, engine, end)])
    }

    /// Chromium's journal mode. A transaction that outgrows the page cache writes into the
    /// database file before it commits, leaving the old pages only in the journal.
    #[test]
    fn a_copy_taken_mid_transaction_reads_the_last_commit() {
        let dir = std::env::temp_dir().join(format!("toki-journal-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("History");
        let _ = std::fs::remove_file(&path);

        let browser = Connection::open(&path).unwrap();
        browser
            .execute_batch("PRAGMA journal_mode = TRUNCATE; PRAGMA cache_size = 1;")
            .unwrap();
        chromium_db(&browser, &[("https://claude.ai/", "Claude", T0)]);
        browser
            .execute_batch("BEGIN; UPDATE urls SET url = 'https://wrong.example/';")
            .unwrap();
        for i in 0..2_000 {
            browser
                .execute(
                    "INSERT INTO urls (url, title) VALUES ('https://filler.example/', ?1)",
                    [format!("filler {i} {}", "x".repeat(200))],
                )
                .unwrap();
        }
        assert!(append_ext(&path, "-journal").metadata().unwrap().len() > 0);

        let mut m = session("Claude - Google Chrome", Engine::Chromium, T0 + MIN);
        let read = match_visits(&path, Engine::Chromium, &mut m).unwrap();

        assert_eq!(read, 1);
        assert_eq!(m.host(0), Some("claude.ai"));
        drop(browser);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_extraction_covers_the_shapes_a_browser_actually_stores() {
        assert_eq!(
            host_of("https://www.youtube.com/watch?v=abc"),
            Some("www.youtube.com")
        );
        assert_eq!(host_of("http://example.com"), Some("example.com"));
        assert_eq!(
            host_of("https://example.com:8443/path"),
            Some("example.com")
        );
        assert_eq!(
            host_of("https://user:pw@example.com/x"),
            Some("example.com")
        );
        assert_eq!(host_of("https://example.com#frag"), Some("example.com"));
        assert_eq!(
            host_of("https://[2606:4700::1111]:443/x"),
            Some("[2606:4700::1111]")
        );
    }

    #[test]
    fn non_web_schemes_are_filtered_out_rather_than_stored() {
        assert_eq!(host_of("chrome://settings"), None);
        assert_eq!(host_of("about:config"), None);
        assert_eq!(host_of("file:///C:/notes.txt"), None);
        assert_eq!(host_of("moz-extension://abc/page.html"), None);
        assert_eq!(host_of("https://"), None);
    }

    #[test]
    fn gecko_microseconds_are_unix_based() {
        assert_eq!(to_millis(T0 * 1_000, Engine::Gecko), T0);
        assert_eq!(from_millis(T0, Engine::Gecko), T0 * 1_000);
    }

    #[test]
    fn chromium_microseconds_are_1601_based() {
        let webkit_us = T0 * 1_000 + WEBKIT_EPOCH_OFFSET_US;
        assert_eq!(to_millis(webkit_us, Engine::Chromium), T0);
        assert_eq!(from_millis(T0, Engine::Chromium), webkit_us);
    }

    #[test]
    fn visits_after_the_last_session_are_not_read() {
        let conn = Connection::open_in_memory().unwrap();
        chromium_db(
            &conn,
            &[
                ("https://checkout.stripe.com/pay", "X", T0 + 60 * MIN),
                ("https://x.com/home", "X", T0),
            ],
        );
        let mut m = session("X - Google Chrome", Engine::Chromium, T0 + MIN);
        assert_eq!(read_visits(&conn, Engine::Chromium, &mut m).unwrap(), 1);
        assert_eq!(m.host(0), Some("x.com"));
    }

    #[test]
    fn a_gecko_database_names_a_zen_window() {
        let conn = gecko_db(&[
            ("https://x.com/CodingNoobie", "Home / X", T0),
            ("chrome://browser/content/browser.xhtml", "Zen", T0),
        ]);
        let mut m = session("Home / X \u{2014} Zen Browser", Engine::Gecko, T0 + MIN);
        assert_eq!(read_visits(&conn, Engine::Gecko, &mut m).unwrap(), 2);
        assert_eq!(m.host(0), Some("x.com"));
    }

    #[test]
    fn several_profiles_feed_one_matcher() {
        let a = Connection::open_in_memory().unwrap();
        chromium_db(&a, &[("https://claude.ai/new", "New chat - Claude", T0)]);
        let b = Connection::open_in_memory().unwrap();
        chromium_db(&b, &[("https://github.com/x", "x - GitHub", T0)]);

        let mut m = VisitMatcher::new([
            (
                "New chat - Claude - Google Chrome",
                Engine::Chromium,
                T0 + MIN,
            ),
            ("x - GitHub - Google Chrome", Engine::Chromium, T0 + MIN),
        ]);
        read_visits(&a, Engine::Chromium, &mut m).unwrap();
        read_visits(&b, Engine::Chromium, &mut m).unwrap();

        assert_eq!(m.host(0), Some("claude.ai"));
        assert_eq!(m.host(1), Some("github.com"));
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_panic() {
        let mut m = session("x - Google Chrome", Engine::Chromium, T0);
        let missing = std::env::temp_dir().join("toki-does-not-exist.sqlite");
        assert!(match_visits(&missing, Engine::Chromium, &mut m).is_err());
    }
}
