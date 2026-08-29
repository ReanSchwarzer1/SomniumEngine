#!/usr/bin/env python3
"""Generate Phase MORROWIND's engine census.

`phase_MORROWIND.md` §4 is a hand-typed audit measured on 2026-08-23.  A
hand-typed audit rots in a week, so MORROWIND-A turns it into this script on the
DOOM-A and CONTROL-A precedent.  The generated report is the authority; §4 of the
plan is the historical baseline it is compared against.

Standard library only, like `tools/reachability/generate.py`.  Rust is read as a
constrained language — line counts, `#[test]` attributes, `pub` items, string
literals and the component-schema macro — not parsed.  Every inference that
could be wrong is stated in the generated report rather than hidden.

Usage:
    python tools/census/generate.py            # write the report
    python tools/census/generate.py --check    # fail if the report is stale
    python tools/census/generate.py --stdout    # print, write nothing
"""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REPORT = ROOT / "dev records" / "phase MORROWIND" / "MORROWIND-A_census.md"

# ---------------------------------------------------------------------------
# The 2026-08-23 baseline, quoted from `phase_MORROWIND.md` §4.
#
# These are *evidence from when the plan was written*, not values to recompute.
# Keeping them here is what lets the generated report show a delta instead of
# silently agreeing with itself.
# ---------------------------------------------------------------------------
PLAN_CRATE_LINES = {
    "somnium_renderer": 50_206,
    "somnium_ui": 27_530,
    "somnium_core": 19_220,
    "somnium_script": 4_815,
    "somnium_script_luau": 4_457,
    "somnium_ecs": 4_018,
    "somnium_asset": 1_639,
    "somnium_voxel": 1_000,
    "somnium_physics": 580,
    "somnium_physics_sys": 334,
    "somnium_audio": 93,
}
PLAN_CRATE_TESTS = {
    "somnium_renderer": 328,
    "somnium_ui": 215,
    "somnium_core": 217,
    "somnium_script": 55,
    "somnium_script_luau": 58,
    "somnium_ecs": 54,
    "somnium_asset": 6,
    "somnium_voxel": 11,
    "somnium_physics": 1,
    "somnium_physics_sys": 0,
    "somnium_audio": 0,
}
PLAN_TOTAL_LINES = 113_892
PLAN_TOTAL_TESTS = 945
PLAN_WGSL_FILES = 48
PLAN_WGSL_LINES = 12_079
PLAN_SCHEMAS = 12
PLAN_ENV_VARS = 96
PLAN_HELLO_ENGINE_LINES = 2_646

# §4.6's absent-system greps.  `expected` is the 2026-08-23 file count; a term
# whose count has *risen* is a system somebody started, and that is worth
# seeing.  `note` reproduces the plan's reading so a false positive is not
# mistaken for a capability.
ABSENCE_TERMS: list[tuple[str, int, str]] = [
    ("bone", 0, "No skeletal animation of any kind (Track 5)."),
    ("armature", 0, "As above."),
    ("skin", 8, "Mostly false positives (`asking`, `masking`); `hlms.rs` names skinning as a hypothetical key."),
    ("navmesh", 0, "No navigation (Track 6)."),
    ("pathfind", 0, "As above."),
    ("gamepad", 0, "No input abstraction (Track 8, Seam 5)."),
    ("action_map", 0, "As above."),
    ("localiz", 0, "No localization (Track 8)."),
    ("state_machine", 0, "No animation or AI state machines (Tracks 5, 6)."),
    ("prefab", 2, "Both are comments in the scripting crate. No prefab system (Track 3)."),
    ("dock", 5, "An unused `IconId::Dock` and a comment. No docking system (Track 2)."),
    ("accessib", 1, "A doc comment about script-accessible fields. No accessibility (Track 1)."),
    ("nine_slice", 1, "The draw call exists; nothing can feed it (Track 1)."),
]

# Dependencies whose use is not a source-level identifier match.  Each entry is
# a dependency name mapped to the reason it is legitimately invisible to the
# grep, so the "unjustified" column means "nobody has explained this" rather
# than "the grep is naive".
DEPENDENCY_EXEMPTIONS: dict[str, str] = {
    "cc": "build-dependency; used from build.rs to compile Jolt.",
    "tracing-subscriber": "installed once at startup; referenced as `tracing_subscriber`.",
}

TEST_ATTR = re.compile(r"^\s*#\[(?:\w+::)*test\b")
PUB_ITEM = re.compile(
    r"^\s*pub(?:\s*\([^)]*\))?\s+"
    r"(?:async\s+|unsafe\s+|const\s+|extern\s+\"[^\"]*\"\s+)*"
    r"(fn|struct|enum|trait|type|mod|const|static|union)\s+([A-Za-z_][A-Za-z0-9_]*)"
)
SCHEMA_MACRO = re.compile(r"component_schema!\s*[({\[]")
ENV_VAR = re.compile(r"SOMNIUM_[A-Z0-9_]+")


@dataclass
class CrateStat:
    name: str
    rs_lines: int = 0
    wgsl_lines: int = 0
    rs_files: int = 0
    wgsl_files: int = 0
    tests: int = 0
    pub_items: dict[str, int] = field(default_factory=dict)

    @property
    def lines(self) -> int:
        return self.rs_lines + self.wgsl_lines

    @property
    def pub_total(self) -> int:
        return sum(self.pub_items.values())


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8", errors="replace")


def count_lines(text: str) -> int:
    """`wc -l` semantics: newline count, plus one for a final unterminated line."""
    if not text:
        return 0
    return text.count("\n") + (0 if text.endswith("\n") else 1)


def source_files(base: Path) -> list[Path]:
    return sorted(
        p
        for p in base.rglob("*")
        if p.suffix in {".rs", ".wgsl"} and "target" not in p.parts
    )


def survey_crates() -> tuple[list[CrateStat], CrateStat]:
    crates: list[CrateStat] = []
    for crate_dir in sorted((ROOT / "crates").iterdir()):
        if not (crate_dir / "Cargo.toml").exists():
            continue
        crates.append(survey_one(crate_dir.name, crate_dir))
    example = survey_one("hello_engine", ROOT / "examples" / "hello_engine")
    return crates, example


def survey_one(name: str, base: Path) -> CrateStat:
    stat = CrateStat(name=name)
    for path in source_files(base):
        text = read(path)
        lines = count_lines(text)
        if path.suffix == ".rs":
            stat.rs_files += 1
            stat.rs_lines += lines
            for line in text.splitlines():
                if TEST_ATTR.match(line):
                    stat.tests += 1
                match = PUB_ITEM.match(line)
                if match:
                    stat.pub_items[match.group(1)] = stat.pub_items.get(match.group(1), 0) + 1
        else:
            stat.wgsl_files += 1
            stat.wgsl_lines += lines
    return stat


def survey_wgsl() -> list[tuple[str, int]]:
    shaders = []
    for path in sorted(ROOT.rglob("*.wgsl")):
        if "target" in path.parts or "example_repo" in path.parts:
            continue
        shaders.append((str(path.relative_to(ROOT)).replace("\\", "/"), count_lines(read(path))))
    return shaders


def survey_absences() -> list[tuple[str, int, int, str]]:
    """`grep -ril <term>` over crates, case-insensitively, per §4.6."""
    files = source_files(ROOT / "crates")
    texts = [read(p).lower() for p in files]
    rows = []
    for term, expected, note in ABSENCE_TERMS:
        hits = sum(1 for text in texts if term in text)
        rows.append((term, hits, expected, note))
    return rows


def survey_schemas() -> int:
    total = 0
    for path in source_files(ROOT / "crates"):
        if path.suffix != ".rs":
            continue
        total += len(SCHEMA_MACRO.findall(read(path)))
    return total


def survey_env_vars() -> list[str]:
    names: set[str] = set()
    for base in (ROOT / "crates", ROOT / "examples"):
        for path in source_files(base):
            if path.suffix != ".rs":
                continue
            names.update(ENV_VAR.findall(read(path)))
    return sorted(names)


def crate_identifiers(dep: str, spec: object) -> list[str]:
    """Every identifier a `use` statement could plausibly spell this dependency.

    Cargo turns `-` into `_`, but a crate is free to declare a different `[lib]
    name` — `md-5` builds `md5` — and a manifest is free to rename with
    `package = "..."`. All three spellings are accepted; a dependency matching
    none of them is genuinely unreferenced from this crate's sources.
    """
    names = {dep, dep.replace("-", "_"), dep.replace("-", "")}
    if isinstance(spec, dict) and "package" in spec:
        real = str(spec["package"])
        names |= {real, real.replace("-", "_"), real.replace("-", "")}
    return sorted(n for n in names if n)


# Dependencies reached through a derive macro rather than a path. `serde` is the
# usual case: `#[derive(Serialize)]` names no crate at the call site.
DERIVE_EXPORTS: dict[str, tuple[str, ...]] = {
    "serde": ("Serialize", "Deserialize"),
    "thiserror": ("Error",),
    "bytemuck": ("Pod", "Zeroable"),
}


def survey_dependencies() -> list[tuple[str, str, str, str]]:
    """Per-crate dependency justification.

    A dependency is *justified* when its crate identifier appears in that
    crate's own sources.  This is a grep, not resolution: a dependency reached
    only through a re-export or a macro will read as unjustified, which is why
    `DEPENDENCY_EXEMPTIONS` exists and why every exemption carries a reason.
    """
    rows: list[tuple[str, str, str, str]] = []
    manifests = [(d.name, d) for d in sorted((ROOT / "crates").iterdir()) if (d / "Cargo.toml").exists()]
    manifests.append(("hello_engine", ROOT / "examples" / "hello_engine"))
    manifests.append(("<workspace>", ROOT))

    for crate_name, crate_dir in manifests:
        manifest = tomllib.loads(read(crate_dir / "Cargo.toml"))
        if crate_name == "<workspace>":
            deps = manifest.get("workspace", {}).get("dependencies", {})
            haystack = "".join(
                read(p) for base in (ROOT / "crates", ROOT / "examples") for p in source_files(base)
            )
        else:
            deps = {}
            for table in ("dependencies", "build-dependencies", "dev-dependencies"):
                deps.update(manifest.get(table, {}))
            haystack = "".join(read(p) for p in source_files(crate_dir))
        for dep, spec in sorted(deps.items()):
            if dep.startswith("somnium_"):
                continue
            idents = crate_identifiers(dep, spec)
            used = any(re.search(rf"\b{re.escape(i)}\b", haystack) for i in idents)
            if not used:
                used = any(
                    re.search(rf"derive\s*\([^)]*\b{re.escape(name)}\b", haystack)
                    for name in DERIVE_EXPORTS.get(dep, ())
                )
            if used:
                verdict, reason = "used", ""
            elif dep in DEPENDENCY_EXEMPTIONS:
                verdict, reason = "exempt", DEPENDENCY_EXEMPTIONS[dep]
            else:
                spellings = ", ".join(f"`{i}`" for i in idents)
                verdict, reason = "UNREFERENCED", f"no match for {spellings}"
            if verdict != "used":
                rows.append((crate_name, dep, verdict, reason))
    return rows


def delta(actual: int, plan: int) -> str:
    diff = actual - plan
    if diff == 0:
        return "—"
    return f"{diff:+,}"


def render() -> str:
    crates, example = survey_crates()
    crates.sort(key=lambda c: c.lines, reverse=True)
    total_lines = sum(c.lines for c in crates)
    total_tests = sum(c.tests for c in crates)
    shaders = survey_wgsl()
    renderer_shaders = [(p, n) for p, n in shaders if "/somnium_renderer/" in p]
    absences = survey_absences()
    schemas = survey_schemas()
    env_vars = survey_env_vars()
    deps = survey_dependencies()

    out: list[str] = []
    w = out.append

    w("# MORROWIND-A — the engine census")
    w("")
    w("**Generated by `tools/census/generate.py`. Do not edit by hand.**")
    w("")
    w("`phase_MORROWIND.md` §4 was measured once, on 2026-08-23, and typed into a")
    w("table. This report regenerates every figure in it from the tree. Where the")
    w("two disagree, this report is the measurement and §4 is the historical")
    w("baseline; the **Δ plan** columns are the difference, stated rather than")
    w("silently corrected.")
    w("")
    w("Method, so the numbers can be disagreed with: line counts are `wc -l`")
    w("semantics over `*.rs` and `*.wgsl` under each crate; tests are lines")
    w("matching `#[test]` or `#[<path>::test]`; `pub` items are a line-anchored")
    w("regex, so items behind `cfg_attr` or written across two lines are missed.")
    w("None of this is a Rust parser and it does not pretend to be one.")
    w("")

    # -- 4.1 ---------------------------------------------------------------
    w("## 1. The shape of the codebase (plan §4.1)")
    w("")
    w("| Crate | Lines | Δ plan | Share | Tests | Δ plan | `.rs` | `.wgsl` |")
    w("|---|---:|---:|---:|---:|---:|---:|---:|")
    for c in crates:
        share = 100.0 * c.lines / total_lines if total_lines else 0.0
        w(
            f"| `{c.name}` | {c.lines:,} | {delta(c.lines, PLAN_CRATE_LINES.get(c.name, 0))} "
            f"| {share:.1f}% | {c.tests} | {delta(c.tests, PLAN_CRATE_TESTS.get(c.name, 0))} "
            f"| {c.rs_files} | {c.wgsl_files} |"
        )
    w(
        f"| **Total** | **{total_lines:,}** | {delta(total_lines, PLAN_TOTAL_LINES)} | "
        f"| **{total_tests}** | {delta(total_tests, PLAN_TOTAL_TESTS)} | | |"
    )
    w("")
    top3 = sum(c.lines for c in crates[:3])
    w(
        f"The top three crates are **{100.0 * top3 / total_lines:.1f}%** of the tree "
        f"({', '.join(f'`{c.name}`' for c in crates[:3])}). The plan's finding was 85.1%."
    )
    w("")
    w(
        f"`examples/hello_engine` is **{example.rs_lines:,} lines** "
        f"({delta(example.rs_lines, PLAN_HELLO_ENGINE_LINES)} against the plan) and is still "
        "one of two programs in the repository. The second, `examples/vvardenfell`, is created "
        "by this sub-phase and is deliberately empty — the second-example rule is a rule about "
        "the *API boundary*, and an empty program that links only public crate APIs already "
        "tests part of it."
    )
    w("")

    # -- public API --------------------------------------------------------
    w("## 2. Public API surface per crate")
    w("")
    w("Not in §4; added because the runtime rule in the preamble is a claim about")
    w("public API and there was no measurement of it. A track that adds a system")
    w("without moving its crate's `pub fn` count has probably added an editor")
    w("panel, which §8 says is not a finished sub-phase.")
    w("")
    kinds = ["fn", "struct", "enum", "trait", "type", "const", "mod"]
    w("| Crate | " + " | ".join(f"`pub {k}`" for k in kinds) + " | Total |")
    w("|---" * (len(kinds) + 2) + "|")
    for c in sorted(crates, key=lambda c: c.pub_total, reverse=True):
        cells = " | ".join(str(c.pub_items.get(k, 0)) for k in kinds)
        w(f"| `{c.name}` | {cells} | **{c.pub_total}** |")
    w("")

    # -- 4.3 ---------------------------------------------------------------
    w("## 3. WGSL inventory (plan §4.3)")
    w("")
    r_files = len(renderer_shaders)
    r_lines = sum(n for _, n in renderer_shaders)
    w(
        f"`somnium_renderer` ships **{r_files} WGSL files, {r_lines:,} lines** "
        f"({delta(r_files, PLAN_WGSL_FILES)} files, {delta(r_lines, PLAN_WGSL_LINES)} lines "
        "against the plan). Repository-wide, including `somnium_ui`'s pass shader, "
        f"the count is **{len(shaders)} files, {sum(n for _, n in shaders):,} lines**."
    )
    w("")
    w("Ten largest, because these are the files a permutation system has to survive:")
    w("")
    w("| Shader | Lines |")
    w("|---|---:|")
    for path, n in sorted(shaders, key=lambda s: s[1], reverse=True)[:10]:
        w(f"| `{path}` | {n:,} |")
    w("")

    # -- 4.6 ---------------------------------------------------------------
    w("## 4. What is absent, by grep (plan §4.6)")
    w("")
    w("`grep -ril <term> crates --include=*.rs --include=*.wgsl`. A count that has")
    w("**risen** against the plan is a system somebody started; a count that has not")
    w("is a system this phase still owes.")
    w("")
    w("| Term | Files | Plan | Δ | Reading |")
    w("|---|---:|---:|---:|---|")
    for term, hits, expected, note in absences:
        w(f"| `{term}` | {hits} | {expected} | {delta(hits, expected)} | {note} |")
    w("")

    # -- 4.8 / 4.9 ---------------------------------------------------------
    w("## 5. Component schemas and environment knobs (plan §4.8, §4.9)")
    w("")
    w(
        f"- **Component schemas registered:** {schemas} "
        f"({delta(schemas, PLAN_SCHEMAS)} against the plan). Counted as "
        "`component_schema!` invocations across `crates/`; the plan's twelve counted "
        "only the registrations in `reflect_registry.rs`, and CONTROL-B added the rest. "
        "§11 row 4 makes a schema "
        "a per-sub-phase obligation, so this number is expected to rise once per new "
        "component and never on its own."
    )
    w(
        f"- **`SOMNIUM_*` variables:** {len(env_vars)} "
        f"({delta(len(env_vars), PLAN_ENV_VARS)} against the plan), over `crates/` and "
        "`examples/`. `phase_CONTROL.md` reports a different figure because it counts "
        "different directories; **CONTROL-A's generated table stays authoritative** and "
        "this row exists so the two numbers do not read as a regression (plan §4.9)."
    )
    w("")

    # -- dependencies ------------------------------------------------------
    w("## 6. Dependency justification (plan §4.7)")
    w("")
    w("Every non-workspace-internal dependency of every crate, checked against every")
    w("spelling Cargo could give it plus the derive macros it re-exports. Only rows")
    w("that are not a plain `used` are listed.")
    w("")
    w("An **UNREFERENCED** row is a *candidate* dead dependency. The check is a grep,")
    w("not a resolver, so a crate reached only through a build script or a macro this")
    w("script does not know about lands here wrongly — which is why the fix is a")
    w("stated exemption or a removal and never a looser grep. §4.7 predicted this")
    w("column would find something, and it does.")
    w("")
    if deps:
        w("| Crate | Dependency | Verdict | Reason |")
        w("|---|---|---|---|")
        for crate_name, dep, verdict, reason in deps:
            w(f"| `{crate_name}` | `{dep}` | {verdict} | {reason} |")
    else:
        w("Every dependency is justified by an identifier match.")
    w("")
    unjustified = [r for r in deps if r[2] == "UNREFERENCED"]
    w(
        f"**{len(unjustified)} unreferenced**, "
        f"**{len([r for r in deps if r[2] == 'exempt'])} exempt with a stated reason.** "
        "PORTAL-0-C removed the nine unreferenced rows this column was reporting "
        "— `rayon` from `somnium_ecs`, `pollster` from `somnium_renderer`, "
        "`base64` from `somnium_core`, `tracing` from `somnium_audio`, "
        "`somnium_voxel` and `somnium_script_luau`, `anyhow` and `rand` from "
        "`hello_engine`, and `anyhow` from the workspace — together with the "
        "dead `egui` / `egui-wgpu` / `egui-winit` triple the plan (§4.7) had "
        "left for this phase. Each was verified to appear in no source file "
        "under any spelling before it was deleted, and `cargo check "
        "--workspace --all-targets` passes without them."
    )
    w("")

    # -- footer ------------------------------------------------------------
    w("---")
    w("")
    w("**Regenerate:** `python tools/census/generate.py`")
    w("")
    w("**Gate:** `python tools/census/generate.py --check` fails when this file is")
    w("stale. GHOSTFENCE runs it, so a sub-phase that changes the tree without")
    w("regenerating the census cannot close.")
    return "\n".join(out) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="fail if the report is stale")
    parser.add_argument("--stdout", action="store_true", help="print instead of writing")
    args = parser.parse_args()

    report = render()

    if args.stdout:
        sys.stdout.write(report)
        return 0

    if args.check:
        if not REPORT.exists():
            print(f"census: {REPORT} does not exist; run tools/census/generate.py", file=sys.stderr)
            return 1
        current = REPORT.read_text(encoding="utf-8")
        if current != report:
            print(
                "census: the checked-in report no longer matches the tree.\n"
                "        run `python tools/census/generate.py` and commit the result.",
                file=sys.stderr,
            )
            return 1
        print("census: up to date")
        return 0

    REPORT.parent.mkdir(parents=True, exist_ok=True)
    REPORT.write_text(report, encoding="utf-8")
    print(f"census: wrote {REPORT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
