<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { update } from "./update.svelte";

  let error = $state<string | null>(null);

  async function download() {
    error = null;
    try {
      await invoke("open_download");
    } catch (e) {
      error = String(e);
    }
  }
</script>

{#if update.available}
  <div class="notice" role="status">
    <div class="title">Toki {update.available} is available</div>
    <button type="button" class="btn btn-primary" onclick={download}>Download</button>
    {#if error}<div class="error">{error}</div>{/if}
  </div>
{/if}

<style>
  .notice {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-bottom: var(--space-3);
    padding: var(--space-3);
    border-radius: var(--radius-control);
    background: var(--surface-raised);
    box-shadow: var(--ring);
  }

  .title {
    font-size: 12px;
    font-weight: 510;
    color: var(--text-secondary);
  }

  .error {
    font-size: 11px;
    color: var(--text-faint);
  }
</style>
