<script lang="ts">
  import "../app.css";
  import "@fontsource/inter/500.css";
  import "@fontsource/inter/600.css";
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  let { children } = $props();

  onMount(async () => {
    try {
      document.documentElement.dataset.theme = await invoke<string>("system_theme");
    } catch {
      // Falls back to the light tokens, which are the `:root` default already.
    }
  });
</script>

{@render children()}
