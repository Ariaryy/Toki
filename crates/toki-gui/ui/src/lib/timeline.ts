import { DAY_MS, type Span } from "./day";

/** Width of one slot. 10 minutes is ~7px on the 1120px window: wide enough to read as a
    block, narrow enough that a quiet day's long sessions still look like themselves. */
export const BUCKET_MS = 10 * 60_000;

/** Same-app sessions closer than this are one visit: a tab switch or a quick alt-tab back
    should not split a block in two. */
const MERGE_GAP_MS = 90_000;

/** A slot needs this much recorded time to be drawn, so a stray ten-second blip does not
    paint ten minutes. */
const MIN_COVER = 0.25;

export interface AppShare {
  exePath: string;
  appName: string;
  ms: number;
}

/** One drawn block: consecutive slots whose dominant app is the same. */
export interface Block {
  startMs: number;
  endMs: number;
  exePath: string;
  appName: string;
  /** Recorded time inside the block — less than its length when the slots were part-idle. */
  coveredMs: number;
  /** Most of the dominant app's time here was held open by audio. */
  audio: boolean;
  /** Everything in the block by time, largest first, for the tooltip. */
  shares: AppShare[];
}

/** Join same-app spans that touch or nearly touch. Input is not assumed sorted. */
function coalesce(spans: Span[]): Span[] {
  const sorted = [...spans].sort((a, b) => a.startMs - b.startMs);
  const out: Span[] = [];
  const last = new Map<string, number>();
  for (const s of sorted) {
    const i = last.get(s.exePath);
    const prev = i === undefined ? undefined : out[i];
    if (prev && s.startMs - prev.endMs <= MERGE_GAP_MS && prev.audio === s.audio) {
      prev.endMs = Math.max(prev.endMs, s.endMs);
    } else {
      last.set(s.exePath, out.push({ ...s }) - 1);
    }
  }
  return out;
}

/**
 * Spans -> blocks that stay readable however busy the day was. Merge same-app visits, drop
 * each into 10-minute slots, colour a slot by the app that held most of it, then join
 * neighbouring slots of one app. Slot boundaries are fixed, so the picture does not shift
 * when a span is added.
 */
export function buildBlocks(spans: Span[]): Block[] {
  const slots = Math.ceil(DAY_MS / BUCKET_MS);
  const per: Map<string, { name: string; ms: number; audioMs: number }>[] = Array.from(
    { length: slots },
    () => new Map(),
  );

  for (const s of coalesce(spans)) {
    const first = Math.max(0, Math.floor(s.startMs / BUCKET_MS));
    const last = Math.min(slots - 1, Math.floor((s.endMs - 1) / BUCKET_MS));
    for (let i = first; i <= last; i++) {
      const ms = Math.min(s.endMs, (i + 1) * BUCKET_MS) - Math.max(s.startMs, i * BUCKET_MS);
      if (ms <= 0) continue;
      const cell = per[i].get(s.exePath) ?? { name: s.appName, ms: 0, audioMs: 0 };
      cell.ms += ms;
      if (s.audio) cell.audioMs += ms;
      per[i].set(s.exePath, cell);
    }
  }

  const blocks: Block[] = [];
  let open: Block | null = null;
  const totals = new Map<string, AppShare>();
  let domMs = 0;
  let domAudioMs = 0;

  const close = () => {
    if (!open) return;
    open.audio = domAudioMs > domMs / 2;
    domMs = domAudioMs = 0;
    open.shares = [...totals.values()].sort((a, b) => b.ms - a.ms);
    blocks.push(open);
    open = null;
    totals.clear();
  };

  for (let i = 0; i < slots; i++) {
    let top: [string, { name: string; ms: number; audioMs: number }] | null = null;
    let covered = 0;
    for (const entry of per[i]) {
      covered += entry[1].ms;
      if (!top || entry[1].ms > top[1].ms) top = entry;
    }
    if (!top || covered < BUCKET_MS * MIN_COVER) {
      close();
      continue;
    }
    const [exePath, cell] = top;
    if (open && open.exePath !== exePath) close();
    const start = i * BUCKET_MS;
    if (!open) {
      open = {
        startMs: start,
        endMs: start,
        exePath,
        appName: cell.name,
        coveredMs: 0,
        audio: false,
        shares: [],
      };
    }
    open.endMs = start + BUCKET_MS;
    open.coveredMs += covered;
    domMs += cell.ms;
    domAudioMs += cell.audioMs;
    for (const [exe, c] of per[i]) {
      const t = totals.get(exe) ?? { exePath: exe, appName: c.name, ms: 0 };
      t.ms += c.ms;
      totals.set(exe, t);
    }
  }
  close();
  return blocks;
}
