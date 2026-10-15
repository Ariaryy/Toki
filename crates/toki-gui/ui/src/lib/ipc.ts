import { invoke } from "@tauri-apps/api/core";

/**
 * Fields here are snake_case on purpose — this mirrors the daemon's actual wire bytes
 * (`toki-daemon`'s `platform::ipc`, unchanged since Phase 5's `scripts/ipc-probe.ps1`), not
 * the camelCase the rest of this app uses for Tauri command returns. Two different JSON
 * boundaries: the IPC pipe's own protocol, and the Tauri command layer built on top of it.
 */
export interface PauseState {
  paused: boolean;
  until_ms: number | null;
}

export interface Ack {
  ok: boolean;
  error?: string;
}

/** Wait for a write and throw if the daemon refused it, so callers can put things back. */
export async function confirmed(ack: Promise<Ack>): Promise<void> {
  const answer = await ack;
  if (!answer.ok) throw new Error(answer.error ?? "refused");
}

function call<T>(request: Record<string, unknown>): Promise<T> {
  return invoke<T>("ipc_call", { request });
}

export function getState(): Promise<PauseState> {
  return call({ type: "get_state" });
}

/** `forMs` is a duration from now, matching the tray's own pause menu; omit to pause until resumed. */
export function setPause(paused: boolean, forMs?: number): Promise<PauseState> {
  return call({ type: "set_pause", paused, for_ms: forMs ?? null });
}

export function resolveNow(): Promise<PauseState> {
  return call({ type: "resolve_now" });
}

export function setAppAlias(exePath: string, alias: string | null): Promise<Ack> {
  return call({ type: "set_app_alias", exe_path: exePath, alias });
}

export function setDomainAlias(domain: string, alias: string | null): Promise<Ack> {
  return call({ type: "set_domain_alias", domain, alias });
}

export function setIgnored(exePath: string, ignored: boolean): Promise<Ack> {
  return call({ type: "set_ignored", exe_path: exePath, ignored });
}

export function setIdleThresholdMs(ms: number): Promise<Ack> {
  return call({ type: "set_idle_threshold_ms", ms });
}

export function setHistoryReading(on: boolean): Promise<Ack> {
  return call({ type: "set_history_reading", on });
}
