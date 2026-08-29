<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  interface Total {
    exePath: string;
    appName: string;
    domain: string | null;
    ms: number;
  }

  interface DomainRow {
    domain: string;
    ms: number;
  }

  interface AppGroup {
    exePath: string;
    appName: string;
    ms: number;
    /** `null` for a non-browser app — nothing to expand into. */
    domains: DomainRow[] | null;
  }

  let groups = $state<AppGroup[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let expanded = $state(new Set<string>());

  onMount(async () => {
    try {
      const totals = await invoke<Total[]>("today_totals");
      groups = group(totals);
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  });

  /**
   * Rows already carry both app and domain per DESIGN.md §5 — grouping them under the
   * browser they came from is a display choice here, not a schema one. A row with a
   * domain of `null` folds into "Unresolved" rather than being dropped, so the app total
   * still accounts for every second the daemon recorded.
   */
  function group(totals: Total[]): AppGroup[] {
    const rowsByApp = new Map<string, Total[]>();
    for (const t of totals) {
      const rows = rowsByApp.get(t.exePath);
      if (rows) rows.push(t);
      else rowsByApp.set(t.exePath, [t]);
    }

    const out: AppGroup[] = [];
    for (const rows of rowsByApp.values()) {
      const ms = rows.reduce((sum, r) => sum + r.ms, 0);
      const isBrowser = rows.some((r) => r.domain !== null);
      let domains: DomainRow[] | null = null;
      if (isBrowser) {
        const byDomain = new Map<string, number>();
        for (const r of rows) {
          const key = r.domain ?? "Unresolved";
          byDomain.set(key, (byDomain.get(key) ?? 0) + r.ms);
        }
        domains = [...byDomain.entries()]
          .map(([domain, domainMs]) => ({ domain, ms: domainMs }))
          .sort((a, b) => b.ms - a.ms);
      }
      out.push({ exePath: rows[0].exePath, appName: rows[0].appName, ms, domains });
    }
    return out.sort((a, b) => b.ms - a.ms);
  }

  function formatDuration(ms: number): string {
    const totalMinutes = Math.round(ms / 60_000);
    const hours = Math.floor(totalMinutes / 60);
    const minutes = totalMinutes % 60;
    if (hours === 0 && minutes === 0) return "<1m";
    if (hours === 0) return `${minutes}m`;
    if (minutes === 0) return `${hours}h`;
    return `${hours}h ${minutes}m`;
  }

  function toggle(exePath: string) {
    const next = new Set(expanded);
    if (next.has(exePath)) next.delete(exePath);
    else next.add(exePath);
    expanded = next;
  }

  function seriesVar(index: number): string {
    return index < 7 ? `var(--series-${index + 1})` : "var(--text-muted)";
  }

  let totalMs = $derived(groups.reduce((sum, g) => sum + g.ms, 0));
  let maxMs = $derived(groups.length > 0 ? groups[0].ms : 0);
</script>

<main class="page">
  <header class="hero">
    <div class="eyebrow">Today</div>
    <div class="hero-value">{loading ? "—" : formatDuration(totalMs)}</div>
  </header>

  <section class="card">
    {#if loading}
      <p class="muted">Loading…</p>
    {:else if error}
      <p class="muted">Couldn't read the database: {error}</p>
    {:else if groups.length === 0}
      <p class="muted">Nothing recorded yet today.</p>
    {:else}
      <ul class="rows">
        {#each groups as g, i (g.exePath)}
          <li>
            <button
              class="row"
              class:expandable={g.domains !== null}
              onclick={() => g.domains && toggle(g.exePath)}
              disabled={g.domains === null}
            >
              <span class="bar-track">
                <span
                  class="bar"
                  style="width: {maxMs > 0 ? (g.ms / maxMs) * 100 : 0}%; background: {seriesVar(i)}"
                ></span>
              </span>
              <span class="name">{g.appName}</span>
              <span class="duration">{formatDuration(g.ms)}</span>
              {#if g.domains !== null}
                <span class="chevron" class:open={expanded.has(g.exePath)}>›</span>
              {/if}
            </button>
            {#if g.domains !== null && expanded.has(g.exePath)}
              <ul class="sub-rows">
                {#each g.domains as d (d.domain)}
                  <li class="sub-row">
                    <span class="name">{d.domain}</span>
                    <span class="duration">{formatDuration(d.ms)}</span>
                  </li>
                {/each}
              </ul>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>
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

  .card {
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    padding: var(--card-padding);
    background: var(--surface);
  }

  .muted {
    margin: 0;
    color: var(--text-secondary);
  }

  .rows,
  .sub-rows {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }

  .rows {
    gap: var(--gap-element);
  }

  .row {
    display: grid;
    grid-template-columns: 96px 1fr auto 16px;
    align-items: center;
    gap: var(--gap-element);
    width: 100%;
    padding: 6px 0;
    border: none;
    background: none;
    font: inherit;
    color: inherit;
    text-align: left;
    cursor: default;
  }

  .row.expandable {
    cursor: pointer;
  }

  .bar-track {
    height: 8px;
    border-radius: var(--radius-input);
    background: var(--border);
    overflow: hidden;
  }

  .bar {
    display: block;
    height: 100%;
    border-radius: 0 4px 4px 0;
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .duration {
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .chevron {
    color: var(--text-muted);
    transition: transform 0.15s ease;
  }

  .chevron.open {
    transform: rotate(90deg);
  }

  .sub-rows {
    padding-left: 104px;
    gap: 2px;
  }

  .sub-row {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: var(--gap-element);
    padding: 4px 0;
    font-size: 13px;
  }
</style>
