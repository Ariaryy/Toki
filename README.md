# Toki

A local-first Windows screen-time tracker. Toki records which application holds the
foreground and, for browsers, which domain, into a private SQLite database on your own
disk. There is no cloud, no account, and no network code anywhere in the daemon.

```
3h 42m today — Zen 1h 20m (github.com, docs.google.com…), Code 1h 05m, Discord 40m…
```

## What it does

- Tracks foreground application time, at second resolution, event-driven rather than
  polled — a tab switch or a two-hour video shows up correctly without waking the CPU on
  a timer.
- For browsers (Zen, Chrome, and Chromium forks like Helium), resolves the active tab's
  **domain** — never the full URL — by reading the browser's own history database. Nothing
  above domain granularity is ever stored.
- Distinguishes real presence from idle time, with a bounded exemption for audio/video
  playback so a film doesn't get logged as "away."
- Survives sleep, lock, crash, and a hard power-off without losing more than ~30 seconds
  of data, via a heartbeat rather than relying on any single OS notification.
- Ships a small always-on collector (`toki-daemon.exe`, aiming for under 5 MB resident)
  and a separate GUI (`toki.exe`) that's launched on demand and reads the database
  read-only — so the thing that's always running never carries a WebView.

## What it deliberately doesn't do

- No cloud, no telemetry, no accounts, no self-update.
- No exact-URL tracking — domain only.
- No limits, blocking, or nudges. Passive logging plus (eventually) a weekly digest.
- No screenshots, no keylogging, no clipboard access.

See [DESIGN.md](DESIGN.md) for the full design, including the reasoning behind each of
these choices, and [CHECKLIST.md](CHECKLIST.md) for exactly what's implemented, what's
been measured on a real machine, and what's still open.

## Status

Under active development. The daemon (capture, storage, presence/idle/audio rules, IPC)
is functional and has been running against real usage. The GUI has a working read-only
**Today** view; Week, Settings, and installer packaging are not built yet. Check
[CHECKLIST.md](CHECKLIST.md) for the current phase-by-phase state — it's kept up to date
as the source of truth, not this file.

## Architecture

```
toki-core/     Pure session-assembly logic. No Windows dependency, fully unit tested.
toki-store/    SQLite schema, interning, crash recovery. Shared by the daemon (the only
               writer) and the GUI (read-only), so the two can never disagree about the
               database's shape or location.
toki-daemon/   Windows collector: WinEvent hooks, presence/idle/audio sampling, tray
               icon, autostart, and a named-pipe IPC server. Always running.
toki-gui/      Tauri v2 + SvelteKit GUI. Launched on demand from the tray, exits fully
               on close, opens the database read-only.
```

## Building from source

Windows only — the daemon uses `windows-rs` throughout and there is no cross-platform
target.

**Prerequisites:**

- Rust, `stable-x86_64-pc-windows-msvc` (via [rustup](https://rustup.rs))
- Visual Studio Build Tools with the "Desktop development with C++" workload (needed by
  `windows-rs` and by `rusqlite`'s bundled SQLite)
- [Bun](https://bun.sh) — used to build the GUI's frontend
- WebView2 Runtime — ships with Windows 10/11 by default, no separate install needed

**The daemon:**

```sh
cargo build --release -p toki-daemon
# binary at target/release/toki-daemon.exe
```

**The GUI:**

```sh
cd crates/toki-gui/ui && bun install && cd ..
./ui/node_modules/.bin/tauri build --no-bundle
# binary at target/release/toki.exe
```

(Run from `crates/toki-gui/`, not `ui/` — that's where `tauri.conf.json` and the Rust
`Cargo.toml` live. `bun x tauri` won't resolve the CLI from here since there's no
`package.json` in this directory; invoking the binary the frontend's `bun install` placed
in `ui/node_modules/.bin/` directly sidesteps that.)

(`--no-bundle` skips producing an installer and just builds the executable — drop it once
you want an NSIS installer, once Phase 8 packaging lands.)

Or build everything at once from the workspace root — this builds every crate, GUI
included, but doesn't run the GUI's frontend build step, so use the Tauri command above
if you need `toki.exe` to actually launch:

```sh
cargo build --workspace --release
```

Verify a change didn't break anything:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets
cargo test --workspace
```

## Running it

```sh
toki-daemon.exe run              # start the collector (stays resident; tray icon)
toki-daemon.exe doctor           # health check — heartbeat age, rows today, unresolved
                                  # domains, whether each browser history file is readable
toki-daemon.exe autostart on     # start with Windows (HKCU, no admin needed)
toki-daemon.exe purge --from <YYYY-MM-DD> --to <YYYY-MM-DD> [--app <exe>]
toki-daemon.exe reresolve --from <YYYY-MM-DD> [--to <YYYY-MM-DD>]

toki.exe                         # open the GUI (also reachable from the tray icon)
```

The database lives at `%LOCALAPPDATA%\Toki\toki.db` — never in Documents or any synced
folder, since it's unencrypted and holds window titles. Logs are in
`%LOCALAPPDATA%\Toki\logs\`, rotated daily, 7 days retained.

## License

[MIT](LICENSE)
