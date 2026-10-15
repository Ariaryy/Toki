<script lang="ts">
  import { onMount } from "svelte";
  import { slide } from "svelte/transition";
  import { formatDuration } from "./format";
  import { seriesVar } from "./series";
  import type { AppGroup } from "./group";

  let {
    groups,
    loading = false,
    error = null,
    emptyMessage = "Nothing recorded.",
  }: {
    groups: AppGroup[];
    loading?: boolean;
    error?: string | null;
    emptyMessage?: string;
  } = $props();

  let expanded = $state(new Set<string>());

  /**
   * Bars grow from nothing on first paint. `scaleX` rather than `width` on purpose:
   * `--disable-gpu` is set in tauri.conf.json, so this is CPU-rasterised and a width
   * animation would relayout every frame (UI-REVAMP.md §7).
   */
  let grown = $state(false);

  onMount(() => {
    const id = requestAnimationFrame(() => (grown = true));
    return () => cancelAnimationFrame(id);
  });

  function toggle(exePath: string) {
    const next = new Set(expanded);
    if (next.has(exePath)) next.delete(exePath);
    else next.add(exePath);
    expanded = next;
  }

  let maxMs = $derived(groups.length > 0 ? groups[0].ms : 0);
  let totalMs = $derived(groups.reduce((sum, g) => sum + g.ms, 0));

  /** Rounded to whole percent; anything that rounds to nothing is shown as "<1%" rather
      than "0%", which reads as "not recorded" for something that was. */
  function share(ms: number): string {
    if (totalMs === 0) return "";
    const pct = (ms / totalMs) * 100;
    return pct < 0.5 ? "<1%" : `${Math.round(pct)}%`;
  }
</script>

<section class="card">
  {#if loading}
    <p class="muted">Loading…</p>
  {:else if error}
    <p class="muted">Couldn't read the database: {error}</p>
  {:else if groups.length === 0}
    <p class="muted">{emptyMessage}</p>
  {:else}
    <ul class="rows">
      {#each groups as g, i (g.exePath)}
        <li>
          <button
            class="row"
            class:expandable={g.domains !== null}
            class:open={expanded.has(g.exePath)}
            aria-expanded={g.domains === null ? undefined : expanded.has(g.exePath)}
            onclick={() => g.domains && toggle(g.exePath)}
            disabled={g.domains === null}
          >
            <span class="bar-track">
              <span
                class="bar"
                style="
                  --fill: {grown && maxMs > 0 ? g.ms / maxMs : 0};
                  background: {seriesVar(i)};
                  transition-delay: {Math.min(i * 20, 140)}ms;
                "
              ></span>
            </span>
            <span class="name">{g.appName}</span>
            <span class="share">{share(g.ms)}</span>
            <span class="duration">{formatDuration(g.ms)}</span>
            <span class="chevron" class:open={expanded.has(g.exePath)}>
              {#if g.domains !== null}›{/if}
            </span>
          </button>
          {#if g.domains !== null && expanded.has(g.exePath)}
            <ul class="sub-rows" transition:slide={{ duration: 150 }}>
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

<style>
  /* Always the last card on its page, so it takes the slack rather than leaving a band of
     empty canvas under it on a short day. Grow but never shrink: `flex: 1` would be
     `1 1 0%`, and on a short window that shrank the card to under its own rows, which then
     spilled out past its bottom border because the list is not a scroll container. The
     page scrolls instead. */
  section {
    flex: 1 0 auto;
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
  }

  .row {
    display: grid;
    grid-template-columns: 124px 1fr 44px 64px 14px;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    padding: 4px var(--space-2);
    margin: 0 calc(var(--space-2) * -1);
    border: none;
    border-radius: var(--radius-control);
    background: none;
    font: inherit;
    color: inherit;
    text-align: left;
    cursor: default;
    transition: background-color var(--dur-fast) var(--ease);
  }

  .row.expandable {
    cursor: pointer;
  }

  .row.expandable:hover,
  .row.open {
    background: var(--wash-1);
  }

  .bar-track {
    display: block;
    height: 6px;
    border-radius: var(--radius-badge);
    background: var(--wash-2);
    overflow: hidden;
  }

  .bar {
    display: block;
    height: 100%;
    border-radius: var(--radius-badge);
    transform: scaleX(var(--fill));
    transform-origin: left center;
    transition: transform var(--dur-slow) var(--ease);
  }

  .name {
    font-weight: 510;
    letter-spacing: -0.006em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .share {
    font-size: 12px;
    color: var(--text-faint);
    text-align: right;
  }

  .duration {
    text-align: right;
    color: var(--text-secondary);
  }

  .chevron {
    display: inline-block;
    color: var(--text-faint);
    font-size: 12px;
    text-align: center;
    transition: transform var(--dur-fast) var(--ease);
  }

  .chevron.open {
    transform: rotate(90deg);
  }

  .sub-rows {
    padding-left: 136px;
    margin-bottom: var(--space-2);
  }

  .sub-row {
    display: grid;
    /* Mirrors the parent's trailing three columns so a domain duration lands directly
       under the app duration above it. */
    grid-template-columns: 1fr 64px 14px;
    gap: var(--space-3);
    padding: 2px var(--space-2);
    font-size: 13px;
    color: var(--text-muted);
  }

  .sub-row .duration {
    color: var(--text-muted);
  }
</style>
