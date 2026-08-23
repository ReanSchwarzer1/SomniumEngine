#!/usr/bin/env python3
"""Generate Phase CONTROL's reachability and hand-wiring audits.

The audit deliberately uses only Python's standard library.  It reads Rust
source as a constrained language: string literals identify environment knobs,
the component-schema macro identifies reflected fields, and the legacy editor
enums/functions identify hand-wired rows.  It is not a general Rust parser;
every inference and fallback is called out in the generated report.
"""

from __future__ import annotations

import argparse
import difflib
import re
import sys
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
REACHABILITY = ROOT / "dev records" / "phase CONTROL" / "CONTROL-A_reachability.md"
CENSUS = ROOT / "dev records" / "phase CONTROL" / "CONTROL-A_census.md"


@dataclass(frozen=True)
class Occurrence:
    path: Path
    line: int
    line_text: str
    context: str
    usage: str


@dataclass(frozen=True)
class Schema:
    rust_type: str
    stable_id: str
    fields: tuple[str, ...]
    editable_fields: tuple[str, ...]


def rust_sources() -> list[Path]:
    roots = (ROOT / "crates", ROOT / "examples")
    return sorted(path for base in roots for path in base.rglob("*.rs"))


def relative(path: Path) -> str:
    return path.relative_to(ROOT).as_posix()


def line_number(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def brace_body(text: str, open_brace: int) -> tuple[str, int]:
    """Return the contents and closing offset of one balanced brace block."""
    depth = 0
    in_string = False
    escaped = False
    for index in range(open_brace, len(text)):
        char = text[index]
        if in_string:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
            continue
        if char == '"':
            in_string = True
        elif char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return text[open_brace + 1 : index], index
    raise ValueError(f"unclosed brace at byte {open_brace}")


def top_level_items(body: str) -> list[str]:
    # Comments frequently contain commas and comparison signs; neither is
    # Rust syntax for the surrounding enum/struct. Remove them before the
    # delimiter walk so prose cannot turn into phantom variants.
    body = re.sub(r"/\*.*?\*/", "", body, flags=re.S)
    body = re.sub(r"//.*$", "", body, flags=re.M)
    items: list[str] = []
    start = 0
    depth = 0
    in_string = False
    escaped = False
    for index, char in enumerate(body):
        if in_string:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
            continue
        if char == '"':
            in_string = True
        elif char in "{([":
            depth += 1
        elif char in "})]":
            depth = max(0, depth - 1)
        elif char == "," and depth == 0:
            item = body[start:index].strip()
            if item:
                items.append(item)
            start = index + 1
    tail = body[start:].strip()
    if tail:
        items.append(tail)
    return items


def strip_comments(item: str) -> str:
    lines = []
    for line in item.splitlines():
        clean = re.sub(r"//.*$", "", line).strip()
        if clean and not clean.startswith("#"):
            lines.append(clean)
    return " ".join(lines)


def enum_variants(text: str, name: str) -> list[str]:
    match = re.search(rf"\benum\s+{re.escape(name)}\s*\{{", text)
    if not match:
        return []
    body, _ = brace_body(text, text.index("{", match.start()))
    out = []
    for item in top_level_items(body):
        clean = strip_comments(item)
        variant = re.match(r"([A-Za-z_][A-Za-z0-9_]*)", clean)
        if variant:
            out.append(variant.group(1))
    return out


def named_struct_fields(text: str, name: str) -> list[str]:
    match = re.search(rf"\bstruct\s+{re.escape(name)}(?:\s*<[^{{;]+>)?\s*\{{", text)
    if not match:
        return []
    body, _ = brace_body(text, text.index("{", match.start()))
    fields = []
    for item in top_level_items(body):
        clean = strip_comments(item)
        field = re.match(r"(?:pub(?:\([^)]*\))?\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*:", clean)
        if field:
            fields.append(field.group(1))
    return fields


def source_occurrences() -> dict[str, list[Occurrence]]:
    found: dict[str, list[Occurrence]] = {}
    active = re.compile(r"\b(SOMNIUM_[A-Z0-9_]+)\b")
    for path in rust_sources():
        text = path.read_text(encoding="utf-8")
        lines = text.splitlines()
        for match in active.finditer(text):
            line = line_number(text, match.start())
            # Keep executable references and documented command examples. The
            # latter are part of the public knob contract and often supply the
            # only concise meaning for a capture-only switch.
            lo = max(0, line - 5)
            hi = min(len(lines), line + 4)
            occurrence = Occurrence(
                path=path,
                line=line,
                line_text=lines[line - 1].strip(),
                context=" ".join(part.strip() for part in lines[lo:hi]),
                usage=text[max(0, match.start() - 32) : min(len(text), match.end() + 220)].replace("\n", " "),
            )
            found.setdefault(match.group(1), []).append(occurrence)
    return found


def primary_occurrence(items: list[Occurrence]) -> Occurrence:
    def score(item: Occurrence) -> tuple[int, str, int]:
        executable = int("std::env::" not in item.usage and not re.search(r"\b(?:num|flag)\s*\(", item.usage))
        return executable, relative(item.path), item.line

    return sorted(items, key=score)[0]


def nearest_comment(occurrence: Occurrence) -> str:
    lines = occurrence.path.read_text(encoding="utf-8").splitlines()
    comments: list[str] = []
    for index in range(occurrence.line - 2, max(-1, occurrence.line - 9), -1):
        line = lines[index].strip()
        if line.startswith(("//", "//!", "///")):
            comments.append(re.sub(r"^//[/!]?[ ]?", "", line))
        elif not line or line.startswith("#"):
            continue
        else:
            break
    text = " ".join(reversed(comments)).strip()
    text = re.sub(r"`", "", text)
    text = re.sub(r"\s+", " ", text)
    return text[:180] + ("…" if len(text) > 180 else "")


def infer_type(name: str, occurrences: list[Occurrence]) -> str:
    context = " ".join(item.usage for item in occurrences)
    if name.endswith(("_PATH", "_FILE", "_DIR")) or name in {
        "SOMNIUM_CAPTURE", "SOMNIUM_CAPTURE_COMPARE", "SOMNIUM_CAPTURE_PNG",
        "SOMNIUM_CAPTURE_DISPLAY_PNG", "SOMNIUM_CAPTURE_UI_PNG", "SOMNIUM_HEIGHTMAP",
        "SOMNIUM_IMPORT", "SOMNIUM_MAP", "SOMNIUM_SCRIPT_CACHE", "SOMNIUM_TIME",
        "SOMNIUM_TIME_COMPARE", "SOMNIUM_TIME_LABEL", "SOMNIUM_TIME_VIEW",
    }:
        return "path/text"
    if re.search(rf'{name}"\)\.as_deref\(\)\s*[!=]=\s*Ok\("[01]"\)', context):
        return "bool"
    if re.search(rf'{name}"\)\.is_ok(?:_and)?', context):
        return "bool"
    if name.endswith(("_MS", "_FLOOR", "_RADIUS", "_PITCH", "_YAW", "_DEGREES", "_ELEVATION", "_AZIMUTH", "_RELIEF")):
        return "float"
    if name.endswith(("_FRAME", "_FRAMES", "_WARMUP", "_EVERY", "_BOUNCES", "_RES")):
        return "integer"
    if re.search(r"parse::<(?:u|i)(?:8|16|32|64|size)>", context):
        return "integer"
    if re.search(r"parse::<f(?:32|64)>", context):
        return "float"
    if re.search(rf'num\("{name}"', context):
        return "number"
    if re.search(r"unwrap_or\(\s*-?[0-9]+\s*\)", context):
        return "integer"
    return "text/selector"


def infer_default(name: str, occurrences: list[Occurrence]) -> str:
    context = " ".join(item.usage for item in occurrences)
    if name in {
        "SOMNIUM_CAPTURE", "SOMNIUM_CAPTURE_COMPARE", "SOMNIUM_CAPTURE_PNG",
        "SOMNIUM_CAPTURE_DISPLAY_PNG", "SOMNIUM_CAPTURE_UI_PNG",
        "SOMNIUM_IMPORT", "SOMNIUM_SCRIPT_CACHE", "SOMNIUM_TIME", "SOMNIUM_TIME_COMPARE",
    }:
        return "unset"
    if re.search(rf'{name}"\)\.as_deref\(\)\s*==\s*Ok\("1"\)', context):
        return "off (unset/other)"
    if re.search(rf'{name}"\)\.as_deref\(\)\s*!=\s*Ok\("0"\)', context):
        return "on (unset/other)"
    helper = re.search(rf'num\("{name}"\s*,\s*([^\),]+)', context)
    if helper:
        return helper.group(1).strip()
    literal = re.search(r"unwrap_or(?:_else)?\([^\n]{0,80}?([0-9]+(?:\.[0-9]+)?|\"[^\"]*\")", context)
    if literal:
        return literal.group(1).strip('"')
    lowered = context.lower()
    if "default off" in lowered:
        return "off"
    if "default on" in lowered:
        return "on"
    return "unset / source-defined fallback"


ACRONYMS = {"Ao": "AO", "Cas": "CAS", "Fsr": "FSR", "Fxaa": "FXAA", "Gi": "GI", "Gtao": "GTAO", "Ibl": "IBL", "Pcss": "PCSS", "Restir": "RESTIR", "Rt": "RT", "Sdf": "SDF", "Ssr": "SSR", "Taa": "TAA"}


def screaming_variant(name: str) -> str:
    words = re.findall(r"[A-Z][a-z0-9]*|[A-Z]+(?=[A-Z]|$)", name)
    return "_".join(ACRONYMS.get(word, word.upper()) for word in words)


def editor_reachability() -> dict[str, str]:
    path = ROOT / "crates/somnium_ui/src/editor_event.rs"
    if not path.exists():
        return {}
    variants = enum_variants(path.read_text(encoding="utf-8"), "PostFxToggle")
    return {f"SOMNIUM_{screaming_variant(variant)}": f"PostFxToggle::{variant}" for variant in variants}


def schema_blocks() -> list[Schema]:
    schemas: list[Schema] = []
    for path in (ROOT / "crates/somnium_core/src").rglob("*.rs"):
        text = path.read_text(encoding="utf-8")
        cursor = 0
        while True:
            match = re.search(r"component_schema!\s*\{", text[cursor:])
            if not match:
                break
            start = cursor + match.start()
            open_brace = text.index("{", start)
            body, close = brace_body(text, open_brace)
            header = re.search(r"\b([A-Za-z_][A-Za-z0-9_]*)\s+as\s+\"([^\"]+)\"", body)
            fields_match = re.search(r"\bfields\s*\{", body)
            if header and fields_match:
                fields_open = body.index("{", fields_match.start())
                fields_body, _ = brace_body(body, fields_open)
                names: list[str] = []
                editable: list[str] = []
                for item in top_level_items(fields_body):
                    clean = strip_comments(item)
                    field = re.match(r"([A-Za-z_][A-Za-z0-9_]*)", clean)
                    if not field:
                        continue
                    name = field.group(1)
                    names.append(name)
                    flags = re.search(r"flags\s*:\s*([^}]+)", clean)
                    # `component_schema!` assigns FieldFlags::DEFAULT when a
                    # declaration omits `flags`; DEFAULT contains EDIT.  An
                    # explicit expression is editable only when it names EDIT
                    # or DEFAULT.  Treating every non-RUNTIME_ONLY expression
                    # as editable incorrectly exposed engine/script-only data
                    # such as RigidBodyComponent::body.
                    explicit = flags.group(1) if flags else None
                    if explicit is None or "FieldFlags::EDIT" in explicit or "FieldFlags::DEFAULT" in explicit:
                        editable.append(name)
                schemas.append(Schema(header.group(1), header.group(2), tuple(names), tuple(editable)))
            cursor = close + 1

    # The two intentionally hand-written schemas expose one field each.
    registry = (ROOT / "crates/somnium_core/src/reflect_registry.rs").read_text(encoding="utf-8")
    if "fn name_schema()" in registry:
        schemas.append(Schema("Name", "somnium.Name", ("value",), ("value",)))
    if "fn mesh_kind_schema()" in registry:
        schemas.append(Schema("MeshKind", "somnium.MeshKind", ("kind",), ("kind",)))
    return sorted({schema.stable_id: schema for schema in schemas}.values(), key=lambda schema: schema.stable_id)


def declared_types() -> dict[str, tuple[Path, int | None]]:
    declarations: dict[str, tuple[Path, int | None]] = {}
    candidates: set[str] = set()
    generic = re.compile(r"(?:get|get_mut|remove_component|ComponentId::of)\s*::\s*<\s*(?:crate::)?([A-Za-z_][A-Za-z0-9_]*)\s*>")
    for path in (ROOT / "crates/somnium_core/src").rglob("*.rs"):
        text = path.read_text(encoding="utf-8")
        candidates.update(generic.findall(text))
    candidates.update(schema.rust_type for schema in schema_blocks())

    declaration_files = list((ROOT / "crates").rglob("*.rs"))
    for name in sorted(candidates):
        for path in declaration_files:
            text = path.read_text(encoding="utf-8")
            if not re.search(rf"\b(?:struct|enum)\s+{re.escape(name)}\b", text):
                continue
            fields = named_struct_fields(text, name)
            if fields:
                count: int | None = len(fields)
            else:
                tuple_match = re.search(rf"\bstruct\s+{re.escape(name)}\s*\((.*?)\)\s*;", text, re.S)
                count = len(top_level_items(tuple_match.group(1))) if tuple_match else None
            declarations[name] = (path, count)
            break
    return declarations


def legacy_rows() -> tuple[dict[str, int], dict[str, int]]:
    event_path = ROOT / "crates/somnium_ui/src/editor_event.rs"
    lib_path = ROOT / "crates/somnium_ui/src/lib.rs"
    if not event_path.exists() or not lib_path.exists():
        return {}, {}
    event = event_path.read_text(encoding="utf-8")
    lib = lib_path.read_text(encoding="utf-8")
    binding_variants = re.findall(r"\bIF::([A-Za-z_][A-Za-z0-9_]*)", function_body(lib, "field_bindings"))
    colors = enum_variants(event, "ColorField")
    toggles = enum_variants(event, "PostFxToggle")

    def owner(variant: str) -> str:
        for prefix, component in (
            ("Light", "LightComponent"), ("Terrain", "TerrainComponent"),
            ("Camera", "CameraSettingsComponent"), ("Post", "PostProcessComponent"),
            ("Water", "WaterComponent"), ("Vessel", "BuoyantVessel"),
            ("Foliage", "FoliageComponent"), ("Material", "MaterialComponent"),
            ("Particle", "ParticleEmitter"),
        ):
            if variant.startswith(prefix):
                return component
        if variant.startswith(("Pos", "Rot", "Scale")):
            return "Transform"
        return "unassigned"

    inspector_fields: dict[str, int] = {}
    rows: dict[str, int] = {}
    for variant in binding_variants:
        component = owner(variant)
        inspector_fields[component] = inspector_fields.get(component, 0) + 1
        rows[component] = rows.get(component, 0) + 1
    for variant in colors:
        component = owner(variant)
        rows[component] = rows.get(component, 0) + 1
    rows["PostProcessComponent"] = rows.get("PostProcessComponent", 0) + len(toggles)
    return rows, inspector_fields


def function_body(text: str, name: str) -> str:
    match = re.search(rf"\bfn\s+{re.escape(name)}\b[^{{]*\{{", text)
    if not match:
        return ""
    body, _ = brace_body(text, text.index("{", match.start()))
    return body


def generated_inspector_fields() -> set[tuple[str, str]]:
    """Conservative static contract for CONTROL-B's schema consumer.

    Until an inspector generator exists, coverage is empty.  Once it exists it
    must visibly iterate schema fields, filter EDIT, and build PropertyRows;
    only then does the gate credit all registry fields reached by that generic
    path.  Runtime widget tests remain CONTROL-B's responsibility.
    """
    candidates = list((ROOT / "crates/somnium_ui/src/editor").glob("*inspector_gen*.rs"))
    text = "\n".join(path.read_text(encoding="utf-8") for path in candidates)
    contract = ("FieldFlags::EDIT", "schema.fields", "PropertyRow")
    if not text or not all(token in text for token in contract):
        return set()
    return {(schema.stable_id, field) for schema in schema_blocks() for field in schema.editable_fields}


LEGACY_EDITORS = {
    "Bool", "I64", "F64", "Str", "Vec2", "Vec3", "Vec4", "Quat", "Color", "Enum"
}


def field_type_variants() -> list[str]:
    text = (ROOT / "crates/somnium_ecs/src/reflect.rs").read_text(encoding="utf-8")
    return enum_variants(text, "FieldType")


def registered_property_editors() -> set[str]:
    files = list((ROOT / "crates/somnium_ui/src/editor").rglob("*property_editor*.rs"))
    files += list((ROOT / "crates/somnium_ui/src/editor/property_editors").rglob("*.rs")) if (ROOT / "crates/somnium_ui/src/editor/property_editors").exists() else []
    if not files:
        # The legacy panel visibly supports these shapes, but has no editor for
        # references or arrays.  This is the precise day-one failure CONTROL-A
        # records; CONTROL-B replaces this fallback with a registry.
        return set(LEGACY_EDITORS)
    text = "\n".join(path.read_text(encoding="utf-8") for path in sorted(set(files)))
    return {variant for variant in field_type_variants() if re.search(rf"FieldType::{variant}\b", text)}


def missing_editable_fields() -> list[str]:
    covered = generated_inspector_fields()
    return [f"{schema.stable_id}.{field}" for schema in schema_blocks() for field in schema.editable_fields if (schema.stable_id, field) not in covered]


def missing_property_editors() -> list[str]:
    covered = registered_property_editors()
    return [variant for variant in field_type_variants() if variant not in covered]


def hand_wiring_census() -> dict[str, int]:
    event_path = ROOT / "crates/somnium_ui/src/editor_event.rs"
    lib_path = ROOT / "crates/somnium_ui/src/lib.rs"
    app_path = ROOT / "crates/somnium_core/src/app.rs"
    event = event_path.read_text(encoding="utf-8") if event_path.exists() else ""
    lib = lib_path.read_text(encoding="utf-8") if lib_path.exists() else ""
    app = app_path.read_text(encoding="utf-8") if app_path.exists() else ""
    values = {
        "InspectorField variants": len(enum_variants(event, "InspectorField")),
        "ColorField variants": len(enum_variants(event, "ColorField")),
        "PostFxToggle variants": len(enum_variants(event, "PostFxToggle")),
        "InspectorHandles fields": len(named_struct_fields(lib, "InspectorHandles")),
        "field_bindings rows": len(re.findall(r"\(\s*h\.[^,]+,\s*IF::", function_body(lib, "field_bindings"))),
        "IF:: occurrences in app.rs": len(re.findall(r"\bIF::", app)),
    }
    values["total"] = sum(values.values())
    return values


def markdown_escape(value: str) -> str:
    return value.replace("|", "\\|").replace("\n", " ")


def generate_reachability() -> str:
    occurrences = source_occurrences()
    reachable = editor_reachability()
    schemas = schema_blocks()
    components = declared_types()
    rows, inspector_fields = legacy_rows()
    census = hand_wiring_census()
    generated = generated_inspector_fields()

    lines = [
        "# CONTROL-A reachability audit",
        "",
        "> Generated by `python tools/reachability/generate.py`. Do not edit this file by hand.",
        "> Run the command again and require an empty diff; `--check` verifies that in CI.",
        "",
        "## Summary",
        "",
        f"- Distinct `SOMNIUM_*` identifiers in executable/example Rust sources: **{len(occurrences)}**.",
        f"- Knobs with a mechanically matched legacy editor route: **{sum(1 for name in occurrences if name in reachable)}**.",
        f"- Reflected component schemas: **{len(schemas)}**; editable fields: **{sum(len(s.editable_fields) for s in schemas)}**.",
        f"- Editable schema fields credited to the generated inspector path: **{len(generated)}**.",
        f"- Current Details hand-wiring census: **{census['total']}** identifiers.",
        "",
        "The environment inventory includes identifiers in executable code and documented command examples. "
        "Meaning, type, and default are conservative source inferences; ambiguous fallbacks say so. "
        "Editor reach is intentionally stricter: a knob is `yes` only when its suffix mechanically matches "
        "a live `PostFxToggle` variant. This reproduces the legacy 18-route definition without a hand-written allow-list.",
        "",
        "## Environment knobs",
        "",
        "| Variable | Source | Meaning | Type | Default | Editor reach | Evidence |",
        "|---|---|---|---|---|---:|---|",
    ]
    for name in sorted(occurrences):
        items = occurrences[name]
        primary = primary_occurrence(items)
        comment = nearest_comment(primary)
        if name not in comment:
            comment = name.removeprefix("SOMNIUM_").replace("_", " ").capitalize()
        evidence = reachable.get(name, "—")
        lines.append(
            f"| `{name}` | `{relative(primary.path)}:{primary.line}` | {markdown_escape(comment)} | "
            f"{infer_type(name, items)} | {markdown_escape(infer_default(name, items))} | "
            f"{'yes' if evidence != '—' else 'no'} | `{evidence}` |"
        )

    lines += [
        "",
        "## Components",
        "",
        "`Source fields` counts named Rust struct fields where that is statically unambiguous. "
        "The inventory contains declared types used through core World component access or `ComponentId::of`, plus every registered schema type. "
        "`Legacy rows` counts `field_bindings` plus routed colour/toggle variants. "
        "`Generated EDIT rows` is deliberately zero until CONTROL-B's generic schema consumer exists.",
        "",
        "| Component type | Declaration | Source fields | Schema | Schema fields | EDIT fields | Legacy rows | InspectorField variants | Generated EDIT rows |",
        "|---|---|---:|---|---:|---:|---:|---:|---:|",
    ]
    by_type = {schema.rust_type: schema for schema in schemas}
    for name, (path, source_fields) in sorted(components.items()):
        schema = by_type.get(name)
        generated_count = sum((schema.stable_id, field) in generated for field in schema.editable_fields) if schema else 0
        lines.append(
            f"| `{name}` | `{relative(path)}` | {source_fields if source_fields is not None else '—'} | "
            f"{f'`{schema.stable_id}`' if schema else 'no'} | {len(schema.fields) if schema else 0} | "
            f"{len(schema.editable_fields) if schema else 0} | {rows.get(name, 0)} | "
            f"{inspector_fields.get(name, 0)} | {generated_count} |"
        )

    lines += [
        "",
        "## Hand-wiring census",
        "",
        "| Surface | Count |",
        "|---|---:|",
    ]
    for key, value in census.items():
        lines.append(f"| {key} | **{value}** |")

    lines += [
        "",
        "## CONTROL-B completeness gates",
        "",
        f"- Missing generated inspector rows: **{len(missing_editable_fields())}** (`SOMNIUM_CONTROL_B_GATES=1 python -m unittest tools.reachability.test_control_b_gates`).",
        f"- Missing registered property editors: **{', '.join(missing_property_editors()) or 'none'}**.",
        "- The tests are opt-in and skipped in ordinary test discovery, because CONTROL-A intentionally records red gates that CONTROL-B closes.",
        "",
        "## Reproduction",
        "",
        "```text",
        "python tools/reachability/generate.py",
        "python tools/reachability/generate.py --check",
        "$env:SOMNIUM_CONTROL_B_GATES='1'; python -m unittest tools.reachability.test_control_b_gates",
        "```",
        "",
    ]
    return "\n".join(lines)


def generate_census() -> str:
    census = hand_wiring_census()
    return "\n".join([
        "# CONTROL hand-wiring census",
        "",
        "CONTROL-A's baseline is generated from the current tree. Later sub-phases add a row after regenerating the reachability report.",
        "",
        "| Sub-phase | InspectorField | ColorField | PostFxToggle | InspectorHandles | field_bindings | IF:: in app.rs | Total |",
        "|---|---:|---:|---:|---:|---:|---:|---:|",
        f"| CONTROL-A | {census['InspectorField variants']} | {census['ColorField variants']} | {census['PostFxToggle variants']} | {census['InspectorHandles fields']} | {census['field_bindings rows']} | {census['IF:: occurrences in app.rs']} | **{census['total']}** |",
        "",
    ])


def check_or_write(path: Path, content: str, check: bool) -> bool:
    content = content.replace("\r\n", "\n")
    old = path.read_text(encoding="utf-8").replace("\r\n", "\n") if path.exists() else ""
    if old == content:
        return True
    if check:
        print("".join(difflib.unified_diff(old.splitlines(True), content.splitlines(True), fromfile=str(path), tofile="generated")))
        return False
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8", newline="\n")
    return True


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true", help="fail if checked-in reports differ")
    parser.add_argument("--gate", choices=("inspector", "editors"), help="run one expected CONTROL-B gate")
    args = parser.parse_args()
    if args.gate == "inspector":
        missing = missing_editable_fields()
        for item in missing:
            print(f"missing inspector row: {item}")
        return int(bool(missing))
    if args.gate == "editors":
        missing = missing_property_editors()
        for item in missing:
            print(f"missing property editor: FieldType::{item}")
        return int(bool(missing))
    ok = check_or_write(REACHABILITY, generate_reachability(), args.check)
    ok = check_or_write(CENSUS, generate_census(), args.check) and ok
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
