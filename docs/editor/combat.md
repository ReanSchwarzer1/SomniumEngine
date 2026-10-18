# Combat

Use **Create > Examples > Combat Sandbox** in Hello Engine, then press **Play**. You are the protagonist; two companions fight beside you and six raiders come at you with an axe, a spear, a knife, a bow, a Lattice rifle and fire magic.

**Create > Examples > Combat Wilds** is open ground instead: a horse, a pack of five rime wolves to the north-east and a bandit camp (a chief, a spear, an axe, a bow) to the north-west, none of whom have seen you yet. Untick **Floor** to drop it on terrain: Play stands everyone on the ground.

## Controls

| Key | Does |
|---|---|
| W A S D | Move, relative to the camera |
| Hold Left Shift | Run. Running into a wall you can reach vaults it (up to 1.25 m) or climbs it (up to 2.3 m); running into deep water swims, and into the bank climbs out |
| Left click | Strike. Click again to chain; hold for the heavy blow; with a bow, hold to draw and release to loose |
| Right click | Guard. The first instant parries (and opens a riposte); holding blocks |
| Space, or tap Left Shift | Dodge |
| 1 to 0 | Cast that spell. Hold the key to shape it: the cursor aims, the wheel changes the measure, R picks which measure, release to cast |
| F | Your sword school's art |
| C | The Kindled Body (stronger by your capacity; burns mana) |
| Q | Lock onto the enemy nearest the cursor |
| Tab | Slow time while focus lasts |
| E | Climb onto the nearest mount (the frost-elk beside you), or off it. Riding, W A S D steers it and Shift gallops; you can still strike, guard and cast |
| H | Whistle for your mount. It comes from up to 150 m, the one you last rode first |

Every binding is data in the combat action map and can be rebound.

East of where you start is a course: a low wall to vault, a ledge to climb, and a walled basin of water deep enough to swim in. The fighters walk the same way you do, so a raider follows you over the wall and up the ledge: Play bakes a navmesh from the solid surfaces, and the course's two **Navigation Link** entities (Details: Destination, Bidirectional, Attachment Radius) tell the AI it can go over and up. Add a link to give the AI another way over something; a running fighter also climbs whatever it is pushing into.

West of the start is a 6 m bluff with a slope up from the north. Mounts walk their own slopes (a horse takes up to 34 degrees, the frost-elk 40) and do not vault or climb, so you can ride one off the edge: a fall past a safe speed hurts, and a long one throws you. Left alone, a mount grazes and ambles a few metres from where you left it, bolts from whoever hurts it, and never walks off a drop by itself.

## Being seen

Enemies do not notice you at once. Seen, their awareness fills, quickly up close and slowly at the edge of sight (faster if you run). Past a third they are suspicious and come to look; full, they fight. A walk is quiet; running feet, hooves and a whistle are heard, which draws a look but not the alarm. At night (the scene's **Time of Day**) they see only a third as far. A blow on someone who has not noticed you lands two and a half times as hard. Their reflexes only parry or dodge what they know is coming.

## Wolves and bandits

A rime wolf pack circles its prey a few metres out and darts in two at a time, leaping from behind; it follows its alpha and runs when the alpha falls. A bandit band holds less well than soldiers: every friend that falls near one shakes it, and when the chief falls the rest scatter. Tick **Leads** on a Combatant to make it the alpha or chief of its side within 15 m.

## Components

| Component | Fields |
|---|---|
| **Combatant** | Player Controlled, Team (0 is your party), AI Style (Wolf gives a wolf's body), Weapon (Fangs for a beast), Sword School, Forms Learned, Rank, Health, Stamina, Capacity (mana, in lamps), Poise, Armor, Speed, Spells (comma-separated), Knows Numbers, Mount (makes it a horse, frost-elk or strider to ride), Leads (an alpha or a chief). **Live**: Health, Stamina, Mana, State (an unridden mount shows grazing, wandering, coming or bolting) while Play runs |
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
