<script lang="ts">
  import { onMount } from "svelte";
  import { formatClock, formatDuration } from "./format";
  import { DAY_MS, type Span } from "./day";

  let {
    spans,
    colors,
    rolledUp = false,
  }: {
    spans: Span[];
    /** `exePath -> series colour`, from `seriesByExe` over the same ranking the list below
        uses. An app the ranking does not reach falls through to the rest colour. */
    colors: Map<string, string>;
    rolledUp?: boolean;
  } = $props();

  /**
   * Segments grow from their left edge, in the order the day happened: the delay is taken
   * from where a span sits on the strip, so the day fills in as a sweep rather than all at
   * once. `scaleX` only — `--disable-gpu` is set, so transform and opacity are the two
   * things that do not relayout every frame (UI-REVAMP.md §7).
   */
  let grown = $state(false);

  onMount(() => {
    const id = requestAnimationFrame(() => (grown = true));
    return () => cancelAnimationFrame(id);
  });

  const pct = (ms: number) => (ms / DAY_MS) * 100;

  /** A one-minute session is 0.07% of a day, which rounds to nothing at this width. The
      floor keeps it a visible tick: the strip's job is to show when, and a span that
      disappears reads as time that was never recorded. */
  const MIN_WIDTH_PCT = 0.14;

  function tooltip(s: Span): string {
    const when = `${formatClock(s.startMs)}–${formatClock(s.endMs)}`;
    const held = s.audio ? ", held by audio" : "";
    return `${s.appName} · ${when} · ${formatDuration(s.endMs - s.startMs)}${held}`;
  }

  /** Every third hour. Denser than this is unreadable at 44px tall, sparser loses noon. */
  const ticks = [0, 3, 6, 9, 12, 15, 18, 21, 24];

  let hasAudio = $derived(spans.some((s) => s.audio));
</script>

<div class="wrap">
  {#if spans.length === 0}
    <div class="track empty">
      <span class="caption">
        {rolledUp
          ? "Older than six months — the totals are kept, the timing isn't."
          : "Nothing recorded on this day."}
      </span>
    </div>
  {:else}
    <div class="track">
      {#each spans as s, i (i)}
        <div
          class="seg"
          class:audio={s.audio}
          title={tooltip(s)}
          style="
            left: {pct(s.startMs)}%;
            width: {Math.max(pct(s.endMs - s.startMs), MIN_WIDTH_PCT)}%;
            background: {colors.get(s.exePath) ?? 'var(--series-rest)'};
            --fill: {grown ? 1 : 0};
            transition-delay: {Math.round((s.startMs / DAY_MS) * 260)}ms;
          "
        ></div>
      {/each}
    </div>
  {/if}

  <div class="axis" aria-hidden="true">
    {#each ticks as hour (hour)}
      <span style="left: {(hour / 24) * 100}%">{String(hour).padStart(2, "0")}</span>
    {/each}
  </div>

  {#if hasAudio}
    <div class="legend">
      <span class="swatch"></span>
      held by audio — present, not typing
    </div>
  {/if}
</div>

<style>
  .wrap {
    display: flex;
    flex-direction: column;
  }

  /* Absolutely-positioned segments rather than SVG: the geometry is one axis, and
     percentage lefts keep the rounded ends crisp at any width. */
  .track {
    position: relative;
    height: 44px;
    border-radius: var(--radius-control);
    background: var(--surface-raised);
    overflow: hidden;
  }

  .track.empty {
    display: grid;
    place-items: center;
  }

  .seg {
    position: absolute;
    top: 0;
    bottom: 0;
    border-radius: var(--radius-badge);
    transform: scaleX(var(--fill));
    transform-origin: left center;
    transition: transform var(--dur-base) var(--ease);
  }

  /* Sessions the audio exemption held open. Hatched rather than given a second colour, so
     they read as "counted, differently" instead of as another app. The stripe is the page
     background, which is why it is the literal near-black rather than a token: the segment
     underneath is an opaque series hue in both themes. */
  .seg.audio {
    background-image: repeating-linear-gradient(
      45deg,
      rgba(8, 9, 10, 0.55) 0,
      rgba(8, 9, 10, 0.55) 2px,
      transparent 2px,
      transparent 5px
    );
  }

  .axis {
    position: relative;
    height: 16px;
    margin-top: var(--space-2);
    font-size: 11px;
    color: var(--text-faint);
  }

  .axis span {
    position: absolute;
    transform: translateX(-50%);
  }

  /* The ends would otherwise hang off the strip they label. */
  .axis span:first-child {
    transform: none;
  }

  .axis span:last-child {
    transform: translateX(-100%);
  }

  .legend {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-3);
    font-size: 11px;
    color: var(--text-faint);
  }

  .swatch {
    width: 18px;
    height: 8px;
    border-radius: var(--radius-badge);
    background: var(--series-1);
    background-image: repeating-linear-gradient(
      45deg,
      rgba(8, 9, 10, 0.55) 0,
      rgba(8, 9, 10, 0.55) 2px,
      transparent 2px,
      transparent 5px
    );
  }
</style>
