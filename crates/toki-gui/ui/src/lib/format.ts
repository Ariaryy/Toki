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
