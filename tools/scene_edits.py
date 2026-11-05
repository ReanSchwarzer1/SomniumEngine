"""Keep the editor's edits (`<scene>.edits.json`) matched to a scene a tool has just regenerated.

The engine applies a scene's edit layer every time the scene loads (editor and game alike), and finds again any object
whose id a generator changed, by its fingerprint: name, mesh, parent's name and the position the generator gave it.
This module does the same matching right after a tool writes a scene, so the tool can say at once which edits still
land, which it re-keyed and which match nothing any more, and so the layer on disk carries the new ids. It never
changes the scene itself, and it never drops an edit: what cannot be matched stays in the file, reported.

    python tools/scene_edits.py scenes/campaign_town.scene.json           # report
    python tools/scene_edits.py scenes/campaign_town.scene.json --fix     # also re-key the layer
    python tools/scene_edits.py --self-check

A generator calls `after_write(scene_path, document)` after writing a scene (or `after_compose` for a streamed world).
Both print one line when the scene has a layer and say nothing otherwise; neither raises. The rules mirror
`crates/somnium_core/src/scene_edits.rs` (the engine's half); keep them in step. Stdlib only.
"""
import json
import math
import os
import sys
from pathlib import Path

FORMAT = "somnium-scene-edits"
SAME_OBJECT_RADIUS = 5.0  # an id whose object moved further than this is a different object
REMATCH_RADIUS = 2.0      # a lost id is found again only within this distance of where it was


def layer_path(scene):
    return Path(str(scene) + ".edits.json")


def read_scene(path):
    """A scene document, from the `SOMNIUM-SCENE` container or plain JSON."""
    text = Path(path).read_text(encoding="utf-8")
    if text.startswith("SOMNIUM-SCENE"):
        text = text.split("\n", 2)[2]
    return json.loads(text)


def read_layer(scene):
    path = layer_path(scene)
    if not path.exists():
        return None
    layer = json.loads(path.read_text(encoding="utf-8"))
    if layer.get("format") != FORMAT:
        raise ValueError(f"{path}: not a {FORMAT} file")
    return layer


def write_layer(scene, layer):
    path = layer_path(scene)
    layer["entities"].sort(key=lambda entry: entry["id"])
    temporary = path.with_name(path.name + ".writing")
    temporary.write_text(json.dumps(layer, indent=2) + "\n", encoding="utf-8", newline="\n")
    os.replace(temporary, path)


def _field(entity, component, name):
    return entity.get("components", {}).get(component, {}).get("fields", {}).get(name)


def name_of(entity):
    return _field(entity, "somnium.Name", "value") or ""


def mesh_of(entity):
    source = _field(entity, "somnium.ImportedMesh", "source")
    if source is not None:
        return f"{source}#{_field(entity, 'somnium.ImportedMesh', 'node') or 0}"
    kind = entity.get("components", {}).get("somnium.MeshKind")
    # The engine writes the fields' JSON text; only presence matters for matching here.
    return json.dumps(kind["fields"], separators=(",", ":")) if kind else ""


def translation_of(entity):
    value = _field(entity, "somnium.Transform", "translation")
    return [float(v) for v in value[:3]] if isinstance(value, list) and len(value) >= 3 else None


def parent_id_of(entity):
    parent = _field(entity, "somnium.Parent", "entity")
    return parent.get("$entity") if isinstance(parent, dict) else None


def _distance(a, b):
    return math.dist(a, b)


def same_name(recorded, name):
    """The same name, or the engine's 63-byte cut of it (recorded names come from the live world)."""
    return recorded == name or (len(recorded.encode("utf-8")) >= 60 and name.startswith(recorded))


class _Index:
    """Every entity across one or more documents: by id, with its file and its parent's name."""

    def __init__(self, documents):
        self.entities = []  # (file, entity)
        for file, document in documents.items():
            self.entities += [(file, e) for e in document.get("entities", [])]
        self.by_id = {e.get("persistent_id"): i for i, (_, e) in enumerate(self.entities)}
        names = {e.get("persistent_id"): name_of(e) for _, e in self.entities}
        self.parent_names = [names.get(parent_id_of(e), "") for _, e in self.entities]


def _mismatch(entry, entity, parent):
    if entry.get("name") and not same_name(entry["name"], name_of(entity)):
        return f'its id now names "{name_of(entity)}", not "{entry["name"]}"'
    mesh = mesh_of(entity)
    if entry.get("mesh") and mesh and mesh != entry["mesh"]:
        return f"its id now names a different mesh ({mesh})"
    if entry.get("parent") and parent and not same_name(entry["parent"], parent):
        return f'its id now sits under "{parent}", not "{entry["parent"]}"'
    return None


def _rematch(entry, index, claimed, radius):
    if not entry.get("name") or entry.get("op") == "add":
        return None
    best = []
    for i, (_, entity) in enumerate(index.entities):
        if not same_name(entry["name"], name_of(entity)) or entity.get("persistent_id") in claimed:
            continue
        if entry.get("mesh") and mesh_of(entity) != entry["mesh"]:
            continue
        if entry.get("parent") and not same_name(entry["parent"], index.parent_names[i]):
            continue
        was, now = entry.get("was"), translation_of(entity)
        if was is None:
            best.append((0.0, i))
        elif now is not None and _distance(was, now) <= radius:
            best.append((_distance(was, now), i))
    best.sort()
    if len(best) == 1:
        return best[0][1]
    if len(best) > 1 and best[1][0] - best[0][0] > 0.01:
        return best[0][1]
    return None


def _resolve(entry, index, claimed):
    """(index of the entity the entry means, None) or (None, why). Mirrors scene_edits.rs `resolve`."""
    present = index.by_id.get(entry["id"])
    if present is None:
        found = _rematch(entry, index, claimed, REMATCH_RADIUS)
        return (found, None) if found is not None else (None, "it is no longer in the scene")
    why = _mismatch(entry, index.entities[present][1], index.parent_names[present])
    if why:
        found = _rematch(entry, index, claimed, REMATCH_RADIUS)
        return (found, None) if found is not None else (None, why)
    was, now = entry.get("was"), translation_of(index.entities[present][1])
    if was is None or now is None:
        return present, None
    wandered = _distance(was, now)
    if wandered <= 0.05:
        return present, None
    nearer = _rematch(entry, index, claimed, wandered - 0.01)
    if nearer is not None and nearer != present:
        return nearer, None
    if wandered <= SAME_OBJECT_RADIUS:
        return present, None
    return None, f"the generator put it {wandered:.1f} m from where it was"


def match(layer, documents, cells=False):
    """Match `layer`'s entries to `documents` ({file: document}), re-keying entries in place.

    `cells`: the documents are a streamed world's cells, so an entry's `from` follows its entity to the cell that holds
    it; otherwise only the scene's own entries (empty `from`) are matched. Returns (kept, rematched, lost), with
    `rematched` as (old id, new id) and `lost` as (id, name, why)."""
    index = _Index(documents)
    kept, rematched, lost, claimed = 0, [], [], set()
    for entry in layer.get("entities", []):
        if entry.get("op") == "add":
            continue
        if bool(entry.get("from")) != cells:
            continue
        found, why = _resolve(entry, index, claimed)
        if found is None:
            lost.append((entry["id"], entry.get("name", ""), why))
            continue
        file, entity = index.entities[found]
        new_id = entity["persistent_id"]
        if new_id != entry["id"]:
            rematched.append((entry["id"], new_id))
            entry["id"] = new_id
        else:
            kept += 1
        if cells:
            entry["from"] = file
        claimed.add(new_id)
    return kept, rematched, lost


def _summary(scene, kept, rematched, lost):
    total = kept + len(rematched) + len(lost)
    parts = [f"{kept} still match"]
    if rematched:
        parts.append(f"{len(rematched)} found again after their ids changed")
    if lost:
        parts.append(f"{len(lost)} match nothing now (kept in the file: "
                     + "; ".join(f'"{name}": {why}' for _, name, why in lost[:3])
                     + (" ..." if len(lost) > 3 else "") + ")")
    return f"editor edits for {Path(scene).name} ({total}): " + ", ".join(parts)


def after_write(scene, document):
    """A generator wrote `scene`: re-key its edit layer to the new ids and say how the edits fared. Never raises."""
    try:
        layer = read_layer(scene)
        if not layer:
            return
        kept, rematched, lost = match(layer, {"": document})
        if rematched:
            write_layer(scene, layer)
        print(_summary(scene, kept, rematched, lost))
    except Exception as error:  # a report must never stop a build
        print(f"scene_edits: could not check {scene}'s editor edits: {error}")


def after_compose(world_scene, cells, project_root):
    """A streamed world was composed: `cells` maps each cell file (absolute) to its document. Never raises."""
    try:
        layer = read_layer(world_scene)
        if not layer:
            return
        root = Path(project_root)
        documents = {Path(os.path.normpath(path)).relative_to(root).as_posix(): doc for path, doc in cells.items()}
        kept, rematched, lost = match(layer, documents, cells=True)
        write_layer(world_scene, layer)  # `from` follows entities between cells even without a re-key
        print(_summary(world_scene, kept, rematched, lost))
    except Exception as error:
        print(f"scene_edits: could not check {world_scene}'s editor edits: {error}")


def report_places(path, document, world_scene):
    """A places scene feeding a composed world was written: say how many of the world's edits it holds. Never raises."""
    try:
        layer = read_layer(world_scene)
        if not layer:
            return
        ids = {e.get("persistent_id") for e in document.get("entities", [])}
        here = sum(1 for entry in layer.get("entities", []) if entry["id"] in ids)
        if here:
            print(f"editor edits: {here} of the world's are on objects in {Path(path).name}; "
                  f"the composer re-checks them all")
    except Exception as error:
        print(f"scene_edits: could not check the world's editor edits: {error}")


def self_check():
    def entity(i, name, at, parent=None):
        components = {"somnium.Name": {"fields": {"value": name}, "version": 1},
                      "somnium.Transform": {"fields": {"translation": at}, "version": 1}}
        if parent:
            components["somnium.Parent"] = {"fields": {"entity": {"$entity": parent}}, "version": 1}
        return {"persistent_id": i, "components": components}

    def change(i, name, was, frm=""):
        return {"id": i, "op": "change", "name": name, "was": was, "from": frm,
                "patches": [{"path": ["components", "somnium.Transform", "fields", "translation"], "value": [0, 0, 0]}]}

    # Kept by id; a renamed id found again; an id handed on to the next weed of the same name follows the weed.
    layer = {"format": FORMAT, "entities": [change("a", "Bench", [1, 0.4, 2]), change("x", "Lamp", [5, 0, 5]),
                                            change("w1", "weeds", [1, 0, 0])]}
    doc = {"entities": [entity("a", "Bench", [1, 0.4, 2]), entity("y", "Lamp", [5.05, 0, 5]),
                        entity("w0", "weeds", [1, 0, 0]), entity("w1", "weeds", [2.5, 0, 0])]}
    kept, rematched, lost = match(layer, {"": doc})
    assert kept == 1 and sorted(rematched) == [("w1", "w0"), ("x", "y")] and not lost, (kept, rematched, lost)
    # An id reused for something else across the street is not taken.
    layer = {"format": FORMAT, "entities": [change("a", "Bench", [1, 0.4, 2])]}
    kept, rematched, lost = match(layer, {"": {"entities": [entity("a", "Crate", [30, 0, 2])]}})
    assert kept == 0 and not rematched and len(lost) == 1, lost
    # Two identical candidates: left alone.
    layer = {"format": FORMAT, "entities": [change("a", "Bench", [1, 0.4, 2])]}
    doc = {"entities": [entity("c", "Bench", [1.5, 0.4, 2]), entity("d", "Bench", [0.5, 0.4, 2])]}
    assert match(layer, {"": doc})[2], "ambiguity is a miss"
    # A cell entry follows its object into another cell; scene entries are not touched by a cell match.
    layer = {"format": FORMAT, "entities": [change("r", "rock", [3, 0, 3], "scenes/cells/s_0_0.scene.json"),
                                            change("s", "sun", [0, 9, 0])]}
    kept, rematched, lost = match(layer, {"scenes/cells/s_1_0.scene.json": {"entities": [entity("r", "rock", [3, 0, 3])]}},
                                  cells=True)
    assert kept == 1 and layer["entities"][0]["from"] == "scenes/cells/s_1_0.scene.json" and not lost
    assert layer["entities"][1].get("from") == "", "the scene's own entry is not a cell's"
    # A name the engine cut at 63 bytes still matches the whole one.
    whole = "Lights 20261004 / panel for Visual 20260930 / signed drawing task spot"
    layer = {"format": FORMAT, "entities": [change("p", whole[:63], [-7.12, 2.0, -14.9])]}
    assert match(layer, {"": {"entities": [entity("p", whole, [-7.12, 2.0, -14.9])]}})[0] == 1
    assert not same_name("Bench", "Bench 2")
    print("self-check ok")


if __name__ == "__main__":
    if "--self-check" in sys.argv:
        self_check()
    elif len(sys.argv) >= 2:
        scene = Path(sys.argv[1])
        layer = read_layer(scene)
        if not layer:
            print(f"{scene}: no editor edits ({layer_path(scene).name} does not exist)")
            sys.exit(0)
        kept, rematched, lost = match(layer, {"": read_scene(scene)})
        print(_summary(scene, kept, rematched, lost))
        for _, name, why in lost:
            print(f'  "{name}": {why}')
        if "--fix" in sys.argv and rematched:
            write_layer(scene, layer)
            print(f"re-keyed {len(rematched)} entries in {layer_path(scene).name}")
    else:
        print(__doc__)
