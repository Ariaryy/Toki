//! Attaching domains to browser sessions, off the hot path.
//!
//! A session is written the moment it ends, carrying only the window caption. Turning that
//! caption into a domain means reading the browsers' own history, which involves copying
//! files and opening SQLite — nowhere near the collector's event loop.
//!
//! Deferring it also fixes an ordering problem. A page's history row is written when the
//! visit is recorded, which can be after the caption already changed. A pass that ran
//! immediately would miss rows that a pass thirty seconds later finds.

use chrono::Utc;
use toki_core::resolve::{Engine, TitleIndex};
use toki_store::{Pending, Resolution, Store};
use tracing::warn;

use crate::paths;

/// How many rows one pass will resolve. High enough that a normal backlog clears in a
/// single pass, low enough that a first run over months of imported data does not hold the
/// write lock for a noticeable time.
pub const BATCH: usize = 5_000;

/// How long after a session ends before it is looked up. The browser writes its history row
/// when it records the visit, which can trail the caption; looked up sooner, a session the
/// GUI's launch pass catches would be a miss that only `toki reresolve` brings back.
const MIN_AGE_MS: i64 = 60_000;

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
    /// Page captions that appear in no history database. Retried by `toki reresolve`.
    pub missed: usize,
    /// Rows whose caption was never a page — a PWA window, Picture-in-Picture, the browser's
    /// own window. Held apart from `missed` because no retry can help them.
    pub not_a_page: usize,
    /// Distinct titles in the index the pass was matched against.
    pub index_titles: usize,
}

impl Stats {
    /// Share of *resolvable* rows that found a domain. This is the number the design's
    /// accuracy claim rests on, and it cannot be measured any other way.
    ///
    /// The denominator excludes `not_a_page`. Counting captions with no URL behind them as
    /// failures is what made Chrome read as 39% while nothing was wrong with it: 1,201 of
    /// its 1,338 recorded failures were one PWA caption, `WhatsApp Web`.
    pub fn hit_rate(&self) -> f64 {
        hit_rate(self.resolved, self.missed)
    }
}

/// Resolved over resolvable, or zero when nothing was resolvable. Shared with `doctor`,
/// which computes the same ratio from the stored states rather than from one pass.
pub fn hit_rate(resolved: usize, missed: usize) -> f64 {
    match resolved + missed {
        0 => 0.0,
        resolvable => resolved as f64 / resolvable as f64,
    }
}

/// Build a fresh index and drain the pending queue.
///
/// The index is rebuilt every pass rather than kept resident. It costs about 140 ms against
/// this machine's five profiles, and holding twenty thousand titles in the daemon for the
/// sake of one lookup every five minutes trades the wrong resource.
pub fn pass(store: &Store) -> rusqlite::Result<Stats> {
    let pending = store.pending(BATCH, Utc::now().timestamp_millis() - MIN_AGE_MS)?;
    if pending.is_empty() {
        return Ok(Stats::default());
    }

    let (index, unread) = build_index();
    let outcomes: Vec<(i64, Resolution)> = pending
        .iter()
        .filter(|row| !unread.contains(&row.engine))
        .map(|row| (row.id, classify(&index, row)))
        .collect();
    // One commit for the pass, rather than one per row. See `record_resolutions`.
    store.record_resolutions(&outcomes)?;

    Ok(Stats {
        attempted: outcomes.len(),
        resolved: count(&outcomes, |o| matches!(o, Resolution::Found(_))),
        missed: count(&outcomes, |o| matches!(o, Resolution::Missed)),
        not_a_page: count(&outcomes, |o| matches!(o, Resolution::NotAPage)),
        index_titles: index.len(),
    })
}

/// The index of every history source, and the engines that have sources but none that
/// could be read this pass. Rows of those engines stay pending: classified against an index
/// with none of their history, every one would be a miss, and a miss is not retried.
fn build_index() -> (TitleIndex, Vec<Engine>) {
    let mut index = TitleIndex::new();
    let mut read = Vec::new();
    let mut unread = Vec::new();
    for source in paths::history_sources() {
        // One unreadable profile — mid-upgrade, or a browser holding an exclusive lock —
        // must not stop the others from contributing.
        match toki_store::history::load(&source.path, source.engine, &mut index) {
            Ok(_) => read.push(source.engine),
            Err(e) => {
                warn!(%e, path = %source.path.display(), "cannot read a browser history");
                unread.push(source.engine);
            }
        }
    }
    unread.retain(|engine| !read.contains(engine));
    (index, unread)
}

/// Which of the three terminal states one pending row belongs in.
///
/// The lookup comes first and the caption test only breaks a tie it already lost: a window
/// with no browser suffix that nonetheless matches a history row keeps its domain, which is
/// strictly better than classifying it away unread.
fn classify(index: &TitleIndex, row: &Pending) -> Resolution {
    match index.resolve(&row.title, row.engine) {
        Some(host) => Resolution::Found(host.to_string()),
        None if row.engine.is_page_caption(&row.title) => Resolution::Missed,
        None => Resolution::NotAPage,
    }
}

fn count(outcomes: &[(i64, Resolution)], is: impl Fn(&Resolution) -> bool) -> usize {
    outcomes.iter().filter(|(_, o)| is(o)).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use toki_core::resolve::Engine;

    fn row(title: &str, engine: Engine) -> Pending {
        Pending {
            id: 1,
            title: title.to_string(),
            engine,
        }
    }

    #[test]
    fn a_page_that_history_knows_resolves() {
        let mut index = TitleIndex::new();
        index.insert("Claude", "claude.ai", 0);
        let outcome = classify(&index, &row("Claude - Google Chrome", Engine::Chromium));
        assert_eq!(outcome, Resolution::Found("claude.ai".into()));
    }

    #[test]
    fn a_page_history_has_not_caught_up_with_is_a_miss_worth_retrying() {
        let index = TitleIndex::new();
        let outcome = classify(&index, &row("Claude - Google Chrome", Engine::Chromium));
        assert_eq!(outcome, Resolution::Missed);
    }

    #[test]
    fn a_window_with_no_page_behind_it_is_not_a_miss() {
        let index = TitleIndex::new();
        assert_eq!(
            classify(&index, &row("WhatsApp Web", Engine::Chromium)),
            Resolution::NotAPage
        );
        assert_eq!(
            classify(&index, &row("Picture-in-Picture", Engine::Gecko)),
            Resolution::NotAPage
        );
    }

    #[test]
    fn a_lookup_that_succeeds_outranks_the_caption_test() {
        // Classifying first would throw away a real domain. An app window whose caption
        // happens to be in history keeps what the lookup found.
        let mut index = TitleIndex::new();
        index.insert("WhatsApp Web", "web.whatsapp.com", 0);
        assert_eq!(
            classify(&index, &row("WhatsApp Web", Engine::Chromium)),
            Resolution::Found("web.whatsapp.com".into())
        );
    }

    #[test]
    fn the_hit_rate_ignores_captions_that_could_never_resolve() {
        // The Chrome figure from real data: 852 resolved, 137 real misses, 1,201 PWA
        // captions. Counting the last as failures read as 39%; it is 86%.
        assert_eq!(format!("{:.0}", hit_rate(852, 137) * 100.0), "86");
        assert_eq!(hit_rate(0, 0), 0.0);
    }
}
