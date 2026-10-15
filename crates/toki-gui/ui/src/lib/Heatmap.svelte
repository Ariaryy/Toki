<script lang="ts">
  import { formatClock, formatDuration } from "./format";
  import { hourLabel } from "./clock.svelte";
  import type { DayHours, DayTotal } from "./week";

  let {
    days,
    hours,
    onpick,
  }: {
    days: DayTotal[];
    /** Same seven dates as `days`, matched by date rather than by position — the two come
        from different queries and a silent misalignment would put Friday's shape on
        Thursday's row. */
    hours: DayHours[];
    /** Opening a day. The row is already labelled with its date, so the affordance is the
        row itself rather than a link tacked onto it. */
    onpick: (date: string) => void;
  } = $props();

  let byDate = $derived(new Map(hours.map((h) => [h.date, h])));

  /** The scale is the busiest single hour of the week, so a quiet week is not stretched to
      look like a busy one — the cells answer "when", and their absolute weight is what makes
      two weeks comparable at a glance. */
  let peakHourMs = $derived(Math.max(1, ...hours.flatMap((h) => h.hours)));

  let maxDayMs = $derived(Math.max(1, ...days.map((d) => d.ms)));

  /** The single busiest day, and the one place this view spends the accent. Ties go to the
      earlier day; there is no meaningful way to pick between them and highlighting both
      would make the accent a category rather than a pointer. */
  let busiest = $derived.by(() => {
    const recorded = days.filter((d) => !d.isFuture && d.ms > 0);
    if (recorded.length === 0) return null;
    return recorded.reduce((best, d) => (d.ms > best.ms ? d : best)).date;
  });

  /**
   * Ink for one cell, as an opacity on the series hue.
   *
   * The curve is the point: a linear ramp made every working hour a near-identical mid-tone
   * and the whole grid read as a flat field. Raising the fraction to 1.45 pushes the middle
   * down so only genuinely heavy hours come out bright, and the 0.06 floor keeps an hour
   * with a few minutes in it visibly different from an empty one.
   */
  function ink(ms: number): number {
    if (ms === 0) return 0;
    return 0.06 + Math.pow(ms / peakHourMs, 1.45) * 0.94;
  }

  /** Every sixth hour. The grid is 24 columns at about 20px, so labelling more collides. */
  const ticks = [0, 6, 12, 18, 23];
</script>

<div class="heat">
  {#each days as d (d.date)}
    {@const detail = byDate.get(d.date)}
    <button
      class="hrow"
      class:busiest={d.date === busiest}
      class:future={d.isFuture}
      onclick={() => !d.isFuture && onpick(d.date)}
      disabled={d.isFuture}
      title={d.isFuture ? "" : `Open ${d.label}`}
    >
      <span class="hlabel">{d.label}</span>

      {#if detail?.rolledUp}
        <span class="rolled">Older than six months. The total is kept, the shape isn't.</span>
      {:else}
        <span class="cells">
          {#each detail?.hours ?? new Array(24).fill(0) as ms, hour (hour)}
            <span class="cell" title={ms === 0 ? "" : `${formatClock(hour * 3_600_000)} · ${formatDuration(ms)}`}>
              {#if ms > 0}<span class="fill" style="opacity: {ink(ms).toFixed(3)}"></span>{/if}
            </span>
          {/each}
        </span>
      {/if}

      <span class="hbar">
        {#if !d.isFuture}<span style="width: {(d.ms / maxDayMs) * 100}%"></span>{/if}
      </span>

      <span class="htotal">{d.isFuture ? "" : formatDuration(d.ms)}</span>
    </button>
  {/each}

  <div class="haxis" aria-hidden="true">
    <div></div>
    <div class="hticks">
      {#each ticks as hour (hour)}
        <span style="left: {((hour + 0.5) / 24) * 100}%">{hourLabel(hour)}</span>
      {/each}
    </div>
    <div></div>
    <div></div>
  </div>
</div>

<style>
  /* Day totals live at the end of each heatmap row rather than in a second card: the row is
     already "this day", and a separate bar chart would say it twice. */
  .hrow,
  .haxis {
    display: grid;
    grid-template-columns: 30px 1fr 104px 52px;
    align-items: center;
    gap: var(--space-3);
  }

  .hrow {
    width: 100%;
    padding: 1px var(--space-2);
    margin: 0 calc(var(--space-2) * -1);
    border: none;
    border-radius: var(--radius-control);
    background: none;
    font: inherit;
    color: inherit;
    text-align: left;
    cursor: pointer;
    transition: background-color var(--dur-fast) var(--ease);
  }

  .hrow:hover:not(:disabled) {
    background: var(--wash-1);
  }

  .hrow.future {
    cursor: default;
  }

  /* A day that has not happened has no total, and an empty track beside it reads as a day
     with nothing in it. The row keeps its space so the grid does not reflow at midnight. */
  .hrow.future .hbar {
    visibility: hidden;
  }

  .hlabel {
    font-size: 11px;
    color: var(--text-faint);
  }

  .hrow.busiest .hlabel,
  .hrow.busiest .htotal {
    color: var(--text-primary);
  }

  .cells {
    display: grid;
    grid-template-columns: repeat(24, 1fr);
    gap: 2px;
  }

  .cell {
    height: 22px;
    border-radius: var(--radius-badge);
    background: var(--surface-raised);
  }

  .fill {
    display: block;
    height: 100%;
    border-radius: var(--radius-badge);
    background: var(--series-1);
  }

  .rolled {
    height: 22px;
    display: flex;
    align-items: center;
    font-size: 11px;
    color: var(--text-faint);
  }

  .hbar {
    display: block;
    height: 6px;
    border-radius: var(--radius-badge);
    background: var(--wash-2);
    overflow: hidden;
  }

  /* Neutral ink, deliberately dimmer than the cells it annotates: the row's shape is the
     figure here and the bar is its footnote. */
  .hbar span {
    display: block;
    height: 100%;
    border-radius: var(--radius-badge);
    background: color-mix(in srgb, var(--text-primary) 34%, transparent);
  }

  .hrow.busiest .hbar span {
    background: var(--accent);
  }

  .htotal {
    font-size: 12px;
    text-align: right;
    color: var(--text-muted);
  }

  .haxis {
    margin-top: var(--space-2);
  }

  .hticks {
    position: relative;
    height: 14px;
    font-size: 11px;
    color: var(--text-faint);
  }

  /* Centred under their column rather than at the grid line, because a cell is an hour-long
     bucket and the label names the bucket, not its left edge. */
  .hticks span {
    position: absolute;
    transform: translateX(-50%);
  }
</style>
