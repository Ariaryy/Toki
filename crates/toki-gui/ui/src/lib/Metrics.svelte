<script lang="ts">
  /**
   * The four-up figure strip at the top of Today and Week.
   *
   * Deliberately dumb: every caller has already formatted its own numbers, because the
   * units differ per cell (a duration, a count, a delta) and a component that tried to
   * format them all would need to be told which is which anyway.
   */
  let {
    items,
  }: {
    items: { value: string; sub: string; title?: string }[];
  } = $props();
</script>

<div class="metrics">
  {#each items as m, i (i)}
    <div class="metric">
      <div class="metric-value">{m.value}</div>
      <div class="metric-sub" title={m.title ?? m.sub}>{m.sub}</div>
    </div>
  {/each}
</div>
