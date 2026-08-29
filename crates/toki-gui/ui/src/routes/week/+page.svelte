<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { formatDuration, formatDelta } from "$lib/format";
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
  }

  let days = $state<DayTotal[]>([]);
  let groups = $state<AppGroup[]>([]);
  let totalMs = $state(0);
  let deltaMs = $state(0);
  let loading = $state(true);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      const summary = await invoke<WeekSummary>("week_totals");
      days = summary.days;
      groups = group(summary.top);
      totalMs = summary.totalMs;
      deltaMs = summary.deltaMs;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  });

  let maxDayMs = $derived(Math.max(1, ...days.map((d) => d.ms)));
</script>

<main class="page">
  <header class="hero">
    <div class="eyebrow">This week</div>
    <div class="hero-value">{loading ? "—" : formatDuration(totalMs)}</div>
    {#if !loading && !error}
      <div class="delta">{formatDelta(deltaMs)}</div>
    {/if}
  </header>

  {#if !loading && !error && days.length > 0}
    <section class="strip" aria-label="Daily totals">
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

  <RankedList {groups} {loading} {error} emptyMessage="Nothing recorded yet this week." />
</main>

<style>
  .page {
    max-width: 640px;
    margin: 0 auto;
    padding: var(--gap-section) 24px;
    display: flex;
    flex-direction: column;
    gap: var(--gap-section);
  }

  .hero {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .eyebrow {
    font-size: 13px;
    color: var(--text-secondary);
  }

  .hero-value {
    font-size: var(--display-size);
    font-weight: 500;
    letter-spacing: -0.02em;
    font-variant-numeric: proportional-nums;
  }

  .delta {
    font-size: 13px;
    color: var(--text-secondary);
  }

  .strip {
    display: flex;
    justify-content: space-between;
    align-items: stretch;
    gap: 8px;
    height: 96px;
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    padding: var(--card-padding);
    background: var(--surface);
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
    width: 100%;
    display: flex;
    align-items: flex-end;
    justify-content: center;
  }

  .bar {
    width: 60%;
    min-height: 2px;
    background: var(--accent);
    border-radius: 3px 3px 0 0;
  }

  .bar.today {
    background: var(--text-primary);
  }

  .label {
    font-size: 11px;
    color: var(--text-muted);
  }

  .label.today {
    color: var(--text-primary);
    font-weight: 500;
  }
</style>
