# Toki — Implementation Checklist

Status of every piece of [DESIGN.md](DESIGN.md). Updated as work lands.

Legend: `[x]` done and verified · `[~]` in progress · `[ ]` not started

**Verified** means it compiles, has tests that pass, and clippy is clean. Anything marked
done that has *not* been exercised against a real Windows desktop says so explicitly.

---

## Phase 0 — Foundations `[x]`

- [x] Workspace, release profile tuned for size (`opt-level = "z"`, LTO, strip)
- [x] `toki-core` — OS-agnostic, no `windows-rs`, no COM
- [x] `event.rs` — `Millis`, `AppKey`, `Presence`, `Event`
- [x] `tracker.rs` — session assembly, noise floor, idle and audio rules · 10 tests
- [x] `time.rs` — local-day bounds, midnight clamp · 4 tests
- [x] `resolve.rs` — title normalization, `TitleIndex`, recency tie-break · 7 tests
- [x] `toki-daemon` CLI skeleton
- [x] Toolchain: rustup `stable-x86_64-pc-windows-msvc`, VS Build Tools C++ workload
- [x] `git init` (nothing committed yet)

---

## Phase 1 — Store `[x]`

SQLite. No Windows APIs, so fully testable in memory. 13 tests.

- [x] `toki-store` crate, `rusqlite` 0.40 bundled
- [x] Schema from DESIGN.md §7, `user_version` set to 1 (no migration path needed yet —
      v1 is the only version that has ever existed)
- [x] WAL, `busy_timeout`, foreign keys on
- [x] Interning for `apps`, `titles`, `domains`, including upgrading an app to a browser
      engine when it is recognised late
- [x] `apply(Action)` — `Emit` inserts, `ExtendLast` moves the previous end forward only
- [x] Heartbeat write and open-session mirror in `meta`
- [x] Crash recovery — closes a dangling session at the last heartbeat, and cannot run twice
- [x] Day totals using the clamp query with the 24h index bound, aliases taking precedence
- [x] Pending-resolution queue, domain backfill, failed-lookup marking
- [x] `purge` by range and by app
- [x] Read-only connection mode for the GUI

---

## Phase 2 — Platform capture `[~]`

Windows-specific. Written and running; the paths that need a real desktop event to fire are
listed as unverified below rather than marked done.

- [x] `SetWinEventHook` — `EVENT_SYSTEM_FOREGROUND`
- [x] `EVENT_OBJECT_NAMECHANGE` filtered to foreground HWND + `OBJID_WINDOW` +
      `CHILDID_SELF`, then deduplicated on (exe, title)
- [x] Hook callback pushes to a channel and returns — never blocks
- [x] Message pump thread, plus a message-only window for power and session notices
- [x] Exe path resolution from HWND, cached per HWND and invalidated on
      `EVENT_OBJECT_DESTROY`
- [x] UWP unwrapping — walk `ApplicationFrameHost` children for the real process
- [x] Display name from the version resource (verified: "Google Chrome", "Windows Explorer")
- [x] Default ignore list in `toki-core::ignore`; the store's `ignored` column carries the
      user's own additions
- [x] `GetLastInputInfo` presence sampling at 15s, wrap-safe against the 49.7-day tick
      counter
- [x] WASAPI `IAudioSessionManager2` probe, invoked only at the idle boundary
- [x] `RegisterSuspendResumeNotification` suspend/resume (message-only windows get no
      `WM_POWERBROADCAST` broadcast, so it is requested explicitly)
- [x] `WTSRegisterSessionNotification` lock/unlock
- [x] Manual check: foreground change, browser tab switch, engine detection, display name
- [ ] Manual check: lock, sleep, 30min idle, a film past the idle threshold

---

## Phase 3 — Resolver `[ ]`

- [x] Locate Zen profiles — `%APPDATA%\zen\Profiles\*\places.sqlite`
- [x] Locate Chrome profiles — `%LOCALAPPDATA%\Google\Chrome\User Data\*\History`
- [x] Locate Helium profiles — `%LOCALAPPDATA%\imput\Helium\User Data\*\History`
- [x] Enumerate every profile rather than assuming `Default` (this machine has 3 Chrome
      profiles; hardcoding `Default` would have dropped two of them)
- [x] Copy file + `-wal` + `-shm` before reading (never `immutable=1`), deleted on drop
- [x] Build `TitleIndex` from each source — `toki-store::history`, 8 tests
- [x] Per-engine epoch handling: Gecko counts microseconds from 1970, Chromium from 1601
- [x] `host_of` filters non-web schemes, so `chrome://`, `about:`, `file://` and extension
      pages never enter the index
- [x] `toki-daemon index` — builds the merged index and reports ambiguity
- [x] Backfill pending rows every 5 minutes, on its own thread and its own connection
- [x] `toki reresolve --from [--to]` — requeue a date range and drain it
- [ ] Immediate pass on GUI launch (needs the IPC layer)
- [ ] Measure real *hit* rate over a full day — see below for the first sample

---

## Phase 4 — Daemon assembly `[x]`

- [x] Wire hook → channel → `Tracker` → `Store`
- [x] 15s presence, 30s heartbeat, 5min resolve. One `recv_timeout` loop rather than a
      timer thread — the collector must be single-threaded anyway, so a second thread would
      only add a lock around the `Tracker`
- [x] Single instance — named mutex `Global\TokiDaemon`
- [x] Ctrl+C and `WM_ENDSESSION` close the open session instead of leaving it to recovery
- [x] Tray icon via `Shell_NotifyIcon`. The glyph is drawn at run time into a DIB rather
      than shipped as an `.ico`, which keeps a resource compiler out of the build for one
      sixteen-pixel mark
- [x] Tray menu: open Toki, pause (15m / 1h / until resumed), quit. Open Toki is greyed
      until the GUI exists beside the daemon
- [x] Pause auto-resumes on next daemon start — by never persisting it. A pause is a
      decision about the next hour, and one that survived a reboot would be a tracker that
      had silently stopped
- [x] Autostart — `HKCU\...\CurrentVersion\Run`, via `toki-daemon autostart [on|off]`.
      `doctor` reports the value, because the failure mode is a stale path
- [x] Console detach: a daemon started from the Run key is the only process on its console
      and lets go of it, so no black window sits on the taskbar all session. Started from a
      terminal the shell is attached too, and output stays put
- [x] `tracing` to `%LOCALAPPDATA%\Toki\logs\`, daily rotation, 7 days
- [x] `toki doctor` — heartbeat age, open session, rows today, pending count, source
      readability. Opens the database read-only so a health check never creates it.

---

## Phase 5 — IPC `[x]`

- [x] Named pipe server at `\\.\pipe\toki`, length-prefixed JSON
- [x] `resolve_now`, `get_state`, `set_pause`
- [x] Liveness via connect success
- [x] Manual check: `get_state`, `set_pause` (timed and resume) and `resolve_now`
      round-tripped against a live daemon over the real pipe — see below

---

## Phase 6 — GUI `[~]`

- [x] Tauri v2 + Svelte scaffold
- [x] Read-only store connection
- [x] Design tokens, light + dark, following `AppsUseLightTheme`
- [x] Categorical data palette excluding `#2563eb`
- [x] Today view — total, per-app, browsers expandable into domains
- [ ] Week view — daily totals, top entities, neutral week-over-week delta
- [ ] Settings — alias editor, ignore list, idle threshold, pause — also needs the idle
      threshold to become configurable somewhere, which nothing writes today
- [ ] First run opens the GUI once, never again

---

## Phase 7 — Retention and digest `[ ]`

- [ ] Roll `sessions` older than 6 months into `daily_rollup`, drop titles
- [ ] Weekly Mon–Sun digest toast on first Monday activity
- [x] `toki purge --from --to [--app]`, inclusive of the whole `--to` day
- [x] `toki reresolve` subcommand

---

## Phase 8 — Packaging `[ ]`

- [ ] NSIS bundle via Tauri
- [ ] Installer owns Run key, Start-menu entry, uninstall
- [ ] GitHub release
- [ ] Measure and record actual daemon RSS against the <5 MB target

---

## Deviations from DESIGN.md

Recorded when implementation forces a change.

| Where | Change | Why |
|---|---|---|
| §4 tray | The pump's window is a hidden top-level window, not message-only | `HWND_MESSAGE` windows receive no broadcasts, so `WM_ENDSESSION` never arrived and the clean-shutdown path was dead; and a menu owned by one cannot take the foreground, so it would not dismiss on a click away |
| §4 tray | Icon painted into a DIB at run time | An `.ico` asset needs a resource compiler in the build. Cutting the clock hands out as transparency rather than drawing them in means one glyph reads on both a light and a dark taskbar |
| §4 pause | Timed pauses end on the collector's 15s tick, not a timer | The shortest pause offered is fifteen minutes; a dedicated timer would buy accuracy nobody can perceive |
| §4 logging | `tracing-subscriber` with `fmt` only, and a hand-written tee | Default features pull env-filter and a regex engine for a level fixed at compile time. Two destinations was the whole requirement |
| §4 audio cap | `end = last_input + cap`, not backdated to last input | Backdating discarded the entire film the exemption exists to keep |
| §2 tracker API | Presence sampling is `on_tick(at, last_input, probe)`, not a `Tick` event | A `Tick` carrying `foreground_audio: bool` would force a WASAPI probe every 15s, contradicting §4 |
| §3 noise floor | `ExtendLast` only fires when the previous end is contiguous with the current start | Otherwise a blip after an idle gap stretches the previous session across the gap and invents time |
| §2 crates | Storage is its own crate, `toki-store`, not a module inside `toki-daemon` | The GUI needs the same schema and the same clamp query; two copies would drift |
| §7 open session | The in-progress session is mirrored into `meta`, not written as a provisional row updated by the heartbeat | A provisional row would also have to be deleted whenever the noise floor discarded the session — a second way for store and tracker to disagree |
| §4 suspend | Requested with `RegisterSuspendResumeNotification`, not by waiting for a broadcast | A message-only window receives no broadcast messages, so `WM_POWERBROADCAST` never arrives on its own |
| §3 capture | A foreground event for an unchanged (exe, title) is dropped, not sent | Returning from an ignored shell surface raises a foreground event. Sending it would split the very session the ignore list exists to keep whole |
| §4 idle | After an idle close, the run loop re-opens the session itself when input returns | Coming back from idle changes no window and renames nothing, so no hook fires. Without this the rest of the day goes unrecorded |
| §4 audio | The probe matches on executable name, not process id | Chromium renders audio from a separate utility process, so the tab playing a video has a different pid from the window in front of you |
| §2 threads | Presence and heartbeat run on the collector's `recv_timeout` loop; only the resolver gets a thread | A timer thread would need a lock around the `Tracker`. The resolver is separate because building the title index takes ~140 ms |
| §8 IPC | Named pipe server is single-instance and blocking, not overlapped | One GUI talking to one daemon has no concurrency to serve; overlapped I/O would roughly triple the code for a wire format that fits in 60 lines |
| §8 IPC | Shutdown dials the pipe itself to unblock a pending `ConnectNamedPipe` | The same idea as `pump::stop` posting a message to unblock `GetMessage` — the only way to make a blocking wait return early is to give it what it is waiting for |
| §8 IPC | `resolve_now` and the tray's pause menu both post onto channels the collector and resolver already read (`Raw`, a new `ResolverMsg`) rather than IPC touching either directly | Keeps the collector the only thing that assembles sessions and the resolver the only thing that reads browser history, regardless of which of the three callers — tray, timer, or now IPC — is asking |
| §2 architecture | `toki.exe` lives at `crates/toki-gui/` (the Tauri/Rust shell) with the Svelte app nested under `crates/toki-gui/ui/`, rather than the default `create-tauri-app` layout of frontend-at-root plus a `src-tauri/` child | Keeps every workspace member under `crates/`, matching `toki-core`/`toki-store`/`toki-daemon` |
| §7 storage | `db_path()` moved into `toki-store`; `toki-daemon::paths::db_path` delegates to it instead of building the path itself | The GUI needs to open the exact file the daemon writes — a second copy of the path logic in `toki-gui` is precisely the drift `toki-store` was split out to prevent (see the Phase 0 deviation on storage being its own crate) |
| §9 visual system | Dark-mode hex values (`--bg`, `--surface`, `--border`, the three ink tokens) are Toki's own defaults, not values DESIGN.md specifies | The design commits only to "a dark set exists and follows `AppsUseLightTheme`" — it never chose the actual hexes, so these are a starting point pending the confirmation the design already flags as outstanding |

---

## Open questions

- Title→domain *hit* rate over a full day of real browsing. The first sample is 100% of 1,
  which says nothing yet. A day of Zen plus three Chrome profiles is the number that counts.
- Whether 2s, 5min and 2h are the right noise floor, idle threshold and audio cap. All three
  are still guesses; a week of data is what settles them.
- Opening the tray menu hands the foreground to the taskbar, which is `explorer.exe`. Under
  two seconds the noise floor absorbs it; hold the menu open longer and the day gains a short
  "Windows Explorer" row. `explorer.exe` cannot go on the ignore list — it is also every file
  window — so the fix, if it is worth one, is ignoring by window class rather than by
  executable.
- Does the version resource name every app worth naming? `UiPath.Studio.Project` showed up
  as its own row for a dialog — real, distinct, and exactly the case aliasing exists for.

### Measured: merged index ambiguity (2026-08-27)

`toki-daemon index` over all five profiles on this machine:

```
Zen      12550 rows    74 ms
Chrome    3220 rows    24 ms   (Default)
Chrome     715 rows     9 ms   (Profile 1)
Chrome    1406 rows    11 ms   (Profile 2)
Helium    1553 rows    12 ms
---
19444 rows  9955 distinct titles  225 ambiguous (2.3%)  built in 136 ms
```

This corrects an earlier worry in this file that merging profiles could only make collisions
worse. It did not: 2.3% merged, against 9.6% for Chrome's `Default` profile alone. Extra
profiles contribute far more unique titles than colliding ones, so the rate fell. 136 ms per
5-minute pass is a rounding error against the budget.

### Measured: release daemon footprint (2026-08-29)

`toki-daemon run`, release profile, a few minutes after start:

```
working set   9.95 MB
private       1.79 MB
threads       6
handles       146
CPU           0.03 s
binary        1.3 MB
```

Private bytes is the figure the sub-5 MB target was about, and it clears it. Working set
counts shared pages from `ntdll`, `user32`, `combase` and the rest, which every process on
the machine is already paying for. Six threads, not three: the runtime adds its own. The
number to re-measure is private bytes after a full day, when the app map and the SQLite page
cache have grown to their real size.

### Measured: first end-to-end capture (2026-08-29)

Five sessions across Chrome and UiPath Studio in the first few minutes:

- Display names came from the version resource — "Google Chrome", "Windows Explorer",
  "UiPath Studio" — not executable paths.
- A Chrome tab switch produced a separate row with a separate caption, confirming the
  `EVENT_OBJECT_NAMECHANGE` hook is what makes per-site tracking possible.
- Both Chrome rows resolved to `drive.google.com` off captions alone.
- Crash recovery closed a session left dangling by a killed daemon, at the last heartbeat,
  30 s after it started.

### Measured: Phase 4 build (2026-08-29)

```
binary        1.5 MB      (1.3 MB before tracing)
private       2.14 MB     detached, minutes after start
working set  13.80 MB
threads       6           handles 202
```

Verified live:

- Autostart written and read back — `HKCU\...\Run\Toki` = `"...\toki-daemon.exe" run`;
  `off` twice in a row is still success, and a bad argument exits non-zero.
- Tray icon accepted by the shell — `HKCU\Control Panel\NotifyIconSettings` holds our
  executable with tooltip `Toki - tracking`. New icons land in the overflow flyout on
  Windows 11 until dragged out.
- The pump's window is top-level, invisible, `WS_EX_TOOLWINDOW` — so `WM_ENDSESSION` can
  reach it without it appearing in the taskbar or Alt-Tab.
- Logs written to `%LOCALAPPDATA%\Toki\logs\toki.2026-08-29.log`, mirrored to the console
  when there is one and file-only when there is not.
- Started detached, the daemon's console count went *down*: it let go of the console
  Windows gave it, which is what the Run key entry would otherwise leave on the taskbar.

Still needs a human: the tray menu itself, and a pause running to its end.

### Verified: Phase 5 IPC (2026-08-29)

Round-tripped against a live release-build daemon from a separate process
(`scripts/ipc-probe.ps1`, `System.IO.Pipes.NamedPipeClientStream`):

```
sent: {"type":"get_state"}
recv: {"paused":false,"until_ms":null}
sent: {"type":"set_pause","paused":true,"for_ms":60000}
recv: {"paused":true,"until_ms":1787998303430}
sent: {"type":"get_state"}
recv: {"paused":true,"until_ms":1787998303430}
sent: {"type":"set_pause","paused":false}
recv: {"paused":false,"until_ms":null}
sent: {"type":"resolve_now"}
recv: {"paused":false,"until_ms":null}
```

The second `get_state` reading back the pause set by the previous call confirms the round
trip actually reached the collector — `Shared` is written only from `Collector::pause` /
`unpause`, never guessed at by the IPC thread. The daemon's own log shows the same two
transitions (`paused minutes=1`, `resumed`), timed to the same second as the pipe replies.

Five sequential connections on the same pipe path, each arriving while the server was
blocked in `ConnectNamedPipe` waiting for the next one, is also the same unblock the
shutdown path relies on (`ipc::stop` dials itself) — indirect but repeated evidence that a
dial reliably wakes a pending accept.

Not yet exercised: the shutdown path itself end to end. `taskkill` without `/F` against a
console process started from Git Bash did not deliver a clean-close signal on this machine,
for both a pre-existing (pre-Phase-5) daemon and the new build alike — a test-harness
limitation, not a regression, since it reproduces identically on the unmodified binary.

### Verified: Phase 6 GUI scaffold + Today view (2026-08-29)

Scaffolded with `create-tauri-app` (`svelte-ts` template, Tauri v2, bun), then relocated: the
Rust/Tauri half to `crates/toki-gui/` and the SvelteKit app (adapter-static, SPA mode) to
`crates/toki-gui/ui/`. Two path gotchas worth recording so they are not re-discovered:

- `tauri.conf.json`'s `frontendDist` and the `beforeDevCommand`/`beforeBuildCommand` hooks
  resolve relative to the config file's own directory (`crates/toki-gui/`), not the
  workspace root — `frontendDist` is `ui/build`, not `../ui/build`, since `ui/` is a child.
- `bun --cwd <path>` silently misparses when followed by `run <script>`; `--cwd=<path>`
  (with `=`) is required for the hook commands.

`cargo build`/`clippy`/`test` all clean across the now-11-crate workspace. `today_totals`
opens `toki-store::db_path()` read-only and calls the existing `day_totals` query verbatim —
no new SQL. Ran the built `target/release/toki.exe` against the live database (screenshot,
not just a build check): total "1h 58m", nine apps ranked by time, dark theme selected
correctly via `AppsUseLightTheme` with no light-mode flash, and clicking a browser row
(`Zen`) expanded real resolved domains plus an "Unresolved" fallback row for the sessions
still in the resolver's queue — confirming `today_totals`'s grouping-by-domain logic against
actual pending/resolved data, not a fixture.

The categorical palette (`app.css`) is the dataviz skill's default ramps with the blue slot
dropped entirely — not just the exact accent hex, per DESIGN.md's "blue... would read as
selected" — leaving 7 hues, revalidated with `scripts/validate_palette.js` against Toki's
own light (`#ffffff`) and dark (`#0a0a0a`) surfaces rather than trusting the reference
palette's own validation, which was run against different surface colors.

Not yet built: Week view, Settings (alias editor, ignore list, idle threshold, pause), and
first-run-opens-once. Settings' idle threshold also has no daemon-side home yet — `Tracker`
takes `Config::default()` today with no persistence or IPC message to change it, which is a
gap below the GUI, not just in it.
