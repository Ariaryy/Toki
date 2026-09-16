<script lang="ts">
  import "../app.css";
  import "@fontsource-variable/inter";
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import { resolveNow } from "$lib/ipc";
  import { applyTheme, loadPref, resolveTheme } from "$lib/theme";
  import RailStatus from "$lib/RailStatus.svelte";

  let { children } = $props();

  onMount(async () => {
    // `app.html` has already painted the last-applied theme from localStorage. This
    // reconciles it with the real preference — which for "system" means Windows may have
    // flipped since the window was last open.
    applyTheme(await resolveTheme(loadPref()));

    // Fire-and-forget: DESIGN.md §6 wants an immediate resolver pass on launch, alongside
    // the daemon's own 5-minute timer. Nothing here waits on it — a page that already
    // loaded shows the newly-resolved domains next time it queries, not this instant.
    resolveNow().catch(() => {});
  });

  // Paths are drawn on a 16x16 grid and inherit `currentColor`, so a tab's icon dims and
  // brightens with its label rather than needing its own selected state.
  const tabs = [
    {
      href: "/",
      label: "Today",
      icon: '<circle cx="8" cy="8" r="6.25" /><path d="M8 4.5V8l2.5 1.5" />',
    },
    { href: "/week", label: "Week", icon: '<path d="M3 13.5V8M8 13.5V2.5M13 13.5V10.5" />' },
    {
      href: "/apps",
      label: "Apps",
      icon:
        '<rect x="2.5" y="2.5" width="4.5" height="4.5" rx="1" />' +
        '<rect x="9" y="2.5" width="4.5" height="4.5" rx="1" />' +
        '<rect x="2.5" y="9" width="4.5" height="4.5" rx="1" />' +
        '<rect x="9" y="9" width="4.5" height="4.5" rx="1" />',
    },
    {
      href: "/settings",
      label: "Settings",
      icon: '<path d="M2.5 5h11M2.5 11h11" /><circle cx="6" cy="5" r="1.7" /><circle cx="10" cy="11" r="1.7" />',
    },
  ];
</script>

<aside class="rail">
  <div class="brand">Toki</div>

  <nav class="nav">
    {#each tabs as tab (tab.href)}
      <a href={tab.href} class:active={page.url.pathname === tab.href}>
        <svg viewBox="0 0 16 16" aria-hidden="true">{@html tab.icon}</svg>
        {tab.label}
      </a>
    {/each}
  </nav>

  <RailStatus />
</aside>

<main class="content">
  {@render children()}
</main>

<style>
  /* The shell is a two-column grid on the document body rather than a wrapper element, so
     the rail can reach the full window height without anything in between needing to know
     about it. `:global` because these live outside this component's markup. */
  :global(body) {
    display: grid;
    grid-template-columns: 220px 1fr;
    height: 100vh;
  }

  .rail {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
    padding: var(--space-4) var(--space-3);
    border-right: 1px solid var(--border);
  }

  .brand {
    padding: 0 var(--space-2);
    font-size: 15px;
    font-weight: 590;
    letter-spacing: -0.012em;
  }

  .nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .nav a {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 7px var(--space-2);
    border-radius: var(--radius-control);
    color: var(--text-muted);
    text-decoration: none;
    font-size: 13px;
    font-weight: 510;
    letter-spacing: -0.006em;
    transition: background-color var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease);
  }

  .nav svg {
    width: 14px;
    height: 14px;
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    opacity: 0.85;
  }

  .nav a:hover {
    background: var(--wash-1);
    color: var(--text-secondary);
  }

  /* Selected is a wash plus full-strength ink, not the accent. The accent is spent once
     per view on the thing being acted on, and "which page am I on" is answered well enough
     by contrast — see UI-REVAMP.md §2. */
  .nav a.active {
    background: var(--wash-2);
    color: var(--text-primary);
  }

  .content {
    overflow-y: auto;
    padding: var(--gap-section) var(--space-8);
    display: flex;
    flex-direction: column;
    gap: var(--gap-section);
  }
</style>
