#!/usr/bin/env bash
# Screenshot every route of the stubbed preview into .shots/<name>.png.
# Window height is 856, not 760: headless Chrome spends 96px on window chrome, so this is
# what puts the *viewport* at the 1120x760 the mockups were drawn at.
# Usage: scripts/shots.sh [port] [suffix]
set -e
PORT=${1:-5177}
SUFFIX=${2:-}
CHROME="/c/Program Files/Google/Chrome/Application/chrome.exe"
OUT="$(cd "$(dirname "$0")/.." && pwd)/.shots"
mkdir -p "$OUT"
shoot() {
  local name=$1 path=$2
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars \
    --window-size=1120,856 --force-device-scale-factor=2 \
    --virtual-time-budget=9000 --force-prefers-reduced-motion \
    --screenshot="$(cygpath -w "$OUT/$name$SUFFIX.png")" \
    "http://127.0.0.1:$PORT$path" >/dev/null 2>&1
  echo "  $name$SUFFIX.png"
}
shoot today ""
shoot week "/week"
shoot apps "/apps"
shoot settings "/settings"
