# Runs before `tauri build` (wired as tauri.conf.json's beforeBuildCommand, cwd
# crates/toki-gui/ — see CHECKLIST.md's Phase 6 note on paths resolving relative to the
# config file's own directory).
#
# Tauri's NSIS bundler copies externalBin sidecars into the installer from
# crates/toki-gui/binaries/toki-daemon-<target-triple>.exe, but nothing else produces that
# file: toki-daemon isn't a dependency of toki-gui, so a plain `cargo build -p toki-gui`
# never builds it. This builds it explicitly and copies it into place before the frontend
# build and the bundler run.

$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot/.."

cargo build --release -p toki-daemon --manifest-path "$root/Cargo.toml"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$triple = (rustc -vV | Select-String "^host: (.+)$").Matches[0].Groups[1].Value
$dest = Join-Path $root "crates/toki-gui/binaries"
New-Item -ItemType Directory -Force -Path $dest | Out-Null
Copy-Item (Join-Path $root "target/release/toki-daemon.exe") (Join-Path $dest "toki-daemon-$triple.exe") -Force

bun "--cwd=$root/crates/toki-gui/ui" run build
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
