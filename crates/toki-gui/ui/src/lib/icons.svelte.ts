import { invoke } from "@tauri-apps/api/core";
import { SvelteMap } from "svelte/reactivity";

/**
 * `exePath -> data URI`, or `null` for an app with no icon to show. Absent means not asked
 * yet. Reactive, so every row naming the same app fills in together when the answer lands,
 * and an app listed on three screens is read once.
 */
const icons = new SvelteMap<string, string | null>();
const asked = new Set<string>();

/** The icon if it is known, `undefined` while it is not. Starts the read on first ask. */
export function iconFor(exePath: string): string | null | undefined {
  if (!asked.has(exePath)) {
    asked.add(exePath);
    invoke<string | null>("app_icon", { exePath })
      .then((uri) => icons.set(exePath, uri))
      // A failed read is the same as no icon: the tile is already drawn.
      .catch(() => icons.set(exePath, null));
  }
  return icons.get(exePath);
}

/** `domain -> data URI` for the sites under a browser row, read from the browser's own
    favicon files by the backend. Same shape as `icons`, and the same rule: `null` is
    "none to show", absent is "not asked yet". */
const sites = new SvelteMap<string, string | null>();
const siteAsked = new Set<string>();
let siteQueue: string[] = [];

/** One call per render pass rather than one per row: the backend opens each browser file
    once for the whole batch, and a locked one costs a copy. */
function flushSites() {
  const batch = siteQueue;
  siteQueue = [];
  invoke<Record<string, string | null>>("domain_icons", { domains: batch })
    .then((found) => {
      for (const d of batch) sites.set(d, found[d] ?? null);
    })
    .catch(() => {
      for (const d of batch) sites.set(d, null);
    });
}

export function siteIconFor(domain: string): string | null | undefined {
  if (!siteAsked.has(domain)) {
    siteAsked.add(domain);
    if (siteQueue.push(domain) === 1) queueMicrotask(flushSites);
  }
  return sites.get(domain);
}
