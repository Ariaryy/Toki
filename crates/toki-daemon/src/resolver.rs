//! Attaching domains to browser sessions, off the hot path.
//!
//! A session is written the moment it ends, carrying only the window caption. Turning that
//! caption into a domain means reading the browsers' own history, which involves copying
//! files and opening SQLite — nowhere near the collector's event loop.
//!
//! Deferring it also fixes an ordering problem. A page's history row is written when the
//! visit is recorded, which can be after the caption already changed. A pass that ran
//! immediately would miss rows that a pass thirty seconds later finds.

use toki_core::resolve::TitleIndex;
use toki_store::Store;

use crate::paths;

/// How many rows one pass will resolve. High enough that a normal backlog clears in a
/// single pass, low enough that a first run over months of imported data does not hold the
/// write lock for a noticeable time.
pub const BATCH: usize = 5_000;

/// What can wake the resolver thread outside its own five-minute clock.
///
/// `RunNow` exists for the GUI's launch-time backfill (DESIGN.md §8): opening the GUI wants
/// the freshest possible domains without waiting out whatever is left of the current
/// interval.
pub enum ResolverMsg {
    Stop,
    RunNow,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    /// Rows that were pending at the start of the pass.
    pub attempted: usize,
    /// Rows that now have a domain.
    pub resolved: usize,
    /// Titles that appear in no history database. Retried by `toki reresolve`.
    pub missed: usize,
    /// Distinct titles in the index the pass was matched against.
    pub index_titles: usize,
}

impl Stats {
    /// Share of attempted rows that found a domain. This is the number the design's
    /// accuracy claim rests on, and it cannot be measured any other way.
    pub fn hit_rate(&self) -> f64 {
        if self.attempted == 0 {
            0.0
        } else {
            self.resolved as f64 / self.attempted as f64
        }
    }
}

/// Build a fresh index and drain the pending queue.
///
/// The index is rebuilt every pass rather than kept resident. It costs about 140 ms against
/// this machine's five profiles, and holding twenty thousand titles in the daemon for the
/// sake of one lookup every five minutes trades the wrong resource.
pub fn pass(store: &Store) -> rusqlite::Result<Stats> {
    let pending = store.pending(BATCH)?;
    if pending.is_empty() {
        return Ok(Stats::default());
    }

    let mut index = TitleIndex::new();
    for source in paths::history_sources() {
        // One unreadable profile — mid-upgrade, or a browser holding an exclusive lock —
        // must not stop the others from contributing.
        let _ = toki_store::history::load(&source.path, source.engine, &mut index);
    }

    let mut stats = Stats {
        attempted: pending.len(),
        index_titles: index.len(),
        ..Stats::default()
    };
    for row in pending {
        match index.resolve(&row.title, row.engine) {
            Some(host) => {
                let host = host.to_string();
                store.set_domain(row.id, Some(&host))?;
                stats.resolved += 1;
            }
            None => {
                store.set_domain(row.id, None)?;
                stats.missed += 1;
            }
        }
    }
    Ok(stats)
}
