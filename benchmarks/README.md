# Benchmarks

Measured on the author's machine against real data. Rerun after each change to the resolver
and add a row; nothing here is synthetic unless it says so.

- `toki-daemon bench [--hours N] [--label L] [--out results/resolver.jsonl]` times the real
  resolver on a copy of the live database (taken with `VACUUM INTO`; the live file is only
  read). Use a release build.
- `python -I scripts/bench/resolution_study.py --label L` compares naming methods on copies
  of `toki.db` and the browser histories, and writes `results/<date>-<label>.json`.

## Resolver cost per pass

One pass over the last 24 h of browser sessions (`--hours 24`), release build, Windows 11.
Five history sources: Zen, Chrome ×3 profiles, Helium.

| Version | Written to disk | Time | Working set (before → peak) | Hit rate |
|---|---|---|---|---|
| step 0, 2026-10-14 (copy every file, index every title) | 43.7 MB | 103 ms | 11.8 → 22.6 MB | 84.1% |

At ~70 passes a day, step 0 writes about 3 GB/day to the SSD.

## How well each method names browser time

193 h of browser sessions (18,918), 26,414 history URLs, 47,347 visits. "Agrees" compares
with the domain the daemon recorded a few minutes after each session.

| Method | Named | Agrees with recorded |
|---|---|---|
| Recorded (resolved ~5 min after the session) | 70.0% | — |
| Every title in history, run today | 70.3% | 98.9% |
| Same, as if run 5 min / 6 h / 72 h after the session | 70.3% | 99.8% / 99.6% / 99.4% |
| Latest visit with the title at or before the session's end | 70.3% | 99.8% |
| Titles only, map of 30 well-known sites | 33.6% | 99.8% |
| Titles only, map learned from history (held-out half) | 42.5% | 98.6% |

How far back the naming visit lies, by share of named time: half within 7 minutes, 90%
within 1.75 h, 95% within 10 h, 99% within 27 days. A short lookback window would lose
names, so the planned resolver streams all visits and keeps only those whose title is
pending.

## Findings that shaped the design

- Running a pass hours late costs almost nothing in accuracy (99.8% → 99.4% agreement at
  72 h), so passes can be rare.
- Chrome holds its history file under an exclusive lock while it runs. A plain read-only
  open fails; a `nolock=1` open reads it in place in 6 ms.
- Under exclusive locking SQLite does not bump the file change counter on commit (stayed
  at 1 across five commits in a test, and Chrome's live counter did not move in an hour).
  The file's last-write time does change on every commit, with the handle held open, so
  the consistency check for a lock-free read uses write time and size instead.
- Zen (Gecko, WAL) allows an ordinary read-only open while it runs: 14 ms, no copy.
