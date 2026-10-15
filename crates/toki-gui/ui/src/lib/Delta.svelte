<script lang="ts">
  import { deltaParts } from "./format";

  let { ms, against }: { ms: number; against: string } = $props();

  let d = $derived(deltaParts(ms, against));
</script>

<!-- Neutral ink, never red or green (DESIGN.md §9): the amount carries the weight, and the
     words after it are context. -->
<span class="delta">
  <span class="amount">{d.arrow ? `${d.arrow} ` : ""}{d.amount}</span>
  <span class="vs">vs {d.against}</span>
</span>

<style>
  .delta {
    display: inline-flex;
    align-items: baseline;
    gap: 6px;
    font-size: 14px;
    white-space: nowrap;
  }

  .amount {
    color: var(--text-secondary);
    font-weight: 510;
    letter-spacing: -0.006em;
  }

  .vs {
    color: var(--text-faint);
  }
</style>
