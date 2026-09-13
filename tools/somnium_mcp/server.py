#!/usr/bin/env python3
"""Minimal MCP stdio adapter for the running Somnium editor (Python 3.10+).

Uses the stable 2025-11-25 MCP lifecycle for compatibility. It does not advertise
Tasks, sampling, resources, or features it cannot implement. Engine behavior is
always supplied by the authenticated running editor, never emulated here.
"""
from __future__ import annotations
import argparse
import base64
import json
from pathlib import Path
import sys

from pipe_client import BridgeError, EditorClient, MAX_REQUEST

PROTOCOL = "2025-11-25"
TOOLS = {
    "authoring.discover": "Discover live editor capabilities, typed command schemas, project/session identity and unsupported reasons. Start here before editing.",
    "authoring.query": "Query current scene, entity, schema, asset or document state. Use persistent IDs and returned revisions; request bounded pages.",
    "authoring.plan": "Dry-run a bounded edit batch against the current revision. Validate fields and targets without changing world/files/history.",
    "authoring.commit": "Commit a validated plan using its revision and idempotency request ID. Returns actual changes and undo receipt; stale plans fail.",
    "authoring.execute": "Execute a declared editor operation such as camera, play, capture or import. Discover supported operation arguments first. Long work returns a job receipt.",
    "authoring.history": "Read or change document-scoped undo history through the same editor implementation. Supply the expected history/revision precondition.",
    "authoring.jobs": "Read or cancel actual engine feedback jobs. Queued work is not finished; inspect terminal status and published revision. Capture results may include an image.",
}


def tool_definitions():
    return [{"name": name, "description": description,
             "inputSchema": {"type": "object", "additionalProperties": True},
             "outputSchema": {"type": "object"},
             "annotations": {"readOnlyHint": name in ("authoring.discover", "authoring.query", "authoring.plan"),
                             "openWorldHint": False}}
            for name, description in TOOLS.items()]


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


class Server:
    def __init__(self, client: EditorClient):
        self.client = client
        self.initialized = False
        self.initialize_requested = False

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
                "capabilities": {"tools": {"listChanged": False}},
                "serverInfo": {"name": "somnium-authoring", "version": "0.1.0"},
                "instructions": "Call authoring.discover to read live schemas and capability limits. Only capabilities confirmed by the editor work. Re-query revisions after human edits. An uncertain mutation outcome must be inspected, not blindly retried."})
        if method == "ping":
            return self.result(request_id, {})
        if not self.initialized:
            return self.error(request_id, -32002, "Complete initialize and notifications/initialized first")
        if method == "tools/list":
            if params.get("cursor"):
                return self.error(request_id, -32602, "This tool list has no further pages")
            return self.result(request_id, {"tools": tool_definitions()})
        if method == "tools/call":
            name = params.get("name")
            if not isinstance(name, str) or name not in TOOLS:
                return self.error(request_id, -32602, "Unknown authoring tool")
            arguments = params.get("arguments", {})
            if not isinstance(arguments, dict):
                return self.error(request_id, -32602, "Tool arguments must be an object")
            try:
                result = self.client.call(name, arguments)
                if not isinstance(result, dict):
                    result = {"value": result}
                is_error = result.get("ok") is False or result.get("isError") is True or ("error" in result and result["error"] is not None)
                content = [{"type": "text", "text": json.dumps(result, ensure_ascii=False, allow_nan=False)}]
                if not is_error and name in ("authoring.execute", "authoring.jobs"):
                    image = capture_image(result, self.client.path.parent.parent)
                    if image:
                        content.append(image)
                return self.result(request_id, {"content": content, "structuredContent": result, "isError": is_error})
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
    parser.add_argument("--timeout", type=float, default=10.0, help="Per-call deadline in seconds, capped at 30")
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
