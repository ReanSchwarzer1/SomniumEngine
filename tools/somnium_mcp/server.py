#!/usr/bin/env python3
"""MCP stdio adapter for the running Somnium editor (Python 3.10+).

Uses the stable 2025-11-25 MCP lifecycle. Engine behaviour is always supplied by the authenticated running editor,
never emulated here.

Phase ALYX-M (18 Oct 2026), for agents that use it every day:
- tool names an MCP client accepts everywhere (`authoring_query`; the dotted `authoring.query` still works);
- focused tools beside the seven authoring families: status, the Output Log, a screenshot that waits for its
  capture, editor commands, and a scene page;
- resources an agent can read without a tool call (status, recent problems, capabilities, commands);
- prompts for the common jobs (look around, fix what the log reports, block out a space);
- the editor decides what an agent may change (Preferences > Agents, or the Authoring panel); a refused call says so.
"""
from __future__ import annotations
import argparse
import base64
import json
import time
import uuid
from pathlib import Path
import sys

from pipe_client import BridgeError, EditorClient, MAX_REQUEST

PROTOCOL = "2025-11-25"
VERSION = "0.2.0"

# Listed tool -> (engine method or None for a composed tool, description, input schema, read only).
_OBJECT = {"type": "object", "additionalProperties": True}
TOOL_SPECS = {
    "authoring_discover": ("authoring.discover",
        "Discover the live editor: project and session identity, typed command schemas, component ids, execute "
        "actions and their limits. Pass section (overview, components, commands, execute) for one part.",
        {"type": "object", "properties": {"section": {"type": "string", "enum": ["overview", "components", "commands", "execute"]}},
         "additionalProperties": True}, True),
    "authoring_query": ("authoring.query",
        "Read state. kind: scene (entities, bounded pages), settings, commands, diagnostics, log, game, documents, "
        "document, editor, workspace, terrain, story. Use persistent ids and the returned revision.",
        {"type": "object", "properties": {"kind": {"type": "string", "enum": ["scene", "settings", "commands",
            "diagnostics", "log", "game", "documents", "document", "editor", "workspace", "terrain",
            "terrain_settings", "foliage_settings", "story"]}}, "required": ["kind"], "additionalProperties": True}, True),
    "authoring_plan": ("authoring.plan",
        "Dry-run a bounded edit batch against the current revision: validates fields and targets without changing "
        "the world, files or history. Returns a plan token for authoring_commit.", _OBJECT, True),
    "authoring_commit": ("authoring.commit",
        "Commit a validated plan with its revision and a request id (idempotency). Returns the actual changes and an "
        "undo receipt; a stale plan fails and must be planned again.", _OBJECT, False),
    "authoring_execute": ("authoring.execute",
        "Run a declared editor operation: action is one of camera, bookmark, select, command, capture, import, "
        "preset, step, input, tap, material_open, material_save, graph_*, terrain_stroke, foliage_stroke, "
        "vertex_paint, content, workspace, save_scene, load_scene. Discover argument schemas first. Long work "
        "returns a job.",
        {"type": "object", "properties": {"action": {"type": "string"}}, "required": ["action"], "additionalProperties": True}, False),
    "authoring_history": ("authoring.history",
        "List, undo or redo through the editor's own history. action: list (read), undo, redo; supply the expected "
        "revision.", _OBJECT, False),
    "authoring_jobs": ("authoring.jobs",
        "List, get or cancel engine jobs (captures, imports, bakes). Queued is not finished; poll to a terminal "
        "status. A finished capture's result carries the image.",
        {"type": "object", "properties": {"action": {"type": "string", "enum": ["list", "get", "cancel"]}, "id": {"type": "string"}},
         "additionalProperties": True}, True),
    "somnium_status": (None,
        "One look at the editor now: whether it is editing or playing, the scene revision, entity count, frame rate, "
        "camera, and how many warnings and errors the Output Log holds. Start a session here.",
        {"type": "object", "additionalProperties": False}, True),
    "somnium_log": (None,
        "Read the editor's Output Log: what failed, what warned. min: debug, info, warn (default), error. after: a "
        "line id from last time, to read only new lines. contains: filter text. limit: up to 200 (default 50).",
        {"type": "object", "properties": {"min": {"type": "string", "enum": ["debug", "info", "warn", "error"]},
            "after": {"type": "integer", "minimum": 0}, "contains": {"type": "string"},
            "limit": {"type": "integer", "minimum": 1, "maximum": 200}}}, True),
    "somnium_screenshot": (None,
        "Capture the viewport and return the picture. live: true during Play (allows the world to move while it "
        "is taken). include_editor: true to keep the game HUD and editor UI in it. Waits for the capture.",
        {"type": "object", "properties": {"live": {"type": "boolean"}, "include_editor": {"type": "boolean"},
            "timeout_seconds": {"type": "number", "minimum": 1, "maximum": 60}}}, True),
    "somnium_commands": (None,
        "List the editor's commands (the same ones its menus, shortcuts and palette run), with whether each can run "
        "now and why not. contains filters by id, label or category.",
        {"type": "object", "properties": {"contains": {"type": "string"}}}, True),
    "somnium_story": (None,
        "The project's story (dialogue, quests, barks, journal notes) without changing anything. operation: overview "
        "(cast, facts, every dialogue and quest), schema (every fact and command conditions and effects may use), "
        "lint (every problem, by file), check (validate document_type dialogue|quest with content = the asset JSON "
        "before writing it with a document plan), simulate (play dialogue from entry with scripted choices, 0-based, "
        "after setup effects; returns the transcript and what changed), strings (the source string table), search "
        "(text). Write story files with authoring_plan/commit kind document (.somdialogue, .somquest, .sombarks, "
        ".somnotes, .somstory).",
        {"type": "object", "properties": {
            "operation": {"type": "string", "enum": ["overview", "schema", "lint", "check", "simulate", "strings", "search"]},
            "document_type": {"type": "string", "enum": ["dialogue", "quest"]},
            "content": {"type": "object"}, "path": {"type": "string"}, "file": {"type": "string"},
            "dialogue": {"type": "string"}, "entry": {"type": "string"},
            "choices": {"type": "array", "items": {"type": "integer", "minimum": 0}},
            "setup": {"type": "string"}, "seed": {"type": "integer", "minimum": 0},
            "max_beats": {"type": "integer", "minimum": 1, "maximum": 2000}, "text": {"type": "string"}},
         "additionalProperties": False}, True),
    "somnium_run_command": (None,
        "Run one editor command by id (from somnium_commands), exactly as its menu item would: e.g. "
        "editor.simulation.play, editor.view.focus_selection, editor.create.point_light.",
        {"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"]}, False),
}
TOOLS = {name: spec[1] for name, spec in TOOL_SPECS.items()}
# The dotted names agents used before 0.2: still accepted, no longer listed (some clients reject dots in names).
ALIASES = {spec[0]: name for name, spec in TOOL_SPECS.items() if spec[0]}

RESOURCES = {
    "somnium://status": ("Editor status", "What the editor is doing now (somnium_status).", "application/json"),
    "somnium://log/problems": ("Recent problems", "The last 50 warnings and errors in the Output Log.", "application/json"),
    "somnium://capabilities": ("Capabilities", "The live discovery overview: identity, schemas, limits.", "application/json"),
    "somnium://commands": ("Editor commands", "Every editor command and whether it can run now.", "application/json"),
}

PROMPTS = {
    "look_around": ("Look at the open scene and report what is there.", [],
        "Use somnium_status, then authoring_query kind scene (a page at a time) and somnium_screenshot to see the "
        "open scene. Report what it contains, anything that looks wrong, and any warnings in somnium_log. Change "
        "nothing."),
    "fix_log_problems": ("Find and fix what the Output Log reports.", [{"name": "focus", "description": "What to look at first (optional)", "required": False}],
        "Read somnium_log (min warn). Group the problems by cause. For each one you can fix through the editor, "
        "authoring_plan the change, show the plan, then authoring_commit it; read somnium_log after (pass after= "
        "the last id) to confirm it is gone. Leave anything that needs code or art, and say why."),
    "write_dialogue": ("Write a dialogue scene as a story asset.", [{"name": "brief", "description": "Who speaks, where, what happens", "required": True}],
        "Read somnium_story operation overview and schema first: the cast, their portrait expressions, the facts "
        "and commands conditions and effects may use. Write the scene as a .somdialogue asset (lines with speaker, "
        "expression and text; choices as options in a line's next list; checks, branches and effects as the schema "
        "allows). Check it with somnium_story operation check, fix every problem, then save it with authoring_plan "
        "and authoring_commit kind document under the story folder. Play it with somnium_story operation simulate "
        "down each branch and report the transcripts.\n\nThe scene: {brief}"),
    "block_out_space": ("Block out a space with primitives and lights.", [{"name": "brief", "description": "What the space is", "required": True}],
        "Block out the space described below with editor commands and plans: floors and walls from cubes, a few "
        "lights, the camera framed on it. Check each step with somnium_screenshot. Keep everything in one undoable "
        "commit where you can.\n\nThe space: {brief}"),
}

INSTRUCTIONS = (
    "Somnium editor over MCP. Start with somnium_status. Read with authoring_query, somnium_log and "
    "somnium_screenshot. Change the scene with authoring_plan then authoring_commit (a plan is checked against the "
    "current revision; plan again if it went stale), or authoring_execute and somnium_run_command for editor "
    "operations. Long work returns a job: poll authoring_jobs. Re-read revisions after a person edits. Never blindly "
    "retry a change whose outcome is unknown: query first. If a call is refused as permission_denied, the person has "
    "turned that off for agents; say so rather than working around it."
)


# Calls that can overwrite or remove something a person made (a save, a load, a delete command, an undo).
DESTRUCTIVE = {"authoring_execute", "authoring_history", "somnium_run_command"}


def title(name: str) -> str:
    """`somnium_run_command` -> "Somnium: run command", for clients that show a title."""
    family, _, rest = name.partition("_")
    return f"{family.capitalize()}: {rest.replace('_', ' ')}"


def tool_definitions():
    return [{"name": name, "title": title(name), "description": description, "inputSchema": schema,
             "outputSchema": {"type": "object"},
             "annotations": {"readOnlyHint": read_only, "destructiveHint": name in DESTRUCTIVE,
                             "idempotentHint": read_only, "openWorldHint": False}}
            for name, (_, description, schema, read_only) in TOOL_SPECS.items()]


def capture_image(result, project: Path):
    """Read only an explicit successful engine capture, beneath this project."""
    candidates = [result]
    if isinstance(result, dict):
        for key in ("result", "receipt", "job"):
            if isinstance(result.get(key), dict):
                candidates.append(result[key])
                if isinstance(result[key].get("result"), dict):
                    candidates.append(result[key]["result"])
    for candidate in candidates:
        if not isinstance(candidate, dict) or not isinstance(candidate.get("capture"), dict):
            continue
        capture = candidate["capture"]
        if not isinstance(capture.get("path"), str):
            continue
        path = Path(capture["path"]).resolve()
        try:
            path.relative_to(project)
            if path.suffix.lower() != ".png" or path.stat().st_size > 4 * 1024 * 1024:
                continue
            data = path.read_bytes()
            if not data.startswith(b"\x89PNG\r\n\x1a\n"):
                continue
            return {"type": "image", "mimeType": "image/png", "data": base64.b64encode(data).decode("ascii")}
        except (OSError, ValueError):
            continue
    return None


def is_failure(result) -> bool:
    return isinstance(result, dict) and (result.get("ok") is False or result.get("isError") is True
                                         or ("error" in result and result["error"] is not None))


class Server:
    def __init__(self, client: EditorClient):
        self.client = client
        self.initialized = False
        self.initialize_requested = False

    # ── composed tools ─────────────────────────────────────────────────────────
    def status(self):
        diagnostics = self.client.call("authoring.query", {"kind": "diagnostics"})
        if is_failure(diagnostics):
            return diagnostics
        problems = self.client.call("authoring.query", {"kind": "log", "min": "warn", "limit": 200})
        lines = problems.get("lines", []) if isinstance(problems, dict) else []
        return {"ok": True, "simulation": diagnostics.get("simulation"), "revision": diagnostics.get("revision"),
                "entities": diagnostics.get("entities"), "fps": diagnostics.get("fps"),
                "camera": diagnostics.get("camera"), "frame": diagnostics.get("frame"),
                "warnings": sum(1 for line in lines if line.get("severity") == "warn"),
                "errors": sum(1 for line in lines if line.get("severity") == "error"),
                "last_log_id": problems.get("last_id") if isinstance(problems, dict) else None}

    def screenshot(self, arguments):
        request = {"action": "capture", "request_id": "mcp-" + uuid.uuid4().hex,
                   "live": bool(arguments.get("live", False)), "include_editor": bool(arguments.get("include_editor", False))}
        submitted = self.client.call("authoring.execute", request)
        if is_failure(submitted):
            return submitted, None
        job = submitted.get("job", {}) if isinstance(submitted, dict) else {}
        job_id = job.get("job_id")
        deadline = time.monotonic() + float(min(max(arguments.get("timeout_seconds", 20), 1), 60))
        while job_id and time.monotonic() < deadline:
            reply = self.client.call("authoring.jobs", {"action": "get", "id": job_id})
            job = reply.get("job", {}) if isinstance(reply, dict) else {}
            if job.get("status") in ("succeeded", "failed", "cancelled"):
                result = {"ok": job.get("status") == "succeeded", "job": job}
                if job.get("status") != "succeeded":
                    result["error"] = {"code": "capture_" + job.get("status", "failed"), "message": job.get("error") or "capture did not finish"}
                return result, capture_image(result, self.client.path.parent.parent)
            time.sleep(0.2)
        return {"ok": False, "job": job, "error": {"code": "capture_timeout",
                "message": "The capture is still running; poll authoring_jobs with this job id."}}, None

    def commands(self, arguments):
        reply = self.client.call("authoring.query", {"kind": "commands"})
        if is_failure(reply):
            return reply
        needle = str(arguments.get("contains", "")).lower()
        rows = [c for c in reply.get("commands", [])
                if not needle or needle in (c.get("id", "") + " " + c.get("label", "") + " " + c.get("category", "")).lower()]
        return {"ok": True, "commands": rows}

    def call_tool(self, name, arguments):
        """(result, extra content) for one listed tool."""
        method = TOOL_SPECS[name][0]
        if method:
            result = self.client.call(method, arguments)
            if not isinstance(result, dict):
                result = {"value": result}
            image = None
            if not is_failure(result) and method in ("authoring.execute", "authoring.jobs"):
                image = capture_image(result, self.client.path.parent.parent)
            return result, image
        if name == "somnium_status":
            return self.status(), None
        if name == "somnium_log":
            query = {"kind": "log"}
            query.update({k: arguments[k] for k in ("min", "after", "contains", "limit") if k in arguments})
            return self.client.call("authoring.query", query), None
        if name == "somnium_screenshot":
            return self.screenshot(arguments)
        if name == "somnium_commands":
            return self.commands(arguments), None
        if name == "somnium_story":
            query = {"kind": "story"}
            query.update(arguments)
            return self.client.call("authoring.query", query), None
        if name == "somnium_run_command":
            command = arguments.get("id")
            if not isinstance(command, str) or not command:
                return {"ok": False, "error": {"code": "invalid_arguments", "message": "id is required"}}, None
            return self.client.call("authoring.execute", {"action": "command", "id": command}), None
        raise KeyError(name)

    # ── resources ──────────────────────────────────────────────────────────────
    def read_resource(self, uri):
        if uri == "somnium://status":
            return self.status()
        if uri == "somnium://log/problems":
            return self.client.call("authoring.query", {"kind": "log", "min": "warn", "limit": 50})
        if uri == "somnium://capabilities":
            return self.client.call("authoring.discover", {"section": "overview"})
        if uri == "somnium://commands":
            return self.commands({})
        raise KeyError(uri)

    # ── protocol ───────────────────────────────────────────────────────────────
    def handle(self, request):
        if not isinstance(request, dict):
            return self.error(None, -32600, "JSON-RPC request must be an object")
        request_id = request.get("id")
        method = request.get("method")
        if request.get("jsonrpc") != "2.0" or not isinstance(method, str):
            return self.error(request_id, -32600, "Invalid JSON-RPC request")
        if "id" not in request:
            if method == "notifications/initialized" and self.initialize_requested:
                self.initialized = True
            return None
        if isinstance(request_id, (dict, list, bool)):
            return self.error(None, -32600, "Invalid JSON-RPC id")
        params = request.get("params", {})
        if not isinstance(params, dict):
            return self.error(request_id, -32602, "Parameters must be an object")
        if method == "initialize":
            self.initialize_requested = True
            return self.result(request_id, {"protocolVersion": PROTOCOL,
                "capabilities": {"tools": {"listChanged": False}, "resources": {"listChanged": False},
                                 "prompts": {"listChanged": False}},
                "serverInfo": {"name": "somnium-authoring", "version": VERSION},
                "instructions": INSTRUCTIONS})
        if method == "ping":
            return self.result(request_id, {})
        if not self.initialized:
            return self.error(request_id, -32002, "Complete initialize and notifications/initialized first")
        if params.get("cursor") and method in ("tools/list", "resources/list", "prompts/list"):
            return self.error(request_id, -32602, "This list has no further pages")
        if method == "tools/list":
            return self.result(request_id, {"tools": tool_definitions()})
        if method == "resources/list":
            return self.result(request_id, {"resources": [{"uri": uri, "name": name, "description": description,
                "mimeType": mime} for uri, (name, description, mime) in RESOURCES.items()]})
        if method == "resources/read":
            uri = params.get("uri")
            if uri not in RESOURCES:
                return self.error(request_id, -32002, "Unknown resource")
            try:
                value = self.read_resource(uri)
            except BridgeError as error:
                value = {"ok": False, "error": error.as_dict()}
            return self.result(request_id, {"contents": [{"uri": uri, "mimeType": "application/json",
                "text": json.dumps(value, ensure_ascii=False, allow_nan=False)}]})
        if method == "prompts/list":
            return self.result(request_id, {"prompts": [{"name": name, "description": description, "arguments": arguments}
                for name, (description, arguments, _) in PROMPTS.items()]})
        if method == "prompts/get":
            name = params.get("name")
            if name not in PROMPTS:
                return self.error(request_id, -32602, "Unknown prompt")
            description, declared, template = PROMPTS[name]
            supplied = params.get("arguments") or {}
            if not isinstance(supplied, dict):
                return self.error(request_id, -32602, "Prompt arguments must be an object")
            for argument in declared:
                if argument.get("required") and not supplied.get(argument["name"]):
                    return self.error(request_id, -32602, f"Prompt argument {argument['name']} is required")
            text = template.format(**{a["name"]: str(supplied.get(a["name"], "")) for a in declared})
            if declared and not any(a.get("required") for a in declared) and supplied:
                text += "\n\nFocus: " + "; ".join(str(v) for v in supplied.values())
            return self.result(request_id, {"description": description,
                "messages": [{"role": "user", "content": {"type": "text", "text": text}}]})
        if method == "tools/call":
            name = params.get("name")
            if isinstance(name, str) and name in ALIASES:
                name = ALIASES[name]
            if not isinstance(name, str) or name not in TOOL_SPECS:
                return self.error(request_id, -32602, "Unknown authoring tool")
            arguments = params.get("arguments", {})
            if not isinstance(arguments, dict):
                return self.error(request_id, -32602, "Tool arguments must be an object")
            try:
                result, image = self.call_tool(name, arguments)
                if not isinstance(result, dict):
                    result = {"value": result}
                content = [{"type": "text", "text": json.dumps(result, ensure_ascii=False, allow_nan=False)}]
                if image:
                    content.append(image)
                return self.result(request_id, {"content": content, "structuredContent": result, "isError": is_failure(result)})
            except BridgeError as error:
                result = {"ok": False, "error": error.as_dict()}
                return self.result(request_id, {"content": [{"type": "text", "text": json.dumps(result)}], "structuredContent": result, "isError": True})
        return self.error(request_id, -32601, "Method not found")

    @staticmethod
    def result(request_id, value):
        return {"jsonrpc": "2.0", "id": request_id, "result": value}

    @staticmethod
    def error(request_id, code, message):
        return {"jsonrpc": "2.0", "id": request_id, "error": {"code": code, "message": message}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--connection", required=True, type=Path, help="Private project runtime/authoring-connection.json")
    parser.add_argument("--project-id", help="Optional required project identity")
    parser.add_argument("--timeout", type=float, default=10.0, help="Per-call deadline in seconds, capped at 120; use 120 for cold asset-heavy scene loads")
    args = parser.parse_args()
    server = Server(EditorClient(args.connection, args.project_id, args.timeout))
    while True:
        line = sys.stdin.buffer.readline(MAX_REQUEST + 2)
        if not line:
            break
        if len(line) > MAX_REQUEST:
            # Bound memory while discarding the rest of this malformed frame.
            while line and not line.endswith(b"\n"):
                line = sys.stdin.buffer.readline(MAX_REQUEST + 2)
            response = Server.error(None, -32600, "Request exceeds 1 MiB limit")
        else:
            try:
                request = json.loads(line, parse_constant=lambda _: (_ for _ in ()).throw(ValueError("non-finite JSON")))
                response = server.handle(request)
            except (ValueError, UnicodeError):
                response = Server.error(None, -32700, "Invalid JSON")
            except Exception as error:
                # Log only exception type: request data may include private content.
                print(f"Somnium adapter error: {type(error).__name__}", file=sys.stderr, flush=True)
                response = Server.error(None, -32603, "Internal adapter error")
        if response is not None:
            sys.stdout.buffer.write(json.dumps(response, ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode("utf-8") + b"\n")
            sys.stdout.buffer.flush()


if __name__ == "__main__":
    main()
