<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import {
    getState,
    setPause,
    setAppAlias,
    setDomainAlias,
    setIgnored,
    setIdleThresholdMs,
    type PauseState,
  } from "$lib/ipc";
  import { applyPref, loadPref, type ThemePref } from "$lib/theme";

  interface AppRow {
    exePath: string;
    name: string;
    /** `""` means no alias — normalized from the backend's `null` so `bind:value` works. */
    alias: string;
    ignored: boolean;
  }

  interface DomainRow {
    domain: string;
    name: string;
    alias: string;
  }

  const MINUTE = 60_000;

  let loading = $state(true);
  let error = $state<string | null>(null);

  let pause = $state<PauseState>({ paused: false, until_ms: null });
  let idleMinutes = $state(5);
  let idleSaved = $state(true);

  let apps = $state<AppRow[]>([]);
  let domains = $state<DomainRow[]>([]);

  // Purely a display preference, so it lives in the webview's own storage rather than
  // going through the daemon — nothing outside this window has any use for it.
  let themePref = $state<ThemePref>("system");
  const themeOptions: { value: ThemePref; label: string }[] = [
    { value: "system", label: "System" },
    { value: "light", label: "Light" },
    { value: "dark", label: "Dark" },
  ];

  async function chooseTheme(pref: ThemePref) {
    themePref = pref;
    await applyPref(pref);
  }

  onMount(async () => {
    // Read before the awaits below: it comes from localStorage, so it costs nothing and
    // shouldn't sit behind the daemon round-trip — or a dead daemon would leave the theme
    // control showing the wrong selection.
    themePref = loadPref();
    try {
      const [state, minutes, rawApps, rawDomains] = await Promise.all([
        getState(),
        invoke<number>("idle_threshold_minutes"),
        invoke<{ exePath: string; name: string; alias: string | null; ignored: boolean }[]>(
          "all_apps",
        ),
        invoke<{ domain: string; name: string; alias: string | null }[]>("all_domains"),
      ]);
      pause = state;
      idleMinutes = minutes;
      apps = rawApps.map((a) => ({ ...a, alias: a.alias ?? "" }));
      domains = rawDomains.map((d) => ({ ...d, alias: d.alias ?? "" }));
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  });

  async function pauseFor(minutes: number | undefined) {
    pause = await setPause(true, minutes === undefined ? undefined : minutes * MINUTE);
  }

  async function resume() {
    pause = await setPause(false);
  }

  async function saveIdleThreshold() {
    await setIdleThresholdMs(Math.max(1, Math.round(idleMinutes)) * MINUTE);
    idleSaved = true;
  }

  async function saveAppAlias(row: AppRow) {
    const alias = row.alias.trim();
    row.alias = alias;
    await setAppAlias(row.exePath, alias || null);
  }

  async function toggleIgnored(row: AppRow) {
    row.ignored = !row.ignored;
    await setIgnored(row.exePath, row.ignored);
  }

  async function saveDomainAlias(row: DomainRow) {
    const alias = row.alias.trim();
    row.alias = alias;
    await setDomainAlias(row.domain, alias || null);
  }

  function formatUntil(ms: number): string {
    const minutes = Math.max(1, Math.round((ms - Date.now()) / MINUTE));
    return `${minutes}m left`;
  }
</script>

<main class="page">
  <header class="hero">
    <div class="eyebrow">Settings</div>
  </header>

  <!-- Outside the loading/error branches below: the theme needs nothing from the daemon,
       so it stays usable on a machine where the daemon has stopped. -->
  <section class="card">
    <h2>Theme</h2>
    <p class="muted">System follows Windows' own light and dark setting.</p>
    <div class="segmented" role="group" aria-label="Theme">
      {#each themeOptions as opt (opt.value)}
        <button
          type="button"
          class="segment"
          class:selected={themePref === opt.value}
          aria-pressed={themePref === opt.value}
          onclick={() => chooseTheme(opt.value)}
        >
          {opt.label}
        </button>
      {/each}
    </div>
  </section>

  {#if loading}
    <p class="muted">Loading…</p>
  {:else if error}
    <p class="muted">Couldn't reach the daemon: {error}</p>
  {:else}
    <section class="card">
      <h2>Pause</h2>
      {#if pause.paused}
        <p class="muted">
          Paused{pause.until_ms ? ` — ${formatUntil(pause.until_ms)}` : " until resumed"}.
        </p>
        <button class="button" onclick={resume}>Resume tracking</button>
      {:else}
        <div class="button-row">
          <button class="button" onclick={() => pauseFor(15)}>Pause 15 minutes</button>
          <button class="button" onclick={() => pauseFor(60)}>Pause 1 hour</button>
          <button class="button" onclick={() => pauseFor(undefined)}>Pause until resumed</button>
        </div>
      {/if}
    </section>

    <section class="card">
      <h2>Idle threshold</h2>
      <p class="muted">How long without input before you're considered away.</p>
      <div class="button-row">
        <input
          class="number"
          type="number"
          min="1"
          bind:value={idleMinutes}
          onchange={() => (idleSaved = false)}
        />
        <span class="muted">minutes</span>
        <button class="button" onclick={saveIdleThreshold} disabled={idleSaved}>
          {idleSaved ? "Saved" : "Save"}
        </button>
      </div>
    </section>

    <section class="card">
      <h2>Apps</h2>
      <p class="muted">Rename how an app shows up, or leave it out of your totals.</p>
      <ul class="list">
        {#each apps as row (row.exePath)}
          <li class="list-row">
            <span class="path" title={row.exePath}>{row.name}</span>
            <input
              class="alias"
              type="text"
              placeholder="alias"
              bind:value={row.alias}
              onblur={() => saveAppAlias(row)}
            />
            <label class="checkbox">
              <input type="checkbox" checked={row.ignored} onchange={() => toggleIgnored(row)} />
              Ignore
            </label>
          </li>
        {/each}
      </ul>
    </section>

    <section class="card">
      <h2>Domains</h2>
      <p class="muted">Rename how a resolved domain shows up.</p>
      <ul class="list">
        {#each domains as row (row.domain)}
          <li class="list-row">
            <span class="path" title={row.domain}>{row.name}</span>
            <input
              class="alias"
              type="text"
              placeholder="alias"
              bind:value={row.alias}
              onblur={() => saveDomainAlias(row)}
            />
          </li>
        {/each}
      </ul>
    </section>
  {/if}
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

  .card {
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    padding: var(--card-padding);
    background: var(--surface);
    display: flex;
    flex-direction: column;
    gap: var(--gap-element);
  }

  h2 {
    font-size: 15px;
    font-weight: 500;
  }

  .muted {
    margin: 0;
    color: var(--text-secondary);
    font-size: 13px;
  }

  /* Segmented control rather than three separate buttons: the options are exclusive, and
     three filled accent buttons would each read as "this one is active". Border-first, per
     DESIGN.md's elevation rule — the accent fill is what marks the selection. */
  .segmented {
    display: flex;
    align-self: flex-start;
    border: 1px solid var(--border);
    border-radius: var(--radius-button);
    overflow: hidden;
  }

  .segment {
    border: none;
    border-right: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-secondary);
    font: inherit;
    font-weight: 500;
    font-size: 13px;
    padding: 8px 16px;
    cursor: pointer;
  }

  .segment:last-child {
    border-right: none;
  }

  .segment:hover:not(.selected) {
    color: var(--text-primary);
  }

  .segment.selected {
    background: var(--accent);
    color: #fff;
  }

  .button-row {
    display: flex;
    align-items: center;
    gap: var(--gap-element);
    flex-wrap: wrap;
  }

  .button {
    border: none;
    border-radius: var(--radius-button);
    background: var(--accent);
    color: #fff;
    font: inherit;
    font-weight: 500;
    padding: 8px 14px;
    cursor: pointer;
    align-self: flex-start;
  }

  .button:hover {
    opacity: 0.9;
  }

  .button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .number {
    width: 64px;
    border: 1px solid var(--border);
    border-radius: var(--radius-input);
    background: var(--bg);
    color: var(--text-primary);
    font: inherit;
    padding: 6px 8px;
  }

  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 320px;
    overflow-y: auto;
  }

  .list-row {
    display: grid;
    grid-template-columns: 1fr 160px auto;
    align-items: center;
    gap: var(--gap-element);
    padding: 6px 0;
    border-top: 1px solid var(--border);
  }

  .list-row:first-child {
    border-top: none;
  }

  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
  }

  .alias {
    border: 1px solid #d4d4d4;
    border-radius: var(--radius-input);
    background: var(--bg);
    color: var(--text-primary);
    font: inherit;
    font-size: 13px;
    padding: 5px 8px;
    width: 100%;
  }

  .checkbox {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 13px;
    color: var(--text-secondary);
    white-space: nowrap;
  }
</style>
