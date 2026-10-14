//! Turning a browser window title into a domain.
//!
//! The window title already identifies the active page in real time, for free, via the
//! title-change hook. What it lacks is the domain. The browser's own history records every
//! visit with the page's title, URL and time, so the two are joined off the hot path.

use std::collections::HashMap;

use crate::event::Millis;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Chromium,
    Gecko,
}

impl Engine {
    /// Which engine, if any, an executable belongs to.
    ///
    /// Matching on the file name rather than the full path is deliberate: the same browser
    /// installs per-user or per-machine, and a portable copy lives wherever it was dropped.
    /// The name is the part that stays put.
    pub fn of_exe(exe_path: &str) -> Option<Self> {
        let file = crate::ignore::exe_name(exe_path);
        // Only browsers whose caption suffix is in `title_suffixes`. Claiming an engine
        // without one queues every session for a lookup that cannot succeed.
        match file.as_str() {
            "chrome.exe" | "helium.exe" | "chromium.exe" => Some(Engine::Chromium),
            "zen.exe" | "firefox.exe" => Some(Engine::Gecko),
            // Linux, where the packaged binary often differs from the command you type:
            // `zen-browser` runs `zen-bin`, `chromium` runs `/usr/lib/chromium/chromium`.
            "chrome" | "helium" | "chromium" => Some(Engine::Chromium),
            "zen" | "zen-bin" | "firefox" | "firefox-bin" => Some(Engine::Gecko),
            _ => None,
        }
    }

    /// Whether a caption is a page window at all.
    ///
    /// A browser showing a page always brands the caption with its own name. What does not
    /// is a window with no page behind it: an installed PWA or app window (`WhatsApp Web`),
    /// Picture-in-Picture, the browser's own empty window (`Zen Browser`), a workspace
    /// switcher. No history row can ever match those, however many times the lookup is
    /// retried, and counting them as misses buries the ones that are.
    ///
    /// Only consulted *after* a lookup has already missed, so a caption that resolves anyway
    /// keeps its domain and this never gets to overrule it.
    pub fn is_page_caption(self, window_title: &str) -> bool {
        let title = window_title.trim();
        self.title_suffixes()
            .iter()
            .any(|suffix| title.strip_suffix(suffix).is_some_and(|t| !t.is_empty()))
    }

    /// What the browser appends to the page title in the window caption. Gecko uses an
    /// em dash, Chromium a hyphen.
    pub fn title_suffixes(self) -> &'static [&'static str] {
        match self {
            Engine::Chromium => &[" - Google Chrome", " - Helium", " - Chromium"],
            Engine::Gecko => &[
                " \u{2014} Zen Browser",
                " \u{2014} Mozilla Firefox",
                " - Mozilla Firefox",
            ],
        }
    }
}

/// Lowercase and drop a leading `www.`, which is most of what makes titles look ambiguous.
pub fn normalize_host(host: &str) -> String {
    let lower = host.trim().to_ascii_lowercase();
    match lower.strip_prefix("www.") {
        Some(rest) => rest.to_string(),
        None => lower,
    }
}

/// Reduce a window caption to the page title a history row would hold.
pub fn normalize_title(window_title: &str, suffixes: &[&str]) -> String {
    let mut title = window_title.trim();
    for suffix in suffixes {
        if let Some(rest) = title.strip_suffix(suffix) {
            title = rest;
            break;
        }
    }
    strip_unread_prefix(title).trim().to_string()
}

/// `"(3) YouTube"` becomes `"YouTube"`. Sites put unread counts in the page title, so the
/// caption and the history row each carry whatever count was showing at the time; stripped
/// on both sides, they match.
fn strip_unread_prefix(title: &str) -> &str {
    let Some(rest) = title.strip_prefix('(') else {
        return title;
    };
    let Some((count, tail)) = rest.split_once(") ") else {
        return title;
    };
    if !count.is_empty() && count.chars().all(|c| c.is_ascii_digit()) {
        tail
    } else {
        title
    }
}

/// How long after a session ends its naming visit may still be stamped. A browser can
/// record the visit a moment after the caption already changed.
pub const VISIT_SLACK_MS: Millis = 2 * 60_000;

/// Names pending sessions from a stream of history visits.
///
/// Each session takes the latest visit with its page title stamped no later than the
/// session's end, which is history as it stood when the session happened: a title that
/// later moved to another site keeps the site it had then, and a pass run hours late gives
/// the same answer as one run at once (measured: 99.8% agreement either way, against 98.9%
/// for "most recent visit ever").
///
/// Only the pending titles are held. Visits stream through `offer` and everything else is
/// dropped on sight, so memory follows the queue, not the size of the history.
pub struct VisitMatcher {
    by_title: HashMap<String, Vec<usize>>,
    ends: Vec<Millis>,
    best: Vec<Option<(Millis, String)>>,
}

impl VisitMatcher {
    /// One entry per session: its window caption, browser engine, and end.
    pub fn new<'a>(sessions: impl IntoIterator<Item = (&'a str, Engine, Millis)>) -> Self {
        let mut by_title: HashMap<String, Vec<usize>> = HashMap::new();
        let mut ends = Vec::new();
        for (caption, engine, end) in sessions {
            let key = normalize_title(caption, engine.title_suffixes());
            if !key.is_empty() {
                by_title.entry(key).or_default().push(ends.len());
            }
            ends.push(end);
        }
        let best = vec![None; ends.len()];
        Self {
            by_title,
            ends,
            best,
        }
    }

    /// Visits stamped after this can name no session, so a reader can stop there.
    pub fn horizon(&self) -> Millis {
        self.ends.iter().max().map_or(0, |end| end + VISIT_SLACK_MS)
    }

    /// Whether a history title belongs to any pending session. Asked before the URL is
    /// parsed, which is the expensive half for the visits that are dropped.
    pub fn wants(&self, history_title: &str) -> bool {
        self.by_title.contains_key(history_key(history_title))
    }

    /// One visit from history: when, the page's title, and its host.
    pub fn offer(&mut self, at: Millis, history_title: &str, host: &str) {
        let Some(ids) = self.by_title.get(history_key(history_title)) else {
            return;
        };
        for &i in ids {
            let later = self.best[i].as_ref().is_none_or(|(seen, _)| at > *seen);
            if later && at <= self.ends[i] + VISIT_SLACK_MS {
                self.best[i] = Some((at, normalize_host(host)));
            }
        }
    }

    /// The host found for the `i`th session given to `new`.
    pub fn host(&self, i: usize) -> Option<&str> {
        self.best[i].as_ref().map(|(_, host)| host.as_str())
    }
}

/// History stores the page title as the site set it, unread count included.
fn history_key(title: &str) -> &str {
    strip_unread_prefix(title.trim()).trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Millis = 60_000;

    fn matcher(captions: &[(&str, Engine, Millis)]) -> VisitMatcher {
        VisitMatcher::new(captions.iter().copied())
    }

    #[test]
    fn an_unread_count_in_history_does_not_stop_a_match() {
        let mut m = matcher(&[
            ("X - Google Chrome", Engine::Chromium, 10 * MIN),
            ("(12) X - Google Chrome", Engine::Chromium, 10 * MIN),
        ]);
        m.offer(MIN, "(4) X", "x.com");
        assert_eq!(m.host(0), Some("x.com"));
        assert_eq!(m.host(1), Some("x.com"));
    }

    #[test]
    fn browsers_are_recognised_by_file_name_wherever_they_are_installed() {
        assert_eq!(
            Engine::of_exe(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
            Some(Engine::Chromium)
        );
        assert_eq!(
            Engine::of_exe(r"C:\Users\a\AppData\Local\imput\Helium\Application\helium.exe"),
            Some(Engine::Chromium)
        );
        assert_eq!(
            Engine::of_exe(r"C:\Program Files\Zen Browser\zen.exe"),
            Some(Engine::Gecko)
        );
        // Forward slashes, because a path can arrive either way.
        assert_eq!(Engine::of_exe("D:/portable/Zen.EXE"), Some(Engine::Gecko));
    }

    #[test]
    fn an_ordinary_app_claims_no_engine() {
        assert_eq!(Engine::of_exe(r"C:\Windows\explorer.exe"), None);
        assert_eq!(
            Engine::of_exe("/opt/zen-browser-bin/zen-bin"),
            Some(Engine::Gecko)
        );
        assert_eq!(
            Engine::of_exe("/usr/lib/chromium/chromium"),
            Some(Engine::Chromium)
        );
        // An Omarchy web app is keyed by its app id, not the browser, and is not a browser.
        assert_eq!(Engine::of_exe("chrome-web.whatsapp.com__-Default"), None);
        assert_eq!(Engine::of_exe(r"C:\a\Code.exe"), None);
        // Substring matches must not count: this is not Chrome.
        assert_eq!(Engine::of_exe(r"C:\a\chrome_updater.exe"), None);
    }

    #[test]
    fn a_window_with_no_page_behind_it_is_not_a_page_caption() {
        // The three shapes seen in real data, and what made Chrome read as 39%: an installed
        // PWA, Picture-in-Picture, and the browser's own empty window.
        assert!(!Engine::Chromium.is_page_caption("WhatsApp Web"));
        assert!(!Engine::Gecko.is_page_caption("Picture-in-Picture"));
        assert!(!Engine::Gecko.is_page_caption("Zen Browser"));
        // A Zen workspace name, which is a caption with no page under it either.
        assert!(!Engine::Gecko.is_page_caption("Spam account"));
        // The right suffix but the wrong engine: Chromium never writes an em dash.
        assert!(!Engine::Chromium.is_page_caption("Hacker News \u{2014} Zen Browser"));
    }

    #[test]
    fn an_ordinary_page_is_a_page_caption() {
        assert!(Engine::Chromium.is_page_caption("New chat - Claude - Google Chrome"));
        assert!(Engine::Gecko.is_page_caption("Hacker News \u{2014} Zen Browser"));
        // A new tab is still a page-shaped caption; whether history has it is the lookup's
        // business, not this test's.
        assert!(Engine::Chromium.is_page_caption("New Tab - Google Chrome"));
        // The suffix alone, with nothing in front, is the browser talking about itself.
        assert!(!Engine::Chromium.is_page_caption(" - Google Chrome"));
    }

    #[test]
    fn browser_suffixes_are_stripped_per_engine() {
        assert_eq!(
            normalize_title(
                "New chat - Claude - Google Chrome",
                Engine::Chromium.title_suffixes()
            ),
            "New chat - Claude"
        );
        assert_eq!(
            normalize_title(
                "Home / X \u{2014} Zen Browser",
                Engine::Gecko.title_suffixes()
            ),
            "Home / X"
        );
    }

    #[test]
    fn only_a_leading_numeric_unread_count_is_stripped() {
        let none: &[&str] = &[];
        assert_eq!(normalize_title("(3) YouTube", none), "YouTube");
        assert_eq!(normalize_title("(12) Discord", none), "Discord");
        assert_eq!(normalize_title("(draft) Notes", none), "(draft) Notes");
        assert_eq!(
            normalize_title("Inbox (5) - achintya - Gmail", none),
            "Inbox (5) - achintya - Gmail"
        );
    }

    #[test]
    fn www_is_normalized_away() {
        assert_eq!(normalize_host("www.youtube.com"), "youtube.com");
        assert_eq!(normalize_host("YouTube.com"), "youtube.com");
        assert_eq!(normalize_host("mail.google.com"), "mail.google.com");
    }

    #[test]
    fn a_window_caption_is_named_by_a_visit_to_its_page() {
        let mut m = matcher(&[
            (
                "New chat - Claude - Google Chrome",
                Engine::Chromium,
                10 * MIN,
            ),
            ("Home / X — Zen Browser", Engine::Gecko, 10 * MIN),
        ]);
        m.offer(MIN, "New chat - Claude", "claude.ai");
        m.offer(MIN, "Home / X", "www.x.com");
        assert_eq!(m.host(0), Some("claude.ai"));
        assert_eq!(m.host(1), Some("x.com"));
    }

    #[test]
    fn a_title_takes_the_site_it_had_when_the_session_happened() {
        // Real collision from Chrome history: a Stripe checkout page carrying the title "X".
        let mut m = matcher(&[
            ("X - Google Chrome", Engine::Chromium, 10 * MIN),
            ("X - Google Chrome", Engine::Chromium, 100 * MIN),
        ]);
        // Order of the stream must not matter.
        m.offer(50 * MIN, "X", "checkout.stripe.com");
        m.offer(5 * MIN, "X", "x.com");
        assert_eq!(m.host(0), Some("x.com"));
        assert_eq!(m.host(1), Some("checkout.stripe.com"));
    }

    #[test]
    fn a_visit_after_the_session_only_counts_within_the_slack() {
        let mut m = matcher(&[("Claude - Google Chrome", Engine::Chromium, 10 * MIN)]);
        m.offer(10 * MIN + VISIT_SLACK_MS + 1, "Claude", "claude.ai");
        assert_eq!(m.host(0), None);
        m.offer(10 * MIN + VISIT_SLACK_MS, "Claude", "claude.ai");
        assert_eq!(m.host(0), Some("claude.ai"));
    }

    #[test]
    fn an_unknown_title_stays_unresolved_rather_than_guessing() {
        let mut m = matcher(&[("Some brand new tab - Google Chrome", Engine::Chromium, MIN)]);
        m.offer(0, "Another page", "example.com");
        assert!(!m.wants("Another page"));
        assert_eq!(m.host(0), None);
    }

    #[test]
    fn the_horizon_is_the_last_end_plus_the_slack() {
        let m = matcher(&[
            ("a - Google Chrome", Engine::Chromium, MIN),
            ("b - Google Chrome", Engine::Chromium, 3 * MIN),
        ]);
        assert_eq!(m.horizon(), 3 * MIN + VISIT_SLACK_MS);
        assert_eq!(matcher(&[]).horizon(), 0);
    }
}
