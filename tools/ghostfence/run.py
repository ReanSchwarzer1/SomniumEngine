#!/usr/bin/env python3
"""GHOSTFENCE — Phase MORROWIND's must-not-break gate.

`phase_MORROWIND.md` S10: *"It is a script, not a habit, and MORROWIND-A writes
it."* This is that script. Every sub-phase runs it before it closes.

Design rule, which is the whole point of the section: **a row that cannot fail
is not a check.** Rows that need a GPU say so and report `SKIP` with the exact
command that would turn them into a `PASS`; they never report a green they did
not earn. `--strict` promotes every SKIP to a failure, which is what a release
gate wants and what a laptop without a windowed GPU session cannot satisfy.

    python tools/ghostfence/run.py              # everything runnable here
    python tools/ghostfence/run.py --fast       # skip the cargo test row
    python tools/ghostfence/run.py --strict     # SKIP counts as failure
    python tools/ghostfence/run.py --row census # one row by name
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from enum import Enum
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from ghostfence import golden  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
EVIDENCE = ROOT / "dev records" / "phase MORROWIND"
GOLDEN_DIR = EVIDENCE / "golden"
GOLDEN_MANIFEST = GOLDEN_DIR / "manifest.json"
CANDIDATE_DIR = ROOT / "target" / "ghostfence"

# ---------------------------------------------------------------------------
# The frozen toolchain line from the plan's preamble.
#
# It lives here, in one place, because the freeze is "a rule against
# *unannounced* change, not against moving". A sub-phase that bumps a version
# edits this dict in the same commit as the manifests, and the row below proves
# the two agree. MORROWIND-A2 is the first sub-phase to do that.
# ---------------------------------------------------------------------------
FROZEN_TOOLCHAIN = {
    "rustc": "1.88",
    "wgpu": "30.0",
    "winit": "0.30",
}

# `thread::spawn` outside the job system is the specific way "one job system"
# gets faked (plan §A.7). Each exemption is a file plus the reason it is not a
# second thread pool; anything not listed here fails the row.
SPAWN_EXEMPTIONS = {
    "crates/somnium_jobs/": "the job system itself — this is the one place a pool is allowed.",
    "crates/somnium_ui/src/theme.rs": "a single-shot test asserting the theme is visible from another thread.",
}

# Second-implementation bans from S10 and §11 row 12. Each is a symbol that
# would only exist if somebody built a parallel system, and a list of the paths
# allowed to define it.
SINGLETON_SYMBOLS: list[tuple[str, str, tuple[str, ...]]] = [
    (
        "struct JobRegistry",
        "one job system - MORROWIND-B promotes it into somnium_jobs",
        ("crates/somnium_jobs/",),
    ),
    (
        "struct JobSystem",
        "one job system - MORROWIND-B promotes it into somnium_jobs",
        ("crates/somnium_jobs/",),
    ),
    (
        "struct ShaderSystem",
        "one shader system",
        ("crates/somnium_shader/",),
    ),
    (
        "struct MaterialSystem",
        "one shader system - `hlms.rs` is retired by MORROWIND-C",
        (),
    ),
]

TEST_FLOOR = 945  # S10: "945 tests green, and the count only goes up".


class Outcome(str, Enum):
    PASS = "PASS"
    FAIL = "FAIL"
    SKIP = "SKIP"


@dataclass
class Result:
    name: str
    outcome: Outcome
    detail: str


def rust_sources() -> list[Path]:
    out = []
    for base in (ROOT / "crates", ROOT / "examples", ROOT / "tools"):
        if not base.exists():
            continue
        out += [p for p in base.rglob("*.rs") if "target" not in p.parts]
    return sorted(out)


def rel(path: Path) -> str:
    return str(path.relative_to(ROOT)).replace("\\", "/")


# ---------------------------------------------------------------------------
# Rows
# ---------------------------------------------------------------------------


def row_census(args: argparse.Namespace) -> Result:
    """The census regenerates without a human editing a table (plan §4, exit)."""
    proc = subprocess.run(
        [sys.executable, str(ROOT / "tools" / "census" / "generate.py"), "--check"],
        capture_output=True,
        text=True,
        # MORROWIND-E2. Windows defaults these pipes to the ANSI code page, and
        # cargo emits UTF-8 - so a single non-ASCII byte anywhere in a test name
        # or a diagnostic killed the reader thread and took the gate with it.
        # A gate that cannot run is not a gate.
        encoding="utf-8",
        errors="replace",
        cwd=ROOT,
    )
    if proc.returncode == 0:
        return Result("census", Outcome.PASS, "MORROWIND-A_census.md matches the tree")
    return Result("census", Outcome.FAIL, (proc.stderr or proc.stdout).strip())


def row_shader_budget(args: argparse.Namespace) -> Result:
    """No module's variant space has outgrown its key (plan S8 item 4).

    A module with six independent defines has 64 possible variants. Past the
    budget in `tools/shadercook/generate.py` the key is too coarse, and the fix
    is splitting the module rather than growing the cache -- a design error
    worth catching here rather than as a startup stall on someone's machine.
    """
    proc = subprocess.run(
        [sys.executable, str(ROOT / "tools" / "shadercook" / "generate.py"), "--check"],
        capture_output=True,
        text=True,
        # MORROWIND-E2. Windows defaults these pipes to the ANSI code page, and
        # cargo emits UTF-8 - so a single non-ASCII byte anywhere in a test name
        # or a diagnostic killed the reader thread and took the gate with it.
        # A gate that cannot run is not a gate.
        encoding="utf-8",
        errors="replace",
        cwd=ROOT,
    )
    if proc.returncode == 0:
        summary = next(
            (line for line in proc.stdout.splitlines() if "variants possible" in line),
            "within budget",
        )
        return Result("shader-budget", Outcome.PASS, summary.strip())
    return Result("shader-budget", Outcome.FAIL, (proc.stderr or proc.stdout).strip())


def row_toolchain(args: argparse.Namespace) -> Result:
    """rustc / wgpu / winit match the frozen line, which lives in this file."""
    problems = []

    toolchain = tomllib.loads((ROOT / "rust-toolchain.toml").read_text(encoding="utf-8"))
    channel = str(toolchain.get("toolchain", {}).get("channel", ""))
    if channel != FROZEN_TOOLCHAIN["rustc"]:
        problems.append(
            f"rust-toolchain.toml pins {channel!r}, frozen line says {FROZEN_TOOLCHAIN['rustc']!r}"
        )

    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    deps = workspace["workspace"]["dependencies"]
    for name in ("wgpu", "winit"):
        spec = deps.get(name)
        declared = spec if isinstance(spec, str) else (spec or {}).get("version", "")
        if declared != FROZEN_TOOLCHAIN[name]:
            problems.append(
                f"Cargo.toml declares {name} {declared!r}, frozen line says "
                f"{FROZEN_TOOLCHAIN[name]!r}"
            )

    if problems:
        return Result(
            "toolchain",
            Outcome.FAIL,
            "; ".join(problems)
            + " - if the bump is intended, edit FROZEN_TOOLCHAIN in tools/ghostfence/run.py "
            "in the same commit, and say so in the sub-phase record",
        )
    frozen = ", ".join(f"{k} {v}" for k, v in FROZEN_TOOLCHAIN.items())
    return Result("toolchain", Outcome.PASS, frozen)


def row_one_job_system(args: argparse.Namespace) -> Result:
    """No second thread pool, no bare `thread::spawn` outside the job system."""
    offenders = []
    pattern = re.compile(r"\bthread::spawn\b")
    for path in rust_sources():
        relative = rel(path)
        # Exemptions are matched as prefixes, so an entry can name one file or
        # a whole crate. `somnium_jobs/` is the second kind: a pool inside the
        # job system is the job system, not a second one.
        if any(relative.startswith(prefix) for prefix in SPAWN_EXEMPTIONS):
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for number, line in enumerate(text.splitlines(), 1):
            # A comment explaining the rule is not a violation of it. Without
            # this, the doc comment in somnium_jobs that describes the ban trips
            # the ban, and the first thing anyone does about a gate that flags
            # its own documentation is turn it off.
            if line.lstrip().startswith("//"):
                continue
            if pattern.search(line):
                offenders.append(f"{relative}:{number}")
    if offenders:
        return Result(
            "one-job-system",
            Outcome.FAIL,
            "thread::spawn outside somnium_jobs at "
            + ", ".join(offenders[:8])
            + (f" (+{len(offenders) - 8} more)" if len(offenders) > 8 else "")
            + " - route it through somnium_jobs, or add a stated exemption",
        )
    return Result(
        "one-job-system",
        Outcome.PASS,
        f"no bare spawns; {len(SPAWN_EXEMPTIONS)} exemptions, each with a reason",
    )


def row_no_second_system(args: argparse.Namespace) -> Result:
    """§11 row 12 — no second job system, shader system, graph or timeline."""
    offenders = []
    for symbol, why, allowed in SINGLETON_SYMBOLS:
        pattern = re.compile(rf"\bpub\s+{re.escape(symbol)}\b")
        for path in rust_sources():
            relative = rel(path)
            if any(relative.startswith(prefix) for prefix in allowed):
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            for number, line in enumerate(text.splitlines(), 1):
                if pattern.search(line):
                    offenders.append(f"{relative}:{number} defines `{symbol}` ({why})")
    if offenders:
        return Result("no-second-system", Outcome.FAIL, "; ".join(offenders))
    return Result(
        "no-second-system",
        Outcome.PASS,
        f"{len(SINGLETON_SYMBOLS)} singleton symbols, each defined only where it is allowed",
    )


def row_golden_images(args: argparse.Namespace) -> Result:
    """Fixed camera, fixed frame, stored reference, perceptual threshold.

    References are produced by a windowed GPU run — `SOMNIUM_CAPTURE_UI_PNG`
    plus `SOMNIUM_CAPTURE_FRAME` and `SOMNIUM_CAPTURE_QUIT=1`. Until one has
    been taken this row is `SKIP` with the command attached, because a golden
    row with no golden is a promise and S10 says a promise is what this table
    is meant to replace.
    """
    if not GOLDEN_MANIFEST.exists():
        return Result(
            "golden-images",
            Outcome.SKIP,
            "no reference set yet - take one with "
            "`python tools/ghostfence/capture.py --reference`",
        )

    manifest = json.loads(GOLDEN_MANIFEST.read_text(encoding="utf-8"))
    failures, passes, skips = [], 0, []
    for entry in manifest.get("images", []):
        name = entry["name"]
        reference = GOLDEN_DIR / entry["reference"]
        candidate = Path(entry.get("candidate") or (CANDIDATE_DIR / f"{name}.png"))
        if not candidate.is_absolute():
            candidate = ROOT / candidate
        threshold = golden.Threshold(**entry.get("threshold", {}))
        region = golden.Region(**entry["region"]) if entry.get("region") else None
        if not reference.exists():
            failures.append(f"{name}: reference {rel(reference)} is missing")
            continue
        if not candidate.exists():
            skips.append(f"{name}: no candidate at {rel(candidate)}")
            continue
        result = golden.compare(
            reference,
            candidate,
            threshold,
            diff_path=CANDIDATE_DIR / f"{name}.diff.png",
            region=region,
        )
        if result.passed:
            passes += 1
        else:
            where = f" - diff at {rel(result.diff_path)}" if result.diff_path else ""
            failures.append(f"{name}: {result.reason}{where}")

    if failures:
        return Result("golden-images", Outcome.FAIL, "; ".join(failures))
    if skips and passes == 0:
        return Result("golden-images", Outcome.SKIP, "; ".join(skips))
    detail = f"{passes} image(s) within threshold"
    if skips:
        detail += f"; {len(skips)} not captured this run"
    return Result("golden-images", Outcome.PASS, detail)


def row_tests(args: argparse.Namespace) -> Result:
    """945 tests green, and the count only goes up.

    `-j 1` is not a superstition: this workspace lives on OneDrive, and a
    parallel link step reliably trips LNK1104 on a file the sync client is
    holding. A gate that fails for a reason unrelated to the code trains people
    to ignore it.
    """
    if args.fast:
        return Result("tests", Outcome.SKIP, "--fast: `cargo test --workspace -j 1` not run")
    proc = subprocess.run(
        ["cargo", "test", "--workspace", "-j", "1"],
        capture_output=True,
        text=True,
        # MORROWIND-E2. Windows defaults these pipes to the ANSI code page, and
        # cargo emits UTF-8 - so a single non-ASCII byte anywhere in a test name
        # or a diagnostic killed the reader thread and took the gate with it.
        # A gate that cannot run is not a gate.
        encoding="utf-8",
        errors="replace",
        cwd=ROOT,
    )
    output = (proc.stdout or "") + (proc.stderr or "")
    passed = sum(int(m) for m in re.findall(r"test result: ok\. (\d+) passed", output))
    failed = sum(int(m) for m in re.findall(r"(\d+) failed", output))
    if proc.returncode != 0:
        tail = "\n".join(output.strip().splitlines()[-15:])
        return Result("tests", Outcome.FAIL, f"cargo test exited {proc.returncode}\n{tail}")
    if passed < TEST_FLOOR:
        return Result(
            "tests",
            Outcome.FAIL,
            f"{passed} tests passed, below the {TEST_FLOOR} floor - the count only goes up",
        )
    return Result("tests", Outcome.PASS, f"{passed} passed, {failed} failed (floor {TEST_FLOOR})")


ROWS = {
    "census": row_census,
    "toolchain": row_toolchain,
    "shader-budget": row_shader_budget,
    "one-job-system": row_one_job_system,
    "no-second-system": row_no_second_system,
    "golden-images": row_golden_images,
    "tests": row_tests,
}

# Rows S10 lists that are owned by a cargo test rather than by this script.
# Naming the owner is the point: an invariant with no named owner is the kind
# of promise this table exists to retire.
DELEGATED = {
    "Hades paint contract": "somnium_ui: the_shell_actually_uses_the_new_paint_capabilities",
    "Nocturne tokens and contrast pairs": "somnium_ui::theme tests (Zeta S8A.3 pairs)",
    "XV terrain contract": "somnium_renderer::terrain tests",
    "Water and foliage numbers": "somnium_renderer::water / foliage tests",
    "Scene round-trip": "somnium_core::scene_schema round-trip tests",
    "Frame time on both shipped maps": "a `.somtime` run - GPU, per sub-phase (plan S10)",
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fast", action="store_true", help="skip the cargo test row")
    parser.add_argument("--strict", action="store_true", help="treat SKIP as failure")
    parser.add_argument("--row", action="append", help="run only the named row(s)")
    args = parser.parse_args()

    selected = args.row or list(ROWS)
    unknown = [name for name in selected if name not in ROWS]
    if unknown:
        print(f"ghostfence: unknown row(s): {', '.join(unknown)}", file=sys.stderr)
        print(f"            known rows: {', '.join(ROWS)}", file=sys.stderr)
        return 2

    print("GHOSTFENCE - phase MORROWIND must-not-break gate")
    print("=" * 72)
    results = [ROWS[name](args) for name in selected]

    width = max(len(r.name) for r in results)
    for result in results:
        print(f"  {result.outcome.value:4}  {result.name:<{width}}  {result.detail}")

    print("=" * 72)
    for invariant, owner in DELEGATED.items():
        print(f"  ----  {invariant}: owned by {owner}")

    failed = [r for r in results if r.outcome is Outcome.FAIL]
    skipped = [r for r in results if r.outcome is Outcome.SKIP]
    print("=" * 72)
    print(
        f"{len(results) - len(failed) - len(skipped)} passed, "
        f"{len(failed)} failed, {len(skipped)} skipped"
    )
    if failed:
        return 1
    if skipped and args.strict:
        print("ghostfence: --strict, and rows were skipped", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
