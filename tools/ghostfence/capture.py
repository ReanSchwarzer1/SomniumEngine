"""Take the GHOSTFENCE golden capture, or a candidate to compare against it.

MORROWIND-E2b. Before this script the `golden-images` row's SKIP message asked a
human to remember six environment variables in the right order, which is the
same as asking them not to run it. A gate whose reference cannot be regenerated
in one command is a gate that will be stale within a month.

    python tools/ghostfence/capture.py              # candidate -> target/ghostfence/
    python tools/ghostfence/capture.py --reference  # approve a new reference

`--reference` overwrites checked-in evidence, so it prints what it is about to
replace and requires `--yes` to do it without asking.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GOLDEN_DIR = ROOT / "dev records" / "phase MORROWIND" / "golden"
CANDIDATE_DIR = ROOT / "target" / "ghostfence"

#: The one capture every golden entry crops a region out of. One run, several
#: pieces of evidence — because a second run is a second set of frame timings,
#: a second autosave state and a second chance for the scene to differ.
CAPTURE_NAME = "editor_shell_1280x720.png"

#: Frame 120 rather than frame 1: the shell has settled, thumbnails have
#: decoded, and Phase 27's motion tracks have all finished (MAX_DURATION_MS is
#: 200 ms, so anything started at load is long done).
CAPTURE_FRAME = "120"


def capture(destination: Path, frame: str, release: bool) -> int:
    destination.parent.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ)
    env["SOMNIUM_CAPTURE_UI_PNG"] = str(destination)
    env["SOMNIUM_CAPTURE_FRAME"] = frame
    env["SOMNIUM_CAPTURE_QUIT"] = "1"
    command = ["cargo", "run", "-p", "hello_engine", "-j", "1"]
    if release:
        command.append("--release")
    print(f"capture: {' '.join(command)} -> {destination}")
    proc = subprocess.run(
        command,
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    output = (proc.stdout or "") + (proc.stderr or "")
    if proc.returncode != 0:
        print(output[-4000:], file=sys.stderr)
        print(f"capture: hello_engine exited {proc.returncode}", file=sys.stderr)
        return proc.returncode
    if not destination.exists():
        # The run succeeded and wrote nothing, which means the capture path was
        # never reached — usually a surface without COPY_SRC usage. The renderer
        # warns about that; surface it rather than reporting a silent success.
        for line in output.splitlines():
            if "capture" in line.lower():
                print(line, file=sys.stderr)
        print(f"capture: no file at {destination}", file=sys.stderr)
        return 1
    size = destination.stat().st_size
    print(f"capture: wrote {destination} ({size:,} bytes)")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--reference",
        action="store_true",
        help="approve the capture as the checked-in reference",
    )
    parser.add_argument("--yes", action="store_true", help="do not ask before overwriting")
    parser.add_argument("--frame", default=CAPTURE_FRAME)
    parser.add_argument("--release", action="store_true")
    args = parser.parse_args()

    reference = GOLDEN_DIR / CAPTURE_NAME
    if not args.reference:
        return capture(CANDIDATE_DIR / CAPTURE_NAME, args.frame, args.release)

    if reference.exists() and not args.yes:
        print(f"about to replace {reference} ({reference.stat().st_size:,} bytes).")
        print("a reference is evidence — re-approve it deliberately.")
        print("re-run with --yes to proceed.")
        return 2

    staging = CANDIDATE_DIR / CAPTURE_NAME
    code = capture(staging, args.frame, args.release)
    if code != 0:
        return code
    reference.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(staging, reference)
    print(f"capture: approved as {reference}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
