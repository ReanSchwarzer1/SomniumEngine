# Combat

Use **Create > Examples > Combat Sandbox** in Hello Engine, then press **Play**. You are the protagonist; two companions fight beside you and six raiders come at you with an axe, a spear, a knife, a bow, a Lattice rifle and fire magic.

## Controls

| Key | Does |
|---|---|
| W A S D | Move, relative to the camera |
| Hold Left Shift | Run |
| Left click | Strike. Click again to chain; hold for the heavy blow; with a bow, hold to draw and release to loose |
| Right click | Guard. The first instant parries (and opens a riposte); holding blocks |
| Space, or tap Left Shift | Dodge |
| 1 to 0 | Cast that spell. Hold the key to shape it: the cursor aims, the wheel changes the measure, R picks which measure, release to cast |
| F | Your sword school's art |
| C | The Kindled Body (stronger by your capacity; burns mana) |
| Q | Lock onto the enemy nearest the cursor |
| Tab | Slow time while focus lasts |
| E | Climb onto the nearest mount (the frost-elk beside you), or off it. Riding, W A S D steers it and Shift gallops; you can still strike, guard and cast |

Every binding is data in the combat action map and can be rebound.

## Components

| Component | Fields |
|---|---|
| **Combatant** | Player Controlled, Team (0 is your party), AI Style, Weapon, Sword School, Forms Learned, Rank, Health, Stamina, Capacity (mana, in lamps), Poise, Armor, Speed, Spells (comma-separated), Knows Numbers, Mount (makes it a horse, frost-elk or strider to ride). **Live**: Health, Stamina, Mana, State while Play runs |
| **Combat Surface** | Surface (grass burns, water freezes, snow melts, stone stops fire), Round, Solid (a collider for pillars and floors). Covers the entity's footprint |
| **Wind Volume** | Direction, Strength, Radius. Fire spreads with it; arrows bend in it |

## What reacts to what

| Cast | On grass | On water | On fire |
|---|---|---|---|
| Flame | burns, and spreads downwind | steam | keeps it burning |
| Frost | | freezes into ice you can walk on, which melts back | puts it out |
| Water | soaks it, so it will not catch | | puts it out |
| Gale | | | throws the fire downwind |
| Stone | | | smothers it |

Bodies follow the same rules: water puts out a burning fighter, a wet one freezes almost twice as fast, and a heavy blow shatters the frozen.

## Spells

A spell is a request in six parts: whose power pays (your well, a crystal, or a chorus of allies), the element, the shape (bolt, lance, wall, wave, step, bloom, ward), the measure (size, range, speed, count, duration), where it goes (the cursor, a target, around you), and how it is released (a full incantation is slow and steady, a short-chant fast, silent casting instant but it costs focus). Without the numbers, measures snap to the old fixed steps ("a forearm", "a bowshot") and count stops at three.
