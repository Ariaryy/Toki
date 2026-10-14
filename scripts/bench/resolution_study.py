"""How well each way of naming browser sessions works, measured on this machine's data.

Never touches the live files: toki.db is read through SQLite's backup API into a scratch
copy, and each browser history is copied (with its journal or WAL) the way the daemon does,
into a fresh temp directory that is removed afterwards.

Methods compared, all by share of browser time named:
  recorded   what the daemon actually stored, resolved a few minutes after each session
  global     the daemon's current method run today: every title in history, most recent
             visit wins
  global@d   the same method as if the pass had run d after the session ended, using only
             visits that existed by then (visits carry timestamps, so this is reconstructed)
  asof(L)    plan 6: the latest visit with a matching title in [end - L, end + slack]. It
             reads the past as it was, so the pass time does not matter
  titles     no history: shipped map of well-known suffixes, and a map learned from history

Run:  python -I scripts/bench/resolution_study.py [--label NAME] [--out DIR]
"""

import argparse
import bisect
import collections
import datetime as dt
import json
import os
import re
import shutil
import sqlite3
import sys
import tempfile
import time

WEBKIT_OFFSET_US = 11_644_473_600_000_000
SUFFIXES = {
    0: [" - Google Chrome", " - Helium", " - Chromium"],  # Chromium
    1: [" — Zen Browser", " — Mozilla Firefox", " - Mozilla Firefox"],  # Gecko
}
SLACK_MS = 2 * 60_000
LOOKBACKS_H = [0.5, 1, 6, 24, 24 * 7, None]
DELAYS_MIN = [5, 30, 120, 360, 1440, 4320]
SIDECARS = ["-wal", "-shm", "-journal"]

GENERIC = {
    "x": "x.com", "youtube": "youtube.com", "github": "github.com", "chatgpt": "chatgpt.com",
    "instagram": "instagram.com", "google search": "google.com", "linkedin": "linkedin.com",
    "google gemini": "gemini.google.com", "gemini": "gemini.google.com", "figma": "figma.com",
    "google drive": "drive.google.com", "claude": "claude.ai", "gmail": "mail.google.com",
    "reddit": "reddit.com", "wikipedia": "wikipedia.org", "stack overflow": "stackoverflow.com",
    "netflix": "netflix.com", "amazon.in": "amazon.in", "whatsapp": "web.whatsapp.com",
    "discord": "discord.com", "notion": "notion.so", "google docs": "docs.google.com",
    "google sheets": "docs.google.com", "twitch": "twitch.tv", "spotify": "open.spotify.com",
    "medium": "medium.com", "facebook": "facebook.com", "outlook": "outlook.live.com",
    "perplexity": "perplexity.ai", "vercel": "vercel.com",
}
SEP = re.compile(r"\s[-–—|·:/]\s")


# ------------------------------------------------------------------ normalisation (as Rust)

def strip_unread(t):
    m = re.match(r"^\((\d+)\) (.*)$", t, re.S)
    return m.group(2) if m else t


def normalize_title(caption, engine):
    t = caption.strip()
    for s in SUFFIXES[engine]:
        if t.endswith(s):
            t = t[: -len(s)]
            break
    return strip_unread(t).strip()


def is_page(caption, engine):
    t = caption.strip()
    return any(t.endswith(s) and t[: -len(s)] for s in SUFFIXES[engine])


def host_of(url):
    for p in ("https://", "http://"):
        if url.startswith(p):
            rest = url[len(p):]
            break
    else:
        return None
    auth = re.split(r"[/?#]", rest, maxsplit=1)[0].rsplit("@", 1)[-1]
    if auth.startswith("["):
        end = auth.find("]")
        host = auth[: end + 1] if end >= 0 else ""
    else:
        host = auth.split(":")[0]
    host = host.strip().lower()
    return (host[4:] if host.startswith("www.") else host) or None


def suffix_token(page):
    return SEP.split(page)[-1].strip().lower() if page else None


# ------------------------------------------------------------------ snapshots

def history_sources():
    la, ad = os.environ["LOCALAPPDATA"], os.environ["APPDATA"]
    roots = [
        ("Zen", 1, os.path.join(ad, "zen", "Profiles"), "places.sqlite"),
        ("Firefox", 1, os.path.join(ad, "Mozilla", "Firefox", "Profiles"), "places.sqlite"),
        ("Chrome", 0, os.path.join(la, "Google", "Chrome", "User Data"), "History"),
        ("Helium", 0, os.path.join(la, "imput", "Helium", "User Data"), "History"),
        ("Chromium", 0, os.path.join(la, "Chromium", "User Data"), "History"),
    ]
    out = []
    for label, engine, root, name in roots:
        if not os.path.isdir(root):
            continue
        for child in sorted(os.listdir(root)):
            p = os.path.join(root, child, name)
            if os.path.isfile(p):
                out.append((label, engine, p))
    return out


def snapshot_history(src, dst):
    shutil.copyfile(src, dst)
    for ext in SIDECARS:
        if os.path.isfile(src + ext):
            shutil.copyfile(src + ext, dst + ext)


def snapshot_toki(dst):
    src = os.path.join(os.environ["LOCALAPPDATA"], "Toki", "toki.db")
    live = sqlite3.connect(f"file:{src.replace(os.sep, '/')}?mode=ro", uri=True)
    copy = sqlite3.connect(dst)
    live.backup(copy)
    live.close()
    return copy


# ------------------------------------------------------------------ data

def load_sessions(conn):
    rows = conn.execute(
        """SELECT s.start_utc_ms, s.end_utc_ms, t.title, d.domain, s.resolve_state, a.engine
           FROM sessions s JOIN apps a ON a.id = s.app_id
           LEFT JOIN titles t ON t.id = s.title_id
           LEFT JOIN domains d ON d.id = s.domain_id
           WHERE a.engine IS NOT NULL AND t.title IS NOT NULL"""
    ).fetchall()
    return [
        dict(start=r[0], end=r[1], ms=max(0, r[1] - r[0]), caption=r[2],
             recorded=(r[3].lower() if r[3] else None), state=r[4], engine=r[5],
             page=normalize_title(r[2], r[5]))
        for r in rows
    ]


def load_visits(path, engine):
    conn = sqlite3.connect(path)
    if engine == 0:
        q = """SELECT v.visit_time, u.url, u.title FROM visits v JOIN urls u ON u.id = v.url
               WHERE u.title IS NOT NULL AND u.title <> ''"""
    else:
        q = """SELECT v.visit_date, p.url, p.title FROM moz_historyvisits v
               JOIN moz_places p ON p.id = v.place_id
               WHERE p.title IS NOT NULL AND p.title <> ''"""
    out = []
    for t, url, title in conn.execute(q):
        host = host_of(url)
        if not host or not t:
            continue
        ms = (t - WEBKIT_OFFSET_US) // 1000 if engine == 0 else t // 1000
        out.append((ms, strip_unread(title.strip()).strip(), host))
    urls = conn.execute("SELECT count(*) FROM " + ("urls" if engine == 0 else "moz_places")).fetchone()[0]
    conn.close()
    return out, urls


# ------------------------------------------------------------------ methods

class VisitIndex:
    """Every visit, grouped by normalised title and sorted by time."""

    def __init__(self, visits):
        by = collections.defaultdict(list)
        for ms, title, host in visits:
            by[title].append((ms, host))
        self.by = {k: sorted(v) for k, v in by.items()}
        self.times = {k: [m for m, _ in v] for k, v in self.by.items()}

    def latest(self, title, upto, since=None):
        times = self.times.get(title)
        if not times:
            return None
        i = bisect.bisect_right(times, upto) - 1
        if i < 0:
            return None
        ms, host = self.by[title][i]
        if since is not None and ms < since:
            return None
        return host, ms


def run_methods(sessions, vi, train_cut):
    methods = collections.OrderedDict()
    methods["recorded"] = lambda s: s["recorded"]
    methods["global (today)"] = lambda s: _host(vi.latest(s["page"], 1 << 62))
    for d in DELAYS_MIN:
        methods[f"global @ +{_fmt_min(d)}"] = (
            lambda s, d=d: _host(vi.latest(s["page"], s["end"] + d * 60_000)))
    for h in LOOKBACKS_H:
        name = f"asof, lookback {_fmt_h(h)}"
        methods[name] = (lambda s, h=h: _host(vi.latest(
            s["page"], s["end"] + SLACK_MS,
            None if h is None else s["end"] - int(h * 3_600_000))))
    learned = learn_map([s for s in sessions if s["start"] < train_cut and s["recorded"]])
    methods["titles: generic map"] = lambda s: GENERIC.get(suffix_token(s["page"])) if is_page(s["caption"], s["engine"]) else None
    methods["titles: learned map (test half)"] = lambda s: learned.get(suffix_token(s["page"])) if is_page(s["caption"], s["engine"]) else None
    return methods


def learn_map(train):
    m = collections.defaultdict(collections.Counter)
    for s in train:
        k = suffix_token(s["page"])
        if k:
            m[k][s["recorded"]] += s["ms"]
    return {k: v.most_common(1)[0][0] for k, v in m.items()}


def _host(hit):
    return hit[0] if hit else None


def _fmt_min(m):
    return f"{m}m" if m < 60 else f"{m // 60}h"


def _fmt_h(h):
    if h is None:
        return "unlimited"
    return f"{int(h * 60)}m" if h < 1 else (f"{int(h)}h" if h < 24 else f"{int(h // 24)}d")


def score(sessions, fn, reference):
    total = sum(s["ms"] for s in sessions) or 1
    named = agree = disagree = 0
    for s in sessions:
        got = fn(s)
        if not got:
            continue
        named += s["ms"]
        ref = reference(s)
        if ref:
            if ref == got:
                agree += s["ms"]
            else:
                disagree += s["ms"]
    both = agree + disagree
    return dict(named=named / total, agree_with_ref=(agree / both) if both else None,
                disagree_share=disagree / total)


def lookback_needed(sessions, vi):
    """Gap between a session's end and the visit that names it, for the asof method."""
    gaps = []
    for s in sessions:
        hit = vi.latest(s["page"], s["end"] + SLACK_MS)
        if hit:
            gaps.append(((s["end"] - hit[1]) / 3_600_000, s["ms"]))
    gaps.sort()
    total = sum(w for _, w in gaps) or 1
    out, acc, qs = {}, 0, [0.5, 0.75, 0.9, 0.95, 0.99]
    for g, w in gaps:
        acc += w
        while qs and acc / total >= qs[0]:
            out[f"p{int(qs.pop(0) * 100)}"] = round(max(g, 0), 2)
    return out


# ------------------------------------------------------------------ schedule

FLOOR_MS, SETTLE_MS, CEILING_MS = 30 * 60_000, 90_000, 6 * 3_600_000


def browsing_ends(conn):
    """When a stretch of browsing ended: a browser session followed by a non-browser one,
    or by a gap (idle, lock, sleep). What the daemon sends `BrowsingEnded` for."""
    rows = conn.execute(
        """SELECT s.start_utc_ms, s.end_utc_ms, a.engine IS NOT NULL
           FROM sessions s JOIN apps a ON a.id = s.app_id ORDER BY s.start_utc_ms"""
    ).fetchall()
    ends = []
    for (start, end, browser), nxt in zip(rows, rows[1:] + [None]):
        if browser and (nxt is None or not nxt[2] or nxt[0] - end > 60_000):
            ends.append(end)
    return ends


def simulate_passes(ends, start, stop):
    """Passes that read history under the daemon's schedule between `start` and `stop`,
    GUI opens aside."""
    passes, last, due, i = [], start, start + CEILING_MS, 0
    ends = [e for e in ends if start <= e < stop]
    while due < stop:
        while i < len(ends) and ends[i] < due:
            if ends[i] - last >= FLOOR_MS:
                due = min(due, ends[i] + SETTLE_MS)
            i += 1
        # A pass with nothing pending stops at its first query and opens no history.
        if any(last < e <= due for e in ends[max(0, i - 50):i]):
            passes.append(due)
        last, due = due, due + CEILING_MS
    return passes


# ------------------------------------------------------------------ main

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--label", default="baseline")
    ap.add_argument("--out", default=os.path.join(os.path.dirname(__file__), "..", "..", "benchmarks", "results"))
    args = ap.parse_args()

    work = tempfile.mkdtemp(prefix="toki-study-")
    try:
        t0 = time.perf_counter()
        toki = snapshot_toki(os.path.join(work, "toki.db"))
        sessions = [s for s in load_sessions(toki) if s["ms"] > 0]
        ends = browsing_ends(toki)
        toki.close()

        visits, sources, total_urls, history_bytes = [], [], 0, 0
        for i, (label, engine, path) in enumerate(history_sources()):
            size = os.path.getsize(path) + sum(
                os.path.getsize(path + e) for e in SIDECARS if os.path.isfile(path + e))
            dst = os.path.join(work, f"h{i}")
            snapshot_history(path, dst)
            v, urls = load_visits(dst, engine)
            visits += v
            total_urls += urls
            history_bytes += size
            sources.append(dict(label=label, bytes=size, urls=urls, visits=len(v)))
        vi = VisitIndex(visits)

        sessions.sort(key=lambda s: s["start"])
        now_ms = int(time.time() * 1000)
        windows = {"all": sessions,
                   "last 30 days": [s for s in sessions if s["start"] >= now_ms - 30 * 86_400_000]}
        train_cut = sessions[len(sessions) // 2]["start"]
        methods = run_methods(sessions, vi, train_cut)
        recorded = methods["recorded"]

        results = {}
        for wname, ss in windows.items():
            res = {}
            for mname, fn in methods.items():
                sub = ss if "test half" not in mname else [s for s in ss if s["start"] >= train_cut]
                res[mname] = score(sub, fn, recorded)
            results[wname] = dict(hours=round(sum(s["ms"] for s in ss) / 3_600_000, 1),
                                  sessions=len(ss), methods=res)

        # Memory proxy: rows a pass holds. Today: every titled URL. asof: visits since the
        # oldest pending session minus the lookback, at a 30-minute pass interval.
        per_30m = len([v for v in visits if v[0] >= now_ms - 30 * 86_400_000]) / (30 * 48)
        per_h = per_30m * 2
        rows_held = {"global (today)": total_urls}
        for h in LOOKBACKS_H[:-1]:
            rows_held[f"asof, lookback {_fmt_h(h)}"] = round(per_30m + per_h * h)

        days = 30
        passes = simulate_passes(ends, now_ms - days * 86_400_000, now_ms)
        schedule = dict(days=days, browsing_ends=len([e for e in ends if e >= now_ms - days * 86_400_000]),
                        history_reads_per_day=round(len(passes) / days, 1),
                        measured_before_per_day="38-105 (5-minute timer, daemon logs Oct 7-12)")

        report = dict(
            label=args.label,
            taken=dt.datetime.now().astimezone().isoformat(timespec="seconds"),
            sqlite=sqlite3.sqlite_version,
            history=dict(sources=sources, bytes_per_full_copy=history_bytes,
                         urls=total_urls, visits=len(visits)),
            lookback_needed_hours=lookback_needed(windows["all"], vi),
            rows_held_per_pass=rows_held,
            schedule=schedule,
            results=results,
            seconds=round(time.perf_counter() - t0, 1),
        )
        os.makedirs(args.out, exist_ok=True)
        path = os.path.join(args.out, f"{dt.date.today()}-{args.label}.json")
        with open(path, "w", encoding="utf-8") as f:
            json.dump(report, f, indent=2)
        print_report(report)
        print(f"\nsaved {os.path.normpath(path)}")
    finally:
        shutil.rmtree(work, ignore_errors=True)


def print_report(r):
    h = r["history"]
    print(f"{r['label']}  {r['taken']}  sqlite {r['sqlite']}")
    print(f"history: {len(h['sources'])} sources, {h['bytes_per_full_copy'] / 1e6:.1f} MB per full copy, "
          f"{h['urls']} urls, {h['visits']} visits")
    print("lookback needed (hours, by share of named time):", r["lookback_needed_hours"])
    print("rows held per pass:", r["rows_held_per_pass"])
    print("schedule:", r.get("schedule"))
    for wname, w in r["results"].items():
        print(f"\n{wname}: {w['hours']} h of browser time, {w['sessions']} sessions")
        print(f"  {'method':36} {'named':>7} {'agrees w/ recorded':>19} {'disagrees':>10}")
        for m, s in w["methods"].items():
            a = "" if s["agree_with_ref"] is None else f"{s['agree_with_ref']:.1%}"
            print(f"  {m:36} {s['named']:7.1%} {a:>19} {s['disagree_share']:10.2%}")


if __name__ == "__main__":
    sys.exit(main())
