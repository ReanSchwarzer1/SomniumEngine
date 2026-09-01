#!/usr/bin/env python3
"""Run DREAMS' matched number-and-picture fixture.

Every state gets the same map, named camera rail, seed, frame, resolution,
warm-up and measured-frame count. Only the feature switch changes.

    python tools/ghostfence/thermometer.py DREAMS-B SOMNIUM_DREAMS_GRAIN
    python tools/ghostfence/thermometer.py DREAMS-C SOMNIUM_DREAMS_BUBBLE --prove-default
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_VIEWS = ("coastal-ground", "island-ground")
FIXED_SEED = str(0x5EED_B10E)
DREAMS_SWITCHES = ("SOMNIUM_DREAMS_GRAIN", "SOMNIUM_DREAMS_STF")


@dataclass(frozen=True)
class EvidencePaths:
    timing: Path
    picture: Path


def evidence_paths(directory: Path, phase: str, view: str, state: str) -> EvidencePaths:
    stem = f"{phase}_{view}_{state}"
    return EvidencePaths(directory / f"{stem}.somtime", directory / f"{stem}.png")


def fixed_environment(
    paths: EvidencePaths,
    view: str,
    switch: str,
    state: str,
    warmup: int,
    frames: int,
) -> dict[str, str]:
    env = os.environ.copy()
    # Ambient developer switches must not leak into a matched run. Seed and
    # rail are reintroduced below; the requested feature is the only DREAMS
    # switch whose state differs across the A/B pair.
    for name in tuple(env):
        if name.startswith("SOMNIUM_DREAMS_"):
            env.pop(name)
    env.update(
        {
            "SOMNIUM_TIME": str(paths.timing),
            "SOMNIUM_TIME_LABEL": f"{view} {switch} {state}",
            "SOMNIUM_TIME_VIEW": view,
            "SOMNIUM_TIME_STATIC": "1",
            "SOMNIUM_TIME_WARMUP": str(warmup),
            "SOMNIUM_TIME_FRAMES": str(frames),
            "SOMNIUM_TIME_QUIT": "1",
            "SOMNIUM_CAPTURE_PNG": str(paths.picture),
            "SOMNIUM_CAPTURE_FRAME": "240",
            "SOMNIUM_VIEWPORT_RES": "2",
            "SOMNIUM_MAXIMIZE": "1",
            "SOMNIUM_SUN_ELEVATION": "45",
            "SOMNIUM_SUN_AZIMUTH": "120",
            "SOMNIUM_DREAMS_SEED": FIXED_SEED,
            "SOMNIUM_DREAMS_RAIL": view,
        }
    )
    for other in DREAMS_SWITCHES:
        if other != switch:
            env[other] = "0"
    if state == "default":
        env.pop(switch, None)
    elif state == "off":
        env[switch] = "0"
    elif state == "on":
        env[switch] = "1"
    else:
        raise ValueError(f"unknown fixture state {state!r}")
    return env


def run_fixture(
    directory: Path,
    phase: str,
    switch: str,
    view: str,
    state: str,
    warmup: int,
    frames: int,
) -> bool:
    paths = evidence_paths(directory, phase, view, state)
    paths.timing.parent.mkdir(parents=True, exist_ok=True)
    env = fixed_environment(paths, view, switch, state, warmup, frames)
    print(f"THERMOMETER {phase} {view} {state}", flush=True)
    proc = subprocess.run(
        ["cargo", "run", "--release", "-p", "hello_engine"],
        cwd=ROOT,
        env=env,
    )
    missing = [path for path in (paths.timing, paths.picture) if not path.is_file()]
    if proc.returncode != 0 or missing:
        detail = ", ".join(str(path) for path in missing)
        print(
            f"FAIL {view} {state}: exit {proc.returncode}"
            + (f", missing {detail}" if missing else ""),
            file=sys.stderr,
        )
        return False
    print(
        f"PASS {paths.timing.relative_to(ROOT)} + {paths.picture.relative_to(ROOT)}",
        flush=True,
    )
    return True


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", help="record stem, for example DREAMS-B")
    parser.add_argument("switch", help="single environment switch changed by the A/B")
    parser.add_argument("--view", action="append", choices=DEFAULT_VIEWS)
    parser.add_argument("--state", action="append", choices=("default", "off", "on"))
    parser.add_argument("--prove-default", action="store_true")
    parser.add_argument("--warmup", type=int, default=180)
    parser.add_argument("--frames", type=int, default=300)
    parser.add_argument(
        "--directory",
        type=Path,
        default=ROOT / "dev records" / "phase DREAMS",
    )
    args = parser.parse_args()
    if args.warmup < 0 or args.frames < 8:
        parser.error("warmup must be non-negative and frames must be at least 8")

    states = tuple(args.state or (("default", "off", "on") if args.prove_default else ("off", "on")))
    ok = True
    for view in args.view or DEFAULT_VIEWS:
        for state in states:
            ok &= run_fixture(
                args.directory,
                args.phase,
                args.switch,
                view,
                state,
                args.warmup,
                args.frames,
            )
    return int(not ok)


if __name__ == "__main__":
    raise SystemExit(main())
