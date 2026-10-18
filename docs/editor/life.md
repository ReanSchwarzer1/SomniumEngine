# Living towns

In Hello Engine, use **Create > Examples > Living Village**, then press **Play**. The village has houses along a high street and two back lanes, an inn, a chapel, a smithy and a storehouse, a green with a well, a pen and two fields. Its **Homes** setting sizes it from a hamlet of a dozen houses to a small town of hundreds.

Each home houses a household, and each trade has its keeper.

- **Days.** People keep their day:
  - farmers go out to the fields from dawn;
  - the smith works the forge;
  - people go to market and to prayer;
  - in the evening they go to the inn, then to bed.
- **Animals and carts.** Livestock grazes in the pen, and carts ply the street by day.
- **The clock** comes from the scene's **Time of Day** when it moves (its timescale). Otherwise an hour passes each minute, starting at half six.
- **The Lid.** Raise Time of Day's **Eclipse** past half and the Lid sends everyone indoors.

## While you play

| What | Does |
|---|---|
| Walk past villagers | Those standing turn to look at you. A game plays its greeting barks here. |
| B | Rings the alarm bell where you stand. Everyone within 80 m runs home and stays in for 45 seconds, beasts bolt to the far side of their pen, and carts stop. |
| A fight | Wherever a **Combatant** in the scene loses health, everyone within 60 m runs home. |

Neighbours idling at the same spot (the well, the green, the market) pair off and talk, facing each other.

## Authoring a place

| Component | Put it on | Does |
|---|---|---|
| **Life Place** | The settlement's root | Its spots and lanes are the entities under it. **Populate** houses a household in every home on its own; **Seed** picks who lives where. **Population** shows the live count. |
| **Life Spot** | A door, a green, a field, a pen, the well | **Kind**: Home, Work, Market, Inn, Chapel, Field, Pen, Green or Well. The entity's position is the spot. For an open spot, its scale on X and Z sets its size. |
| **Spline** | Under the place | A lane people walk and carts drive. Lanes join where they share a point, so put a point at every crossing. |
| **Villager**, **Livestock**, **Cart** | Any entity | A hand-placed person, beast or cart. It belongs to the nearest place, and a person's home is the nearest home. **Doing** shows what the villager is about. |
| **Animation LOD** | An animated rig | Far rigs are evaluated less often, and past **Cull** not at all. Past **Detail**, foot IK and look-at are dropped. Time a rig skips is owed back, so it keeps its place in a clip. |

A place that a streamed cell loads after Play began is picked up when it appears. When its cell unloads, its people live on. When it loads again, the same people are there, where life left them.

## Cost

Simulation LOD measures from the camera:

- villagers near it step every frame;
- further out, they step every few frames;
- far off, they step rarely, and they are not drawn.

Only the nearest 160 villagers, 48 beasts and 16 carts out of doors get a body. A town of any size costs the same number of entities.

The MCP authoring state's `life` entry reports the counts, the step and write times, how many bodies are held, how many villagers have greeted you, and how many fights rang the alarm. The log repeats the times every ten seconds.

Measured in a release build (the 150-home village, 344 agents): life costs about 0.1 ms of a 5 ms frame. A 400-home town over a whole day averages 0.035 ms a tick.

## Skinned bodies

**Create > Examples > Skinned Crowd** lays out a field of animated bodies; its **Count** sets how many.

- **Skinned Body** draws a skinned glTF at its entity, playing a clip on the body's own clock. Its **Time** says where in the clip it starts, so a crowd is not in step.
  - A file is read once however many bodies use it.
  - Far bodies are posed less often. Past 160 m they are not drawn and cost the GPU nothing.
  - **LOD 1** and **LOD 2** (lighter files with the same skeleton and clips) draw beyond their distances.
- **Skinned Socket**, on an entity under a Skinned Body, makes that entity follow one of the body's joints (`weapon_r`, `sheath`, `back`). The entity's own Transform is its offset from the joint. Put a sword under it and the sword stays in the hand.

## For captures

| Variable | Does |
|---|---|
| `SOMNIUM_LIFE_VILLAGE=1` | Builds the village at the origin at startup |
| `SOMNIUM_LIFE_HOMES=n` | Sets its size |
| `SOMNIUM_LIFE_HOUR=h` | Starts the day at hour `h` |
| `SOMNIUM_SKINNED_CROWD=n` | Lays out `n` skinned bodies at the origin |
