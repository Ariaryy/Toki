import { getState, type PauseState } from "./ipc";

/**
 * Whether the pipe answered. There is no explicit liveness call — a successful
 * `get_state` *is* the liveness check, which is the whole reason DESIGN.md §8 notes that
 * "(connection succeeds)" counts as a message in the IPC table.
 */
export type Liveness = "checking" | "running" | "unreachable";

/**
 * Pause and liveness, shared across routes.
 *
 * This exists because the rail footer and the Settings page need the same answer, and
 * before the revamp only Settings had it: the daemon could be paused, or stopped outright,
 * and every other view looked exactly like a normal quiet day. A tracker that has silently
 * stopped recording is the failure DESIGN.md §11 says it fears most, so the window should
 * not need a trip to Settings to admit to it.
 */
class Daemon {
  liveness = $state<Liveness>("checking");
  pause = $state<PauseState>({ paused: false, until_ms: null });

  async refresh(): Promise<void> {
    try {
      this.pause = await getState();
      this.liveness = "running";
    } catch {
      // Deliberately not surfaced as an error string. The only actionable reading of a
      // failed pipe call is "the daemon isn't running", and the exception text says that
      // far worse than those three words do.
      this.liveness = "unreachable";
    }
  }

  /**
   * Take a `PauseState` a caller already has. `set_pause` returns the new state, so the
   * Settings page would otherwise have to round-trip a second time to tell the rail what
   * it just did.
   */
  adopt(pause: PauseState): void {
    this.pause = pause;
    this.liveness = "running";
  }
}

export const daemon = new Daemon();
