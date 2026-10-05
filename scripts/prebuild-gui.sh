#!/usr/bin/env bash
# Linux twin of prebuild-gui.ps1, run by `cargo tauri build` via tauri.linux.conf.json.
# Builds the daemon where Tauri's externalBin expects it, then the frontend it embeds.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cargo build --release -p toki-daemon --manifest-path "$root/Cargo.toml"

triple="$(rustc -vV | sed -n 's/^host: //p')"
mkdir -p "$root/crates/toki-gui/binaries"
cp "$root/target/release/toki-daemon" "$root/crates/toki-gui/binaries/toki-daemon-$triple"

bun "--cwd=$root/crates/toki-gui/ui" run build
