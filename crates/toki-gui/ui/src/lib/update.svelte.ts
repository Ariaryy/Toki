import { getVersion } from "@tauri-apps/api/app";

/**
 * The once-a-day "is there a newer Toki?" check. Runs in the window, not the daemon, so
 * the always-on collector keeps no network code; someone who never opens the window is
 * never told. One unauthenticated GET to GitHub's releases API, and the only thing read
 * from the answer is the version number. The switch and the last answer live in the
 * webview's storage: nothing outside this window has any use for them.
 */
const LATEST_URL = "https://api.github.com/repos/Ciatel-Arlanta/Toki/releases/latest";
const DAY_MS = 24 * 60 * 60 * 1000;
const ENABLED_KEY = "toki.updateCheck";
const LATEST_KEY = "toki.latestVersion";
const CHECKED_KEY = "toki.updateCheckedAt";

/** `"v0.3.2"` or `"0.3.2"` to `[0, 3, 2]`; anything else is refused. */
function parse(version: string): number[] | null {
  const m = /^v?(\d+)\.(\d+)\.(\d+)$/.exec(version.trim());
  return m ? m.slice(1).map(Number) : null;
}

function newer(candidate: number[], current: number[]): boolean {
  const i = candidate.findIndex((part, k) => part !== current[k]);
  return i !== -1 && candidate[i] > current[i];
}

class Update {
  enabled = $state(localStorage.getItem(ENABLED_KEY) !== "off");
  /** The newer version to offer, or null when this one is the newest known. */
  available = $state<string | null>(null);

  setEnabled(on: boolean): void {
    this.enabled = on;
    localStorage.setItem(ENABLED_KEY, on ? "on" : "off");
    if (on) void this.check();
    else this.available = null;
  }

  /** Cheap to call often: it asks GitHub only when the last answer is a day old. */
  async check(): Promise<void> {
    if (!this.enabled) return;
    const checkedAt = Number(localStorage.getItem(CHECKED_KEY) ?? 0);
    if (Date.now() - checkedAt >= DAY_MS) await this.fetchLatest();
    const latest = parse(localStorage.getItem(LATEST_KEY) ?? "");
    const current = parse(await getVersion());
    this.available =
      latest && current && newer(latest, current) ? latest.join(".") : null;
  }

  private async fetchLatest(): Promise<void> {
    try {
      const reply = await fetch(LATEST_URL, {
        headers: { Accept: "application/vnd.github+json" },
      });
      if (!reply.ok) return;
      const { tag_name } = (await reply.json()) as { tag_name?: unknown };
      if (typeof tag_name === "string" && parse(tag_name)) {
        localStorage.setItem(LATEST_KEY, tag_name);
      }
      localStorage.setItem(CHECKED_KEY, String(Date.now()));
    } catch {
      // Offline, or GitHub unreachable: try again next time the window asks.
    }
  }
}

export const update = new Update();
