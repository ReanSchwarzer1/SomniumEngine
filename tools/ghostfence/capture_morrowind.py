#!/usr/bin/env python3
"""Capture native MORROWIND designer surfaces without changing golden references.

python tools/ghostfence/capture_morrowind.py
python tools/ghostfence/capture_morrowind.py --state animation --skip-build
"""
from __future__ import annotations
import argparse
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
STATES = ("scatter", "behavior", "animation", "navigation", "shell")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", choices=STATES)
    parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()
    logs = ROOT / "target"
    logs.mkdir(exist_ok=True)
    if not args.skip_build:
        with (logs / "morrowind-capture-build.log").open("w", encoding="utf-8") as log:
            built = subprocess.run(["cargo", "build", "-p", "hello_engine", "--offline", "-j", "1"], cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
        if built.returncode:
            print("Build failed: target/morrowind-capture-build.log", flush=True)
            return built.returncode
    binary = ROOT / "target" / "debug" / ("hello_engine.exe" if os.name == "nt" else "hello_engine")
    for state in (args.state,) if args.state else STATES:
        output = ROOT / ("target/ghostfence/editor_shell_1280x720.png" if state == "shell" else f"dev records/phase MORROWIND/morrowind-{state}-1280x720.png")
        output.parent.mkdir(parents=True, exist_ok=True)
        previous_write = output.stat().st_mtime_ns if output.exists() else None
        env = dict(os.environ)
        env.update(SOMNIUM_AUDIT_UI_STATE="shell" if state == "shell" else f"morrowind-{state}", SOMNIUM_AUDIT_WINDOW_SIZE="1280x720", SOMNIUM_CAPTURE_UI_PNG=str(output), SOMNIUM_CAPTURE_FRAME="120", SOMNIUM_CAPTURE_QUIT="1")
        with (logs / f"morrowind-capture-{state}.log").open("w", encoding="utf-8") as log:
            try:
                run = subprocess.run([str(binary)], cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=180)
            except subprocess.TimeoutExpired:
                print(f"{state}: timed out; see target/morrowind-capture-{state}.log", flush=True)
                return 1
        if run.returncode or not output.exists() or output.stat().st_mtime_ns == previous_write:
            print(f"{state}: failed ({run.returncode}); see target/morrowind-capture-{state}.log", flush=True)
            return run.returncode or 1
        print(f"{state}: {output.relative_to(ROOT)} ({output.stat().st_size} bytes)", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
