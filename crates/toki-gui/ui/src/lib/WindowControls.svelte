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

<!-- Window buttons, for the frameless Windows window; the bar they sit in is drawn by the
     layout. -->
<div class="controls" class:blurred={!focused}>
  <button type="button" aria-label="Minimize" onclick={() => win?.minimize()}>
    <span class="dash" aria-hidden="true"></span>
  </button>
  <button
    type="button"
    aria-label={maximized ? "Restore" : "Maximize"}
    onclick={() => win?.toggleMaximize()}
  >
    <svg viewBox="0 0 10 10" aria-hidden="true">
      {#if maximized}
        <!-- Two squares, the back one cut away a pixel around the front so the pair reads as two
             sheets and not as one smudge where the strokes meet. -->
        <mask id="restore-cut">
          <rect width="10" height="10" fill="#fff" />
          <rect x="-1" y="1" width="10" height="10" rx="2" fill="#000" />
        </mask>
        <rect x="2.5" y=".5" width="7" height="7" rx="1.3" mask="url(#restore-cut)" />
        <rect x=".5" y="2.5" width="7" height="7" rx="1.3" />
      {:else}
        <rect x=".5" y=".5" width="9" height="9" rx="1.5" />
      {/if}
    </svg>
  </button>
  <button type="button" class="close" aria-label="Close" onclick={() => win?.close()}>
    <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M.5.5l9 9M9.5.5l-9 9" /></svg>
  </button>
</div>

<style>
  .controls {
    display: flex;
    height: 100%;
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
    /* Near-white, as the system's own caption glyphs are, so they read as window controls and
       not as part of the page's muted chrome. */
    color: var(--text-secondary);
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

  /* The Windows 11 caption glyphs: hairline strokes, and rounded corners on the square and
     the restore pair. Smooth rendering throughout: snapping the line to the pixel grid made it
     two pixels thick at fractional display scales, where the system's own is one. */
  /* Minimize is a border, not a stroke: a border snaps to a whole device pixel at any display
     scale, so it is one crisp row like the system's, where a 1px stroke blurs across two at
     125% or 150%. */
  .dash {
    display: block;
    width: 10px;
    border-top: 1px solid currentColor;
  }

  button svg {
    width: 10px;
    height: 10px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1;
    shape-rendering: geometricPrecision;
  }

  /* An unfocused window dims its caption, as native frames do, so which window has the
     keyboard is still visible at a glance. */
  .blurred button:not(:hover) {
    color: var(--text-faint);
  }
</style>
