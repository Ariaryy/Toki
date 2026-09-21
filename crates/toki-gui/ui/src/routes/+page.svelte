<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { formatClock, formatDelta, formatDuration } from "$lib/format";
  import { group, type AppGroup } from "$lib/group";
  import { seriesByExe } from "$lib/series";
  import { shiftDate, todayIso, type DaySummary } from "$lib/day";
  import Metrics from "$lib/Metrics.svelte";
  import RankedList from "$lib/RankedList.svelte";
  import Timeline from "$lib/Timeline.svelte";

  /**
   * The local date on screen, held in the URL rather than in a variable, so the Week
   * heatmap can open a day by navigating to it and Back returns to the week.
   *
   * Walked by whole days rather than by timestamps: stepping across the hour of a DST
   * change still lands on the day before.
   */
  let date = $derived(page.url.searchParams.get("date") ?? todayIso());

  /** Replaced, not pushed: stepping through a fortnight should not bury the Week view a
      dozen entries deep in history. */
  function show(iso: string) {
    void goto(`/?date=${iso}`, { replaceState: true, noScroll: true, keepFocus: true });
  }
  let summary = $state<DaySummary | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);

  // Refetches whenever the date changes. The previous day stays on screen while the next
  // loads rather than being cleared: these are indexed scans against a local file, so
  // blanking the page would only ever show a flash of nothing.
  $effect(() => {
    void load(date);
  });

  async function load(iso: string) {
    loading = true;
    try {
      const next = await invoke<DaySummary>("day_summary", { date: iso });
      // Stepping faster than the queries return would otherwise let an earlier one land last.
      if (next.date !== date) return;
      summary = next;
      error = null;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  function goBack() {
    if (summary?.hasEarlier) show(shiftDate(date, -1));
  }

  function goForward() {
    if (summary && !summary.isToday) show(shiftDate(date, 1));
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "ArrowLeft") goBack();
    else if (e.key === "ArrowRight") goForward();
    else return;
    e.preventDefault();
  }

  /**
   * The open session keeps counting between fetches. Nothing is re-queried for this — the
   * daemon writes the session's end only when it closes, so the elapsed figure is arithmetic
   * on the start we already have, and a poll would cost a database open to learn nothing.
   */
  let sinceLoad = $state(0);

  $effect(() => {
    if (!summary?.now) return;
    const loadedAt = Date.now();
    sinceLoad = 0;
    const id = setInterval(() => (sinceLoad = Date.now() - loadedAt), 15_000);
    return () => clearInterval(id);
  });

  let groups = $derived<AppGroup[]>(summary ? group(summary.totals) : []);
  let colors = $derived(seriesByExe(groups));

  let comparison = $derived(summary?.isToday ? "yesterday" : "the day before");

  /** "Today", "Yesterday", or the date itself. The stepper repeats the date underneath only
      when the title has not already said it. */
  let title = $derived.by(() => {
    if (!summary) return "Today";
    if (summary.isToday) return "Today";
    if (summary.date === shiftDate(todayIso(), -1)) return "Yesterday";
    return summary.label;
  });

  /** First start to last end, which is not the same as "was at the computer for" — the gaps
      between are exactly what the strip below is for. */
  let extent = $derived.by(() => {
    const spans = summary?.spans ?? [];
    if (spans.length === 0) return "";
    const from = Math.min(...spans.map((s) => s.startMs));
    const to = Math.max(...spans.map((s) => s.endMs));
    return `${formatClock(from)} – ${formatClock(to)}`;
  });

  let metrics = $derived.by(() => {
    if (!summary) return [];
    const { longest, browserCount, appCount, lateNightMs } = summary;
    return [
      {
        value: formatDuration(summary.totalMs),
        sub: formatDelta(summary.deltaMs, comparison),
      },
      {
        value: longest ? formatDuration(longest.ms) : "—",
        sub: longest
          ? `Longest — ${longest.appName}, ${formatClock(longest.startMs)}`
          : "No unbroken stretch",
      },
      {
        value: String(appCount),
        sub: browserCount === 1 ? "apps, 1 browser" : `apps, ${browserCount} browsers`,
      },
      {
        value: lateNightMs === 0 ? "—" : formatDuration(lateNightMs),
        sub: "after midnight",
      },
    ];
  });
</script>

<svelte:window onkeydown={onKeydown} />

<header class="page-head">
  <div>
    <div class="eyebrow">{title}</div>
    <div class="display">{summary ? formatDuration(summary.totalMs) : "—"}</div>
  </div>

  <div class="stepper">
    {#if summary && title !== summary.label}<span class="caption">{summary.label}</span>{/if}
    <button
      class="step"
      onclick={goBack}
      disabled={!summary?.hasEarlier}
      aria-label="Previous day"
      title="Previous day (left arrow)">‹</button
    >
    <button
      class="step"
      onclick={goForward}
      disabled={!summary || summary.isToday}
      aria-label="Next day"
      title="Next day (right arrow)">›</button
    >
  </div>
</header>

{#if summary && !error}
  <Metrics items={metrics} />

  <div class="split" class:solo={!summary.now}>
    <section class="card">
      <div class="card-head">
        <h2 class="heading">Day</h2>
        {#if extent}<span class="caption">{extent}</span>{/if}
      </div>
      <Timeline spans={summary.spans} {colors} rolledUp={summary.rolledUp} />
    </section>

    {#if summary.now}
      <section class="card now">
        <div class="eyebrow">Now</div>
        <div class="now-app">
          <span
            class="now-dot"
            style="background: {colors.get(summary.now.exePath) ?? 'var(--series-rest)'}"
          ></span>
          <span class="heading">{summary.now.appName}</span>
        </div>
        <div class="now-elapsed">{formatDuration(summary.now.elapsedMs + sinceLoad)}</div>
        <div class="caption">since {formatClock(summary.now.startMs)}</div>
      </section>
    {/if}
  </div>
{/if}

<RankedList
  {groups}
  loading={loading && summary === null}
  {error}
  emptyMessage={summary?.isToday === false
    ? "Nothing recorded on this day."
    : "Nothing recorded yet today."}
/>

<style>
  .split {
    display: grid;
    grid-template-columns: 1fr 268px;
    gap: var(--space-3);
  }

  /* Off hours, or on a day in the past, there is no open session to show and the timeline
     takes the whole width rather than leaving a hole where the card was. */
  .split.solo {
    grid-template-columns: 1fr;
  }

  .card-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    margin-bottom: var(--space-3);
  }

  .now {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .now-app {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-1);
  }

  .now-dot {
    width: 7px;
    height: 7px;
    border-radius: var(--radius-pill);
  }

  .now-elapsed {
    margin-top: var(--space-2);
    font-size: 28px;
    font-weight: 510;
    line-height: 1.15;
    letter-spacing: -0.022em;
  }
</style>
