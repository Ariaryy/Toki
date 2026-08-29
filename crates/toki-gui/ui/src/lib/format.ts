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
 */
export function formatDelta(ms: number): string {
  if (Math.abs(ms) < 60_000) return "±0 vs last week";
  const arrow = ms > 0 ? "↑" : "↓";
  return `${arrow} ${formatDuration(Math.abs(ms))} vs last week`;
}
