/**
 * Which categorical hue a row gets, by its rank.
 *
 * Shared so the ranked list, the timeline and the Apps table agree: a colour here means
 * "the Nth biggest thing on this screen", and two views disagreeing about N would make the
 * timeline unreadable against the list beneath it.
 *
 * Past the seventh the palette stops rather than generating an eighth hue — see the note on
 * `--series-*` in `app.css`.
 */
export function seriesVar(index: number): string {
  return index < 7 ? `var(--series-${index + 1})` : "var(--series-rest)";
}

/** Rank-ordered `exePath -> colour`, for views that draw the same apps in a different shape. */
export function seriesByExe(ordered: { exePath: string }[]): Map<string, string> {
  return new Map(ordered.map((row, i) => [row.exePath, seriesVar(i)]));
}
