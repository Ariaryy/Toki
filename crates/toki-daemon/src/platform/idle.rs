//! When the keyboard and mouse were last touched.
//!
//! `GetLastInputInfo` reports a tick count, not a wall clock, and the tick count wraps every
//! 49.7 days. Both are converted here so the rest of the program only ever sees Unix
//! milliseconds.

use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

use toki_core::event::Millis;

/// Wall-clock time of the last keyboard or mouse input.
///
/// Returns `now` when the query fails, which reads as "input just happened". Failing the
/// other way would let a broken call silently close every session at the idle threshold.
pub fn last_input_at(now: Millis) -> Millis {
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    if unsafe { GetLastInputInfo(&mut info) }.as_bool() {
        // `dwTime` is a 32-bit tick count and wraps; `GetTickCount64` does not. Truncating
        // the 64-bit value to 32 bits puts both in the same wrapped space, so the
        // subtraction is correct across a wrap.
        let now_ticks = unsafe { GetTickCount64() } as u32;
        let idle_ms = now_ticks.wrapping_sub(info.dwTime) as i64;
        now - idle_ms
    } else {
        now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_input_is_never_in_the_future_and_never_absurdly_old() {
        let now = 1_787_788_800_000_i64;
        let at = last_input_at(now);
        assert!(at <= now, "input cannot be reported after now");
        // A wrap handled wrongly shows up as a gap of weeks. Under a day is the sane band
        // for a machine that is running this test.
        assert!(now - at < 86_400_000, "idle span looks like a tick wrap");
    }
}
