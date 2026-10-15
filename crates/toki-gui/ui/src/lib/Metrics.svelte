<script lang="ts">
  /**
   * The figure strip under the headline on Today and Week. As many cells as the caller gives.
   *
   * Deliberately dumb: every caller has already formatted its own numbers, because the
   * units differ per cell (a duration, a count, a delta) and a component that tried to
   * format them all would need to be told which is which anyway.
   */
  import AppIcon from "./AppIcon.svelte";
  import SiteIcon from "./SiteIcon.svelte";

  /** A tiny icon inside the sub line, between `before` and the name, for a cell that is about
      one app or site: "Longest in [icon] Code at 14:00". */
  type Lead =
    | { kind: "app"; exePath: string; name: string; color: string }
    | { kind: "site"; domain: string };

  let {
    items,
  }: {
    items: {
      value: string;
      sub: string;
      before?: string;
      /** Text after `sub` that must stay whole when the line is tight, so it is `sub` that
          gives way: "Code at 2:00 PM" loses letters from "Code", not from the time. */
      after?: string;
      title?: string;
      lead?: Lead;
    }[];
  } = $props();
</script>

<div class="metrics" style="--n: {items.length}">
  {#each items as m, i (i)}
    <div class="metric">
      <div class="metric-value">{m.value}</div>
      <div class="metric-sub" title={m.title ?? [m.before, m.sub, m.after].filter(Boolean).join(" ")}>
        {#if m.before}<span class="metric-before">{m.before}</span>{/if}
        <span class="metric-subject">
          {#if m.lead}
              {#if m.lead.kind === "app"}
                <AppIcon exePath={m.lead.exePath} name={m.lead.name} color={m.lead.color} size={10} fallback={false} />
              {:else}
                <SiteIcon domain={m.lead.domain} size={10} fallback={false} />
              {/if}
          {/if}
          <span class="metric-sub-text">{m.sub}</span>
        </span>
        {#if m.after}<span class="metric-before">{m.after}</span>{/if}
      </div>
    </div>
  {/each}
</div>
