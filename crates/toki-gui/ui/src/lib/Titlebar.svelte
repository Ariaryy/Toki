<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";

  // Windows only: `tauri.windows.conf.json` turns the native frame off there, and nowhere
  // else. On Linux the compositor owns decorations (Hyprland draws none at all), so a bar
  // drawn here would be a second title bar, or the only one on a desktop that hides them.

  let maximized = $state(false);
  let focused = $state(true);

  // Null outside a real Tauri window (the stubbed preview), where the buttons do nothing.
  const win = (() => {
    try {
      return getCurrentWindow();
    } catch {
      return null;
    }
  })();

  onMount(() => {
    if (!win) return;
    const sync = () =>
      win
        .isMaximized()
        .then((m) => (maximized = m))
        .catch(() => {});
    sync();
    // Maximize, restore, a drag to the top edge and Win+Up all end in a resize, so that one
    // event covers every way the state can change, including the ones not made here.
    const unlisten = [
      win.onResized(sync),
      win.onFocusChanged(({ payload }) => (focused = payload)),
    ].map((p) => p.catch(() => undefined));
    return () => unlisten.forEach((p) => p.then((off) => off?.()));
  });
</script>

<!-- `data-tauri-drag-region` acts on the element that carries it, not its children, so every
     non-interactive piece repeats it. A double-click on any of them maximizes, as on a native
     caption. -->
<header class="titlebar" class:blurred={!focused} data-tauri-drag-region>
  <div class="brand" data-tauri-drag-region>
    <img src="/favicon.png" alt="" width="16" height="16" data-tauri-drag-region />
    <span data-tauri-drag-region>Toki</span>
  </div>

  <div class="controls">
    <button type="button" aria-label="Minimize" onclick={() => win?.minimize()}>
      <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M0 5.5h10" /></svg>
    </button>
    <button
      type="button"
      aria-label={maximized ? "Restore" : "Maximize"}
      onclick={() => win?.toggleMaximize()}
    >
      <svg viewBox="0 0 10 10" aria-hidden="true">
        {#if maximized}
          <path d="M2.5 2.5V.5h7v7h-2" />
          <rect x=".5" y="2.5" width="7" height="7" />
        {:else}
          <rect x=".5" y=".5" width="9" height="9" />
        {/if}
      </svg>
    </button>
    <button type="button" class="close" aria-label="Close" onclick={() => win?.close()}>
      <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M.5.5l9 9M9.5.5l-9 9" /></svg>
    </button>
  </div>
</header>

<style>
  .titlebar {
    grid-column: 1 / -1;
    grid-row: 1;
    display: grid;
    /* The first column matches the rail below it, so the rail's border runs unbroken to
       the top of the window and the bar reads as part of the shell, not a strip above it. */
    grid-template-columns: 220px 1fr auto;
    height: 36px;
    user-select: none;
    -webkit-user-select: none;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    /* Lines the wordmark up with the nav labels: the rail's padding plus a link's. */
    padding: 0 calc(var(--space-3) + var(--space-2));
    border-right: 1px solid var(--border);
    font-size: 13px;
    font-weight: 590;
    letter-spacing: -0.012em;
    color: var(--text-primary);
    transition: color var(--dur-fast) var(--ease);
  }

  .brand img {
    display: block;
  }

  .controls {
    grid-column: 3;
    display: flex;
  }

  /* 46px is the width Windows gives its own caption buttons, so the targets sit where a
     hand that knows the platform expects them. */
  button {
    width: 46px;
    height: 100%;
    display: grid;
    place-items: center;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-muted);
    transition: background-color var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease);
  }

  button:hover {
    background: var(--wash-2);
    color: var(--text-primary);
  }

  button:active {
    background: var(--wash-3);
  }

  button:focus-visible {
    outline: 1px solid var(--accent);
    outline-offset: -1px;
  }

  /* Close keeps the platform's red. It is the one control whose meaning people read by
     colour before shape, and a themed close would be the button nobody can find. */
  .close:hover {
    background: #c42b1c;
    color: #ffffff;
  }

  .close:active {
    background: #b22a1b;
    color: #ffffff;
  }

  svg {
    width: 10px;
    height: 10px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1;
    shape-rendering: crispEdges;
  }

  .close svg {
    shape-rendering: geometricPrecision;
  }

  /* An unfocused window dims its caption, as native frames do, so which window has the
     keyboard is still visible at a glance. */
  .blurred .brand,
  .blurred button:not(:hover) {
    color: var(--text-faint);
  }
</style>
