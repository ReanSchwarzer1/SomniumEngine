"""Verify textured particles through Hello's native preset, Details schema and transport.
Run against an idle Hello editor. Creates and removes one temporary scene entity.
Captures and a receipt go to the requested output directory; no scene file is saved.
"""
import argparse
import json
import time
import uuid
from pathlib import Path
from pipe_client import EditorClient


def run(connection, output):
    client = EditorClient(connection, timeout=30)
    output.parent.mkdir(parents=True, exist_ok=True)
    report = {"complete": False, "cases": {}}
    actor = None

    def call(method, params):
        result = client.call("authoring." + method, params)
        assert result.get("ok") is True, (method, result)
        return result

    def query(**args):
        return call("query", args)

    def execute(action, **args):
        return call("execute", dict(action=action, request_id=uuid.uuid4().hex, **args))

    def wait(predicate):
        until = time.monotonic() + 30
        while time.monotonic() < until:
            result = predicate()
            if result:
                return result
            time.sleep(0.05)
        raise AssertionError("Editor acceptance timed out")

    def transact(label, operations):
        plan = call("plan", dict(expected_revision=query(kind="scene", limit=0)["revision"],
                                 label=label, operations=operations))
        return call("commit", dict(request_id=uuid.uuid4().hex, plan_token=plan["plan_token"]))

    def fields():
        entity = query(kind="scene", entity=actor)["entities"][0]
        return next(c["fields"] for c in entity["components"] if c["id"] == "somnium.ParticleEmitter")

    def transport(command, state):
        execute("command", id="editor.simulation." + command)
        wait(lambda: query(kind="diagnostics")["simulation"] == state)

    def capture(label):
        job = execute("capture", include_editor=True)["job"]["job_id"]
        def finished():
            item = call("jobs", dict(action="get", id=job))["job"]
            assert item["status"] not in ("failed", "cancelled"), item
            return item if item["status"] == "succeeded" else None
        report["cases"][label] = wait(finished)

    discovery = call("discover", {"section": "full"})
    assert discovery["game"]["label"] == "Hello Engine"
    assert query(kind="diagnostics")["simulation"] == "Editing", "Use idle Hello Engine"
    before = {e["id"] for e in query(kind="scene", search="Textured Smoke Preview")["entities"]}
    try:
        execute("preset", id="hello.textured_particles", args={})
        candidates = [e for e in query(kind="scene", search="Textured Smoke Preview")["entities"]
                      if e["id"] not in before]
        assert len(candidates) == 1
        actor = candidates[0]["id"]
        wait(lambda: fields()["texture_status"] == "Ready")
        required = {"enabled", "texture", "texture_status", "aspect", "additive", "world_up",
                    "local_space", "tip_tint", "atlas_columns", "atlas_rows", "atlas_frames",
                    "atlas_fps", "rotation_spread", "spin", "burst", "color_over_life"}
        assert required <= fields().keys()
        execute("select", entity=actor, frame=False)
        capture("native_details_and_smoke")
        report["cases"]["preset_and_texture"] = {"pass": True, "fields": sorted(required)}
        transport("play", "Playing")
        transport("pause", "Paused")
        edits = {"enabled": False, "burst": 5, "spin": 0.0, "rotation_spread": 1.0}
        transact("One-shot smoke preview", [dict(op="set", entity=actor,
                 component="somnium.ParticleEmitter", field=key, value=value) for key, value in edits.items()])
        assert int(fields()["burst"]) == 5
        execute("step", count=1)
        wait(lambda: query(kind="diagnostics")["pending_steps"] == 0)
        assert int(fields()["burst"]) == 0
        capture("paused_step_burst")
        execute("step", count=60)
        wait(lambda: query(kind="diagnostics")["pending_steps"] == 0)
        capture("smoke_after_sixty_steps")
        transport("stop", "Editing")
        assert fields()["enabled"] is True and abs(fields()["spin"] - 0.2) < 1e-5
        report["cases"]["play_pause_step_stop"] = {"pass": True, "burst_consumed": True,
                                                    "authored_values_restored": True}
        report["complete"] = True
    finally:
        state = query(kind="diagnostics")["simulation"]
        if state != "Editing":
            transport("stop", "Editing")
        if actor is not None:
            transact("Remove acceptance smoke", [dict(op="delete", entity=actor)])
        output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(output)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--connection", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    run(args.connection, args.output)
