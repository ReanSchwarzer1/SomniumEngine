"""Exercise native renderer controls, content, localisation and OS panels in Hello.

Transient source fixtures are confined to one unique content folder. Localisation
files are backed up byte-for-byte before native save/export and restored in finally.
Run against an idle Hello Engine; this intentionally changes visible editor state.
"""
import argparse
import json
import time
import uuid
from pathlib import Path

from pipe_client import EditorClient


def run(connection, output):
    client = EditorClient(connection, timeout=30)
    root = Path(__file__).resolve().parents[2]
    report = json.loads(output.read_text()) if output.exists() else {"cases": {}}

    def call(method, params):
        result = client.call(method, params)
        assert result.get("ok") is True, (method, params, result)
        return result

    def query(**kw):
        return call("authoring.query", kw)

    def execute(action, **kw):
        return call("authoring.execute", dict(action=action, request_id=str(uuid.uuid4()), **kw))

    def revision():
        return query(kind="scene", limit=0)["revision"]

    def undo():
        history = call("authoring.history", {})
        return call("authoring.history", dict(action="undo", expected_revision=history["revision"], expected_cursor=history["cursor"]))

    def record(name, **details):
        report["cases"][name] = dict(pass_=True, **details)
        report["cases"][name]["pass"] = report["cases"][name].pop("pass_")
        output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(name, flush=True)

    discovery = call("authoring.discover", {"section": "full"})
    assert discovery["game"]["label"] == "Hello Engine"
    output.with_name("capability-inventory.json").write_text(json.dumps(discovery, indent=2) + "\n", encoding="utf-8")
    report["complete"] = False
    terrain = next(e for e in query(kind="scene", search="Terrain")["entities"] if any(c["id"] == "somnium.Terrain" for c in e["components"]))["id"]

    def control(kind, field, value):
        current = query(kind=kind, entity=terrain)
        return execute(kind, entity=terrain, expected_revision=revision(), field=field, expected_value=current["values"][field], value=value)

    original = query(kind="terrain_settings", entity=terrain)
    changed = control("terrain_settings", "wetness", .37)
    assert abs(changed["values"]["wetness"] - .37) < .00001
    refused = client.call("authoring.execute", dict(action="terrain_settings", request_id=str(uuid.uuid4()), entity=terrain, expected_revision=revision(), field="wetness", expected_value=original["values"]["wetness"], value=.7))
    assert not refused["ok"]
    control("terrain_settings", "wetness", original["values"]["wetness"])
    record("renderer_controls", native_ranges=len(original["fields"]), value_change=True, stale_value_rejected=True, restored=True)

    original = query(kind="foliage_settings")
    control("foliage_settings", "max_slope", 37.0)
    tq = query(kind="terrain", entity=terrain)
    painted = execute("foliage_stroke", entity=terrain, expected_revision=revision(), terrain_revision=tq["terrain_revision"], samples=[[0, 0]], kind=original["values"]["kind"], radius=2.0, single=True, min_layer_weight=0.0, seed=71)
    assert query(kind="foliage_settings")["values"]["max_slope"] == 37.0
    if painted["total"] != tq["foliage_count"]:
        undo()
    control("foliage_settings", "max_slope", original["values"]["max_slope"])
    record("foliage_controls", native_ranges=len(original["fields"]), custom_slope_survives_stroke=True, restored=True)

    folder = root / "assets" / ("authoring-acceptance-" + uuid.uuid4().hex[:10])
    relative = folder.relative_to(root).as_posix()
    def content(op, **kw):
        return execute("content", operation=op, **kw)
    try:
        content("new_folder", parent="assets", name=folder.name)
        content("new_script", parent=relative, name="ProbeScript")
        content("rename", path=relative + "/ProbeScript.luau", name="RenamedScript")
        content("new_material", parent=relative, name="ProbeMaterial")
        assert (folder / "RenamedScript.luau").is_file()
        plan = call("authoring.plan", dict(label="Content acceptance actor", expected_revision=revision(), operations=[dict(op="create", key="fixture", name="Content acceptance actor", components={"somnium.Transform": {"translation": [0, 160, 0], "scale": [2, 2, 2]}, "somnium.MeshKind": {"kind": "Cube"}})]))
        actor = call("authoring.commit", dict(request_id=str(uuid.uuid4()), plan_token=plan["plan_token"]))["created"]["fixture"]
        time.sleep(.2)
        content("assign_material", entity=actor, expected_revision=revision(), path=relative + "/ProbeMaterial.sommat")
        unique = content("make_unique", entity=actor, expected_revision=revision(), path=relative + "/ProbeMaterial.sommat", component="somnium.Material", field="asset")
        assert Path(unique["path"]).is_file()
        picked = query(kind="pick", origin=[0, 160, 5], direction=[0, 0, -1], max_distance=10)
        assert any(hit["entity"] == actor for hit in picked["hits"]), picked
        hit = next(hit for hit in picked["hits"] if hit["entity"] == actor)
        assert abs(hit["distance"] - 4.0) < .001, hit
        record("positive_pick", native_bounds_hit=True, distance_metres=hit["distance"])
        undo(); undo(); undo()
        record("content_operations", create_folder=True, create_script=True, rename=True, create_material=True, assign_material=True, make_unique=True, assignment_undo=True)
    finally:
        # Never recursively delete: remove only flat files generated in our own folder.
        assert folder.resolve().parent == (root / "assets").resolve()
        if folder.exists():
            for path in folder.iterdir():
                assert path.is_file() and path.resolve().parent == folder.resolve()
                path.unlink()
            folder.rmdir()

    locale = root / "assets" / "locale"
    backups = {p: p.read_bytes() for p in locale.glob("*.json")}
    csv = locale / "localisation.csv"
    backups[csv] = csv.read_bytes() if csv.exists() else None
    def grid(operation, **kw):
        state = query(kind="workspace", target="localisation")
        return execute("workspace", target="localisation", operation=operation, expected_view=state["view_token"], **kw)
    try:
        baseline = query(kind="workspace", target="localisation")
        row = baseline["rows"][0]["id"]
        changed = grid("cell", row=row, column="en", text="Native localisation acceptance")
        assert changed["rows"][0]["cells"]["en"] == "Native localisation acceptance"
        grid("save")
        assert b"Native localisation acceptance" in (locale / "en.json").read_bytes()
        grid("export")
        assert b"Native localisation acceptance" in csv.read_bytes()
        restored = grid("undo")
        assert restored["csv"] == baseline["csv"]
        grid("range", rows=[row], columns=["en", "fr"], text="Range acceptance")
        assert grid("undo")["csv"] == baseline["csv"]
        grid("save")
        record("localisation", native_cells=True, range=True, native_undo=True, saved_catalogue=True, exported_csv=True, source_restored=True)
    finally:
        for path, data in backups.items():
            if data is None:
                path.unlink(missing_ok=True)
            else:
                path.write_bytes(data)

    baseline = query(kind="workspace")["panels"]
    details = next(p for p in baseline if p["panel"] == "Details")
    assert not details["floating"], "Start with Details docked for this acceptance"
    execute("workspace", target="panels", operation="float", panel="details")
    for _ in range(100):
        current = next(p for p in query(kind="workspace")["panels"] if p["panel"] == "Details")
        if current["size"]:
            break
        time.sleep(.05)
    assert current["floating"] and current["size"]
    execute("workspace", target="panels", operation="place", panel="details", position=[100, 100], size=[480, 600])
    time.sleep(.3)
    actual = next(p for p in query(kind="workspace")["panels"] if p["panel"] == "Details")
    assert actual["size"] == [480, 600], actual
    execute("workspace", target="panels", operation="dock", panel="details")
    assert not next(p for p in query(kind="workspace")["panels"] if p["panel"] == "Details")["floating"]
    record("native_panels", floating_window=True, requested_size=actual["size"], dock_restored=True)
    report["complete"] = True
    output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--connection", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    run(args.connection, args.output)
