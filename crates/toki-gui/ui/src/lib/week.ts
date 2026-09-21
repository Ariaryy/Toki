import type { Total } from "./group";

/** One day's total inside a week. Days after today are present with `ms: 0` rather than
    omitted, so the strip always has seven of them. */
export interface DayTotal {
  date: string;
  label: string;
  ms: number;
  isToday: boolean;
  isFuture: boolean;
}

/** 24 hourly totals for one local date. A day past the rollup cutoff carries zeroes and
    `rolledUp: true` — its total survives in `DayTotal`, its shape does not. */
export interface DayHours {
  date: string;
  hours: number[];
  rolledUp: boolean;
}

/** Mirrors `WeekSummary` in `toki-gui`'s lib.rs. */
export interface WeekSummary {
  days: DayTotal[];
  top: Total[];
  totalMs: number;
  deltaMs: number;
  weeksAgo: number;
  hasEarlier: boolean;
  hours: DayHours[];
  averageMs: number;
  lateNightMs: number;
}
