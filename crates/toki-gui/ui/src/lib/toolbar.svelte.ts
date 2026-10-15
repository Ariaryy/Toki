import type { Snippet } from "svelte";

/**
 * What the top bar shows for the page on screen: its name, and the controls that belong to
 * it (a date stepper, a range picker). A page registers both when it mounts and withdraws
 * them when it leaves, so the bar is drawn once by the layout while each page still owns the
 * state those controls act on.
 *
 * A module singleton rather than context: there is one window and one bar.
 */
class Toolbar {
  /** `null` until a page sets one, and the layout falls back to the route's own name. */
  title = $state<string | null>(null);
  /** Raw: a snippet is a function and has no business being proxied. */
  actions = $state.raw<Snippet | null>(null);

  set(title: string, actions: Snippet | null = null) {
    this.title = title;
    this.actions = actions;
  }

  clear() {
    this.title = null;
    this.actions = null;
  }
}

export const toolbar = new Toolbar();
