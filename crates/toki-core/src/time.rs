//! Local-day arithmetic.
//!
//! Sessions are stored whole, in UTC, alongside the offset in force when they were written.
//! A "day" is therefore a window computed from that offset, and a session that straddles
//! midnight is clamped into each day it touches rather than being split on disk.

use chrono::{DateTime, NaiveDate};

use crate::event::Millis;

pub const DAY_MS: Millis = 24 * 60 * 60 * 1_000;

/// Which local date a timestamp falls on, given the offset in force at the time.
pub fn local_date(utc_ms: Millis, offset_min: i32) -> NaiveDate {
    DateTime::from_timestamp_millis(utc_ms + offset_min as i64 * 60_000)
        .expect("timestamp within chrono's range")
        .date_naive()
}

/// Half-open UTC bounds `[start, end)` of a local calendar day.
pub fn day_bounds(date: NaiveDate, offset_min: i32) -> (Millis, Millis) {
    let local_midnight = date
        .and_hms_opt(0, 0, 0)
        .expect("midnight is a valid time")
        .and_utc()
        .timestamp_millis();
    let start = local_midnight - offset_min as i64 * 60_000;
    (start, start + DAY_MS)
}

/// Milliseconds of `[start, end)` that fall inside `[window_start, window_end)`.
///
/// This is the clamp the daily queries apply, expressed once so the SQL and any in-process
/// aggregation cannot drift apart.
pub fn overlap_ms(start: Millis, end: Millis, window_start: Millis, window_end: Millis) -> Millis {
    (end.min(window_end) - start.max(window_start)).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IST: i32 = 330; // +05:30

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn a_local_day_is_a_full_day_offset_from_utc() {
        let (start, end) = day_bounds(date(2026, 8, 27), IST);
        assert_eq!(end - start, DAY_MS);
        // 2026-08-27 00:00 IST is 2026-08-26 18:30 UTC.
        assert_eq!(local_date(start, IST), date(2026, 8, 27));
        assert_eq!(local_date(start - 1, IST), date(2026, 8, 26));
        assert_eq!(local_date(end - 1, IST), date(2026, 8, 27));
    }

    #[test]
    fn a_session_across_midnight_is_credited_to_both_days() {
        let (d27_start, d27_end) = day_bounds(date(2026, 8, 27), IST);
        let (d28_start, d28_end) = day_bounds(date(2026, 8, 28), IST);

        // 23:40 to 00:20 local.
        let start = d27_end - 20 * 60 * 1_000;
        let end = d27_end + 20 * 60 * 1_000;

        assert_eq!(overlap_ms(start, end, d27_start, d27_end), 20 * 60 * 1_000);
        assert_eq!(overlap_ms(start, end, d28_start, d28_end), 20 * 60 * 1_000);
        assert_eq!(
            overlap_ms(start, end, d27_start, d27_end) + overlap_ms(start, end, d28_start, d28_end),
            end - start
        );
    }

    #[test]
    fn a_session_outside_the_window_contributes_nothing() {
        let (start, end) = day_bounds(date(2026, 8, 27), IST);
        assert_eq!(overlap_ms(start - 5_000, start, start, end), 0);
        assert_eq!(overlap_ms(end, end + 5_000, start, end), 0);
    }

    #[test]
    fn a_row_keeps_the_offset_it_was_written_with() {
        // The same instant belongs to different dates depending on the offset stored on
        // the row, which is why past IST days stay IST days after a move.
        let noon_ist = day_bounds(date(2026, 8, 27), IST).0 + 12 * 60 * 60 * 1_000;
        assert_eq!(local_date(noon_ist, IST), date(2026, 8, 27));
        assert_eq!(local_date(noon_ist, -480), date(2026, 8, 26)); // UTC-8
    }
}
