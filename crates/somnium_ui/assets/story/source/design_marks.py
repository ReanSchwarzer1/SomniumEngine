"""Objective marks for the story HUD, drawn in VectorCraft (the developer's Craft tools), headless over MCP.

The client is the one Rimeborn's tools/craft.py uses (itself TSF's designcraft client), reduced to what a mark needs.
Usage: python design_marks.py <out_dir>
"""
import json
import subprocess
import sys
from pathlib import Path

VECTORCRAFT = Path(r"C:/Users/adhir/Downloads/Craft_Source/vectorcraft-main/target/release/vectorcraft-cli.exe")


class Craft:
    def __init__(self, exe):
        self.proc = subprocess.Popen([str(exe), "mcp", "--headless"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.DEVNULL, text=True, encoding="utf-8", bufsize=1)
        self.next_id = 0
        self.request("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                    "clientInfo": {"name": "somnium-story-marks", "version": "1"}})
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}}) + "\n")
        self.proc.stdin.flush()

    def close(self):
        if self.proc.poll() is None:
            self.proc.stdin.close()
            try:
                self.proc.wait(20)
            except subprocess.TimeoutExpired:
                self.proc.kill()

    def request(self, method, params):
        self.next_id += 1
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params}) + "\n")
        self.proc.stdin.flush()
        while True:
            line = self.proc.stdout.readline()
            if not line:
                raise RuntimeError(f"server closed during {method}")
            message = json.loads(line)
            if message.get("id") == self.next_id:
                if "error" in message:
                    raise RuntimeError(f"{method}: {message['error']}")
                return message["result"]

    def tool(self, name, arguments=None):
        result = self.request("tools/call", {"name": name, "arguments": arguments or {}})
        if result.get("isError"):
            raise RuntimeError(f"{name}: {result.get('content')}")
        texts = [c.get("text", "") for c in result.get("content", []) if c.get("type") == "text"]
        try:
            return json.loads(texts[0]) if texts else result
        except (ValueError, IndexError):
            return texts[0] if texts else result

    def run(self, command, params=None):
        return self.tool("run_command", {"command": command, "params": params or {}})


def item(reply):
    return reply["id"] if isinstance(reply, dict) and "id" in reply else reply


def mark(vc, size, parts, out, name):
    vc.run("file.new", {"width": size, "height": size})
    for d, fill, stroke, width, opts in parts:
        args = {"d": d, "fill": fill, "stroke": stroke}
        if width is not None:
            args["strokeWidth"] = width
        it = item(vc.tool("draw_path", args))
        if opts:
            vc.run("stroke.set", {"ids": [it], **opts})
    vc.tool("save_file", {"path": str(out / f"{name}.vectorcraft")})
    vc.tool("export", {"path": str(out / f"{name}.svg")})
    vc.tool("export", {"path": str(out / f"{name}.png"), "options": {"ppi": 72.0 * 256 / size, "antiAlias": "art"}})


def main(out):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    round_ = {"cap": "round", "join": "round"}
    vc = Craft(VECTORCRAFT)
    try:
        # The mark over an objective: a tall, thin lozenge like a lamp's flame seen from afar, a small solid core, and a
        # short stem that points down at the place itself. Outline only, so the world shows through.
        mark(vc, 24, [
            ("M 12 2.5 L 17.2 9.5 L 12 16.5 L 6.8 9.5 Z", "none", "#ffffff", 1.35, {"join": "round"}),
            ("M 12 7.2 L 13.7 9.5 L 12 11.8 L 10.3 9.5 Z", "#ffffff", "none", None, None),
            ("M 12 18.4 L 12 21.4", "none", "#ffffff", 1.35, round_),
        ], out, "objective_mark")
        # The pointer at the screen's edge when the objective is off it: one open chevron facing +x (the widget turns
        # it toward the target) with the same small core behind it, so it reads as the same mark leaning away.
        mark(vc, 24, [
            ("M 9 5.5 L 16.5 12 L 9 18.5", "none", "#ffffff", 1.6, round_),
            ("M 6 10.3 L 7.7 12 L 6 13.7 L 4.3 12 Z", "#ffffff", "none", None, None),
        ], out, "objective_pointer")
    finally:
        vc.close()
    print("exported", sorted(p.name for p in out.iterdir()))


if __name__ == "__main__":
    main(sys.argv[1])
