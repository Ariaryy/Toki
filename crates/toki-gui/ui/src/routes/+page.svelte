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

<header class="page-head">
  <div>
    <div class="eyebrow">Today</div>
    <div class="display">{loading ? "—" : formatDuration(totalMs)}</div>
  </div>
</header>

<RankedList {groups} {loading} {error} emptyMessage="Nothing recorded yet today." />

<style>
  .page-head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--space-4);
  }
</style>
