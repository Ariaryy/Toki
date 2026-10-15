/**
 * How a time of day is written, 24-hour or 12-hour, and the one place that writes it.
 *
 * Every clock on screen goes through `clockText` or `hourLabel`, so the setting changes all
 * of them at once and no view carries its own idea of what "14:00" looks like. The
 * preference is reactive: a view that formats inside its markup or a `$derived` re-renders
 * when it changes, with no reload.
 *
 * Purely a display preference, so like the theme it lives in the webview's own storage and
 * never goes near the daemon.
 */

export type ClockPref = "24h" | "12h";

const KEY = "toki:clock";

function load(): ClockPref {
  try {
    return localStorage.getItem(KEY) === "12h" ? "12h" : "24h";
  } catch {
    return "24h";
  }
}

class Clock {
  pref = $state<ClockPref>(load());

  set(pref: ClockPref): void {
    this.pref = pref;
    try {
      localStorage.setItem(KEY, pref);
    } catch {
      // Only costs the choice surviving a restart.
    }
  }
}

export const clock = new Clock();

/** A non-breaking space, so "8:10 PM" never wraps between the time and its period. */
const NBSP = "\u00a0";

const pad = (n: number) => String(n).padStart(2, "0");

/**
 * A time of day from whole minutes after local midnight. 24-hour is "08:10", and "24:00" for
 * the instant a session clamped to the end of its day lands on; 12-hour is "8:10 PM", with
 * that same instant read as "12:00 AM", which is what midnight is called.
 */
export function clockText(minutesAfterMidnight: number, pref: ClockPref = clock.pref): string {
  const hours = Math.floor(minutesAfterMidnight / 60);
  const minutes = minutesAfterMidnight % 60;
  if (pref === "24h") return `${pad(hours)}:${pad(minutes)}`;
  const h = hours % 24;
  return `${h % 12 || 12}:${pad(minutes)}${NBSP}${h < 12 ? "AM" : "PM"}`;
}

/**
 * A whole hour for an axis: "09", or "9 AM". Hour 24, the right-hand end of a day strip, is
 * "24" in one and "12 AM" in the other.
 */
export function hourLabel(hour: number, pref: ClockPref = clock.pref): string {
  if (pref === "24h") return pad(hour);
  const h = hour % 24;
  return `${h % 12 || 12}${NBSP}${h < 12 ? "AM" : "PM"}`;
}
