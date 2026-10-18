"""Phase ALYX-A: Somnium's own editor icons, drawn in VectorCraft, and the Tabler glyphs vendored beside them.

Each Somnium icon is geometry on the family grid (24 x 24, 2-unit round-capped strokes, about 2 units of margin), the
same grid the vendored Tabler set uses, so the two are optically one family. VectorCraft (headless, over its MCP
server, `vectorcraft-cli mcp --headless`) draws every icon on its own 24 x 24 artboard and exports the SVG the engine
compiles in (`crates/somnium_ui/assets/icons/somnium/`), and one board of all of them, large, for review.

    python tools/alyx_icons.py            # draw and export the Somnium icons, and the review board
    python tools/alyx_icons.py --tabler DIR   # copy downloaded Tabler outline SVGs (tabler-icons 3.49, MIT) in, renamed

VectorCraft is built from the developer's copy of its source (`Downloads/Craft_Source/vectorcraft-main`,
`cargo build --release -p vectorcraft-cli`).
"""
import argparse
import json
import os
import re
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ICONS = ROOT / "crates/somnium_ui/assets/icons"
REVIEW = ROOT / "dev records/phase ALYX"
VECTORCRAFT = Path(os.environ.get("VECTORCRAFT_CLI",
                                  r"C:/Users/adhir/Downloads/Craft_Source/vectorcraft-main/target/release/vectorcraft-cli.exe"))

# name -> SVG path data on the 24-unit grid. Strokes only; the atlas keeps coverage and tints at draw time.
SOMNIUM = {
    # The authoring bridge: the editor (a ring) linked to an agent (a box with its signal).
    "mcp-link": ["M3 12a4 4 0 1 0 8 0a4 4 0 1 0 -8 0", "M11 12h3",
                 "M15 8h4a1 1 0 0 1 1 1v6a1 1 0 0 1 -1 1h-4a1 1 0 0 1 -1 -1v-6a1 1 0 0 1 1 -1", "M17 4v2", "M17 18v2"],
    # Two states and the transitions between them.
    "state-machine": ["M4 3h5a1 1 0 0 1 1 1v3a1 1 0 0 1 -1 1h-5a1 1 0 0 1 -1 -1v-3a1 1 0 0 1 1 -1",
                      "M15 16h5a1 1 0 0 1 1 1v3a1 1 0 0 1 -1 1h-5a1 1 0 0 1 -1 -1v-3a1 1 0 0 1 1 -1",
                      "M10 5.5h3a4 4 0 0 1 4 4v3.5", "M15 11l2 2l2 -2",
                      "M14 18.5h-3a4 4 0 0 1 -4 -4v-3.5", "M5 13l2 -2l2 2"],
    # Samples placed on two axes: a blend space.
    "blend-space": ["M4 4v16h16", "M8.5 15a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0",
                    "M15.5 15a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0", "M12 8a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0"],
    # A strip of film with a motion curve through it.
    "anim-clip": ["M4 5h16a1 1 0 0 1 1 1v12a1 1 0 0 1 -1 1h-16a1 1 0 0 1 -1 -1v-12a1 1 0 0 1 1 -1",
                  "M7 5v2", "M12 5v2", "M17 5v2", "M7 17v2", "M12 17v2", "M17 17v2",
                  "M6 13.5c2.5 -5 5 -5 6 -2.5s3.5 2.5 6 -2.5"],
    # A skeleton: head, spine, arms, legs.
    "rig": ["M10 5a2 2 0 1 0 4 0a2 2 0 1 0 -4 0", "M12 7v6", "M6 11l6 -2l6 2", "M12 13l-3 8", "M12 13l3 8"],
    # The material preview ball: a sphere, its highlight and a band.
    "material-ball": ["M4 12a8 8 0 1 0 16 0a8 8 0 1 0 -16 0", "M8.5 9a4.5 4.5 0 0 1 3 -2.2", "M4.5 14.5c4 2 11 2 15 0"],
    # A cube drawn as its triangles: the wireframe view.
    "wireframe": ["M12 3l8 4.5v9l-8 4.5l-8 -4.5v-9z", "M4 7.5l8 4.5l8 -4.5", "M12 12v9", "M4 7.5l8 13.5",
                  "M20 7.5l-8 13.5", "M12 3v9"],
    # A lamp throwing a window's pattern on the floor: a light cookie.
    "light-cookie": ["M9 3h6v3h-6z", "M10 6l-5 10", "M14 6l5 10", "M4 16h16v5h-16z", "M12 16v5"],
    # A screen whose picture ripples: the picture shift.
    "picture-shift": ["M4 4h16a1 1 0 0 1 1 1v10a1 1 0 0 1 -1 1h-16a1 1 0 0 1 -1 -1v-10a1 1 0 0 1 1 -1",
                      "M8 20h8", "M12 16v4", "M6 10c2 -3 4 3 6 0s4 3 6 0"],
    # A walkable floor cut into triangles: the navigation mesh.
    "nav-mesh": ["M3 18l5 -11l8 1l5 10z", "M8 7l4 11", "M16 8l-4 10"],
    # A texel grid, finer in one corner: texel density.
    "texel-density": ["M4 4h16v16h-16z", "M12 4v16", "M4 12h16", "M8 4v8", "M4 8h8"],
    # An eye whose world bends: the dream lens.
    "dream-lens": ["M3 12c2.5 -4 5.5 -6 9 -6s6.5 2 9 6c-2.5 4 -5.5 6 -9 6s-6.5 -2 -9 -6",
                   "M9.5 12a2.5 2.5 0 1 0 5 0a2.5 2.5 0 1 0 -5 0", "M7 20c1.5 -1 3.5 1 5 0s3.5 1 5 0"],
    # A screen showing a stepped, low-resolution edge: the retro picture.
    "retro-picture": ["M4 5h16a1 1 0 0 1 1 1v12a1 1 0 0 1 -1 1h-16a1 1 0 0 1 -1 -1v-12a1 1 0 0 1 1 -1",
                      "M6 16h3v-3h3v-3h3v-3h3"],
    # Three levels of detail, nested.
    "lod": ["M12 3l9 16h-18z", "M12 9.5l5 8.5h-10z"],
    # A real frame and the one generated beside it.
    "frame-generation": ["M3 5h11v9h-11z", "M10 10h11v9h-11z", "M6 8l2 1.5l-2 1.5"],
    # Light from a source striking the floor and bouncing on: path tracing (the older RayTrace glyph is the traced
    # reflection).
    "path-trace": ["M4 5a2 2 0 1 0 4 0a2 2 0 1 0 -4 0", "M7.5 7.5l4.5 11l6 -9", "M3 19h18", "M15.5 9.5l2.5 0l0 2.5"],
}

# Tabler outline name -> the name it is vendored under (the engine's own vocabulary).
TABLER = {
    "copy": "copy", "clipboard": "paste", "cut": "cut", "pencil": "rename", "refresh": "refresh", "star": "favorite",
    "history": "history", "sort-ascending": "sort", "layout-grid": "grid-view", "list": "list-view",
    "folder-plus": "new-folder", "file-plus": "new-asset", "external-link": "show-in-explorer", "focus-2": "focus",
    "bookmark": "bookmark", "tag": "tag", "dots": "more", "dots-vertical": "more-vertical", "menu-2": "menu",
    "keyboard": "keyboard", "palette": "theme", "device-desktop": "display", "accessible": "accessibility",
    "mouse": "mouse", "adjustments-horizontal": "adjustments", "bell": "notifications", "info-circle": "info",
    "bug": "debug", "terminal-2": "console", "code": "code", "table": "table", "chart-line": "chart", "cpu": "cpu",
    "database": "database", "plug": "plug", "plug-connected": "plug-connected", "shield-check": "shield-check",
    "shield-x": "shield-x", "key": "key", "activity": "activity", "robot": "agent", "user": "user", "world": "world",
    "box": "local-space", "magnet": "snap", "grid-dots": "grid", "perspective": "perspective", "sun": "lit",
    "sun-off": "unlit", "player-record": "record", "player-skip-back": "skip-back",
    "player-skip-forward": "skip-forward", "repeat": "loop", "zoom-in": "zoom-in", "zoom-out": "zoom-out",
    "timeline": "timeline", "keyframe": "keyframe", "keyframes": "keyframes", "bone": "bone", "walk": "locomotion",
    "hierarchy": "hierarchy", "binary-tree": "behavior-tree", "vector-bezier-2": "curve", "color-swatch": "swatches",
    "brush": "brush", "color-picker": "eyedropper", "package": "prefab", "components": "component", "puzzle": "plugin",
    "map-2": "map", "route": "route", "atom": "physics", "shape": "collider", "weight": "rigid-body",
    "wave-sine": "waveform", "movie": "sequence", "screenshot": "screenshot", "cloud": "weather", "moon": "moon",
    "clock": "clock", "file-export": "export", "lock-open": "unlocked", "pin": "pin", "unlink": "unlink",
    "minus": "remove", "crosshair": "pivot", "ruler-measure": "measure", "stack-2": "layers",
    "layout-sidebar": "panel-left", "layout-sidebar-right": "panel-right", "layout-bottombar": "panel-bottom",
    "layout-dashboard": "workspace", "app-window": "app-window", "home": "home", "arrow-left": "back",
    "arrow-right": "forward", "arrow-up": "up", "device-gamepad-2": "gamepad", "book-2": "docs", "sparkles": "effects",
}


class Craft:
    """Newline-delimited JSON-RPC 2.0 over stdio to a Craft app's MCP server."""

    def __init__(self, exe):
        self.proc = subprocess.Popen([str(exe), "mcp", "--headless"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.DEVNULL, text=True, encoding="utf-8", bufsize=1)
        self.next_id = 0
        self.request("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                    "clientInfo": {"name": "somnium-alyx-icons", "version": "1"}})
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}}) + "\n")
        self.proc.stdin.flush()

    def request(self, method, params):
        self.next_id += 1
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params}) + "\n")
        self.proc.stdin.flush()
        while True:
            line = self.proc.stdout.readline()
            if not line:
                raise RuntimeError(f"VectorCraft closed during {method}")
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

    def close(self):
        if self.proc.poll() is None:
            self.proc.stdin.close()
            try:
                self.proc.wait(10)
            except subprocess.TimeoutExpired:
                self.proc.kill()


def stroke(vc, d, dx=0.0, dy=0.0):
    item = vc.tool("draw_path", {"d": d, "fill": "none", "stroke": "#000000", "strokeWidth": 2})["id"]
    vc.tool("run_command", {"command": "stroke.set", "params": {"ids": [item], "cap": "round", "join": "round"}})
    if dx or dy:
        vc.tool("transform", {"ids": [item], "dx": dx, "dy": dy})


def family_svg(exported: str) -> str:
    """VectorCraft's export, restated in the family's form: a 24 x 24 view box, strokes in currentColor."""
    paths = re.findall(r'<path[^>]*\sd="([^"]+)"', exported)
    if not paths:
        raise RuntimeError("VectorCraft exported no paths")
    body = "".join(f'<path d="{d}"/>' for d in paths)
    return ('<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" '
            'stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">' + body + "</svg>\n")


def draw_somnium(vc, scratch: Path):
    out = ICONS / "somnium"
    for name, paths in SOMNIUM.items():
        vc.tool("run_command", {"command": "file.new", "params": {"width": 24, "height": 24}})
        for d in paths:
            stroke(vc, d)
        raw = scratch / f"{name}.svg"
        vc.tool("export", {"path": str(raw)})
        (out / f"{name}.svg").write_text(family_svg(raw.read_text(encoding="utf-8")), encoding="utf-8", newline="\n")
        print("drew", name)
    # One board of them all, a 32-unit cell each, for review at 8x.
    cell, cols = 32, 8
    rows = (len(SOMNIUM) + cols - 1) // cols
    vc.tool("run_command", {"command": "file.new", "params": {"width": cell * cols, "height": cell * rows}})
    for k, paths in enumerate(SOMNIUM.values()):
        for d in paths:
            stroke(vc, d, (k % cols) * cell + 4, (k // cols) * cell + 4)
    REVIEW.mkdir(parents=True, exist_ok=True)
    vc.tool("save_file", {"path": str(scratch / "alyx_somnium_icons.vectorcraft")})
    vc.tool("export", {"path": str(REVIEW / "icons_somnium_x8.png"), "scale": 8})


def manifest_add(names):
    path = ICONS / "icon-manifest.json"
    data = json.loads(path.read_text(encoding="utf-8"))
    known = {entry["id"] for entry in data["icons"]}
    for name in names:
        if name not in known:
            runtime = "".join(part.capitalize() for part in name.split("-"))
            data["icons"].append({"id": name, "file": f"somnium/{name}.svg", "runtime": runtime})
    path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8", newline="\n")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--tabler", type=Path, help="a folder of downloaded Tabler outline SVGs to vendor")
    parser.add_argument("--scratch", type=Path, default=Path(os.environ.get("TEMP", ".")) / "alyx_icons")
    args = parser.parse_args()
    if args.tabler:
        for source, name in TABLER.items():
            shutil.copyfile(args.tabler / f"{source}.svg", ICONS / "tabler" / f"{name}.svg")
        print("vendored", len(TABLER), "Tabler icons")
        return
    args.scratch.mkdir(parents=True, exist_ok=True)
    vc = Craft(VECTORCRAFT)
    try:
        draw_somnium(vc, args.scratch)
    finally:
        vc.close()
    manifest_add(SOMNIUM)


if __name__ == "__main__":
    main()
