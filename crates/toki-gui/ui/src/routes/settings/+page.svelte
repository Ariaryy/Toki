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
  import { daemon } from "$lib/daemon.svelte";
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
      adopt(state);
      idleMinutes = minutes;
      apps = rawApps.map((a) => ({ ...a, alias: a.alias ?? "" }));
      domains = rawDomains.map((d) => ({ ...d, alias: d.alias ?? "" }));
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  });

  /** Every pause answer is authoritative for the rail too, so it goes to both. */
  function adopt(state: PauseState) {
    pause = state;
    daemon.adopt(state);
  }

  async function pauseFor(minutes: number | undefined) {
    adopt(await setPause(true, minutes === undefined ? undefined : minutes * MINUTE));
  }

  async function resume() {
    adopt(await setPause(false));
  }

  // The daemon refuses anything outside 15s-2h, so clamp here rather than let a typo come
  // back as an error the user has to decode. One minute is the smallest whole minute above
  // that floor.
  const IDLE_MIN_MINUTES = 1;
  const IDLE_MAX_MINUTES = 120;

  async function saveIdleThreshold() {
    const minutes = Math.min(
      IDLE_MAX_MINUTES,
      Math.max(IDLE_MIN_MINUTES, Math.round(idleMinutes)),
    );
    idleMinutes = minutes;
    await setIdleThresholdMs(minutes * MINUTE);
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

<header class="page-head">
  <div>
    <div class="eyebrow">Settings</div>
    <div class="display">Toki</div>
  </div>
</header>

<div class="cols">
  <div class="stack">
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
        <p class="muted">Nothing is recorded while paused. The daemon keeps running.</p>
        {#if pause.paused}
          <div class="banner">
            <span class="dot"></span>
            Paused{pause.until_ms ? ` — ${formatUntil(pause.until_ms)}` : " until resumed"}
          </div>
          <div class="button-row">
            <button class="button primary" onclick={resume}>Resume tracking</button>
          </div>
        {:else}
          <div class="button-row">
            <button class="button" onclick={() => pauseFor(15)}>15 minutes</button>
            <button class="button" onclick={() => pauseFor(60)}>1 hour</button>
            <button class="button" onclick={() => pauseFor(undefined)}>Until resumed</button>
          </div>
        {/if}
      </section>

      <section class="card">
        <h2>Idle threshold</h2>
        <p class="muted">How long without input before you're counted as away.</p>
        <div class="button-row">
          <input
            class="number"
            type="number"
            min={IDLE_MIN_MINUTES}
            max={IDLE_MAX_MINUTES}
            bind:value={idleMinutes}
            onchange={() => (idleSaved = false)}
          />
          <span class="muted">minutes</span>
          <span class="spacer"></span>
          <button class="button" onclick={saveIdleThreshold} disabled={idleSaved}>
            {idleSaved ? "Saved" : "Save"}
          </button>
        </div>
        <p class="note">
          Audio keeps a session alive past this for up to 2 hours, so a film is not logged as
          time away.
        </p>
      </section>
    {/if}
  </div>

  {#if !loading && !error}
    <div class="stack">
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
            <li class="list-row domain">
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
    </div>
  {/if}
</div>

<style>
  .page-head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--space-4);
  }

  .cols {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
    align-items: start;
  }

  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }

  .card {
    display: flex;
    flex-direction: column;
    gap: var(--gap-element);
  }

  h2 {
    font-size: 14px;
  }

  .muted {
    margin: 0;
    color: var(--text-muted);
    font-size: 12px;
  }

  .note {
    margin: var(--space-1) 0 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-control);
    background: var(--surface-raised);
    font-size: 11px;
    line-height: 1.45;
    color: var(--text-faint);
  }

  /* Segmented control rather than three separate buttons: the options are exclusive, and
     three filled buttons would each read as "this one is active". The selection is marked
     by a wash and full-strength ink rather than the accent — on this page the accent is
     spent on resuming, which is the only action with consequences. */
  .segmented {
    display: flex;
    align-self: flex-start;
    padding: 2px;
    border-radius: var(--radius-control);
    background: var(--surface-raised);
    box-shadow: var(--ring);
  }

  .segment {
    padding: 5px var(--space-3);
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

  .segment:hover:not(.selected) {
    color: var(--text-secondary);
  }

  .segment.selected {
    background: var(--wash-2);
    color: var(--text-primary);
  }

  .banner {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-control);
    background: color-mix(in srgb, var(--series-3) 12%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--series-3) 30%, transparent);
    font-size: 12px;
    font-weight: 510;
    color: var(--text-secondary);
  }

  .banner .dot {
    width: 6px;
    height: 6px;
    flex: none;
    border-radius: var(--radius-pill);
    background: var(--series-3);
  }

  .button-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .spacer {
    flex: 1;
  }

  .button {
    padding: 6px var(--space-3);
    border: none;
    border-radius: var(--radius-control);
    background: var(--wash-2);
    box-shadow: var(--ring);
    color: var(--text-secondary);
    font: inherit;
    font-size: 12px;
    font-weight: 510;
    cursor: pointer;
    transition: color var(--dur-fast) var(--ease), background-color var(--dur-fast) var(--ease);
  }

  .button:hover:not(:disabled) {
    color: var(--text-primary);
    background: var(--wash-3);
  }

  /* One per view. */
  .button.primary {
    background: var(--accent);
    box-shadow: none;
    color: #fff;
  }

  .button.primary:hover:not(:disabled) {
    background: var(--accent);
    opacity: 0.9;
  }

  .button:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .number {
    width: 62px;
    padding: 5px var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    background: var(--surface-raised);
    color: var(--text-primary);
    font: inherit;
    font-size: 13px;
    text-align: right;
  }

  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 300px;
    overflow-y: auto;
  }

  .list-row {
    display: grid;
    grid-template-columns: 1fr 150px auto;
    align-items: center;
    gap: var(--space-2);
    padding: 3px 0;
    border-top: 1px solid color-mix(in srgb, var(--border) 55%, transparent);
  }

  .list-row.domain {
    grid-template-columns: 1fr 150px;
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

  /* Twenty stacked hard borders is loud in a dense form, so the field reads as text until
     it is pointed at — the reason DESIGN.md §9 already softened these to Smoke. */
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

  .checkbox {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    font-size: 12px;
    color: var(--text-muted);
    white-space: nowrap;
  }
</style>
