<script lang="ts">
  import { onMount } from "svelte";
  import { daemon } from "./daemon.svelte";

  /**
   * A timed pause is the one thing on this page that goes stale just by being looked at,
   * so the remaining-minutes label gets a clock. Half a minute is well under the
   * granularity it displays, and the ticker only exists while a timed pause is running —
   * an unpaused window sets no interval at all.
   */
  let now = $state(Date.now());

  let untilMs = $derived(daemon.pause.paused ? daemon.pause.until_ms : null);

  $effect(() => {
    if (untilMs === null) return;
    const id = setInterval(() => (now = Date.now()), 30_000);
    return () => clearInterval(id);
  });

  onMount(() => {
    void daemon.refresh();
  });

  let status = $derived.by(() => {
    if (daemon.liveness === "checking") return { tone: "", label: "Checking…", detail: "" };
    if (daemon.liveness === "unreachable") {
      return { tone: "dead", label: "Not running", detail: "Nothing is being recorded" };
    }
    if (!daemon.pause.paused) return { tone: "live", label: "Tracking", detail: "" };
    return {
      tone: "paused",
      label: "Paused",
      detail: untilMs === null ? "until resumed" : `${remaining(untilMs, now)} left`,
    };
  });

  function remaining(until: number, from: number): string {
    const minutes = Math.max(1, Math.round((until - from) / 60_000));
    if (minutes < 60) return `${minutes}m`;
    const hours = Math.floor(minutes / 60);
    const rest = minutes % 60;
    return rest === 0 ? `${hours}h` : `${hours}h ${rest}m`;
  }
</script>

<div class="rail-footer">
  <div class="status">
    <span class="dot {status.tone}"></span>
    {status.label}
  </div>
  {#if status.detail}
    <div class="detail">{status.detail}</div>
  {/if}
</div>

<style>
  .rail-footer {
    margin-top: auto;
    padding: var(--space-3) var(--space-2) var(--space-1);
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .status {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 12px;
    font-weight: 510;
    color: var(--text-secondary);
  }

  /* The one place the rail carries colour for its own sake, and it is status rather than
     decoration: green tracking, amber paused, red gone. */
  .dot {
    width: 6px;
    height: 6px;
    flex: none;
    border-radius: var(--radius-pill);
    background: var(--text-faint);
  }

  .dot.live {
    background: var(--series-2);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--series-2) 18%, transparent);
  }

  .dot.paused {
    background: var(--series-3);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--series-3) 18%, transparent);
  }

  .dot.dead {
    background: var(--series-7);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--series-7) 18%, transparent);
  }

  .detail {
    padding-left: 14px;
    font-size: 12px;
    color: var(--text-faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
