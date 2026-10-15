//! The weekly digest: total hours and the top five entities for the week that just ended,
//! against the week before it.
//!
//! See DESIGN.md §10: fired once, on the first foreground activity of Monday, rather than on
//! a timer — the day boundary is local midnight, so a scheduled fire could land while the
//! machine is asleep or mid-session.

use std::collections::HashMap;

use chrono::{Duration, NaiveDate};
use toki_core::event::Millis;
use toki_store::{Store, Total};

pub struct Digest {
    pub total_ms: Millis,
    pub delta_ms: Millis,
    /// Up to five (app, domain) entities by time, most first.
    pub top: Vec<Total>,
}

/// Totals for the Monday-Sunday week that ended at `monday` (exclusive), and the delta
/// against the week before it.
///
/// Both weeks are fully in the past by the time the digest fires, so — unlike the GUI's
/// in-progress Week view — there is no partial-week comparison to make; each side is a plain
/// seven-day sum.
pub fn for_week_ending(store: &Store, monday: NaiveDate) -> Result<Digest, String> {
    let (total_ms, top) = week_total(store, monday - Duration::days(7))?;
    let (prev_total_ms, _) = week_total(store, monday - Duration::days(14))?;
    Ok(Digest {
        total_ms,
        delta_ms: total_ms - prev_total_ms,
        top,
    })
}

/// A week's grand total plus its top five (app, domain) entities, built from seven calls to
/// `day_totals` — the same primitive `toki-gui`'s Week view sums, so both stay provably
/// consistent with what Today shows for the same day.
fn week_total(store: &Store, week_start: NaiveDate) -> Result<(Millis, Vec<Total>), String> {
    let mut merged: HashMap<(String, Option<String>), Total> = HashMap::new();
    let mut total_ms = 0;
    for offset in 0..7 {
        let date = week_start + Duration::days(offset);
        for t in store.day_totals(date).map_err(|e| e.to_string())? {
            total_ms += t.ms;
            let key = (t.exe_path.clone(), t.domain.clone());
            merged.entry(key).and_modify(|e| e.ms += t.ms).or_insert(t);
        }
    }
    let mut top: Vec<Total> = merged.into_values().collect();
    top.sort_by_key(|t| std::cmp::Reverse(t.ms));
    top.truncate(5);
    Ok((total_ms, top))
}

/// Balloon title and body text. `NOTIFYICONDATAW` allows 63 wide chars for the title and 255
/// for the body; the fill routine that copies these in truncates silently rather than
/// overflowing, so going over just loses the tail rather than corrupting anything.
pub fn format(d: &Digest) -> (String, String) {
    let sign = if d.delta_ms < 0 { "-" } else { "+" };
    let mut body = format!(
        "{} total ({sign}{} vs the week before)",
        fmt_hours(d.total_ms),
        fmt_hours(d.delta_ms.abs())
    );
    for t in &d.top {
        let name = t.domain.as_deref().unwrap_or(&t.app_name);
        body.push_str(&format!("\n{name} — {}", fmt_hours(t.ms)));
    }
    ("Toki — last week".to_string(), body)
}

fn fmt_hours(ms: Millis) -> String {
    let minutes = ms / 60_000;
    let (h, m) = (minutes / 60, minutes % 60);
    if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use toki_core::event::AppKey;
    use toki_core::time::day_bounds;
    use toki_core::tracker::{Action, Session};
    use toki_core::Presence;

    const IST: i32 = 330;
    const HOUR: Millis = 3_600_000;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn emit(store: &Store, date: NaiveDate, hour: i64, len: Millis, exe: &str, title: &str) {
        let (day_start, _) = day_bounds(date, IST);
        let start = day_start + hour * HOUR;
        store
            .apply(
                &Action::Emit(Session {
                    start,
                    end: start + len,
                    app: AppKey::new(exe),
                    title: title.to_string(),
                    presence: Presence::Input,
                }),
                IST,
                None,
            )
            .unwrap();
    }

    /// Monday 2026-08-31, so the just-ended week is 2026-08-24..=2026-08-30 and the one
    /// before it is 2026-08-17..=2026-08-23.
    #[test]
    fn sums_the_week_that_ended_and_diffs_against_the_one_before() {
        let store = Store::open_in_memory().unwrap();
        emit(&store, date(2026, 8, 26), 9, 2 * HOUR, "code.exe", "a");
        emit(&store, date(2026, 8, 17), 9, HOUR, "code.exe", "a");

        let digest = for_week_ending(&store, date(2026, 8, 31)).unwrap();
        assert_eq!(digest.total_ms, 2 * HOUR);
        assert_eq!(digest.delta_ms, HOUR);
        assert_eq!(digest.top.len(), 1);
        assert_eq!(digest.top[0].exe_path, "code.exe");
    }

    #[test]
    fn keeps_only_the_top_five_entities() {
        let store = Store::open_in_memory().unwrap();
        for i in 0..6 {
            emit(
                &store,
                date(2026, 8, 24),
                i,
                (i + 1) * 60_000,
                &format!("app{i}.exe"),
                "t",
            );
        }
        let digest = for_week_ending(&store, date(2026, 8, 31)).unwrap();
        assert_eq!(digest.top.len(), 5);
        assert_eq!(digest.top[0].exe_path, "app5.exe");
    }

    #[test]
    fn format_reports_hours_and_a_signed_delta() {
        let digest = Digest {
            total_ms: 90 * 60_000,
            delta_ms: -30 * 60_000,
            top: vec![],
        };
        let (title, body) = format(&digest);
        assert_eq!(title, "Toki — last week");
        assert!(body.contains("1h 30m total"));
        assert!(body.contains("-30m"));
    }
}
