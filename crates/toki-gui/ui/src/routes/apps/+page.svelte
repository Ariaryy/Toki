<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { formatDuration } from "$lib/format";
  import { seriesVar } from "$lib/series";
  import { setAppAlias, setIgnored } from "$lib/ipc";

  interface AppEntry {
    exePath: string;
    name: string;
    /** `""` means no alias — normalized from the backend's `null` so `bind:value` works. */
    alias: string;
    ignored: boolean;
    ms: number;
  }

  /** `null` is all time, which the backend resolves to the first date with anything in it. */
  const ranges: { label: string; days: number | null }[] = [
    { label: "7 days", days: 7 },
    { label: "30 days", days: 30 },
    { label: "All time", days: null },
  ];

  let days = $state<number | null>(30);
  let rows = $state<AppEntry[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  let query = $state("");
  let sort = $state<"time" | "name">("time");
  let showIgnored = $state(true);

  $effect(() => {
    void load(days);
  });

  async function load(range: number | null) {
    loading = true;
    try {
      const raw = await invoke<
        { exePath: string; name: string; alias: string | null; ignored: boolean; ms: number }[]
      >("app_directory", { days: range });
      rows = raw.map((r) => ({ ...r, alias: r.alias ?? "" }));
      error = null;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  async function saveAlias(row: AppEntry) {
    const alias = row.alias.trim();
    row.alias = alias;
    await setAppAlias(row.exePath, alias || null);
    // The displayed name is `COALESCE(alias, detected_name, exe_path)` in the store, so it
    // has to follow the alias here too rather than wait for the next query.
    row.name = alias || row.exePath;
  }

  async function toggleIgnored(row: AppEntry) {
    row.ignored = !row.ignored;
    await setIgnored(row.exePath, row.ignored);
  }

  /**
   * Colour is by rank over the whole range, ignored rows included, so a swatch means the
   * same thing here as on Today: the Nth biggest thing. Hiding ignored rows must not
   * renumber the ones that remain.
   */
  let colors = $derived(
    new Map(
      [...rows].sort((a, b) => b.ms - a.ms).map((r, i) => [r.exePath, seriesVar(i)] as const),
    ),
  );

  let maxMs = $derived(Math.max(1, ...rows.map((r) => r.ms)));
  let hiddenCount = $derived(rows.filter((r) => r.ignored).length);

  let visible = $derived.by(() => {
    const needle = query.trim().toLowerCase();
    const matched = rows.filter((r) => {
      if (!showIgnored && r.ignored) return false;
      if (needle === "") return true;
      // The executable is searched too: it is what someone has when they are looking for
      // the row behind a name they do not recognise.
      return (
        r.name.toLowerCase().includes(needle) ||
        r.exePath.toLowerCase().includes(needle) ||
        r.alias.toLowerCase().includes(needle)
      );
    });
    return matched.sort((a, b) =>
      sort === "time" ? b.ms - a.ms : a.name.localeCompare(b.name),
    );
  });

  onMount(() => {
    // Focus follows the page: typing is the first thing anyone does on a table of 40 apps.
    document.querySelector<HTMLInputElement>(".search input")?.focus();
  });
</script>

<header class="page-head">
  <div>
    <div class="eyebrow">Apps</div>
    <div class="display">{loading && rows.length === 0 ? "—" : `${rows.length} seen`}</div>
  </div>

  <div class="seg-group" role="group" aria-label="Range">
    {#each ranges as r (r.label)}
      <button
        type="button"
        class="seg-btn"
        class:on={days === r.days}
        aria-pressed={days === r.days}
        onclick={() => (days = r.days)}
      >
        {r.label}
      </button>
    {/each}
  </div>
</header>

<div class="toolbar">
  <div class="search">
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <circle cx="7" cy="7" r="4.5" /><path d="M10.5 10.5 14 14" />
    </svg>
    <input type="search" placeholder="Search apps and aliases…" bind:value={query} />
  </div>
  <button
    class="btn"
    onclick={() => (sort = sort === "time" ? "name" : "time")}
    title="Sort by time or by name"
  >
    Sort: {sort === "time" ? "Time" : "Name"}
  </button>
  <button
    class="btn"
    onclick={() => (showIgnored = !showIgnored)}
    aria-pressed={!showIgnored}
    disabled={hiddenCount === 0}
  >
    {showIgnored ? `Hide ignored (${hiddenCount})` : `Show ignored (${hiddenCount})`}
  </button>
</div>

<section class="card table-card">
  {#if error}
    <p class="muted">Couldn't read the database: {error}</p>
  {:else if loading && rows.length === 0}
    <p class="muted">Loading…</p>
  {:else if visible.length === 0}
    <p class="muted">{query ? `Nothing matching "${query}".` : "No apps recorded yet."}</p>
  {:else}
    <table>
      <thead>
        <tr>
          <th style="width: 210px">App</th>
          <th style="width: 190px">Alias</th>
          <th style="width: 96px" class="num" class:sorted={sort === "time"}>
            Time{sort === "time" ? " ↓" : ""}
          </th>
          <th style="width: 100px">Share</th>
          <th class="num">Ignore</th>
        </tr>
      </thead>
      <tbody>
        {#each visible as row (row.exePath)}
          <tr class:ignored={row.ignored}>
            <td>
              <div class="app-name">
                <span
                  class="swatch"
                  style="background: {colors.get(row.exePath) ?? 'var(--series-rest)'}"
                ></span>
                {row.name}
              </div>
              <div class="exe" title={row.exePath}>{row.exePath}</div>
            </td>
            <td>
              <input
                class="alias"
                type="text"
                placeholder="alias"
                aria-label="Alias for {row.name}"
                bind:value={row.alias}
                onblur={() => saveAlias(row)}
              />
            </td>
            <td class="num">{row.ms === 0 ? "—" : formatDuration(row.ms)}</td>
            <td>
              <span class="mini">
                <span
                  style="
                    width: {(row.ms / maxMs) * 100}%;
                    background: {colors.get(row.exePath) ?? 'var(--series-rest)'};
                  "
                ></span>
              </span>
            </td>
            <td>
              <button
                class="toggle"
                class:on={row.ignored}
                role="switch"
                aria-checked={row.ignored}
                aria-label="Ignore {row.name}"
                title={row.ignored
                  ? "Ignored — left out of every total"
                  : "Leave this app out of your totals"}
                onclick={() => toggleIgnored(row)}
              >
                <i></i>
              </button>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</section>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .search {
    flex: 1;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 7px var(--space-3);
    border-radius: var(--radius-control);
    background: var(--surface);
    box-shadow: var(--ring);
    color: var(--text-faint);
    font-size: 13px;
  }

  .search:focus-within {
    box-shadow: var(--ring-strong);
  }

  .search svg {
    width: 13px;
    height: 13px;
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
  }

  /* The field carries the whole row's chrome, so the input itself has none — otherwise the
     focus ring lands inside a box that is already a box. */
  .search input {
    flex: 1;
    border: none;
    background: none;
    outline: none;
    color: var(--text-primary);
    font: inherit;
    font-size: 13px;
  }

  .search input::placeholder {
    color: var(--text-faint);
  }

  .seg-group {
    display: flex;
    padding: 2px;
    border-radius: var(--radius-control);
    background: var(--surface);
    box-shadow: var(--ring);
  }

  .seg-btn {
    padding: 4px var(--space-3);
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--text-muted);
    font: inherit;
    font-size: 12px;
    font-weight: 510;
    cursor: pointer;
    transition: background-color var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease);
  }

  .seg-btn:hover:not(.on) {
    color: var(--text-secondary);
  }

  .seg-btn.on {
    background: var(--wash-3);
    color: var(--text-primary);
  }

  /* The card is the scroll container, so the header row stays put over a long directory. */
  .table-card {
    overflow: hidden auto;
    min-height: 0;
    flex: 1;
    padding-top: 0;
  }

  .muted {
    margin: var(--card-padding) 0 0;
    color: var(--text-secondary);
  }

  table {
    width: 100%;
    border-collapse: collapse;
  }

  thead th {
    position: sticky;
    top: 0;
    padding: var(--card-padding) var(--space-2) var(--space-2);
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    text-align: left;
    font-size: 11px;
    font-weight: 510;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-faint);
  }

  thead th.num {
    text-align: right;
  }

  /* The sorted column is marked by ink weight, not by the accent — the accent means
     "selected", and a sort order is not a selection. */
  thead th.sorted {
    color: var(--text-secondary);
  }

  tbody td {
    padding: 5px var(--space-2);
    border-bottom: 1px solid color-mix(in srgb, var(--border) 55%, transparent);
    vertical-align: middle;
  }

  tbody tr:last-child td {
    border-bottom: none;
  }

  .app-name {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-weight: 510;
    letter-spacing: -0.006em;
  }

  .swatch {
    width: 7px;
    height: 7px;
    flex: none;
    border-radius: var(--radius-pill);
  }

  .exe {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12px;
    color: var(--text-faint);
  }

  /* Forty stacked hard borders is loud in a dense table, so the field reads as text until
     it is pointed at — the reason DESIGN.md §9 already softened these. */
  .alias {
    width: 100%;
    padding: 3px var(--space-2);
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--text-secondary);
    font: inherit;
    font-size: 13px;
    transition: border-color var(--dur-fast) var(--ease);
  }

  .alias::placeholder {
    color: var(--text-faint);
  }

  .alias:hover,
  .alias:focus {
    border-color: var(--border);
  }

  .num {
    text-align: right;
    color: var(--text-secondary);
  }

  .mini {
    display: block;
    width: 84px;
    height: 5px;
    border-radius: var(--radius-badge);
    background: var(--wash-2);
    overflow: hidden;
  }

  .mini span {
    display: block;
    height: 100%;
    border-radius: var(--radius-badge);
  }

  /* A switch, off by default: ignoring an app is the unusual choice, and this is the one
     control on the page with a consequence, so it gets the accent. */
  .toggle {
    position: relative;
    display: block;
    width: 26px;
    height: 15px;
    margin-left: auto;
    padding: 0;
    border: none;
    border-radius: var(--radius-pill);
    background: var(--wash-3);
    cursor: pointer;
    transition: background-color var(--dur-fast) var(--ease);
  }

  .toggle i {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 11px;
    height: 11px;
    border-radius: var(--radius-pill);
    background: var(--text-muted);
    /* `translateX` rather than moving `left`: transform and opacity are the two things that
       do not relayout on a CPU-rasterised webview. */
    transition: transform var(--dur-fast) var(--ease), background-color var(--dur-fast) var(--ease);
  }

  .toggle.on {
    background: var(--accent);
  }

  .toggle.on i {
    transform: translateX(11px);
    background: #ffffff;
  }

  /* Dimmed rather than hidden: an ignored app's time is still real, and seeing what it
     costs is the whole reason to look at this page. */
  tr.ignored .app-name,
  tr.ignored .num {
    color: var(--text-faint);
  }

  tr.ignored .swatch,
  tr.ignored .mini {
    opacity: 0.35;
  }
</style>
