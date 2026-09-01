#!/usr/bin/env python3
"""Cook Somnium's authored Slang modules into checked-in SPIR-V.

The compiler is a developer tool, not a Cargo build dependency. Point
``SOMNIUM_SLANGC`` at ``slangc`` or put it on PATH. A local archive extracted
under ``target/slang-<version>/bin`` is also discovered, which keeps one-off
evaluation out of global machine state.

    python tools/slangcook/run.py          # refresh checked-in artifacts
    python tools/slangcook/run.py --check  # recompile and compare, write nothing
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST_PATH = Path(__file__).with_name("manifest.json")


@dataclass(frozen=True)
class Module:
    source: Path
    artifact: Path
    entry: str
    stage: str


def load_manifest() -> tuple[str, list[Module]]:
    raw = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    version = str(raw["compiler"]["version"])
    modules = [
        Module(
            source=ROOT / row["source"],
            artifact=ROOT / row["artifact"],
            entry=str(row["entry"]),
            stage=str(row["stage"]),
        )
        for row in raw["modules"]
    ]
    return version, modules


def find_compiler(version: str) -> Path | None:
    explicit = os.environ.get("SOMNIUM_SLANGC")
    candidates = [Path(explicit)] if explicit else []
    on_path = shutil.which("slangc")
    if on_path:
        candidates.append(Path(on_path))
    executable = "slangc.exe" if os.name == "nt" else "slangc"
    candidates.append(ROOT / "target" / f"slang-{version}" / "bin" / executable)
    return next((path for path in candidates if path.is_file()), None)


def compiler_version(compiler: Path) -> str:
    proc = subprocess.run(
        [str(compiler), "-v"],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        cwd=ROOT,
    )
    if proc.returncode != 0:
        raise RuntimeError((proc.stderr or proc.stdout).strip())
    return (proc.stdout or proc.stderr).strip()


def compile_module(compiler: Path, module: Module, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    proc = subprocess.run(
        [
            str(compiler),
            str(module.source),
            "-target",
            "spirv",
            "-profile",
            "sm_6_6",
            "-entry",
            module.entry,
            "-stage",
            module.stage,
            "-matrix-layout-column-major",
            "-o",
            str(destination),
        ],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        cwd=ROOT,
    )
    if proc.returncode != 0:
        raise RuntimeError((proc.stderr or proc.stdout).strip())


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(check: bool) -> int:
    version, modules = load_manifest()
    compiler = find_compiler(version)
    if compiler is None:
        print(
            f"slangcook: Slang {version} not found; set SOMNIUM_SLANGC or install slangc on PATH",
            file=sys.stderr,
        )
        return 2
    reported = compiler_version(compiler)
    reported_version = re.search(r"\b\d+\.\d+\.\d+\b", reported)
    if reported_version is None or reported_version.group(0) != version:
        print(
            f"slangcook: expected compiler {version}, got {reported!r} from {compiler}",
            file=sys.stderr,
        )
        return 2

    failures = 0
    with tempfile.TemporaryDirectory(prefix="somnium-slang-") as temporary:
        temp = Path(temporary)
        for index, module in enumerate(modules):
            if not module.source.is_file():
                print(f"FAIL {module.source.relative_to(ROOT)}: source is missing")
                failures += 1
                continue
            destination = temp / f"{index}.spv" if check else module.artifact
            try:
                compile_module(compiler, module, destination)
            except RuntimeError as error:
                print(f"FAIL {module.source.relative_to(ROOT)}\n{error}")
                failures += 1
                continue
            if check:
                if not module.artifact.is_file():
                    print(f"FAIL {module.artifact.relative_to(ROOT)}: artifact is missing")
                    failures += 1
                elif destination.read_bytes() != module.artifact.read_bytes():
                    print(
                        f"FAIL {module.artifact.relative_to(ROOT)}: stale "
                        f"({digest(module.artifact)[:12]} != {digest(destination)[:12]})"
                    )
                    failures += 1
                else:
                    print(
                        f"PASS {module.source.relative_to(ROOT)} -> "
                        f"{module.artifact.relative_to(ROOT)} ({destination.stat().st_size:,} bytes)"
                    )
            else:
                print(
                    f"WROTE {module.artifact.relative_to(ROOT)} "
                    f"({destination.stat().st_size:,} bytes, sha256 {digest(destination)[:12]})"
                )
    return int(failures != 0)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="compare recooked bytes without writing")
    return run(parser.parse_args().check)


if __name__ == "__main__":
    raise SystemExit(main())
