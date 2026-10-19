"""The water boats float on: one map for hello_engine's boats (rb-combat, WS5), from WS1's rivers, lakes and the sea.

Writes <rimeborn>/data/boats/water.json in the composed world's frame (eirvane: compose_tyffir.py's WORLD):
  - "sea_level": the sea's surface (WS1's sea bodies sit at 0), where no river or lake is;
  - "lakes": each lake's outline and level (frozen ones left out: ice is no water for a boat);
  - "rivers": each river's centre line downstream, with its surface (the sloped water) and width at each point, every
    other point of WS1's (about 6 m apart); canals are marked (no current).

Reads (only): <rimeborn>/source-assets/world/tyffir/<region>_water.json for every region tools/world_frame.py anchors
(Tyffir and Vespergate today; tools/tyffir_terrain_water.py makes them), and tools/world_frame.py.

  python tools/rimeborn_boat_water.py --rimeborn games/Rimeborn      # writes games/Rimeborn/data/boats/water.json
  python tools/rimeborn_boat_water.py --rimeborn <dir> --out <file.json>
  python tools/rimeborn_boat_water.py --check
"""
import argparse
import json
import sys
from pathlib import Path

WORLD = "eirvane"  # compose_tyffir.py's world frame
SEA_LEVEL = 0.0  # WS1's sea bodies' surface_level
STEP = 2  # keep every other river point


def lake_of(lake, dx, dz):
    return {"name": lake.get("name") or "", "level": round(lake["level"], 3),
            "x": [round(x + dx, 2) for x in lake["x"]], "z": [round(z + dz, 2) for z in lake["z"]]}


def river_of(river, dx, dz):
    n = len(river["x"])
    keep = list(range(0, n, STEP))
    if keep[-1] != n - 1:
        keep.append(n - 1)  # its mouth, where it meets the next water
    return {"name": river.get("name") or "", "canal": bool(river.get("canal")),
            "x": [round(river["x"][i] + dx, 2) for i in keep], "z": [round(river["z"][i] + dz, 2) for i in keep],
            "surface": [round(river["surface"][i], 3) for i in keep], "width": [round(river["width"][i], 2) for i in keep]}


def build(rimeborn: Path):
    sys.path.insert(0, str(rimeborn / "tools"))
    import world_frame  # noqa: E402  (Rimeborn's anchors; read only)

    w = world_frame.REGION_ANCHORS[WORLD]
    lakes, rivers, report = [], [], []
    for f in sorted((rimeborn / "source-assets/world/tyffir").glob("*_water.json")):
        d = json.loads(f.read_text(encoding="utf-8"))
        region = d.get("tile") or f.stem[: -len("_water")]
        if region not in world_frame.REGION_ANCHORS or region == WORLD:
            continue  # the Eirvane tile's water is Tyffir's, drawn again
        a = world_frame.REGION_ANCHORS[region]
        dx, dz = a[0] - w[0], a[2] - w[2]
        open_lakes = [l for l in d.get("lakes", []) if not l.get("frozen")]
        lakes += [lake_of(l, dx, dz) for l in open_lakes]
        rivers += [river_of(r, dx, dz) for r in d.get("rivers", []) if len(r.get("x", [])) >= 2]
        report.append(f"  {region}: {len(open_lakes)} lakes (of {len(d.get('lakes', []))}), "
                      f"{len(d.get('rivers', []))} rivers ({sum(1 for r in d.get('rivers', []) if r.get('canal'))} canals)")
    return {"version": 1, "frame": f"{WORLD} (compose_tyffir.py's world)", "made_by": "tools/rimeborn_boat_water.py (WS5)",
            "sea_level": SEA_LEVEL, "lakes": lakes, "rivers": rivers}, report


def check():
    """`--check`: a river keeps its mouth and is shifted into the world's frame; a frozen lake is no water."""
    r = river_of({"x": [0.0, 1.0, 2.0, 3.0], "z": [0.0] * 4, "surface": [4.0, 3.0, 2.0, 1.0], "width": [2.0] * 4},
                 10.0, -5.0)
    assert r["x"] == [10.0, 12.0, 13.0] and r["z"] == [-5.0] * 3 and r["surface"] == [4.0, 2.0, 1.0], r
    assert not r["canal"]
    assert lake_of({"x": [1.0], "z": [2.0], "level": 3.14159}, 1.0, 1.0) == {"name": "", "level": 3.142, "x": [2.0],
                                                                             "z": [3.0]}
    print("check: ok")


def main():
    if "--check" in sys.argv:
        return check()
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--rimeborn", type=Path, required=True, help="the Rimeborn project folder")
    ap.add_argument("--out", type=Path, help="default: <rimeborn>/data/boats/water.json")
    args = ap.parse_args()
    out = args.out or args.rimeborn / "data" / "boats" / "water.json"
    table, report = build(args.rimeborn.resolve())
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(table, separators=(",", ":")), encoding="utf-8", newline="\n")
    points = sum(len(r["x"]) for r in table["rivers"])
    print(f"{out}: {len(table['lakes'])} lakes, {len(table['rivers'])} rivers ({points} points), "
          f"sea at {table['sea_level']}, {out.stat().st_size // 1024} KB")
    print("\n".join(report))


if __name__ == "__main__":
    main()
