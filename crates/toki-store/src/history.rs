//! Reading the *browsers'* SQLite, as opposed to Toki's own.
//!
//! Both engines keep a history database that is, for our purposes, a title-to-URL
//! dictionary. This module turns one into a [`TitleIndex`]; matching a live window caption
//! against it is [`toki_core::resolve`]'s job.
//!
//! The file is copied before it is opened. Opening the live file with `immutable=1` would
//! be faster but skips the WAL, so it returns whatever was true at the last checkpoint —
//! exactly the recent rows the resolver needs most.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, Result};
use toki_core::event::Millis;
use toki_core::resolve::{Engine, TitleIndex};

/// Microseconds between 1601-01-01 and 1970-01-01. Chromium counts from the former.
const WEBKIT_EPOCH_OFFSET_US: i64 = 11_644_473_600_000_000;

/// Load one history database into `index`, returning how many rows were usable.
///
/// Failure is not fatal to the caller: a browser mid-upgrade, a profile that has never been
/// opened, or a schema change all just mean this source contributes nothing this pass.
pub fn load(path: &Path, engine: Engine, index: &mut TitleIndex) -> Result<usize> {
    let copy = TempCopy::of(path)?;
    let conn = Connection::open_with_flags(&copy.path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    read_into(&conn, engine, index)
}

/// The query half, separated so it can be tested against a synthetic database.
pub fn read_into(conn: &Connection, engine: Engine, index: &mut TitleIndex) -> Result<usize> {
    let sql = match engine {
        Engine::Gecko => {
            "SELECT url, title, last_visit_date FROM moz_places
             WHERE title IS NOT NULL AND title <> ''"
        }
        Engine::Chromium => {
            "SELECT url, title, last_visit_time FROM urls
             WHERE title IS NOT NULL AND title <> ''"
        }
    };

    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<i64>>(2)?.unwrap_or(0),
        ))
    })?;

    let mut used = 0;
    for row in rows {
        let (url, title, stamp) = row?;
        let Some(host) = host_of(&url) else {
            continue;
        };
        index.insert(&title, host, to_millis(stamp, engine));
        used += 1;
    }
    Ok(used)
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

/// Host of an `http`/`https` URL.
///
/// Returning `None` for every other scheme is the filter that keeps `chrome://`,
/// `about:`, `file://` and extension pages out of the index.
pub fn host_of(url: &str) -> Option<&str> {
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

/// A copy of a database file, plus its WAL sidecars, deleted on drop.
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

        std::fs::copy(source, &path).map_err(to_sqlite_err)?;
        // Sidecars hold everything written since the last checkpoint. Missing ones are
        // normal — the browser may not be running.
        for ext in ["-wal", "-shm"] {
            let from = append_ext(source, ext);
            if from.is_file() {
                let _ = std::fs::copy(&from, append_ext(&path, ext));
            }
        }
        Ok(Self { path })
    }
}

impl Drop for TempCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        for ext in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(append_ext(&self.path, ext));
        }
    }
}

static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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

    fn gecko_db(rows: &[(&str, &str, i64)]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE moz_places (url TEXT, title TEXT, last_visit_date INTEGER)",
        )
        .unwrap();
        for (url, title, stamp) in rows {
            conn.execute(
                "INSERT INTO moz_places VALUES (?1, ?2, ?3)",
                rusqlite::params![url, title, stamp],
            )
            .unwrap();
        }
        conn
    }

    fn chromium_db(rows: &[(&str, &str, i64)]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE urls (url TEXT, title TEXT, last_visit_time INTEGER)")
            .unwrap();
        for (url, title, stamp) in rows {
            conn.execute(
                "INSERT INTO urls VALUES (?1, ?2, ?3)",
                rusqlite::params![url, title, stamp],
            )
            .unwrap();
        }
        conn
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
        // 2026-08-27T00:00:00Z
        let unix_ms = 1_787_788_800_000_i64;
        assert_eq!(to_millis(unix_ms * 1_000, Engine::Gecko), unix_ms);
    }

    #[test]
    fn chromium_microseconds_are_1601_based() {
        let unix_ms = 1_787_788_800_000_i64;
        let webkit_us = unix_ms * 1_000 + WEBKIT_EPOCH_OFFSET_US;
        assert_eq!(to_millis(webkit_us, Engine::Chromium), unix_ms);
    }

    #[test]
    fn a_never_visited_row_does_not_win_the_recency_tie_break() {
        let conn = chromium_db(&[
            ("https://checkout.stripe.com/pay", "X", 0),
            (
                "https://x.com/home",
                "X",
                1_787_788_800_000_000 + WEBKIT_EPOCH_OFFSET_US,
            ),
        ]);
        let mut index = TitleIndex::new();
        assert_eq!(read_into(&conn, Engine::Chromium, &mut index).unwrap(), 2);
        assert_eq!(
            index.resolve("X - Google Chrome", Engine::Chromium),
            Some("x.com")
        );
    }

    #[test]
    fn a_gecko_database_resolves_a_zen_window_caption() {
        let conn = gecko_db(&[
            (
                "https://x.com/CodingNoobie",
                "Home / X",
                1_787_788_800_000_000,
            ),
            (
                "chrome://browser/content/browser.xhtml",
                "Zen",
                1_787_788_800_000_000,
            ),
        ]);
        let mut index = TitleIndex::new();
        // The chrome:// row is skipped, so only one is usable.
        assert_eq!(read_into(&conn, Engine::Gecko, &mut index).unwrap(), 1);
        assert_eq!(
            index.resolve("Home / X \u{2014} Zen Browser", Engine::Gecko),
            Some("x.com")
        );
    }

    #[test]
    fn several_profiles_merge_into_one_index() {
        let a = chromium_db(&[(
            "https://claude.ai/new",
            "New chat - Claude",
            1_000 + WEBKIT_EPOCH_OFFSET_US,
        )]);
        let b = chromium_db(&[(
            "https://github.com/x",
            "x - GitHub",
            2_000 + WEBKIT_EPOCH_OFFSET_US,
        )]);

        let mut index = TitleIndex::new();
        read_into(&a, Engine::Chromium, &mut index).unwrap();
        read_into(&b, Engine::Chromium, &mut index).unwrap();

        assert_eq!(index.len(), 2);
        assert_eq!(
            index.resolve("New chat - Claude - Google Chrome", Engine::Chromium),
            Some("claude.ai")
        );
        assert_eq!(
            index.resolve("x - GitHub - Google Chrome", Engine::Chromium),
            Some("github.com")
        );
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_panic() {
        let mut index = TitleIndex::new();
        let missing = std::env::temp_dir().join("toki-does-not-exist.sqlite");
        assert!(load(&missing, Engine::Chromium, &mut index).is_err());
    }
}
