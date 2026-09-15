<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { formatDuration, formatDelta, formatDateRange } from "$lib/format";
  import { group, type AppGroup, type Total } from "$lib/group";
  import RankedList from "$lib/RankedList.svelte";

  interface DayTotal {
    date: string;
    label: string;
    ms: number;
    isToday: boolean;
    isFuture: boolean;
  }

  interface WeekSummary {
    days: DayTotal[];
    top: Total[];
    totalMs: number;
    deltaMs: number;
    weeksAgo: number;
    hasEarlier: boolean;
  }

  /** 0 is the current week, 1 last week, and so on. */
  let weeksAgo = $state(0);
  let days = $state<DayTotal[]>([]);
  let groups = $state<AppGroup[]>([]);
  let totalMs = $state(0);
  let deltaMs = $state(0);
  let hasEarlier = $state(false);
  let loading = $state(true);
  let error = $state<string | null>(null);

  // Refetches whenever the page changes. The previous week's figures stay on screen while
  // the next ones load rather than being cleared: the query is a handful of indexed scans
  // against a local file, so blanking the page would only ever show a flash of nothing.
  $effect(() => {
    void load(weeksAgo);
  });

  async function load(week: number) {
    loading = true;
    try {
      const summary = await invoke<WeekSummary>("week_totals", { weeksAgo: week });
      // Paging faster than the queries return would otherwise let an earlier one land last.
      if (summary.weeksAgo !== weeksAgo) return;
      days = summary.days;
      groups = group(summary.top);
      totalMs = summary.totalMs;
      deltaMs = summary.deltaMs;
      hasEarlier = summary.hasEarlier;
      error = null;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  function goBack() {
    if (hasEarlier) weeksAgo += 1;
  }

  function goForward() {
    if (weeksAgo > 0) weeksAgo -= 1;
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "ArrowLeft") goBack();
    else if (e.key === "ArrowRight") goForward();
    else return;
    e.preventDefault();
  }

  let title = $derived.by(() => {
    if (weeksAgo === 0) return "This week";
    if (weeksAgo === 1) return "Last week";
    if (days.length < 7) return `${weeksAgo} weeks ago`;
    return formatDateRange(days[0].date, days[6].date);
  });

  /** The stepper's own label. Redundant with the eyebrow only for weeks 0 and 1, where the
      eyebrow says "This week" and the reader may still want the dates. */
  let range = $derived(days.length === 7 ? formatDateRange(days[0].date, days[6].date) : "");

  // "vs last week" is only true from the current week. From any other, the comparison is
  // with the week before the one on screen.
  let comparison = $derived(weeksAgo === 0 ? "last week" : "the week before");

  let emptyMessage = $derived(
    weeksAgo === 0 ? "Nothing recorded yet this week." : "Nothing recorded that week.",
  );

  let maxDayMs = $derived(Math.max(1, ...days.map((d) => d.ms)));
</script>

<svelte:window onkeydown={onKeydown} />

<header class="page-head">
  <div>
    <div class="eyebrow">{title}</div>
    <div class="display">{days.length === 0 ? "—" : formatDuration(totalMs)}</div>
    {#if days.length > 0 && !error}
      <div class="delta">{formatDelta(deltaMs, comparison)}</div>
    {/if}
  </div>

  <div class="stepper">
    {#if range}<span class="caption">{range}</span>{/if}
    <button
      class="step"
      onclick={goBack}
      disabled={!hasEarlier}
      aria-label="Previous week"
      title="Previous week (left arrow)">‹</button
    >
    <button
      class="step"
      onclick={goForward}
      disabled={weeksAgo === 0}
      aria-label="Next week"
      title="Next week (right arrow)">›</button
    >
  </div>
</header>

{#if !error && days.length > 0}
  <section class="card strip" aria-label="Daily totals">
    {#each days as d (d.date)}
      <div class="day">
        <div class="bar-track">
          {#if !d.isFuture}
            <div
              class="bar"
              class:today={d.isToday}
              style="height: {(d.ms / maxDayMs) * 100}%"
            ></div>
          {/if}
        </div>
        <div class="label" class:today={d.isToday}>{d.label}</div>
      </div>
    {/each}
  </section>
{/if}

<RankedList {groups} loading={loading && days.length === 0} {error} {emptyMessage} />

<style>
  .page-head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--space-4);
  }

  .delta {
    margin-top: 2px;
    font-size: 13px;
  }

  .stepper {
    display: flex;
    align-items: center;
    gap: 2px;
  }

  .stepper .caption {
    margin-right: var(--space-2);
  }

  .step {
    width: 24px;
    height: 24px;
    display: grid;
    place-items: center;
    padding: 0;
    border: none;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--text-muted);
    font: inherit;
    font-size: 14px;
    line-height: 1;
    cursor: pointer;
    transition: background-color var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease);
  }

  .step:hover:not(:disabled) {
    background: var(--wash-2);
    color: var(--text-primary);
  }

  .step:disabled {
    /* Kept in place rather than hidden: a control that vanishes at the ends of the range
       moves the label out from under the pointer mid-click. */
    opacity: 0.3;
    cursor: default;
  }

  .strip {
    display: flex;
    justify-content: space-between;
    align-items: stretch;
    gap: var(--space-2);
    height: 132px;
  }

  .day {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
  }

  .bar-track {
    flex: 1;
    width: 56px;
    align-self: center;
    display: flex;
    align-items: flex-end;
    justify-content: center;
    border-radius: var(--radius-control);
    background: var(--surface-raised);
  }

  .bar {
    width: 100%;
    min-height: 2px;
    border-radius: 3px 3px 0 0;
    /* Neutral ink by default. The accent is spent at most once per view, and on this one
       the thing worth spending it on is today — see UI-REVAMP.md §2. */
    background: color-mix(in srgb, var(--text-primary) 34%, transparent);
  }

  .bar.today {
    background: var(--accent);
  }

  .label {
    font-size: 11px;
    color: var(--text-faint);
  }

  .label.today {
    color: var(--text-primary);
    font-weight: 510;
  }
</style>
