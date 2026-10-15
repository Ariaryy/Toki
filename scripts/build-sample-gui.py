"""Build a native release window with invented activity and real installed app icons.

Run on Windows: python scripts/build-sample-gui.py
Only generated frontend assets are modified. A normal frontend build removes sample mode.
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
    # Keep real Tauri IPC for window controls and native app icons. Website icons are
    # deliberately absent here: a sample launch must not open browser history/profile files.
    script = """<script>
    (() => {
      const native = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
      const samples = SAMPLE_DATA;
      window.__TAURI_INTERNALS__.invoke = (command, args, options) => {
        if (command === 'domain_icons') return Promise.resolve({});
        if (Object.prototype.hasOwnProperty.call(samples, command))
          return Promise.resolve(structuredClone(samples[command]));
        if (command === 'open_folder' || command === 'open_download') return Promise.resolve();
        return native(command, args, options);
      };
      document.addEventListener('DOMContentLoaded', () => {
        document.title = 'Toki — Sample data';
      });
    })();
    </script>""".replace("SAMPLE_DATA", json.dumps(data))
    bun = shutil.which("bun")
    run(bun, "--cwd=crates/toki-gui/ui", "run", "build")
    index = ROOT / "crates/toki-gui/ui/build/index.html"
    index.write_text(index.read_text(encoding="utf-8").replace("<head>", "<head>" + script, 1), encoding="utf-8")
    env = os.environ.copy()
    env["TAURI_CONFIG"] = json.dumps({
        "identifier": "dev.achintya.toki.sample",
        "productName": "Toki Sample",
        "bundle": {"externalBin": []},
    })
    run("cargo", "build", "--release", "-p", "toki-gui", "--features", "custom-protocol", env=env)
    dest = ROOT / "target/sample"
    dest.mkdir(exist_ok=True)
    shutil.copy2(ROOT / "target/release/toki.exe", dest / "toki-sample.exe")
    print(f"Sample release: {dest / 'toki-sample.exe'}", flush=True)
    print("Rebuild the frontend before producing a normal release.", flush=True)


if __name__ == "__main__":
    main()
