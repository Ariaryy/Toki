<script lang="ts">
  import { siteIconFor } from "./icons.svelte";
  import { UNRESOLVED } from "./group";

  let { domain, size = 16 }: { domain: string; size?: number } = $props();

  /** "Unresolved" is a label, not a site. */
  let src = $derived(domain === UNRESOLVED ? null : siteIconFor(domain));
  let initial = $derived(domain.match(/[\p{L}\p{N}]/u)?.[0]?.toUpperCase() ?? "?");
</script>

<span class="icon" style="--size: {size}px" aria-hidden="true">
  {#if src}
    <img {src} alt="" draggable="false" />
  {:else}
    <span class="tile">{initial}</span>
  {/if}
</span>

<style>
  .icon {
    position: relative;
    flex: none;
    width: var(--size);
    height: var(--size);
  }

  /* Neutral, unlike an app's tile: a site has no series colour, and a row of coloured tiles
     under a browser would read as more apps. */
  .tile {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    border-radius: calc(var(--size) * 0.28);
    background: var(--wash-2);
    color: var(--text-muted);
    font-size: calc(var(--size) * 0.56);
    font-weight: 600;
    line-height: 1;
  }

  img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
    border-radius: calc(var(--size) * 0.2);
    animation: appear var(--dur-base) var(--ease);
  }

  @keyframes appear {
    from {
      opacity: 0;
    }
  }
</style>
