"""Tyffir's towns as life: one table of places for somnium_life (rb-combat, WS5), from Rimeborn's settlements.

Writes a Life Table (somnium_life::LifeTable; read by the engine's somnium.LifeTable component): every settlement WS2
built, as a place in the composed world's frame (eirvane: compose_tyffir.py's WORLD), with
  - a spot at the door of each building people live or work in (by its type, else its use): Home, Work, Inn, Chapel,
    Market, and a Pen at a byre's door so a farmstead keeps beasts;
  - a spot for each of WS1's open areas (green, market, parade, yard);
  - its streets (WS1's site plan) as lanes people walk, thinned to about 12 m with each street's ends snapped onto the
    street they meet so junctions join; roads and highways are also roads carts drive;
  - its own people (WS3's `scenes/char_<place>_people.scene.json`): the bodies it casts there, by role. Children and
    elders by the body's age (SOURCE.json), smiths, priests, keepers and herders by their family, anyone else as the
    town's adults; guards stay WS3's set pieces. Over the table's looks (WS3's crowd contract).

Reads (only): <rimeborn>/data/scatter/tyffir_places.json (WS1: places, streets, plots, areas; tyffir-local),
<rimeborn>/data/placements/realm/*.json (WS2: the realm's buildings; tyffir-local), <rimeborn>/data/placements/*.json
(WS2: the Eirvane tile's towns and farmsteads; eirvane frame), <rimeborn>/tools/world_frame.py (the anchors),
<rimeborn>/scenes/char_*_people.scene.json and <rimeborn>/assets/characters/*/SOURCE.json (WS3: who lives where).

  python tools/rimeborn_life_places.py --rimeborn games/Rimeborn      # writes games/Rimeborn/data/life/tyffir_life.json
  python tools/rimeborn_life_places.py --rimeborn <dir> --out <file.json>
"""
import argparse
import json
import math
import re
import struct
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

# Looks: WS3's crowd contract (WS3_CHARACTERS.md, "The crowd contract"). Bodies by somnium_life role: crowd_* children
# and elders by their age (SOURCE.json), the other crowd_* adults farmers, labourers and anyone a role has no bodies for.
TRADES = {"Smith": "smith", "Priest": "priest", "Keeper": "keeper", "Herder": "herder"}
BEASTS = {"Sheep": "sheep", "Cow": "march_cow", "Goat": "goat", "Pig": "pig", "Hen": "hen", "Dog": "sheepdog"}
POSES = {"Idle": "Idle_Neutral_Loop", "Walk": "Loco_Walk_F_Loop", "Run": "Loco_Jog_F_Loop", "Talk": "Talk_Loop",
         "Work": "Bench_Work_Loop", "Farm": "Hoe_Loop", "Tend": "Farm_Water", "Trade": "Haggle_Loop",
         "Pray": "Kneel_Pray_Loop", "Sit": "Sit_Idle_Loop", "Drive": "Drive_Reins_Loop"}
# A clip may list alternatives, `A|B`: a body plays the first its file has (a youth built with the adults' clips, an
# elder without the old walk, a hen that runs instead of trotting).
CHILD_POSES = {"Idle": "Child_Idle_Loop|Idle_Neutral_Loop", "Walk": "Child_Walk_F_Loop|Loco_Walk_F_Loop",
               "Run": "Child_Run_F_Loop|Loco_Jog_F_Loop",
               # on the spot: Child_Hopscotch and Child_Tag travel (WS3)
               "Play": "Child_Spin_Loop|Child_Fidget_Loop|Idle_Neutral_Loop"}
# A trade's own clips (with what they hold, clips.json `gear`), kept only where every body of the role carries the clip.
ROLE_POSES = {"Smith": {"Work": "Hammer_Anvil_Loop"}, "Keeper": {"Work": "Sweep_Loop"},
              "Labourer": {"Work": "Chop_Wood_Loop"}, "Herder": {"Tend": "Farm_Water"}}
ELDER_POSES = {"Idle": "Idle_Old_Loop|Idle_Neutral_Loop", "Walk": "Loco_Old_Walk_F_Loop|Loco_Walk_F_Loop"}
BEAST_POSES = {"Graze": "Graze_Loop|Idle_Loop", "Walk": "Walk_F_Loop", "Run": "Trot_F_Loop|Run_F_Loop|Walk_F_Loop",
               "Idle": "Idle_Loop"}
CART = "cart_horse"  # a cart is drawn as its horse, walking the road or standing (there is no cart mesh yet)
CART_POSES = {"Drive": "Walk_F_Loop", "Idle": "Idle_Loop"}
# Loco_Walk_F_Loop's root speed, and Loco_Jog_F_Loop's measured from its planted feet (WS3: Loco_Run_F_Loop is a
# 1.5 m/s trot that skates at a flee's pace; the jog reads right from 5.6 to 6.0 m/s).
WALK_SPEED, RUN_SPEED = 0.86, 5.6
SNAP = 3.0  # a street's end this close to another street joins it
CHILD_BELOW, ELDER_FROM = 16, 50  # a body's age (SOURCE.json) that makes it a child's or an elder's (WS3's elders: 52+)
NOT_VILLAGERS = {"guard"}  # families WS3 casts as set pieces, not people on a routine


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


def age_of(folder: Path) -> float:
    """A WS3 body's age, from its SOURCE.json (30 when it has none)."""
    try:
        return float(json.loads((folder / "SOURCE.json").read_text(encoding="utf-8")).get("age", 30))
    except (OSError, ValueError):
        return 30.0


def age_stage(age: float) -> str:
    """The role name a body of `age` is cast in when no trade claims it."""
    return "Child" if age < CHILD_BELOW else "Elder" if age >= ELDER_FROM else "default"


def clips_in(glb: Path) -> set:
    """The animation names a glb carries (its JSON chunk only)."""
    try:
        with open(glb, "rb") as f:
            head = f.read(20)
            n = struct.unpack("<I", head[12:16])[0]
            doc = json.loads(f.read(n))
        return {a.get("name", "") for a in doc.get("animations", [])}
    except (OSError, ValueError, struct.error):
        return set()


def height_of(folder: Path) -> float:
    """A WS3 body's height in metres, from its SOURCE.json (1.76, the clips' reference body, when it has none)."""
    try:
        return float(json.loads((folder / "SOURCE.json").read_text(encoding="utf-8")).get("height_m") or 1.76)
    except (OSError, ValueError):
        return 1.76


def looks_of(rimeborn: Path):
    """WS3's bodies that exist on disk, as somnium_life looks (paths relative to the assets folder)."""
    chars, creatures = rimeborn / "assets/characters", rimeborn / "assets/creatures"
    body = lambda name: f"characters/{name}/{name}.glb"  # noqa: E731
    have = lambda name: (chars / name / f"{name}.glb").exists()  # noqa: E731
    crowd = [f"crowd_{i:02}" for i in range(64) if have(f"crowd_{i:02}")]
    numbered = lambda prefix: [n for n in (f"{prefix}_{i:02}" for i in range(16)) if have(n)]  # noqa: E731
    stage = {n: age_stage(age_of(chars / n)) for n in crowd}
    adults = [body(n) for n in crowd if stage[n] == "default"]
    villagers = {"default": adults, "Farmer": adults, "Labourer": adults,
                 "Elder": [body(n) for n in crowd if stage[n] == "Elder"],
                 "Child": [body(n) for n in crowd if stage[n] == "Child"]}
    for role, prefix in TRADES.items():
        villagers[role] = [body(n) for n in numbered(prefix)]
    villagers = {k: v for k, v in villagers.items() if v}
    beasts = {kind: f"creatures/{name}/{name}.glb" for kind, name in BEASTS.items()
              if (creatures / name / f"{name}.glb").exists()}
    # A trade's own clip only where every body of the role carries it (a missing clip holds the last pose).
    role_poses = {}
    for role, poses in ROLE_POSES.items():
        bodies = villagers.get(role, [])
        kept = {pose: clip for pose, clip in poses.items()
                if bodies and all(clip in clips_in(rimeborn / "assets" / b) for b in bodies)}
        if kept:
            role_poses[role] = kept
    try:
        gear = json.loads((rimeborn / "assets/animation/clips.json").read_text(encoding="utf-8")).get("gear", {})
    except (OSError, ValueError):
        gear = {}
    return {"villagers": villagers, "beasts": beasts, "poses": POSES, "child_poses": CHILD_POSES,
            "elder_poses": ELDER_POSES, "beast_poses": BEAST_POSES, "walk_speed": WALK_SPEED,
            "run_speed": RUN_SPEED, "role_poses": role_poses, "gear": gear,
            **({"cart": f"creatures/{CART}/{CART}.glb", "cart_poses": CART_POSES}
               if (creatures / CART / f"{CART}.glb").exists() else {})}


def asset_id(rel: str) -> str:
    """somnium_asset::AssetId::from_relative_path: FNV-1a 128 over the normalised relative path."""
    h = 0x6C62272E07BB014262B821756295C58D
    for b in rel.replace(chr(92), "/").strip("/").lower().encode("utf-8"):
        h = ((h ^ b) * 0x0000000001000000000000000000013B) % (1 << 128)
    return f"{h or 1:032x}"


def people_index(rimeborn: Path):
    """Every person body WS3 made: the asset id of it and of each variant (its LODs, the lighter `_placed` copies the
    people scenes use) -> (the full body's path, family, age). Life wears the full body: it has every clip."""
    index = {}
    for glb in sorted((rimeborn / "assets/characters").glob("*/*.glb")):
        name = glb.parent.name
        if glb.stem != name and not glb.stem.startswith(f"{name}_"):
            continue
        age = age_of(glb.parent)
        family = re.sub(r"_\d+r?$", "", name)
        index[asset_id(f"characters/{name}/{glb.name}")] = (f"characters/{name}/{name}.glb", family, age)
    return index


def people_of(rimeborn: Path, name: str, index):
    """The bodies WS3 casts in `name` (its char_<place>_people scene), by somnium_life role name; {} without one."""
    slug = re.sub(r"[^a-z0-9]+", "_", name.lower()).strip("_")
    scene = rimeborn / "scenes" / f"char_{slug}_people.scene.json"
    if not scene.exists():
        return {}
    roles = {}
    for line in scene.read_text(encoding="utf-8").splitlines():
        if not line.startswith('{"entities"'):
            continue
        for e in json.loads(line)["entities"]:
            comps = e.get("components", {})
            body = comps.get("hello.SkinnedBody") or comps.get("hello.AnimatedPreview")
            source = ((body or {}).get("fields", {}).get("source") or {}).get("$asset")
            if source not in index:
                continue  # a beast, a cart horse, or not a WS3 person
            path, family, age = index[source]
            if family in NOT_VILLAGERS:
                continue
            trade = next((role for role, prefix in TRADES.items() if prefix == family), None)
            role = trade or age_stage(age)
            roles.setdefault(role, set()).add(path)
    return {role: sorted(paths) for role, paths in sorted(roles.items())}


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
    index = people_index(rimeborn)

    def add(name, centre, radius, spots, lanes, roads):
        if not any(s["kind"] == "Home" for s in spots):
            report.append(f"  {name}: no homes, left out")
            return
        entry = {"centre": [round(v, 3) for v in centre], "radius": radius, "seed": zlib.crc32(name.encode("utf-8")),
                 "place": {"name": name, "spots": spots, "lanes": lanes, "roads": roads}}
        people = people_of(rimeborn, name, index)
        if people:
            entry["people"] = people
        places.append(entry)
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
             "made_by": "tools/rimeborn_life_places.py (WS5)", "places": places, "looks": looks_of(rimeborn)}
    looks = table["looks"]
    # Every body anyone may wear, by its height: gear is sized to its holder.
    worn = {b for bodies in looks["villagers"].values() for b in bodies}
    worn |= {b for p in places for bodies in p.get("people", {}).values() for b in bodies}
    looks["heights"] = {b: round(height_of(rimeborn / "assets" / Path(b).parent), 3) for b in sorted(worn)}
    report.append(f"  looks: {sum(len(v) for v in looks['villagers'].values())} bodies over "
                  f"{len(looks['villagers'])} roles, {len(looks['beasts'])} beasts; trades' own clips "
                  f"{looks['role_poses']}; gear for {len(looks['gear'])} clips; {len(looks['heights'])} heights")
    cast = [p for p in places if p.get("people")]
    report.append(f"  own people: {len(cast)} of {len(places)} places (WS3's char_<place>_people scenes)")
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
