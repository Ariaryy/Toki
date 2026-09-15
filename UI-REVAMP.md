# Toki — UI revamp plan

Branch: `ui-revamp`. Experimental. Nothing here is committed to DESIGN.md until it has been
seen running on the real desktop.

The premise: `toki.exe` is launched on demand and exits fully on close, so the GUI may spend
resources the daemon may not. The 5 MB resident target belongs to `toki-daemon`. This plan
spends the GUI's budget on density and legibility, not on effects.

---

## 1. Why revamp

Not taste. Six concrete failures, each traceable to a line of the current code.

| Problem | Evidence |
|---|---|
| A third of the window is empty | `max-width: 640px` on all three routes, inside a 960×720 window |
| Today is nearly empty | `routes/+page.svelte` is 62 lines — one number, one list. No hour distribution, no comparison, no current state |
| The week strip is inert | The seven bars are `<div>`s. No drill-down, and Today has no date navigation at all while Week pages back to `earliest_date` |
| Settings will not scale | `all_apps` renders as an unbounded flat list with no search, sort, or filter. It is already long and only grows |
| The accent does two contradictory jobs | `--accent` marks the selected tab *and* fills the week bars. DESIGN.md §9 reserved blue for "selected" precisely so it would not read as data |
| Collected data is never shown | `sessions.start/end/presence`, `last_heartbeat`, `open_session`, `pending_count`, `resolution_by_app`, `earliest_date` — all stored, none surfaced |

The last row is the important one. Toki already records enough to answer *when* during the day
something happened, and the UI only ever answers *how much*. The revamp is mostly about
spending data that is already on disk.

---

## 2. Reference

Direction taken from Linear's published system ("midnight precision instrument"), via Refero's
`DESIGN.md` extract. Chosen over the two alternatives because Toki is a private instrument read
in short bursts, often late — not a SaaS dashboard (Dub, the current §9 reference) and not a
report to be read at leisure (Steep, editorial serif analytics).

What is taken: near-black canvas, elevation by hairline border rather than shadow, Inter in a
narrow 400–590 weight band with tight tracking, a three-value radius vocabulary, one chromatic
accent spent once per view, 4px spacing base.

What is **not** taken:

- **Acid lime `#e4f222` as accent.** Toki keeps `#2563eb`, brightened for the darker canvas.
  The categorical data palette was deliberately built to exclude blue so blue could mean
  "selected" (§9). Swapping the accent would either break that rule or force the palette to be
  re-derived and re-validated for CVD.
- **"No additional chromatic accents."** That rule governs chrome, not encoding. Toki's primary
  view is a per-app breakdown needing 6–10 distinguishable hues. The existing palette stays as
  a separate, explicitly-scoped axis.
- **0.5px borders, unconditionally.** See §7 — this is a Mac assumption.

Refero's dashboard guidance also rules out one trend worth naming: bento grids do not suit
data-dense dashboards, because asymmetric tiles defeat uniform scanning. The layout below is a
regular grid, not a bento.

---

## 3. Tokens

Replaces the `:root` block in `ui/src/app.css`. Dark is now the designed-for case; light is
maintained, not merely tolerated.

```
                        dark              light
--bg                    #08090a           #ffffff
--surface               #0f1011           #ffffff
--surface-raised        #161718           #fafafa
--border                #23252a           #e5e5e5
--border-strong         #383b3f           #d4d4d4
--text-primary          #ffffff           #171717
--text-secondary        #d0d6e0           #4b4b4b
--text-muted            #8a8f98           #7a7a7a
--text-faint            #62666d           #9a9a9a
--accent                #4d7cfe           #2563eb
```

- **Radii** collapse from five values to four: `2px` badge, `6px` control, `12px` card, `pill`.
  `--radius-card-lg` (16px) is dropped; it is used nowhere that 12px would not serve.
- **Elevation** is `box-shadow: inset 0 0 0 1px var(--border)` on cards. No drop shadows on
  containers. Popovers and tooltips are the sole exception and get `0 4px 32px rgba(8,9,10,.6)`.
- **Spacing** on a 4px base: `4 8 12 16 24 32 48`. Card padding 16, element gap 8, section gap
  24. Linear's 96px section rhythm is scaled for a 1200px marketing page and would eat this
  window, the same reasoning §9 already applied to display type.
- **Type** moves from two static Inter weights to `@fontsource-variable/inter`, with
  `font-feature-settings: "cv01", "ss03", "zero"` and `font-variant-numeric: tabular-nums` set
  globally. Weights 400 / 510 / 590 only — nothing at 700+. Tracking `-0.012em` at 20–32px,
  `-0.022em` above.
- **Numerals.** The Today hero currently sets `proportional-nums`. It becomes tabular, because
  the figure updates while the window is open and proportional digits make it twitch.
- **Data palette** keeps all seven validated hues. Only luminance is re-checked against
  `#08090a`, which is close enough to the current `#0a0a0a` that most values should survive
  unchanged. Re-run the dataviz CVD/contrast check before committing; do not eyeball it.

---

## 4. Shell

Top tabs become a **220px left rail**, and the window grows from 960×720 to **1120×760**
(`minWidth` 880) in `tauri.conf.json`. Content then gets ~875px, enough for a real two-column
grid, and the rail costs nothing the 640px column was not already wasting.

```
┌──────────────┬──────────────────────────────────────────┐
│  Toki     ●  │                                          │
│              │              content grid                │
│  Today       │                                          │
│  Week        │                                          │
│  Apps        │                                          │
│  Settings    │                                          │
│              │                                          │
│ ──────────── │                                          │
│ Tracking     │                                          │
│ Zen · 14m    │                                          │
└──────────────┴──────────────────────────────────────────┘
```

The rail footer is the part that earns it. Pause state and daemon liveness currently live only
on the Settings page, which means the app can be paused and every other view looks identical to
normal. The footer shows one of: `Tracking · <app> · <elapsed>`, `Paused — 12m left` (with a
resume button), or `Daemon not running`. That last state is the failure mode DESIGN.md §11 says
it fears most, and the GUI presently cannot show it.

`Apps` is a new route, split out of Settings — see §5.4.

---

## 5. Views

### 5.1 Today

Four regions, top-down.

1. **Metric strip** — four figures, no more: total today, delta vs yesterday (Charcoal with an
   arrow, never red/green — §9 holds), longest unbroken session, distinct apps used.
2. **Day timeline** (wide, left) — the flagship. A horizontal 24-hour strip with each session
   drawn as a segment in its app's series colour, idle gaps left empty, and sessions whose
   `presence` was held by the audio exemption hatched rather than solid. Hover gives app,
   domain, and range. This is the single view that answers "when", which Toki has never shown.
3. **Now** (narrow, right) — current app and elapsed, read from the `open_session` mirror in
   `meta`. No new IPC required.
4. **Ranked list** — full width, restyled: bar, name, duration, share %, expandable to domains.

Today also gains date navigation, bounded below by `earliest_date` and above by today.

### 5.2 Week

1. **Metric strip** — total, week-over-week delta, daily average, busiest day.
2. **Hourly heatmap**, 7 rows × 24 columns. The highest information density available from this
   schema, and the view that actually answers the question the dark palette exists for: how much
   of this is happening after midnight.
3. **Day bars become buttons** — clicking one navigates to Today for that date.
4. **Top-five trend** — small multiples, one sparkline per app across the seven days.
5. **Ranked list**, as today.

### 5.3 Settings

Reduced to settings, plus a diagnostics block that finally surfaces what §11 collects: schema
version, last heartbeat, pending resolutions, earliest recorded date, row count, retention
horizon. Theme, pause, and idle threshold stay as they are.

### 5.4 Apps (new)

The alias editor, rebuilt as a searchable, sortable table: name, alias, time over a selectable
range, share, ignore toggle. This is what Settings' flat list should have been, and moving it
out is what makes Settings readable again.

---

## 6. Backend

New work in `toki-store`, exposed through `toki-gui/src/lib.rs`. Every query must preserve the
`daily_rollup` UNION semantics that `day_totals` documents, or old days silently read as zero.

| Query | Feeds | Note |
|---|---|---|
| `day_sessions(date, offset)` | Today timeline | Clamped spans `{start, end, app, domain, presence}`. Raw sessions only |
| `hour_histogram(from, to, offset)` | Week heatmap | Sessions clamped into hour buckets. Raw sessions only |
| `daily_series(exe_path, from, to)` | Week trend | Must union the rollup |
| `totals_between(from, to)` | Apps table | Generalises `day_totals`; must union the rollup |
| `status()` | Rail footer, diagnostics | `last_heartbeat`, `open_session`, `pending_count`, `schema_version` |
| `earliest_date()` | Date nav bounds | Already exists, just needs exposing |

**The rollup constraint.** `daily_rollup` stores `(local_date, app_id, domain_id, seconds)` and
nothing else. Intra-day timing does not survive it. So the timeline and the heatmap are
structurally blind to anything older than six months, and must say so — "no minute-level detail
before <date>; totals only" — rather than drawing an empty day that looks like a day you did not
use the computer. The two queries marked *raw sessions only* return a flag for this; the UI is
required to render it.

---

## 7. Risks and deviations

| Risk | Call |
|---|---|
| **0.5px hairlines** | Linear can assume a 2× display. Windows at 100% DPI cannot, and a 0.5px border there rounds to nothing or to a muddy grey. Use 1px at a lower-contrast colour by default, and 0.5px only behind `@media (min-resolution: 1.5dppx)` |
| **`--disable-gpu` is set** | `tauri.conf.json` passes `--disable-gpu --disable-gpu-compositing`, worth ~70 MB. Animation is therefore CPU-rasterised: `transform` and `opacity` only, never `filter` or `backdrop-filter`. This is also why Mica/acrylic is out of scope |
| **Charting library** | Permitted, but recommended against. The four charts needed — horizontal timeline, 7×24 heatmap, day bars, sparklines — are rectangles positioned by arithmetic. A rendering library would own the DOM and fight the token system. Proposal: add `d3-scale` and `d3-time` (~15 KB, tree-shaken) for scale and tick maths only, and keep hand-rolled SVG. Raise this if the result does not look good enough |
| **DESIGN.md §9 is superseded** | §9 documents the Dub-derived system. It needs rewriting, and the deviations table needs a row. Do this at the end, against what shipped, not against this plan |
| **Window size change** | 960×720 → 1120×760 affects anyone with an existing window position. Acceptable for a personal tool pre-1.0 |
| **Verification** | CHECKLIST notes synthetic UI automation is unsafe on this machine. Every phase below needs a human look at the real window; a browser preview cannot confirm it, because `invoke` does not resolve outside Tauri |

---

## 8. Phases

Each phase builds, passes `clippy` and `cargo test`, and leaves the app usable.

- **A — Tokens and shell.** New `app.css`, variable Inter, rail, window size, motion primitives.
  No data changes. Every existing view still works, restyled.
- **B — Backend.** The six queries in §6, with tests, including the rollup-horizon flag. Views
  unchanged; verified through `cargo test` rather than the UI.
- **C — Today.** Metric strip, timeline, Now card, date navigation.
- **D — Week.** Heatmap, clickable days, trend sparklines.
- **E — Apps and Settings.** New route, searchable table, diagnostics block.
- **F — Polish.** Motion pass (150–200ms, `transform`/`opacity`, staggered rows capped at 20ms,
  all of it disabled under `prefers-reduced-motion`), focus rings, keyboard navigation for the
  timeline and heatmap, then the DESIGN.md §9 rewrite and CHECKLIST entries.

A is worth doing and looking at before B is written. If the direction is wrong, it is wrong
after one afternoon rather than after the queries exist.
