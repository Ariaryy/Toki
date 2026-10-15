<script lang="ts">
  import { iconFor } from "./icons.svelte";

  let {
    exePath,
    name,
    color,
    size = 28,
    fallback = true,
  }: {
    exePath: string;
    name: string;
    /** The app's series colour, which tints the tile it falls back to. */
    color: string;
    size?: number;
    fallback?: boolean;
  } = $props();

  let src = $derived(iconFor(exePath));

  /** First letter or digit, so "(1) Inbox" is not an opening bracket. */
  let initial = $derived(name.match(/[\p{L}\p{N}]/u)?.[0]?.toUpperCase() ?? "?");
</script>

{#if src || fallback}
<span class="icon" style="--size: {size}px; --tint: {color}" aria-hidden="true">
  {#if src}
    <img {src} alt="" draggable="false" />
  {:else}
    <span class="tile">{initial}</span>
  {/if}
</span>
{/if}

<style>
  .icon {
    position: relative;
    flex: none;
    width: var(--size);
    height: var(--size);
  }

  /* What a row shows until the icon is read, and for good when there is none. */
  .tile {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    border-radius: calc(var(--size) * 0.28);
    background: color-mix(in srgb, var(--tint) 16%, transparent);
    color: var(--tint);
    font-size: calc(var(--size) * 0.46);
    font-weight: 600;
    line-height: 1;
  }

  img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
    /* Opacity is one of the two things that stay cheap under --disable-gpu. */
    animation: appear var(--dur-base) var(--ease);
  }

  @keyframes appear {
    from {
      opacity: 0;
    }
  }
</style>
