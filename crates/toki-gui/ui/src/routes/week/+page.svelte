<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { goto } from "$app/navigation";
  import { formatDuration, formatDelta, formatDateRange } from "$lib/format";
  import { group, type AppGroup } from "$lib/group";
  import type { DayHours, DayTotal, WeekSummary } from "$lib/week";
  import Heatmap from "$lib/Heatmap.svelte";
  import Metrics from "$lib/Metrics.svelte";
  import RankedList from "$lib/RankedList.svelte";

  /** 0 is the current week, 1 last week, and so on. */
  let weeksAgo = $state(0);
  let days = $state<DayTotal[]>([]);
  let hours = $state<DayHours[]>([]);
  let groups = $state<AppGroup[]>([]);
  let totalMs = $state(0);
  let deltaMs = $state(0);
  let averageMs = $state(0);
  let lateNightMs = $state(0);
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
      hours = summary.hours;
      groups = group(summary.top);
      totalMs = summary.totalMs;
      deltaMs = summary.deltaMs;
      averageMs = summary.averageMs;
      lateNightMs = summary.lateNightMs;
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

  /** A day in the heatmap opens on Today. Pushed rather than replaced, so Back returns to
      the week it was opened from. */
  function openDay(date: string) {
    void goto(`/?date=${date}`);
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

  let busiest = $derived.by(() => {
    const recorded = days.filter((d) => !d.isFuture && d.ms > 0);
    if (recorded.length === 0) return null;
    return recorded.reduce((best, d) => (d.ms > best.ms ? d : best));
  });

  let metrics = $derived([
    { value: formatDuration(totalMs), sub: formatDelta(deltaMs, comparison) },
    { value: formatDuration(averageMs), sub: "daily average" },
    {
      value: busiest ? busiest.label : "—",
      sub: busiest ? `busiest — ${formatDuration(busiest.ms)}` : "no busiest day yet",
    },
    { value: lateNightMs === 0 ? "—" : formatDuration(lateNightMs), sub: "after midnight" },
  ]);
</script>

<svelte:window onkeydown={onKeydown} />

<header class="page-head">
  <div>
    <div class="eyebrow">{title}</div>
    <div class="display">{days.length === 0 ? "—" : formatDuration(totalMs)}</div>
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
  <Metrics items={metrics} />

  <section class="card">
    <div class="card-head">
      <h2 class="heading">When</h2>
      <div class="legend" aria-hidden="true">
        less
        <i class="lo"></i><i style="opacity: 0.25"></i><i style="opacity: 0.5"></i><i
          style="opacity: 0.75"
        ></i><i></i>
        more
      </div>
    </div>

    <Heatmap {days} {hours} onpick={openDay} />
  </section>
{/if}

<RankedList {groups} loading={loading && days.length === 0} {error} {emptyMessage} />

<style>
  .card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-3);
  }

  .legend {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    font-size: 11px;
    color: var(--text-faint);
  }

  .legend i {
    width: 12px;
    height: 12px;
    border-radius: var(--radius-badge);
    background: var(--series-1);
  }

  .legend .lo {
    background: var(--surface-raised);
  }
</style>
