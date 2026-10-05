#!/usr/bin/env bash
# Install Toki for the current user, with no root: binaries to ~/.local/bin, a launcher
# entry and icon, then autostart at login and start tracking now. Run it from the release
# tarball, or from a checkout after `tauri build` (it looks in target/release too).
# Re-running it upgrades in place.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
src="$here"
[ -x "$src/toki-daemon" ] || src="$here/../target/release"
icons="$here/icons"
[ -d "$icons" ] || icons="$here/../crates/toki-gui/icons"

bin="$HOME/.local/bin"
share="${XDG_DATA_HOME:-$HOME/.local/share}"

install -Dm755 "$src/toki-daemon" "$bin/toki-daemon"
install -Dm755 "$src/toki" "$bin/toki"
install -Dm644 "$icons/32x32.png" "$share/icons/hicolor/32x32/apps/toki.png"
install -Dm644 "$icons/128x128.png" "$share/icons/hicolor/128x128/apps/toki.png"
install -Dm644 /dev/stdin "$share/applications/toki.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Toki
Comment=Local-first screen-time tracker
Exec=$bin/toki
Icon=toki
Categories=Utility;
StartupWMClass=toki
DESKTOP

"$bin/toki-daemon" autostart on
# Restart rather than start, so an upgrade takes effect immediately.
systemctl --user restart toki-daemon.service
echo "Toki is installed and tracking. Open it from your launcher, or run: toki"
