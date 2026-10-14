//! Turning a browser window title into a domain.
//!
//! The window title already identifies the active page in real time, for free, via the
//! title-change hook. What it lacks is the domain. The browser's own history database is a
//! title-to-URL dictionary, so the two are joined off the hot path.
//!
//! Measured on real history (2026-08-27): after `www.` normalization, 0.5% of Zen's 7,581
//! distinct titles and 9.6% of Chrome's 1,393 map to more than one domain. Chrome's residual
//! is concentrated in auth redirects and payment handoffs, which are held for seconds, so
//! time-weighted error is well below the row-count figure.

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

/// `"(3) YouTube"` becomes `"YouTube"`. Browsers put unread counts in the caption; the
/// history row has none, so leaving it on guarantees a miss.
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

struct Entry {
    host: String,
    last_visit: Millis,
    /// This title has been seen pointing at more than one host. The recency tie-break
    /// picks one, but the count of these is how the design's accuracy claim is checked.
    ambiguous: bool,
}

/// Title-to-domain map built from a browser's history database.
#[derive(Default)]
pub struct TitleIndex {
    by_title: HashMap<String, Entry>,
}

impl TitleIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.by_title.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_title.is_empty()
    }

    /// Most recently visited wins. This is the tie-break that removes most of the residual
    /// ambiguity, since the live tab is nearly always the recent one.
    pub fn insert(&mut self, title: &str, host: &str, last_visit: Millis) {
        let key = title.trim();
        let host = normalize_host(host);
        if key.is_empty() || host.is_empty() {
            return;
        }
        match self.by_title.get_mut(key) {
            Some(existing) => {
                if existing.host != host {
                    existing.ambiguous = true;
                }
                if last_visit > existing.last_visit {
                    existing.host = host;
                    existing.last_visit = last_visit;
                }
            }
            None => {
                self.by_title.insert(
                    key.to_string(),
                    Entry {
                        host,
                        last_visit,
                        ambiguous: false,
                    },
                );
            }
        }
    }

    /// How many titles map to more than one host. Only the recency tie-break separates
    /// them, so this is the population where resolution can be wrong.
    pub fn ambiguous(&self) -> usize {
        self.by_title.values().filter(|e| e.ambiguous).count()
    }

    pub fn resolve(&self, window_title: &str, engine: Engine) -> Option<&str> {
        let key = normalize_title(window_title, engine.title_suffixes());
        self.by_title.get(&key).map(|e| e.host.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_window_caption_resolves_through_the_history_index() {
        let mut ix = TitleIndex::new();
        ix.insert("New chat - Claude", "claude.ai", 1_000);
        ix.insert("Home / X", "x.com", 1_000);

        assert_eq!(
            ix.resolve("New chat - Claude - Google Chrome", Engine::Chromium),
            Some("claude.ai")
        );
        assert_eq!(
            ix.resolve("Home / X \u{2014} Zen Browser", Engine::Gecko),
            Some("x.com")
        );
    }

    #[test]
    fn ambiguous_titles_resolve_to_the_most_recently_visited_host() {
        // Real collision from Chrome history: a Stripe checkout page carrying the title "X".
        let mut ix = TitleIndex::new();
        ix.insert("X", "checkout.stripe.com", 1_000);
        ix.insert("X", "www.x.com", 2_000);
        assert_eq!(
            ix.resolve("X - Google Chrome", Engine::Chromium),
            Some("x.com")
        );

        // Order of insertion must not matter.
        let mut ix = TitleIndex::new();
        ix.insert("X", "www.x.com", 2_000);
        ix.insert("X", "checkout.stripe.com", 1_000);
        assert_eq!(
            ix.resolve("X - Google Chrome", Engine::Chromium),
            Some("x.com")
        );
    }

    #[test]
    fn an_unknown_title_stays_unresolved_rather_than_guessing() {
        let ix = TitleIndex::new();
        assert_eq!(
            ix.resolve("Some brand new tab - Google Chrome", Engine::Chromium),
            None
        );
    }

    #[test]
    fn rows_without_a_usable_title_or_host_are_skipped() {
        let mut ix = TitleIndex::new();
        ix.insert("   ", "example.com", 1);
        ix.insert("Something", "  ", 1);
        assert!(ix.is_empty());
    }
}
