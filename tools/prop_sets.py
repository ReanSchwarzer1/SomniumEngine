"""Turn a scene's prop groups into `somnium.PropSet` entities (open-world render scale, 19 Oct 2026).

A prop group is a top-level entity holding only a name and a transform, whose children each hold only a mesh of one
glTF: `somnium.ImportedMesh`, a `somnium.Name`, an identity `somnium.Transform`, the `somnium.Parent` and a
`somnium.Material` with no override. It is how a kit places a barrel or a well. Groups that draw the same source and
the same nodes, inside one square of `--area` metres, become one prop set at the square's centre; each group becomes
a copy (its position, its yaw or rotation, its uniform scale). Anything else is left as it was.

    python tools/prop_sets.py IN.scene.json OUT.scene.json [--area 128] [--draw-distance 0]
                              [--shadow-distance 0] [--collide-bounds] [--min-copies 1]

Never writes IN. The authoring contract is `docs/editor/prop_sets.md`.
"""
import argparse
import hashlib
import json
import math
import sys

MAGIC = "SOMNIUM-SCENE"
CHILD_KEYS = {"somnium.Name", "somnium.Transform", "somnium.Parent", "somnium.ImportedMesh", "somnium.Material"}
GROUP_KEYS = {"somnium.Name", "somnium.Transform"}


def read_scene(path):
    """The header (or None) and the document of a framed or bare scene file."""
    text = open(path, encoding="utf-8").read()
    if text.startswith(MAGIC):
        first, rest = text.split("\n", 1)
        length = int(first.split()[2])
        return json.loads(rest[:length]), json.loads(rest[length + 1:])
    return None, json.loads(text)


def write_scene(path, header, document):
    body = json.dumps(document, indent=2)
    if header is None:
        open(path, "w", encoding="utf-8", newline="").write(body)
        return
    header = dict(header, entity_count=len(document["entities"]))
    head = json.dumps(header, separators=(",", ":"))
    with open(path, "w", encoding="utf-8", newline="") as out:
        out.write(f"{MAGIC} 1 {len(head.encode('utf-8'))}\n{head}\n{body}")


def fields(entity, key):
    return entity["components"].get(key, {}).get("fields", {})


def identity(transform):
    return (all(abs(v) < 1e-6 for v in transform.get("translation", [0, 0, 0]))
            and all(abs(a - b) < 1e-6 for a, b in zip(transform.get("rotation", [0, 0, 0, 1]), [0, 0, 0, 1]))
            and all(abs(v - 1) < 1e-6 for v in transform.get("scale", [1, 1, 1])))


def yaw_of(rotation):
    """The yaw of a rotation about +Y alone, or None."""
    x, y, z, w = rotation
    if abs(x) > 1e-5 or abs(z) > 1e-5:
        return None
    return 2.0 * math.atan2(y, w)


def convert(document, area, draw_distance, shadow_distance, collide_bounds, min_copies):
    entities = document["entities"]
    by_id = {e["persistent_id"]: e for e in entities}
    children = {}
    for e in entities:
        parent = fields(e, "somnium.Parent").get("entity")
        if isinstance(parent, dict) and "$entity" in parent:
            children.setdefault(parent["$entity"], []).append(e)
    # Candidate groups: top level, a name and a transform only, and children that are only kit meshes.
    kinds = {}
    for pid, kids in children.items():
        group = by_id.get(pid)
        if group is None or set(group["components"]) - GROUP_KEYS or group.get("scripts") or group.get("prefab"):
            continue
        if any(set(k["components"]) - CHILD_KEYS or "somnium.ImportedMesh" not in k["components"]
               or k.get("scripts") or k["persistent_id"] in children
               or not identity(fields(k, "somnium.Transform"))
               or fields(k, "somnium.Material").get("asset") is not None for k in kids):
            continue
        sources = {fields(k, "somnium.ImportedMesh")["source"] for k in kids}
        if len(sources) != 1:
            continue
        transform = fields(group, "somnium.Transform")
        scale = transform.get("scale", [1, 1, 1])
        if max(scale) - min(scale) > 1e-5:
            continue
        nodes = sorted(fields(k, "somnium.ImportedMesh")["node"] for k in kids)
        name = fields(group, "somnium.Name").get("value", "prop")
        key = (sources.pop(), tuple(nodes))
        kinds.setdefault(key, []).append((pid, name, transform, scale[0], kids))
    removed = set()
    sets = []
    for (source, nodes), groups in sorted(kinds.items()):
        squares = {}
        for g in groups:
            x, _, z = g[2].get("translation", [0, 0, 0])
            squares.setdefault((math.floor(x / area), math.floor(z / area)), []).append(g)
        for (sx, sz), members in sorted(squares.items()):
            if len(members) < min_copies:
                continue
            ys = [m[2].get("translation", [0, 0, 0])[1] for m in members]
            centre = [(sx + 0.5) * area, sum(ys) / len(ys), (sz + 0.5) * area]
            copies, rotations, scales = [], [], []
            yaw_only = all(yaw_of(m[2].get("rotation", [0, 0, 0, 1])) is not None for m in members)
            for _, _, transform, scale, _ in members:
                x, y, z = transform.get("translation", [0, 0, 0])
                rotation = transform.get("rotation", [0, 0, 0, 1])
                yaw = yaw_of(rotation) if yaw_only else 0.0
                copies.append([round(x - centre[0], 4), round(y - centre[1], 4), round(z - centre[2], 4), round(yaw, 5)])
                rotations.append([round(v, 6) for v in rotation])
                scales.append(round(scale, 5))
            prop = {
                "enabled": True,
                "source": source,
                "parts": list(nodes),
                "copies": copies,
                "draw_distance": draw_distance,
                "casts_shadow": True,
                "shadow_distance": shadow_distance,
                "collide_bounds": collide_bounds,
            }
            if not yaw_only:
                prop["rotations"] = rotations
            if any(abs(s - 1.0) > 1e-5 for s in scales):
                prop["scales"] = scales
            label = members[0][1]
            key = f"{source}|{nodes}|{sx}|{sz}".encode()
            sets.append({
                "components": {
                    "somnium.Name": {"fields": {"value": f"{label} x{len(members)}"}, "version": 1},
                    "somnium.Transform": {"fields": {"translation": centre, "rotation": [0.0, 0.0, 0.0, 1.0],
                                                     "scale": [1.0, 1.0, 1.0]}, "version": 1},
                    "somnium.PropSet": {"fields": prop, "version": 1},
                },
                "persistent_id": hashlib.md5(key).hexdigest(),
                "prefab": None,
                "scripts": [],
            })
            for pid, _, _, _, kids in members:
                removed.add(pid)
                removed.update(k["persistent_id"] for k in kids)
    kept = [e for e in entities if e["persistent_id"] not in removed]
    document = dict(document, entities=kept + sets)
    copies = sum(len(s["components"]["somnium.PropSet"]["fields"]["copies"]) for s in sets)
    return document, {"entities_before": len(entities), "entities_after": len(kept) + len(sets),
                      "sets": len(sets), "copies": copies, "removed": len(removed)}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("scene_in")
    parser.add_argument("scene_out")
    parser.add_argument("--area", type=float, default=128.0, help="square, metres, a set's copies come from")
    parser.add_argument("--draw-distance", type=float, default=0.0)
    parser.add_argument("--shadow-distance", type=float, default=0.0)
    parser.add_argument("--collide-bounds", action="store_true")
    parser.add_argument("--min-copies", type=int, default=1, help="fewer groups of a kind in a square stay entities")
    args = parser.parse_args(argv)
    if args.scene_in == args.scene_out:
        sys.exit("prop_sets.py never writes its input: give another OUT")
    header, document = read_scene(args.scene_in)
    document, report = convert(document, args.area, args.draw_distance, args.shadow_distance,
                               args.collide_bounds, args.min_copies)
    write_scene(args.scene_out, header, document)
    print(json.dumps(report))


if __name__ == "__main__":
    main()
