#!/usr/bin/env python3
"""Report the shipped shader variant set, ahead of time (MORROWIND-C, §8 item 5).

`phase_MORROWIND.md` §8 asks for *"a `tools/` cooker that compiles the shipped
variant set at build time so a release build has no first-use hitch."*

**This tool does the half that can be done without a GPU, and says so.** It
enumerates the variant set from the tree — every registered module, every
`//!if` in it, and therefore every define that could produce a variant — and
writes the budget table the plan sketches. What it deliberately does *not* do is
produce compiled pipeline binaries: wgpu has no offline pipeline cache that
survives a driver update, so a "cooked pipeline" would be a file that is
correct until the user installs a graphics driver and then silently wrong. The
honest AOT step for wgpu is **warming the in-process cache at load**, which
`ShaderSystem::request` exists for and which Track 4's cook will drive.

What this tool is genuinely good for is the thing §8 item 4 asks and that no
amount of running the engine will tell you: **how big the variant space is
against how much of it anybody uses.**

    python tools/shadercook/generate.py           # print the report
    python tools/shadercook/generate.py --check   # fail if a module is over budget

A module with six independent defines has 64 possible variants and probably
compiles eleven. If `possible` grows past `MAX_VARIANTS_PER_MODULE`, the key is
too coarse and the fix is splitting the module, not a bigger cache — and that is
a design error worth catching at build time rather than as a startup stall.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SHADERS = ROOT / "crates" / "somnium_renderer" / "src" / "shaders"
REGISTRY = ROOT / "crates" / "somnium_renderer" / "src" / "shaders.rs"

#: Past this, a module's key is too coarse. 64 is the plan's own example of a
#: module at the edge of reasonable; 128 is where "split it" stops being advice.
MAX_VARIANTS_PER_MODULE = 128

INCLUDE = re.compile(r'^\s*//!include\s+"([\w.]+)"\s*$')
CONDITION = re.compile(r"^\s*//!if\s+!?(\w+)\s*$")
REGISTERED = re.compile(r'^\s*"([\w.]+\.wgsl)",\s*$', re.MULTILINE)
DEFINE_NAME = re.compile(r'\(\s*(\w+),\s*"(\w+)"\s*\)')


def registered_modules() -> list[str]:
    """Module names, read from the `register_modules!` block in `shaders.rs`."""
    text = REGISTRY.read_text(encoding="utf-8")
    start = text.index("register_modules!(")
    end = text.index(");", start)
    return REGISTERED.findall(text[start:end])


def registered_defines() -> dict[str, int]:
    """Define names to bit indices, from `define::ALL`."""
    text = REGISTRY.read_text(encoding="utf-8")
    start = text.index("pub const ALL:")
    end = text.index(";", start)
    out = {}
    for bit_expr, name in DEFINE_NAME.findall(text[start:end]):
        # `ALL` names bits by constant, so resolve `SKINNED_BIT` to its value.
        m = re.search(rf"pub const {re.escape(bit_expr)}: u32 = (\d+);", text)
        out[name] = int(m.group(1)) if m else -1
    return out


def parse(name: str) -> tuple[list[str], set[str]]:
    """A module's direct includes and the defines it branches on."""
    path = SHADERS / name
    if not path.exists():
        return [], set()
    includes, defines = [], set()
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if m := INCLUDE.match(line):
            includes.append(m.group(1))
        elif m := CONDITION.match(line):
            defines.add(m.group(1))
    return includes, defines


def transitive(name: str, seen: set[str] | None = None) -> tuple[set[str], set[str]]:
    """Every module reachable from `name`, and every define any of them uses.

    A define used by an *included* module still multiplies the including
    module's variant space, which is the part that is easy to miss by reading
    one file — and the reason this report exists rather than a convention.
    """
    seen = seen if seen is not None else set()
    if name in seen:
        return set(), set()
    seen.add(name)
    includes, defines = parse(name)
    modules = {name}
    for child in includes:
        child_modules, child_defines = transitive(child, seen)
        modules |= child_modules
        defines |= child_defines
    return modules, defines


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="fail if a module is over budget")
    args = parser.parse_args()

    known = registered_defines()
    rows = []
    for name in registered_modules():
        modules, defines = transitive(name)
        unknown = sorted(d for d in defines if d not in known)
        count = len(defines)
        rows.append((name, len(modules), sorted(defines), unknown, 1 << min(count, 62)))

    rows.sort(key=lambda r: (-r[4], r[0]))

    width = max((len(r[0]) for r in rows), default=6)
    print(f"{'module':<{width}}  modules  defines  possible  branches on")
    over, bad = [], []
    for name, module_count, defines, unknown, possible in rows:
        names = ", ".join(defines) if defines else "-"
        print(f"{name:<{width}}  {module_count:>7}  {len(defines):>7}  {possible:>8}  {names}")
        if possible > MAX_VARIANTS_PER_MODULE:
            over.append((name, possible))
        if unknown:
            bad.append((name, unknown))

    total = sum(r[4] for r in rows)
    print()
    print(f"{len(rows)} modules, {total} variants possible in total.")
    print(
        f"Defines registered in shaders.rs: "
        f"{', '.join(f'{n}={b}' for n, b in sorted(known.items())) or '(none)'}"
    )
    print()
    print(
        "Pipelines are not cooked to disk: wgpu has no offline cache that survives\n"
        "a driver update, so a cooked pipeline is a file that is correct until the\n"
        "user updates their driver. The AOT step for wgpu is warming the in-process\n"
        "cache at load - `ShaderSystem::request`, driven by Track 4's cook."
    )

    if bad:
        print()
        for name, unknown in bad:
            print(
                f"ERROR {name}: branches on unregistered define(s) "
                f"{', '.join(unknown)} - add them to `define::ALL` in shaders.rs",
                file=sys.stderr,
            )
    if over:
        print()
        for name, possible in over:
            print(
                f"ERROR {name}: {possible} possible variants exceeds the "
                f"{MAX_VARIANTS_PER_MODULE} budget - split the module rather than "
                "growing the cache",
                file=sys.stderr,
            )

    if args.check and (over or bad):
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
