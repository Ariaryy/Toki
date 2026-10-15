import type { Total } from "./group";

/** Milliseconds in a local day. Day-relative offsets from the backend are all against this. */
export const DAY_MS = 86_400_000;

/**
 * One session, clamped to the day it is being drawn on. Offsets are milliseconds after local
 * midnight rather than timestamps, so a position on the strip is arithmetic — see the note
 * on `formatClock` in `format.ts`.
 */
export interface Span {
  startMs: number;
  endMs: number;
  exePath: string;
  appName: string;
  domain: string | null;
  /** Held open by the audio exemption: present, but not typed into. Drawn hatched. */
  audio: boolean;
}

/** The session in progress. No domain — the resolver runs every five minutes, so an open
    session's domain is genuinely unknown and guessing it would be a lie. */
export interface NowPlaying {
  exePath: string;
  appName: string;
  startMs: number;
  elapsedMs: number;
}

export interface Longest {
  ms: number;
  exePath: string;
  appName: string;
  startMs: number;
}

/** Everything Today draws, for one local date. Mirrors `DaySummary` in `toki-gui`'s lib.rs. */
export interface DaySummary {
  date: string;
  label: string;
  isToday: boolean;
  hasEarlier: boolean;
  totals: Total[];
  totalMs: number;
  deltaMs: number;
  spans: Span[];
  /** Past the six-month rollup: totals survive, timing does not. The timeline has to say so
      rather than draw an empty strip. */
  rolledUp: boolean;
  longest: Longest | null;
  appCount: number;
  browserCount: number;
  lateNightMs: number;
  now: NowPlaying | null;
}

/** The ISO date `days` before `iso`, in local terms. Used by the date stepper, which walks
    dates rather than timestamps so it never lands on a different day than it asked for. */
export function shiftDate(iso: string, days: number): string {
  const [y, m, d] = iso.split("-").map(Number);
  const shifted = new Date(y, m - 1, d + days);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${shifted.getFullYear()}-${pad(shifted.getMonth() + 1)}-${pad(shifted.getDate())}`;
}

/** Today in the same ISO form the backend uses. */
export function todayIso(): string {
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}
