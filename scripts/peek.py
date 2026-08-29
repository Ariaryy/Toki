"""Dump what the daemon has recorded. Read-only; safe to run while it is collecting."""

import os
import sqlite3
import sys

db = os.path.join(os.environ["LOCALAPPDATA"], "Toki", "toki.db")
conn = sqlite3.connect("file:{}?mode=ro".format(db.replace(os.sep, "/")), uri=True)

STATES = {0: "pending", 1: "done", 2: "not-browser", 3: "failed"}

rows = conn.execute(
    """
    SELECT s.id,
           datetime(s.start_utc_ms/1000, 'unixepoch', '+330 minutes'),
           (s.end_utc_ms - s.start_utc_ms) / 1000.0,
           COALESCE(a.detected_name, a.exe_path),
           a.engine,
           s.resolve_state,
           (SELECT domain FROM domains d WHERE d.id = s.domain_id),
           t.title
    FROM sessions s
    JOIN apps a ON a.id = s.app_id
    LEFT JOIN titles t ON t.id = s.title_id
    ORDER BY s.id DESC
    LIMIT ?
    """,
    (int(sys.argv[1]) if len(sys.argv) > 1 else 40,),
).fetchall()

print("{:>4}  {:19}  {:>7}  {:22}  {:12}  {:22}  {}".format(
    "id", "start (IST)", "secs", "app", "resolve", "domain", "title"))
for sid, start, secs, app, engine, state, domain, title in reversed(rows):
    print("{:>4}  {:19}  {:>7.1f}  {:22.22}  {:12}  {:22.22}  {:.60}".format(
        sid, start, secs, app, STATES.get(state, state), domain or "-", title or ""))

total = conn.execute("SELECT count(*), sum(end_utc_ms - start_utc_ms)/1000.0 FROM sessions").fetchone()
print("\n{} sessions, {:.1f}s recorded".format(total[0], total[1] or 0.0))
print("apps: " + ", ".join(
    "{}{}".format(name, " [browser]" if eng is not None else "")
    for name, eng in conn.execute(
        "SELECT COALESCE(detected_name, exe_path), engine FROM apps ORDER BY id")))
