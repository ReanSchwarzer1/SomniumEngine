#!/usr/bin/env python3
"""Read-only proof against an ACTUAL running editor. Fails if no editor responds.

Verifies fragmented framing, new connections, auth/project/session rejection,
malformed JSON recovery, native allowlisting and a real MCP subprocess roundtrip.
Never writes a scene or fabricates an engine response.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import time
import uuid

from pipe_client import EditorClient, WindowsPipe, read_descriptor


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--connection", required=True, type=Path)
    parser.add_argument("--output", type=Path, help="Optional evidence JSON path (contains no token)")
    args = parser.parse_args()
    descriptor = read_descriptor(args.connection)
    client = EditorClient(args.connection, descriptor["project_id"], timeout=15)
    checks = []
    first = client.call("authoring.discover", {}, chunk_size=3)
    if first.get("ok") is False or first.get("error"):
        raise RuntimeError("Actual editor rejected discovery")
    checks.append("actual_editor_fragmented_discovery")
    second = client.call("authoring.discover", {})
    if second.get("ok") is False or second.get("error"):
        raise RuntimeError("Actual editor failed reconnect")
    checks.append("actual_editor_reconnect")
    for field, value, expected in (
        ("token", "wrong", "unauthorized"),
        ("project_id", "wrong", "wrong_project"),
        ("session_id", "wrong", "session_restarted"),
        ("method", "shell", "method_not_found"),
    ):
        request = {"id":uuid.uuid4().hex,"project_id":descriptor["project_id"],"session_id":descriptor["session_id"],
                   "token":descriptor["token"],"method":"authoring.discover","params":{}}
        request[field] = value
        with WindowsPipe(descriptor["pipe_name"], time.monotonic()+15) as pipe:
            pipe.write(json.dumps(request).encode()+b"\n", chunk_size=7)
            reply = json.loads(pipe.readline())
        if reply.get("error",{}).get("code") != expected:
            raise RuntimeError(f"Actual editor failed {expected} rejection")
        checks.append("actual_editor_"+expected)
    with WindowsPipe(descriptor["pipe_name"], time.monotonic()+15) as pipe:
        pipe.write(b"{invalid\n", chunk_size=2)
        reply = json.loads(pipe.readline())
        if reply.get("error",{}).get("code") != "parse_error":
            raise RuntimeError("Actual editor failed malformed-frame rejection")
    client.call("authoring.discover", {})
    checks.append("actual_editor_parse_error_recovery")
    messages = [
        {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"somnium-probe","version":"1"}}},
        {"jsonrpc":"2.0","method":"notifications/initialized"},
        {"jsonrpc":"2.0","id":2,"method":"tools/list"},
        {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"authoring.discover","arguments":{}}},
    ]
    completed = subprocess.run([sys.executable,"-B",str(Path(__file__).with_name("server.py")),"--connection",str(args.connection),"--timeout","15"],
        input="\n".join(json.dumps(message) for message in messages)+"\n",text=True,capture_output=True,timeout=30)
    replies = [json.loads(line) for line in completed.stdout.splitlines()]
    if completed.returncode or len(replies) != 3 or replies[-1].get("result",{}).get("isError",True):
        raise RuntimeError("MCP subprocess did not complete actual editor discovery")
    checks.append("actual_mcp_subprocess_editor_roundtrip")
    report = {"kind":"live_editor_transport_evidence","project_id":descriptor["project_id"],"session_id":descriptor["session_id"],
              "checks":checks,"discovery":first,"scope":"Read-only transport proof; scene edits/imports/captures require their separate integration evidence."}
    if args.output:
        args.output.parent.mkdir(parents=True,exist_ok=True)
        args.output.write_text(json.dumps(report,indent=2),encoding="utf-8")
    print(json.dumps(report,indent=2))


if __name__ == "__main__":
    main()
