"""Where the slice's boats are moored, until WS2's and rb-vernacular's piers and quays say (rb-combat, WS5).

Writes <rimeborn>/data/boats/moorings.json in the composed scene's frame (eirvane: compose_tyffir.py's WORLD), for
compose_tyffir to put a `hello.Boat` at each: [{"place", "kind", "water", "at": [x, y, z], "yaw_deg"}], with yaw about
+Y turning the boat's bow (+Z) to (sin, 0, cos).
  - Each town of the Life Table gets one, on the nearest water near it: a lake (a rowboat, 4 m in from the shore,
    along it), or a river wide enough to row (a rowboat or, on a broad one, a currach, along the flow), or a canal (a
    barge where it is broad).
  - Each canal also gets a barge every CANAL_EVERY metres.
  - The sea has none yet: WS1's water surface has no coastline (the sea is wherever the ground is below 0).

Reads (only): <rimeborn>/data/life/tyffir_life.json (places, scene frame), <rimeborn>/data/scatter/water_surfaces.json
(WS1's water surface, world metres; its "scene_origin" if it has one, else eirvane's anchor from tools/world_frame.py).

  python tools/rimeborn_moorings.py --rimeborn games/Rimeborn      # writes games/Rimeborn/data/boats/moorings.json
  python tools/rimeborn_moorings.py --rimeborn <dir> --out <file.json>
  python tools/rimeborn_moorings.py --check
"""
import argparse
import json
import math
import sys
from pathlib import Path

WORLD = "eirvane"  # compose_tyffir.py's world frame
REACH = 120.0  # m past a town's radius its water may be
SHORE_IN = 4.0  # m in from a lake's shore
ROWABLE = 2.2  # a river's half width a rowboat needs (beam 1.3 m and its oars)
BROAD = 4.0  # half width for a currach on a river, a barge on a canal
CANAL_EVERY = 300.0  # m between barges along a canal


def inside(outline, x, z):
    odd, j = False, len(outline) - 1
    for i in range(len(outline)):
        (ax, az), (bx, bz) = outline[i], outline[j]
        if (az > z) != (bz > z) and x < (bx - ax) * (z - az) / (bz - az) + ax:
            odd = not odd
        j = i
    return odd


def yaw_of(dx, dz):
    return round(math.degrees(math.atan2(dx, dz)), 1)


def lake_mooring(lake, cx, cz):
    """4 m in from the lake's shore point nearest (cx, cz), its bow along the shore: (distance, x, z, yaw)."""
    o = lake["outline"]
    k = min(range(len(o)), key=lambda i: math.dist(o[i], (cx, cz)))
    mx, mz = sum(p[0] for p in o) / len(o), sum(p[1] for p in o) / len(o)
    (px, pz), (nx, nz) = o[k - 1], o[(k + 1) % len(o)]
    tx, tz = nx - px, nz - pz
    # Inward: the shore's normal on the side the lake's middle is.
    ix, iz = -tz, tx
    if ix * (mx - o[k][0]) + iz * (mz - o[k][1]) < 0:
        ix, iz = -ix, -iz
    n = math.hypot(ix, iz) or 1.0
    x, z = o[k][0] + ix / n * SHORE_IN, o[k][1] + iz / n * SHORE_IN
    if not inside(o, x, z):
        return None
    return math.dist((x, z), (cx, cz)), x, z, yaw_of(tx, tz)


def reach_mooring(reach, cx, cz):
    """The reach's point nearest (cx, cz) where it is wide enough, its bow downstream: (distance, x, z, yaw, hw)."""
    pts = reach["pts"]
    best = None
    for k in range(len(pts) - 1):
        x, z, _, hw = pts[k][:4]
        if hw < ROWABLE:
            continue
        d = math.dist((x, z), (cx, cz))
        if best is None or d < best[0]:
            best = (d, x, z, yaw_of(pts[k + 1][0] - x, pts[k + 1][1] - z), hw, pts[k][2])
    return best


def moorings(places, water, ox, oz):
    out = []
    lakes = [l for l in water.get("lakes", []) if not l.get("frozen") and len(l.get("outline", [])) >= 3]
    for p in places:
        name = p["place"]["name"]
        cx, cz = p["centre"][0] + ox, p["centre"][2] + oz
        reach = float(p.get("radius", 60)) + REACH
        found = []
        for l in lakes:
            b = l.get("bbox") or [min(q[0] for q in l["outline"]), min(q[1] for q in l["outline"]),
                                  max(q[0] for q in l["outline"]), max(q[1] for q in l["outline"])]
            if b[0] - reach > cx or b[2] + reach < cx or b[1] - reach > cz or b[3] + reach < cz:
                continue
            m = lake_mooring(l, cx, cz)
            if m and m[0] <= reach:
                found.append((m[0], "lake", "rowboat", m[1], m[2], l["level"], m[3]))
        for r in water.get("rivers", []):
            m = reach_mooring(r, cx, cz)
            if m and m[0] <= reach:
                kind = ("barge" if m[4] >= BROAD else "rowboat") if r.get("canal") else (
                    "currach" if m[4] >= BROAD else "rowboat")
                found.append((m[0], "canal" if r.get("canal") else "river", kind, m[1], m[2], m[5], m[3]))
        if found:
            _, w, kind, x, z, level, yaw = min(found)
            out.append({"place": name, "kind": kind, "water": w,
                        "at": [round(x - ox, 2), round(level + 0.15, 3), round(z - oz, 2)], "yaw_deg": yaw})
    for r in water.get("rivers", []):
        if not r.get("canal"):
            continue
        run, pts = CANAL_EVERY * 0.5, r["pts"]
        for k in range(len(pts) - 1):
            (x, z, s, hw), (nx, nz) = pts[k][:4], pts[k + 1][:2]
            run += math.dist((x, z), (nx, nz))
            if run >= CANAL_EVERY and hw >= ROWABLE:
                run = 0.0
                out.append({"place": r.get("name") or "a canal", "kind": "barge" if hw >= BROAD else "rowboat",
                            "water": "canal", "at": [round(x - ox, 2), round(s + 0.15, 3), round(z - oz, 2)],
                            "yaw_deg": yaw_of(nx - x, nz - z)})
    return out


def build(rimeborn: Path):
    water = json.loads((rimeborn / "data/scatter/water_surfaces.json").read_text(encoding="utf-8"))
    table = json.loads((rimeborn / "data/life/tyffir_life.json").read_text(encoding="utf-8"))
    origin = water.get("scene_origin")
    if origin is None:
        sys.path.insert(0, str(rimeborn / "tools"))
        import world_frame  # noqa: E402  (Rimeborn's anchors; read only)
        origin = world_frame.REGION_ANCHORS[WORLD]
    m = moorings(table["places"], water, origin[0], origin[2])
    return {"version": 1, "frame": f"{WORLD} (compose_tyffir.py's world)", "made_by": "tools/rimeborn_moorings.py (WS5)",
            "moorings": m}


def check():
    """`--check`: a lake's mooring is inside it, along its shore; a river's on a wide enough point, downstream."""
    side = [float(i) for i in range(0, 40, 4)]  # a 40 m square, a point every 4 m as WS1's outlines
    lake = {"level": 5.0, "outline": [[x, 0.0] for x in side] + [[40.0, z] for z in side]
            + [[40.0 - x, 40.0] for x in side] + [[0.0, 40.0 - z] for z in side]}
    d, x, z, yaw = lake_mooring(lake, 20.0, -30.0)
    assert inside(lake["outline"], x, z) and abs(z - SHORE_IN) < 1e-6 and abs(abs(yaw) - 90.0) < 1e-6, (x, z, yaw)
    reach = {"pts": [[0, 100, 3, 1.0], [10, 100, 2.9, 3.0], [20, 100, 2.8, 3.0]]}
    d, x, z, yaw, hw, s = reach_mooring(reach, 0.0, 90.0)
    assert (x, yaw, s) == (10, 90.0, 2.9), (x, yaw)
    got = moorings([{"place": {"name": "T"}, "centre": [-1000.0, 0.0, -2000.0], "radius": 30}],
                   {"lakes": [lake], "rivers": []}, 1020.0, 2000.0)
    assert got and got[0]["kind"] == "rowboat" and got[0]["at"][1] == 5.15, got
    print("check: ok")


def main():
    if "--check" in sys.argv:
        return check()
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--rimeborn", type=Path, required=True, help="the Rimeborn project folder")
    ap.add_argument("--out", type=Path, help="default: <rimeborn>/data/boats/moorings.json")
    args = ap.parse_args()
    out = args.out or args.rimeborn / "data" / "boats" / "moorings.json"
    doc = build(args.rimeborn.resolve())
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(doc, indent=1), encoding="utf-8", newline="\n")
    kinds = {}
    for m in doc["moorings"]:
        kinds[m["kind"]] = kinds.get(m["kind"], 0) + 1
    print(f"{out}: {len(doc['moorings'])} moorings {kinds}")


if __name__ == "__main__":
    main()
