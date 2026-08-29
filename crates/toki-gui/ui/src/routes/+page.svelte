<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { formatDuration } from "$lib/format";
  import { group, type AppGroup, type Total } from "$lib/group";
  import RankedList from "$lib/RankedList.svelte";

  let groups = $state<AppGroup[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

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

  let totalMs = $derived(groups.reduce((sum, g) => sum + g.ms, 0));
</script>

<main class="page">
  <header class="hero">
    <div class="eyebrow">Today</div>
    <div class="hero-value">{loading ? "—" : formatDuration(totalMs)}</div>
  </header>

  <RankedList {groups} {loading} {error} emptyMessage="Nothing recorded yet today." />
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
</style>
