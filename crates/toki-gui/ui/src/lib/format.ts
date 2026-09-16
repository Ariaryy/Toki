export function formatDuration(ms: number): string {
  const totalMinutes = Math.round(ms / 60_000);
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  if (hours === 0 && minutes === 0) return "<1m";
  if (hours === 0) return `${minutes}m`;
  if (minutes === 0) return `${hours}h`;
  return `${hours}h ${minutes}m`;
}

/**
 * A week-over-week (or any signed) delta as an arrow plus a magnitude — never a color,
 * per DESIGN.md §9: "more YouTube this week is not an error condition."
 *
 * `against` names what the comparison is with. It defaults to "last week", which is right
 * while you are looking at the current one; paged back, the week before the one on screen
 * is not last week.
 */
export function formatDelta(ms: number, against = "last week"): string {
  if (Math.abs(ms) < 60_000) return `±0 vs ${against}`;
  const arrow = ms > 0 ? "↑" : "↓";
  return `${arrow} ${formatDuration(Math.abs(ms))} vs ${against}`;
}

/**
 * "8 – 14 Sep", or "29 Dec – 4 Jan" across a month boundary. The year is shown only when
 * the week is not in the current one, which is almost never and would otherwise be noise.
 */
export function formatDateRange(startIso: string, endIso: string): string {
  const start = new Date(startIso);
  const end = new Date(endIso);
  const thisYear = new Date().getFullYear();
  const sameMonth = start.getMonth() === end.getMonth();
  const month = (d: Date) => d.toLocaleString(undefined, { month: "short" });
  const head = sameMonth ? `${start.getDate()}` : `${start.getDate()} ${month(start)}`;
  const year = end.getFullYear() === thisYear ? "" : ` ${end.getFullYear()}`;
  return `${head} – ${end.getDate()} ${month(end)}${year}`;
}

/**
 * A time of day from milliseconds after local midnight — "08:10", or "24:00" for the
 * instant a session clamped to the end of its day lands on.
 *
 * The backend hands out day offsets rather than timestamps precisely so this stays
 * arithmetic: a timeline tick is a position on a 24-hour strip, not a calendar instant, and
 * routing it through `Date` would reintroduce the timezone the offsets exist to remove.
 */
export function formatClock(msAfterMidnight: number): string {
  const total = Math.round(msAfterMidnight / 60_000);
  const hours = Math.floor(total / 60);
  const minutes = total % 60;
  return `${String(hours).padStart(2, "0")}:${String(minutes).padStart(2, "0")}`;
}

/** "31.4 MB". Decimal units, because that is what Explorer's properties dialog shows. */
export function formatBytes(bytes: number): string {
  if (bytes < 1000) return `${bytes} B`;
  const units = ["kB", "MB", "GB"];
  let value = bytes / 1000;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}

/** "6s ago", "4m ago", "2h ago" — coarse on purpose; the question is only ever whether the
    daemon is still writing, not exactly when it last did. */
export function formatAgo(ms: number): string {
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  return `${Math.round(minutes / 60)}h ago`;
}

/**
 * An ISO local date as "27 Aug 2026". Parsed by hand rather than through `new Date(iso)`,
 * which reads a bare `YYYY-MM-DD` as UTC and lands on the day before west of Greenwich.
 */
export function formatDay(iso: string): string {
  const months = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun",
    "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
  ];
  const [y, m, d] = iso.split("-").map(Number);
  return `${d} ${months[m - 1]} ${y}`;
}

/** "18,402" — thousands separators, for counts big enough that the eye needs the grouping. */
export function formatCount(n: number): string {
  return n.toLocaleString();
}
