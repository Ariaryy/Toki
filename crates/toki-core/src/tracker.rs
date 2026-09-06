use serde::{Deserialize, Serialize};

use crate::event::{AppKey, Event, Millis, Presence};

const MINUTE: Millis = 60 * 1_000;
const HOUR: Millis = 60 * MINUTE;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Focus shorter than this is absorbed into the preceding session rather than recorded.
    ///
    /// Raised from 2s to 5s on 2026-09-05 against eight days of real capture: 30% of all
    /// rows were 2–5s focuses carrying 3.6% of tracked time, and 45% of them were a flick
    /// away and straight back to the app already in front — time the preceding session is
    /// the more honest home for.
    pub noise_floor_ms: Millis,
    /// Input silence after which you are considered away, unless audio says otherwise.
    pub idle_threshold_ms: Millis,
    /// Ceiling on how long audio alone may keep a session alive, measured from last input.
    pub audio_cap_ms: Millis,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            noise_floor_ms: 5 * 1_000,
            idle_threshold_ms: 5 * MINUTE,
            audio_cap_ms: 2 * HOUR,
        }
    }
}

/// Upper bounds, in ms, of the absorbed-pause histogram's buckets. Anything at or above the
/// last bound lands in an overflow bucket, so the array is one longer than this.
///
/// Chosen to bracket the 5-minute idle threshold at the granularity the question needs:
/// "if the threshold were 2 minutes instead, how much of what we currently count as active
/// would stop being counted?" is answered by summing the buckets at or above 120s.
pub const IDLE_BUCKET_BOUNDS_MS: [Millis; 7] =
    [15_000, 30_000, 60_000, 120_000, 180_000, 240_000, 300_000];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdleBucket {
    pub count: u32,
    pub total_ms: Millis,
}

/// How much input silence the idle threshold has silently absorbed into open sessions.
///
/// A pause shorter than the threshold never closes a session, so the time is counted as
/// active and leaves no trace in `sessions` — start and end survive, the gap inside does
/// not. Without this, the threshold cannot be evaluated from stored data at any sample
/// size, which is exactly where eight days of capture left it. Aggregate-only and
/// deliberately small: a single `meta` row, no schema change, no per-session cost.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdleHistogram {
    pub buckets: [IdleBucket; IDLE_BUCKET_BOUNDS_MS.len() + 1],
}

impl IdleHistogram {
    /// Record one pause that input returned from, so the session it belonged to stayed open.
    pub fn record(&mut self, pause_ms: Millis) {
        let i = IDLE_BUCKET_BOUNDS_MS
            .iter()
            .position(|&bound| pause_ms < bound)
            .unwrap_or(IDLE_BUCKET_BOUNDS_MS.len());
        self.buckets[i].count += 1;
        self.buckets[i].total_ms += pause_ms;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub start: Millis,
    pub end: Millis,
    pub app: AppKey,
    pub title: String,
    pub presence: Presence,
}

impl Session {
    pub fn duration_ms(&self) -> Millis {
        self.end - self.start
    }
}

/// What the store should do. Nothing else in the system decides to write a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Emit(Session),
    /// Move the previously emitted session's end forward, absorbing a sub-noise-floor blip.
    ExtendLast {
        end: Millis,
    },
}

struct Open {
    start: Millis,
    app: AppKey,
    title: String,
    /// Most recent input seen. Seeded from the focus event, because a foreground change is
    /// almost always something the user just did.
    last_input: Millis,
    /// Set the first time audio kept this session alive past the idle threshold; cleared
    /// when input returns. Also the origin of the audio cap window.
    audio_since: Option<Millis>,
}

pub struct Tracker {
    cfg: Config,
    open: Option<Open>,
    /// End of the last emitted session, used to decide whether a blip has anything
    /// contiguous to merge into.
    last_end: Option<Millis>,
    paused: bool,
    idle: IdleHistogram,
}

impl Tracker {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            open: None,
            last_end: None,
            paused: false,
            idle: IdleHistogram::default(),
        }
    }

    /// Counts accumulated since the process started. The daemon persists these and seeds
    /// them back on the next run, so restarts don't reset the sample.
    pub fn idle_histogram(&self) -> IdleHistogram {
        self.idle
    }

    pub fn set_idle_histogram(&mut self, idle: IdleHistogram) {
        self.idle = idle;
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Replace the thresholds live. Safe at any point, including mid-session: every field
    /// is read fresh off `self.cfg` on the next tick or close, nothing is precomputed off
    /// the old value, so there is no stale deadline hanging around to reconcile.
    pub fn set_config(&mut self, cfg: Config) {
        self.cfg = cfg;
    }

    /// True while a session is open. The GUI shows this as "currently tracking".
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// When the open session began, for mirroring it where a crash can find it.
    ///
    /// The tracker's own copy is the only correct one: a focus event's timestamp is not
    /// always the start, since the noise floor can fold a blip back into what came before.
    pub fn open_start(&self) -> Option<Millis> {
        self.open.as_ref().map(|o| o.start)
    }

    pub fn on_event(&mut self, ev: Event) -> Vec<Action> {
        match ev {
            Event::Focus { at, app, title } => {
                if self.paused {
                    return Vec::new();
                }
                let actions = self.close(at);
                self.open = Some(Open {
                    start: at,
                    app,
                    title,
                    last_input: at,
                    audio_since: None,
                });
                actions
            }
            Event::Suspend { at } | Event::Lock { at } | Event::Shutdown { at } => self.close(at),
            Event::PauseStart { at } => {
                self.paused = true;
                self.close(at)
            }
            Event::PauseEnd { .. } => {
                self.paused = false;
                Vec::new()
            }
        }
    }

    /// Presence sample. `audio` is only invoked once idle has actually been reached, so the
    /// WASAPI check costs nothing on the overwhelming majority of ticks.
    pub fn on_tick(
        &mut self,
        at: Millis,
        last_input_at: Millis,
        mut audio: impl FnMut() -> bool,
    ) -> Vec<Action> {
        if self.paused {
            return Vec::new();
        }
        let Some(open) = self.open.as_mut() else {
            return Vec::new();
        };

        if last_input_at > open.last_input {
            // Input came back, so whatever silence preceded it never reached the threshold
            // and was counted as active time. This is the only moment that fact is
            // observable — after `last_input` moves the gap is gone for good.
            self.idle.record(last_input_at - open.last_input);
            open.last_input = last_input_at;
            open.audio_since = None;
        }

        if at - open.last_input < self.cfg.idle_threshold_ms {
            return Vec::new();
        }

        if audio() {
            let since = *open.audio_since.get_or_insert(open.last_input);
            if at - since < self.cfg.audio_cap_ms {
                return Vec::new();
            }
            // Credit the audio stretch up to the cap. Ending at the last input instead
            // would throw away the whole film the exemption exists to keep.
            let end = since + self.cfg.audio_cap_ms;
            return self.close(end);
        }

        let end = open.last_input;
        self.close(end)
    }

    /// Close any open session at `at`. Used for clean shutdown and, at startup, to close a
    /// session left dangling by a crash at its last heartbeat.
    pub fn flush(&mut self, at: Millis) -> Vec<Action> {
        self.close(at)
    }

    fn close(&mut self, at: Millis) -> Vec<Action> {
        let Some(open) = self.open.take() else {
            return Vec::new();
        };
        let end = at.max(open.start);

        if end - open.start < self.cfg.noise_floor_ms {
            // A blip only merges into something it is actually contiguous with. After an
            // idle gap the previous session ended earlier than this one began, and
            // stretching it across the gap would invent time that was never spent.
            return match self.last_end {
                Some(prev_end) if prev_end == open.start => {
                    self.last_end = Some(end);
                    vec![Action::ExtendLast { end }]
                }
                _ => Vec::new(),
            };
        }

        self.last_end = Some(end);
        vec![Action::Emit(Session {
            start: open.start,
            end,
            app: open.app,
            title: open.title,
            presence: if open.audio_since.is_some() {
                Presence::AudioOnly
            } else {
                Presence::Input
            },
        })]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Millis = 1_000;

    fn focus(at: Millis, exe: &str) -> Event {
        Event::Focus {
            at,
            app: AppKey::new(exe),
            title: format!("{exe} window"),
        }
    }

    fn emitted(actions: &[Action]) -> Vec<&Session> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Emit(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn ordinary_switch_emits_one_closed_session() {
        let mut t = Tracker::new(Config::default());
        assert!(t.on_event(focus(0, "code.exe")).is_empty());

        let out = t.on_event(focus(30 * S, "chrome.exe"));
        let s = emitted(&out);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].start, 0);
        assert_eq!(s[0].end, 30 * S);
        assert_eq!(s[0].app.as_str(), "code.exe");
        assert_eq!(s[0].presence, Presence::Input);
    }

    #[test]
    fn blip_is_absorbed_into_the_contiguous_previous_session() {
        let mut t = Tracker::new(Config::default());
        t.on_event(focus(0, "code.exe"));
        t.on_event(focus(30 * S, "discord.exe")); // emits code.exe 0..30s
        let out = t.on_event(focus(31 * S, "code.exe")); // discord held 1s

        assert_eq!(out, vec![Action::ExtendLast { end: 31 * S }]);
    }

    /// Pins the floor itself, which the 1s blip above does not: 3s sat either side of the
    /// old 2s value and the new 5s one, so only this distinguishes them.
    #[test]
    fn focus_between_the_old_and_new_floor_is_absorbed() {
        let mut t = Tracker::new(Config::default());
        t.on_event(focus(0, "code.exe"));
        t.on_event(focus(30 * S, "discord.exe"));
        let out = t.on_event(focus(33 * S, "code.exe")); // discord held 3s

        assert_eq!(out, vec![Action::ExtendLast { end: 33 * S }]);
    }

    #[test]
    fn focus_at_the_floor_is_recorded() {
        let mut t = Tracker::new(Config::default());
        t.on_event(focus(0, "code.exe"));
        t.on_event(focus(30 * S, "discord.exe"));
        let out = t.on_event(focus(35 * S, "code.exe")); // discord held exactly 5s

        let s = emitted(&out);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].app.as_str(), "discord.exe");
        assert_eq!(s[0].duration_ms(), 5 * S);
    }

    #[test]
    fn absorbed_pauses_are_counted_but_a_closing_one_is_not() {
        let mut t = Tracker::new(Config::default());
        t.on_event(focus(0, "code.exe"));

        // Silent from 0 to 90s, then input returns: under the 5-minute threshold, so the
        // session stays open and those 90s are counted as active.
        t.on_tick(90 * S, 90 * S, || false);
        // And again, a shorter one.
        t.on_tick(110 * S, 110 * S, || false);

        let h = t.idle_histogram();
        // 90s lands in [60s, 120s), 20s in [15s, 30s).
        assert_eq!(h.buckets[3].count, 1);
        assert_eq!(h.buckets[3].total_ms, 90 * S);
        assert_eq!(h.buckets[1].count, 1);
        assert_eq!(h.buckets[1].total_ms, 20 * S);

        // A pause that actually reaches the threshold closes the session instead, so it is
        // not absorbed time and must not be counted as any.
        let before = t.idle_histogram();
        t.on_tick(110 * S + 5 * MINUTE, 110 * S, || false);
        assert_eq!(t.idle_histogram(), before);
    }

    #[test]
    fn histogram_survives_a_restart() {
        let mut t = Tracker::new(Config::default());
        t.on_event(focus(0, "code.exe"));
        t.on_tick(90 * S, 90 * S, || false);
        let saved = t.idle_histogram();

        let mut next = Tracker::new(Config::default());
        next.set_idle_histogram(saved);
        next.on_event(focus(0, "code.exe"));
        next.on_tick(90 * S, 90 * S, || false);

        assert_eq!(next.idle_histogram().buckets[3].count, 2);
    }

    #[test]
    fn blip_after_an_idle_gap_is_dropped_rather_than_stretched() {
        let mut t = Tracker::new(Config::default());
        t.on_event(focus(0, "code.exe"));
        // Away at 60s; session closes backdated there.
        t.on_tick(60 * S + 5 * MINUTE, 60 * S, || false);

        // Return much later, brush past a window, then settle.
        t.on_event(focus(HOUR, "discord.exe"));
        let out = t.on_event(focus(HOUR + S, "code.exe"));

        assert!(
            out.is_empty(),
            "must not extend across the away gap: {out:?}"
        );
    }

    #[test]
    fn idle_closes_the_session_backdated_to_last_input() {
        let mut t = Tracker::new(Config::default());
        t.on_event(focus(0, "code.exe"));

        assert!(t.on_tick(60 * S, 60 * S, || false).is_empty());

        let last_input = 60 * S;
        let out = t.on_tick(last_input + 5 * MINUTE, last_input, || false);
        let s = emitted(&out);
        assert_eq!(s.len(), 1);
        assert_eq!(
            s[0].end, last_input,
            "ending at detection time would invent 5 minutes per break"
        );
    }

    #[test]
    fn audio_probe_is_not_called_before_the_idle_threshold() {
        let mut t = Tracker::new(Config::default());
        t.on_event(focus(0, "chrome.exe"));

        let mut probes = 0;
        for tick in 1..=10 {
            let at = tick * 15 * S;
            t.on_tick(at, at, || {
                probes += 1;
                true
            });
        }
        assert_eq!(probes, 0, "WASAPI must stay off the routine path");
    }

    #[test]
    fn audio_keeps_a_film_alive_past_the_idle_threshold() {
        let mut t = Tracker::new(Config::default());
        t.on_event(focus(0, "chrome.exe"));

        let last_input = 10 * S;
        for minute in 5..90 {
            let out = t.on_tick(last_input + minute * MINUTE, last_input, || true);
            assert!(out.is_empty(), "closed at minute {minute}");
        }

        let out = t.on_event(focus(last_input + 90 * MINUTE, "code.exe"));
        let s = emitted(&out);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].end, last_input + 90 * MINUTE);
        assert_eq!(s[0].presence, Presence::AudioOnly);
    }

    #[test]
    fn audio_cap_credits_the_stretch_instead_of_discarding_it() {
        let cfg = Config::default();
        let mut t = Tracker::new(cfg);
        t.on_event(focus(0, "spotify.exe"));

        let last_input = 10 * S;
        let mut out = Vec::new();
        let mut at = last_input + 5 * MINUTE;
        while out.is_empty() && at < last_input + 12 * HOUR {
            out = t.on_tick(at, last_input, || true);
            at += MINUTE;
        }

        let s = emitted(&out);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].end, last_input + cfg.audio_cap_ms);
        assert_eq!(s[0].presence, Presence::AudioOnly);
    }

    #[test]
    fn returning_input_restarts_the_audio_cap_window() {
        let cfg = Config::default();
        let mut t = Tracker::new(cfg);
        t.on_event(focus(0, "chrome.exe"));

        // Idle with audio for an hour, then touch the keyboard.
        t.on_tick(70 * MINUTE, 10 * S, || true);
        t.on_tick(71 * MINUTE, 71 * MINUTE, || false);

        // The cap must now be measured from the new input, not the old one.
        let out = t.on_tick(71 * MINUTE + 90 * MINUTE, 71 * MINUTE, || true);
        assert!(out.is_empty(), "cap window did not reset: {out:?}");
    }

    #[test]
    fn suspend_closes_at_the_event_rather_than_running_overnight() {
        let mut t = Tracker::new(Config::default());
        t.on_event(focus(0, "code.exe"));

        let out = t.on_event(Event::Suspend { at: 4 * HOUR });
        let s = emitted(&out);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].end, 4 * HOUR);
    }

    #[test]
    fn paused_time_is_recorded_as_nothing_at_all() {
        let mut t = Tracker::new(Config::default());
        t.on_event(focus(0, "code.exe"));

        let out = t.on_event(Event::PauseStart { at: 10 * MINUTE });
        assert_eq!(emitted(&out).len(), 1);

        assert!(t.on_event(focus(20 * MINUTE, "chrome.exe")).is_empty());
        assert!(t.on_tick(25 * MINUTE, 25 * MINUTE, || true).is_empty());
        assert!(!t.is_open());

        t.on_event(Event::PauseEnd { at: 30 * MINUTE });
        t.on_event(focus(30 * MINUTE, "chrome.exe"));
        let out = t.flush(40 * MINUTE);
        let s = emitted(&out);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].start, 30 * MINUTE, "paused span must not be credited");
    }
}
