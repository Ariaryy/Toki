<script lang="ts">
  import { onMount } from "svelte";
  import { formatClock, formatDuration } from "./format";
  import { hourLabel } from "./clock.svelte";
  import { DAY_MS, type Span } from "./day";
  import { buildBlocks, type Block } from "./timeline";

  let {
    spans,
    colors,
    rolledUp = false,
    highlight = null,
  }: {
    spans: Span[];
    /** `exePath -> series colour`, from `seriesByExe` over the same ranking the list below
        uses. An app the ranking does not reach falls through to the rest colour. */
    colors: Map<string, string>;
    rolledUp?: boolean;
    /** An app hovered in the list below: its blocks keep their colour, the rest go grey. */
    highlight?: string | null;
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

  /** Raw sessions run to hundreds on a busy day and turn the strip into a barcode, so it
      draws merged blocks instead. Per-session detail lives in the tooltip's breakdown. */
  let blocks = $derived(buildBlocks(spans));

  /** One shared tooltip rather than one per block: hundreds of nodes on a busy day, and a
      single element can be clamped inside the card. `x` is the block's centre in px,
      relative to `.wrap`. */
  let hover = $state<{ block: Block; x: number } | null>(null);
  let shown = $state(false);
  let hideTimer: ReturnType<typeof setTimeout> | undefined;
  let wrapWidth = $state(0);
  let tipWidth = $state(0);

  function show(e: PointerEvent, block: Block) {
    const seg = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const wrap = (e.currentTarget as HTMLElement).closest(".stage")!.getBoundingClientRect();
    // The card stays mounted between blocks and only its text and position change; `hover`
    // is never cleared, just hidden, so sliding along the strip does not replay the fade-in.
    clearTimeout(hideTimer);
    hover = { block, x: seg.left - wrap.left + seg.width / 2 };
    shown = true;
  }

  /** Delayed so that crossing the hairline between two blocks does not start a fade-out. */
  function hide() {
    clearTimeout(hideTimer);
    hideTimer = setTimeout(() => (shown = false), 60);
  }

  /** Centred on the block, kept inside the card so the ends of the day do not clip it. */
  let tipLeft = $derived(
    hover ? Math.min(Math.max(hover.x, tipWidth / 2), wrapWidth - tipWidth / 2) : 0,
  );

  const REST = "var(--series-rest)";
  /** Stands in for every app past the palette. `colors` has an entry for all of them, just
      all the same grey, so the test is the colour and not whether there is an entry:
      keyed by app, a day with a hundred apps would be a hundred legend entries. */
  const OTHER = "<other>";
  const keyOf = (exePath: string) => (colors.get(exePath) ?? REST) === REST ? OTHER : exePath;

  /** Legend: the apps actually drawn on the strip, biggest first. Time is what each app
      held inside the strip's slots, so it can differ a little from the list below, which
      counts every second. An app that never wins a slot has no colour on the strip and so
      no entry. */
  let legend = $derived.by(() => {
    const drawn = new Set(blocks.map((b) => keyOf(b.exePath)));
    const byKey = new Map<string, { name: string; ms: number; apps: Set<string> }>();
    for (const b of blocks) {
      for (const sh of b.shares) {
        const key = keyOf(sh.exePath);
        if (!drawn.has(key)) continue;
        const row = byKey.get(key) ?? { name: sh.appName, ms: 0, apps: new Set() };
        row.ms += sh.ms;
        row.apps.add(sh.exePath);
        byKey.set(key, row);
      }
    }
    return [...byKey.entries()]
      .map(([key, row]) => ({
        key,
        ms: row.ms,
        name: key === OTHER ? `${row.apps.size} other ${row.apps.size === 1 ? "app" : "apps"}` : row.name,
      }))
      // "Other" is a bucket, not an app, so it goes last whatever its time.
      .sort((a, b) => (a.key === OTHER ? 1 : b.key === OTHER ? -1 : b.ms - a.ms));
  });

  /** The legend entry under the pointer. Everything else on the strip dims. */
  let focus = $state<string | null>(null);

  /** A block is set back when something else is being pointed at, in the legend (by colour
      group) or in the list (by exact app: "Other" in the legend is many apps, the list
      row is one). */
  const dimmed = (exePath: string) =>
    (focus !== null && focus !== keyOf(exePath)) || (highlight !== null && highlight !== exePath);

  const active = $derived(focus !== null || highlight !== null);

  const colorOf = (exePath: string) => colors.get(exePath) ?? REST;

  /** Plain text for screen readers; the visual tooltip is not in the accessibility tree. */
  function tooltip(b: Block): string {
    const when = `${formatClock(b.startMs)}–${formatClock(b.endMs)}`;
    const held = b.audio ? ", held by audio" : "";
    const head = `${b.appName} · ${when} · ${formatDuration(b.coveredMs)}${held}`;
    if (b.shares.length < 2) return head;
    const rest = b.shares
      .slice(0, 4)
      .map((s) => `${s.appName} ${formatDuration(s.ms)}`)
      .join(", ");
    return `${head}
${rest}`;
  }

  /** Every third hour. Denser than this is unreadable at 44px tall, sparser loses noon. */
  const ticks = [0, 3, 6, 9, 12, 15, 18, 21, 24];

  let hasAudio = $derived(blocks.some((b) => b.audio));
</script>

<div class="wrap">
  <div class="stage" bind:clientWidth={wrapWidth}>
    {#if spans.length === 0}
      <div class="track empty">
        <span class="caption">
          {rolledUp
            ? "Older than six months. The totals are kept, the timing isn't."
            : "Nothing recorded on this day."}
        </span>
      </div>
    {:else}
      <div class="track">
        {#each blocks as b, i (i)}
          <div
            class="seg"
            class:audio={b.audio}
            class:dim={dimmed(b.exePath)}
            role="img"
            aria-label={tooltip(b)}
            onpointerenter={(e) => show(e, b)}
            onpointerleave={hide}
            style="
              left: {pct(b.startMs)}%;
              width: {pct(b.endMs - b.startMs)}%;
              background-color: {dimmed(b.exePath) ? 'var(--text-faint)' : colorOf(b.exePath)};
              --fill: {grown ? 1 : 0};
              transition-delay: {Math.round((b.startMs / DAY_MS) * 260)}ms;
            "
          ></div>
        {/each}
      </div>
    {/if}

    {#if hover}
      {@const b = hover.block}
      <div class="tip" class:shown style="left: {tipLeft}px; --arrow: {hover.x - tipLeft + tipWidth / 2}px" bind:clientWidth={tipWidth} aria-hidden="true">
        <div class="tip-head">
          <span class="dot" style="background: {colorOf(b.exePath)}"></span>
          <span class="tip-name">{b.appName}</span>
          <span class="tip-dur">{formatDuration(b.coveredMs)}</span>
        </div>
        <div class="tip-sub">
          {formatClock(b.startMs)}–{formatClock(b.endMs)}{b.audio ? " · held by audio" : ""}
        </div>
        {#if b.shares.length > 1}
          <div class="tip-rows">
            {#each b.shares.slice(0, 4) as sh (sh.exePath)}
              <div class="tip-row">
                <span class="dot" style="background: {colorOf(sh.exePath)}"></span>
                <span class="tip-name">{sh.appName}</span>
                <span class="tip-dur">{formatDuration(sh.ms)}</span>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  </div>

  <div class="axis" aria-hidden="true">
    {#each ticks as hour (hour)}
      <span style="left: {(hour / 24) * 100}%">{hourLabel(hour)}</span>
    {/each}
  </div>

  {#if legend.length > 0}
    <ul class="legend">
      {#each legend as item (item.key)}
        <li
          class:dim={active && focus !== item.key && highlight !== item.key}
          onpointerenter={() => (focus = item.key)}
          onpointerleave={() => (focus = null)}
        >
          <span
            class="dot"
            style="background: {item.key === OTHER ? REST : colorOf(item.key)}"
          ></span>
          <span class="legend-name">{item.name}</span>
          <span class="legend-ms">{formatDuration(item.ms)}</span>
        </li>
      {/each}
      {#if hasAudio}
        <li class="audio-key" title="Present, but not typing: audio playback held the session open">
          <span class="swatch"></span>
          <span class="legend-name">Held by audio</span>
        </li>
      {/if}
    </ul>
  {/if}
</div>

<style>
  .wrap {
    display: flex;
    flex-direction: column;
  }

  /* Sits above the strip, in the card's own space. `pointer-events: none` so it can never
     take the hover away from the block it describes. Fade only: `--disable-gpu` makes
     anything but opacity and transform relayout. */
  .stage {
    position: relative;
  }

  .tip {
    position: absolute;
    bottom: calc(100% + 10px);
    transform: translateX(-50%);
    z-index: 10;
    min-width: 168px;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
    background: var(--surface-raised);
    box-shadow: var(--shadow-popover);
    font-size: 12px;
    pointer-events: none;
    white-space: nowrap;
    opacity: 0;
    transition: opacity var(--dur-fast) var(--ease);
  }

  .tip.shown {
    opacity: 1;
  }

  /* Points at the block. `--arrow` is the block's centre measured from the box's left edge,
     so it stays on target when the box is clamped against the card's side. */
  .tip::after {
    content: "";
    position: absolute;
    top: 100%;
    left: var(--arrow);
    width: 8px;
    height: 8px;
    margin: -4px 0 0 -4px;
    transform: rotate(45deg);
    background: var(--surface-raised);
    border-right: 1px solid var(--border-strong);
    border-bottom: 1px solid var(--border-strong);
  }

  .tip-head,
  .tip-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .tip-head {
    color: var(--text-primary);
    font-weight: 500;
  }

  .tip-name {
    flex: 1;
  }

  .tip-dur {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .tip-sub {
    margin: 2px 0 0 calc(8px + var(--space-2));
    color: var(--text-muted);
  }

  .tip-rows {
    margin-top: var(--space-2);
    padding-top: var(--space-2);
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 2px;
    color: var(--text-secondary);
  }

  .dot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 2px;
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
    /* A hairline of track between neighbours, so two blocks of different apps never fuse. */
    border-inline: 0.5px solid var(--surface-raised);
    background-clip: padding-box;
    transform: scaleX(var(--fill));
    transform-origin: left center;
    transition:
      transform var(--dur-base) var(--ease),
      opacity var(--dur-fast) var(--ease);
  }

  /* Sessions the audio exemption held open. Hatched rather than given a second colour, so
     they read as "counted, differently" instead of as another app. The stripe is the page
     background, which is why it is the literal near-black rather than a token: the segment
     underneath is an opaque series hue in both themes. */
  /* Set back, not hidden: grey and faint so the app being pointed at stands alone while the
     shape of the day stays readable. The colour is swapped inline rather than filtered,
     because a filter is CPU-rasterised under --disable-gpu. */
  .seg.dim {
    opacity: 0.35;
  }

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
    flex-wrap: wrap;
    /* Centred under the strip, wrapped rows included, so a short legend does not hang off
       the left edge. */
    justify-content: center;
    gap: var(--space-1) var(--space-4);
    margin: var(--space-3) 0 0;
    padding: 0;
    list-style: none;
    font-size: 12px;
    color: var(--text-muted);
  }

  .legend li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    /* Room to hover without the pointer slipping between entries. */
    padding: 2px 0;
  }

  .legend li {
    transition: opacity var(--dur-fast) var(--ease);
  }

  .legend li.dim {
    opacity: 0.4;
  }

  .legend-name {
    color: var(--text-secondary);
  }

  .legend-ms {
    color: var(--text-faint);
    font-variant-numeric: tabular-nums;
  }

  /* Same size as the colour squares so it reads as one more key, not a footnote. */
  .audio-key .swatch {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: var(--text-muted);
    background-image: repeating-linear-gradient(
      45deg,
      rgba(8, 9, 10, 0.55) 0,
      rgba(8, 9, 10, 0.55) 1.5px,
      transparent 1.5px,
      transparent 3px
    );
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
