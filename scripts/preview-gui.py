"""Serve the built GUI with a stubbed Tauri bridge, for screenshots.

The real `toki.exe` needs WebView2, the daemon and a populated database. This serves
`crates/toki-gui/ui/build` over HTTP with SPA fallback and injects a `__TAURI_INTERNALS__`
stub, so the actual Svelte components render against fixed sample data. It is a *layout and
token* check, not a correctness check — nothing here touches the real store, and the numbers
below are invented.

    python scripts/preview-gui.py [port]
"""

import http.server
import json
import sys
from datetime import date, timedelta
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "crates" / "toki-gui" / "ui" / "build"


def totals():
    """Shape mirrors `Total` in ui/src/lib/group.ts: one row per (app, domain)."""
    zen = [
        ("github.com", 64),
        ("docs.rs", 38),
        ("youtube.com", 31),
        ("news.ycombinator.com", 18),
        ("localhost", 10),
    ]
    rows = [
        {"exePath": "zen.exe", "appName": "Zen", "domain": d, "ms": m * 60_000}
        for d, m in zen
    ]
    flat = [
        ("Code.exe", "Code", 118),
        ("Discord.exe", "Discord", 34),
        ("WindowsTerminal.exe", "Terminal", 27),
        ("Spotify.exe", "Spotify", 18),
        ("Obsidian.exe", "Obsidian", 9),
        ("explorer.exe", "Explorer", 5),
    ]
    rows += [
        {"exePath": e, "appName": n, "domain": None, "ms": m * 60_000}
        for e, n, m in flat
    ]
    return rows


def week():
    # Monday of the current week, so the "today" bar lands on a real weekday.
    today = date.today()
    monday = today - timedelta(days=today.weekday())
    minutes = [408, 432, 302, 391, 475, 138, 224]
    labels = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
    days = []
    for i, (label, m) in enumerate(zip(labels, minutes)):
        d = monday + timedelta(days=i)
        future = d > today
        days.append(
            {
                "date": d.isoformat(),
                "label": label,
                "ms": 0 if future else m * 60_000,
                "isToday": d == today,
                "isFuture": future,
            }
        )
    top = [
        ("zen.exe", "Zen", "github.com", 610),
        ("zen.exe", "Zen", "youtube.com", 372),
        ("Code.exe", "Code", None, 760),
        ("Discord.exe", "Discord", None, 198),
        ("WindowsTerminal.exe", "Terminal", None, 171),
        ("Spotify.exe", "Spotify", None, 124),
        ("Obsidian.exe", "Obsidian", None, 82),
    ]
    return {
        "days": days,
        "top": [
            {"exePath": e, "appName": n, "domain": d, "ms": m * 60_000}
            for e, n, d, m in top
        ],
        "totalMs": sum(d["ms"] for d in days),
        "deltaMs": -135 * 60_000,
        "weeksAgo": 0,
        "hasEarlier": True,
    }


APPS = [
    ("zen.exe", "Zen", "Zen", False),
    ("Code.exe", "Code", None, False),
    ("Discord.exe", "Discord", None, False),
    ("WindowsTerminal.exe", "Terminal", "Terminal", False),
    ("Spotify.exe", "Spotify", None, False),
    ("Obsidian.exe", "Obsidian", None, False),
    ("explorer.exe", "Explorer", None, True),
    ("steamwebhelper.exe", "Steam", "Steam", False),
    ("UiPath.Studio.Project", "UiPath", "UiPath", False),
    ("LockApp.exe", "Lock screen", None, True),
]

DOMAINS = [
    ("github.com", "github.com", None),
    ("docs.rs", "docs.rs", None),
    ("youtube.com", "YouTube", "YouTube"),
    ("news.ycombinator.com", "Hacker News", "Hacker News"),
    ("localhost", "localhost", None),
]


def responses():
    return {
        "today_totals": totals(),
        "week_totals": week(),
        "all_apps": [
            {"exePath": e, "name": n, "alias": a, "ignored": g} for e, n, a, g in APPS
        ],
        "all_domains": [{"domain": d, "name": n, "alias": a} for d, n, a in DOMAINS],
        "idle_threshold_minutes": 5,
        "system_theme": "dark",
        "ipc_call": {"paused": False, "until_ms": None},
    }


def stub():
    return (
        "<script>window.__TAURI_INTERNALS__={invoke:function(c){"
        "var r=" + json.dumps(responses()) + ";"
        "return c in r?Promise.resolve(r[c]):Promise.reject('no stub for '+c);}};</script>"
    )


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=str(ROOT), **kw)

    def do_GET(self):
        path = ROOT / self.path.lstrip("/").split("?")[0]
        # SPA fallback: adapter-static emits one index.html and routes on the client, so
        # /week and /settings have no file of their own.
        if not path.is_file():
            html = (ROOT / "index.html").read_text(encoding="utf-8")
            body = html.replace("<head>", "<head>" + stub(), 1).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        super().do_GET()

    def log_message(self, *a):
        pass


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 5177
    print(f"serving {ROOT} on http://127.0.0.1:{port}")
    http.server.HTTPServer(("127.0.0.1", port), Handler).serve_forever()
