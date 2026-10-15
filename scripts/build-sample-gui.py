"""Build a native release window with invented activity and real installed app icons.

Run on Windows: python scripts/build-sample-gui.py
An explicit Cargo feature supplies fixtures; normal releases never enable sample mode.
The isolated executable does not replace the installed app. All activity commands,
including mutations, are intercepted; native window controls and icon reads stay real.
"""

import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent.parent


def run(*args, **kwargs):
    subprocess.run(args, cwd=ROOT, check=True, **kwargs)


def main():
    spec = importlib.util.spec_from_file_location("preview", ROOT / "scripts/preview-gui.py")
    preview = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(preview)
    processes = subprocess.check_output(
        ["powershell", "-NoProfile", "-Command",
         "Get-Process | Where-Object Path | Select-Object -ExpandProperty Path -Unique | ConvertTo-Json"],
        text=True,
    )
    installed = {Path(p).name.lower(): p for p in json.loads(processes)}
    # Fixed names in the fixtures are mapped to actual executable paths, never user history.
    def paths(value):
        if isinstance(value, dict):
            return {k: installed.get(v.lower(), v) if k == "exePath" else paths(v)
                    for k, v in value.items()}
        if isinstance(value, list):
            return [paths(v) for v in value]
        return value

    data = paths(preview.responses(busy=True))
    data["diagnostics"]["appVersion"] = "0.4.0 (sample data)"
    # Window controls and executable icon extraction stay native. No browser files are
    # opened, and every activity/mutation command returns a fixture instead of using IPC.
    data.update(domain_icons={}, open_folder=None, open_download=None)
    dest = ROOT / "target/sample"
    dest.mkdir(exist_ok=True)
    fixtures = dest / "responses.json"
    fixtures.write_text(json.dumps(data), encoding="utf-8")
    bun = shutil.which("bun")
    run(bun, "--cwd=crates/toki-gui/ui", "run", "build")
    env = os.environ.copy()
    env["TOKI_SAMPLE_DATA"] = str(fixtures)
    env["TAURI_CONFIG"] = json.dumps({
        "identifier": "dev.achintya.toki.sample",
        "productName": "Toki Sample",
        "bundle": {"externalBin": []},
        "app": {"windows": [{"label": "main", "title": "Toki — Sample data",
                 "width": 1120, "height": 760, "minWidth": 880, "minHeight": 600,
                 "decorations": False, "shadow": True}]},
    })
    run("cargo", "build", "--release", "-p", "toki-gui", "--features", "custom-protocol,sample-data", env=env)
    shutil.copy2(ROOT / "target/release/toki.exe", dest / "toki-sample.exe")
    print(f"Sample release: {dest / 'toki-sample.exe'}", flush=True)


if __name__ == "__main__":
    main()
