<script lang="ts">
  import Skeleton from "./Skeleton.svelte";
  let { table = false }: { table?: boolean } = $props();
</script>

<div class="list-loading" role="status" aria-label="Loading apps" aria-busy="true">
  <div class="head">
    <h2 class="heading">Apps</h2>
    <Skeleton width="48px" />
  </div>
  {#each [1, 2, 3, 4, 5, 6] as row}
    <div class="row" class:table>
      <Skeleton width="28px" height="28px" round />
      <div class="name"><Skeleton width={row % 2 ? "100px" : "128px"} /></div>
      <div class="bar"><Skeleton height={table ? "28px" : "8px"} /></div>
      <Skeleton width="48px" />
      {#if table}<Skeleton width="30px" height="18px" round />{/if}
    </div>
  {/each}
</div>

<style>
  .head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 16px; }
  .row { display: flex; align-items: center; gap: 12px; height: 40px; }
  .row.table { height: 54px; border-top: 1px solid var(--border); }
  .name { flex: 1; min-width: 0; }
  .bar { width: 46%; margin-right: 12px; }
  .table .bar { width: 30%; }
</style>
