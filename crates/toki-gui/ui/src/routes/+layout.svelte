<script lang="ts">
  import "../app.css";
  import "@fontsource/inter/500.css";
  import "@fontsource/inter/600.css";
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { page } from "$app/state";
  import { resolveNow } from "$lib/ipc";

  let { children } = $props();

  onMount(async () => {
    try {
      document.documentElement.dataset.theme = await invoke<string>("system_theme");
    } catch {
      // Falls back to the light tokens, which are the `:root` default already.
    }

    // Fire-and-forget: DESIGN.md §6 wants an immediate resolver pass on launch, alongside
    // the daemon's own 5-minute timer. Nothing here waits on it — a page that already
    // loaded shows the newly-resolved domains next time it queries, not this instant.
    resolveNow().catch(() => {});
  });

  const tabs = [
    { href: "/", label: "Today" },
    { href: "/week", label: "Week" },
    { href: "/settings", label: "Settings" },
  ];
</script>

<nav class="tabs">
  {#each tabs as tab (tab.href)}
    <a href={tab.href} class="tab" class:active={page.url.pathname === tab.href}>
      {tab.label}
    </a>
  {/each}
</nav>

{@render children()}

<style>
  .tabs {
    display: flex;
    gap: 4px;
    max-width: 640px;
    margin: 0 auto;
    padding: 16px 24px 0;
    border-bottom: 1px solid var(--border);
  }

  .tab {
    padding: 8px 4px 12px;
    color: var(--text-secondary);
    text-decoration: none;
    font-size: 13px;
    font-weight: 500;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
  }

  .tab:hover {
    color: var(--text-primary);
  }

  .tab.active {
    color: var(--text-primary);
    border-bottom-color: var(--accent);
  }
</style>
