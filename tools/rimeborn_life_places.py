"""Tyffir's towns as life: one table of places for somnium_life (rb-combat, WS5), from Rimeborn's settlements.

Writes a Life Table (somnium_life::LifeTable; read by the engine's somnium.LifeTable component): every settlement WS2
built, as a place in the composed world's frame (eirvane: compose_tyffir.py's WORLD), with
  - a spot at the door of each building people live or work in (by its type, else its use): Home, Work, Inn, Chapel,
    Market, and a Pen at a byre's door so a farmstead keeps beasts;
  - a spot for each of WS1's open areas (green, market, parade, yard);
  - its streets (WS1's site plan) as lanes people walk, thinned to about 12 m with each street's ends snapped onto the
    street they meet so junctions join; roads and highways are also roads carts drive.

Reads (only): <rimeborn>/data/scatter/tyffir_places.json (WS1: places, streets, plots, areas; tyffir-local),
<rimeborn>/data/placements/realm/*.json (WS2: the realm's buildings; tyffir-local), <rimeborn>/data/placements/*.json
(WS2: the Eirvane tile's towns and farmsteads; eirvane frame), <rimeborn>/tools/world_frame.py (the anchors).

  python tools/rimeborn_life_places.py --rimeborn games/Rimeborn      # writes games/Rimeborn/data/life/tyffir_life.json
  python tools/rimeborn_life_places.py --rimeborn <dir> --out <file.json>
"""
import argparse
import json
import math
import sys
import zlib
from pathlib import Path

WORLD = "eirvane"  # compose_tyffir.py's world frame

# A building's spot by its type, else by its use; None: nobody lives or works there (walls, towers, gates).
BY_TYPE = {
    "church": "Chapel", "chapel": "Chapel", "shrine": "Chapel", "temple": "Chapel",
    "inn": "Inn", "tavern": "Inn",
    "market_hall": "Market",
    "smithy": "Work", "storehouse": "Work", "barn": "Work", "pithead": "Work", "harbour": "Work", "hall": "Work",
    "lancehall": "Work", "mill": "Work", "workshop": "Work",
    "byre": "Pen",
    "terrace": "Home", "row": "Home", "cottage": "Home", "house": "Home", "farmhouse": "Home", "longhouse": "Home",
    "barracks": "Home",
    "curtain": None, "tower": None, "white_tower": None, "gatehouse": None, "keep": None, "memorial": None,
}
BY_USE = {"house": "Home", "cottage": "Home", "farm": "Home", "barracks": "Home", "workshop": "Work",
          "warehouse": "Work", "pithead": "Work", "hall": "Work", "inn": "Inn"}
BY_AREA = {"green": "Green", "market": "Market", "parade": "Green", "yard": "Green"}
INDOOR = {"Home", "Work", "Inn", "Chapel"}
ROAD_KINDS = {"road", "highway"}
LANE_STEP = 3  # keep every third street point (WS1 spaces them 4 m)
SNAP = 3.0  # a street's end this close to another street joins it


def front(az_deg):
    """WS1/WS2's front azimuth (from north, clockwise) as an x, z direction (x east, z south)."""
    a = math.radians(az_deg)
    return math.sin(a), -math.cos(a)


def kind_of(b):
    t = (b.get("type") or "").lower()
    if t in BY_TYPE:
        return BY_TYPE[t]
    return BY_USE.get((b.get("use") or "").lower())


def door(position, az, depth):
    fx, fz = front(az)
    reach = depth * 0.5 + 0.6
    return [position[0] + fx * reach, position[1], position[2] + fz * reach]


def lanes_of(streets, dx, dz, height):
    """WS1's streets as lanes: thinned, ends snapped onto the streets they meet, lifted to the ground."""
    pts = [[list(p) for p in s["pts"]] for s in streets]
    keep = [{0, len(p) - 1} | set(range(0, len(p), LANE_STEP)) for p in pts]
    cell = {}
    for si, p in enumerate(pts):
        for k, q in enumerate(p):
            cell.setdefault((int(q[0] // SNAP), int(q[1] // SNAP)), []).append((si, k))
    for si, p in enumerate(pts):
        for end in (0, len(p) - 1):
            q = p[end]
            best = None
            cx, cz = int(q[0] // SNAP), int(q[1] // SNAP)
            for ox in (-1, 0, 1):
                for oz in (-1, 0, 1):
                    for sj, k in cell.get((cx + ox, cz + oz), []):
                        if sj == si:
                            continue
                        d = math.dist(q, pts[sj][k])
                        if d < SNAP and (best is None or d < best[0]):
                            best = (d, sj, k)
            if best:
                _, sj, k = best
                p[end] = list(pts[sj][k])
                keep[sj].add(k)
    lanes, roads = [], []
    for s, p, kept in zip(streets, pts, keep):
        line = [[round(q[0] + dx, 3), height(q[0], q[1]), round(q[1] + dz, 3)] for i, q in enumerate(p) if i in kept]
        if len(line) < 2:
            continue
        lanes.append(line)
        if s.get("kind") in ROAD_KINDS:
            roads.append(line)
    return lanes, roads


def heights_from(plots, ground):
    """The ground near a (place-frame) point: the nearest plot's floor (WS1), else the place's ground."""
    grid = {}
    for pl in plots:
        c = pl["centre"]
        grid.setdefault((int(c[0] // 40), int(c[1] // 40)), []).append((c, pl["floor_y"]))

    def height(x, z):
        best = None
        for ox in (-1, 0, 1):
            for oz in (-1, 0, 1):
                for c, y in grid.get((int(x // 40) + ox, int(z // 40) + oz), []):
                    d = (c[0] - x) ** 2 + (c[1] - z) ** 2
                    if best is None or d < best[0]:
                        best = (d, y)
        return round(best[1] if best else ground, 3)

    return height


def area_spot(a, dx, dz):
    poly = a["poly"]
    cx = sum(p[0] for p in poly) / len(poly)
    cz = sum(p[1] for p in poly) / len(poly)
    s = sum(poly[i][0] * poly[(i + 1) % len(poly)][1] - poly[(i + 1) % len(poly)][0] * poly[i][1]
            for i in range(len(poly)))
    radius = max(4.0, min(30.0, math.sqrt(abs(s) * 0.5 / math.pi)))
    return {"kind": BY_AREA[a["use"]], "position": [round(cx + dx, 3), a["floor_y"], round(cz + dz, 3)],
            "radius": round(radius, 2), "label": a["name"]}


def building_spots(buildings, plots_by_id, dx, dy, dz):
    spots = []
    for b in buildings:
        kind = kind_of(b)
        if kind is None:
            continue
        plot = plots_by_id.get(b.get("plot"))
        depth = plot["size"][1] if plot else 8.0
        p = b["position_local"]
        at = door([p[0] + dx, p[1] + dy, p[2] + dz], b["yaw_front_az_deg"], depth)
        spots.append({"kind": kind, "position": [round(v, 3) for v in at],
                      "radius": 0.5 if kind in INDOOR else 6.0, "label": b["id"]})
    return spots


def build(rimeborn: Path):
    sys.path.insert(0, str(rimeborn / "tools"))
    import world_frame  # noqa: E402  (Rimeborn's anchors; read only)

    def shift(frame):
        a, w = world_frame.REGION_ANCHORS[frame], world_frame.REGION_ANCHORS[WORLD]
        return (a[0] - w[0], a[1] - w[1], a[2] - w[2])

    ws1 = json.loads((rimeborn / "data/scatter/tyffir_places.json").read_text(encoding="utf-8"))
    by_name = {p["name"].lower(): p for p in ws1["places"]}
    tdx, tdy, tdz = shift("tyffir")
    places, report = [], []

    def add(name, centre, radius, spots, lanes, roads):
        if not any(s["kind"] == "Home" for s in spots):
            report.append(f"  {name}: no homes, left out")
            return
        places.append({"centre": [round(v, 3) for v in centre], "radius": radius,
                       "seed": zlib.crc32(name.encode("utf-8")),
                       "place": {"name": name, "spots": spots, "lanes": lanes, "roads": roads}})
        kinds = {}
        for s in spots:
            kinds[s["kind"]] = kinds.get(s["kind"], 0) + 1
        report.append(f"  {name}: {kinds}, {len(lanes)} lanes ({sum(len(l) for l in lanes)} points), "
                      f"{len(roads)} roads")

    # The realm (tyffir-local, y absolute), with WS1's streets, plots and areas.
    for f in sorted((rimeborn / "data/placements/realm").glob("*.json")):
        d = json.loads(f.read_text(encoding="utf-8"))
        site = by_name.get(d["settlement"].lower())
        if site is None:
            report.append(f"  {d['settlement']}: not in WS1's site plan, left out")
            continue
        plots = {pl["id"]: pl for pl in site["plots"]}
        spots = building_spots(d["buildings"], plots, tdx, tdy, tdz)
        spots += [area_spot(a, tdx, tdz) for a in site["areas"] if a["use"] in BY_AREA]
        lanes, roads = lanes_of(site["streets"], tdx, tdz, heights_from(site["plots"], site["ground_y"]))
        add(d["settlement"], (site["x"] + tdx, site["ground_y"], site["z"] + tdz), float(site["radius_m"]),
            spots, lanes, roads)

    # The Eirvane tile's towns and farmsteads (eirvane frame, y absolute); WS1's streets where it has the place.
    for f in sorted((rimeborn / "data/placements").glob("*.json")):
        if f.stem.endswith(("_layout", "_plan")) or f.stem == "kaelmar_spire":
            continue
        d = json.loads(f.read_text(encoding="utf-8"))
        name = d.get("settlement") or f.stem.replace("_", " ").title()
        spots = building_spots(d.get("buildings", []), {}, 0.0, 0.0, 0.0)
        site = by_name.get(name.lower())
        lanes, roads = [], []
        if site:
            lanes, roads = lanes_of(site["streets"], tdx, tdz, heights_from(site["plots"], site["ground_y"]))
            spots += [area_spot(a, tdx, tdz) for a in site["areas"] if a["use"] in BY_AREA]
        s = d.get("site") or {}
        if "x" in s:
            centre = (s["x"], s.get("y", 0.0), s["z"])
            radius = float(s.get("radius_m", s.get("radius", 60)))
        else:
            xs = [sp["position"] for sp in spots] or [[0.0, 0.0, 0.0]]
            centre = tuple(sum(p[i] for p in xs) / len(xs) for i in range(3))
            radius = max(math.dist((centre[0], centre[2]), (p[0], p[2])) for p in xs) + 10.0
        add(name, centre, radius, spots, lanes, roads)

    # A town WS2 built twice (the realm's and the Eirvane tile's own placement, at the same centre): the fuller one.
    fullest = {}
    for p in places:
        name = p["place"]["name"]
        if name not in fullest or len(p["place"]["spots"]) > len(fullest[name]["place"]["spots"]):
            if name in fullest:
                report.append(f"  {name}: built twice, the fuller kept")
            fullest[name] = p
    places = list(fullest.values())
    table = {"version": 1, "frame": f"{WORLD} (compose_tyffir.py's world)",
             "made_by": "tools/rimeborn_life_places.py (WS5)", "places": places}
    return table, report


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--rimeborn", type=Path, required=True, help="the Rimeborn project folder")
    ap.add_argument("--out", type=Path, help="default: <rimeborn>/data/life/tyffir_life.json")
    args = ap.parse_args()
    out = args.out or args.rimeborn / "data" / "life" / "tyffir_life.json"
    table, report = build(args.rimeborn.resolve())
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(table, separators=(",", ":")), encoding="utf-8", newline="\n")
    homes = sum(1 for p in table["places"] for s in p["place"]["spots"] if s["kind"] == "Home")
    print(f"{out}: {len(table['places'])} places, {homes} homes")
    print("\n".join(report))


if __name__ == "__main__":
    main()
