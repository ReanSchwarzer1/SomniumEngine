# Prop sets: thousands of props as one entity

A **Prop Set** (`somnium.PropSet`) draws many copies of one glTF from a single entity. It is for the things a town is dressed with: barrels, crates, fences, hurdles, woodpiles, gravestones, carts, lamp posts, market clutter. It works in the spirit of Unreal's hierarchical instanced static meshes.

As entities, a barrel of four parts is five entities. Every frame each part is a row in the game's mesh loop and a draw the renderer sorts and culls, and every part is an entry in the scene file. In a prop set the barrel is one line in a list. Its copies are set on the GPU once and culled there one by one: off screen, hidden behind something, or past the set's draw distance. They cast shadows and, within 60 m of the camera, show in traced reflections and bounce light.

A town of 1,000 houses with 120 props each:

| | Entities | The game's mesh loop each frame |
|---|---|---|
| Props as entities | 187,000 | 11.8 ms |
| Props as sets (8,000 of them) | 75,000 | 0.96 ms |

That is a release build. On top of the mesh loop, the renderer pays for every submitted draw each frame, about 0.14 µs each; a set's copies cost it almost nothing.

## When to use one

Use a set for anything placed as-is and never touched one by one. Keep an ordinary entity for:

- anything with a script, an interaction, a door or a light;
- buildings with levels of detail (`somnium.LodGroup`) or cutaways;
- anything the player picks up or moves;
- anything that is painted with vertex paint.

The editor selects a set as one entity through the Outliner. Clicking a single copy in the viewport does not select it.

## The fields

Each field is shown with how it appears in a scene file:

```json
"somnium.PropSet": { "version": 1, "fields": {
  "enabled": true,
  "source": "assets/props/tyffir_props.gltf",
  "part_prefix": "barrel/",
  "copies": [[2.0, 0.0, -3.5, 1.57], [4.1, 0.0, -3.2, 0.3]],
  "draw_distance": 160.0,
  "casts_shadow": true,
  "shadow_distance": 60.0,
  "collide_bounds": true
}}
```

- **Source**: the glTF every copy draws, project-relative, the same path an Imported Mesh names.
- **Parts**: which of the source's mesh nodes draw, by the ordinal an Imported Mesh's `node` uses. When it is empty, every mesh node draws.
- **Part Prefix**: used instead of Parts. Every mesh node whose name starts with it draws. In a kit that names nodes `cart_0/walls/boards`, `cart_0/walls/iron`…, the prefix `cart_0/` draws the whole cart, one node per material.
- **Copies**: one entry per copy, in the set entity's own frame: `[x, y, z, yaw]`. The yaw is in radians about +Y. Moving the entity moves the whole set.
- **Scales**: empty, or one uniform scale per copy.
- **Rotations**: empty, or one rotation per copy, `[x, y, z, w]`, used instead of its yaw (for a fallen barrel).
- **Tints** and **Tint Of**: a small palette of linear RGB colours multiplied into the base colour, and one palette index per copy. An index past the end of the palette leaves that copy untinted. Each tint draws through a tinted copy of the part's material, so keep the palette short: a handful of colours, not one per copy.
- **Draw Distance**: horizontal metres from the camera a copy draws to. 0 means no limit. 120 to 200 m suits small props seen from the isometric camera.
- **Casts Shadow** and **Shadow Distance**: a Shadow Distance of 0 casts as far as the copy draws. Copies too small on screen to matter cast nothing, just as with ordinary meshes.
- **Collide Bounds**: each copy gets one static box over its parts' bounds.
- **Box Centres** and **Box Halves**: static boxes in the copy's own frame (centre, half extents), paired by index, when one box over the bounds is too coarse. Without either, the set does not collide.

Each copy places the source's root. Every part draws where it sits in the file, so a cart's wheels stay on the cart.

## Rules for streaming

A set is one top-level entity, and a streamed cell takes it whole at its own position. Keep a set to one place: a yard, a street, a square of about 128 m. Do not make one set for a whole region. The converter below cuts sets on a 128 m grid for this reason.

A set's source is decoded off the frame the first time a set names it. Its copies appear a few frames after the cell does.

## Converting what is already there

`tools/prop_sets.py` turns a scene's prop groups into prop sets. A prop group is a top-level entity with only a name and a transform, whose children are only kit meshes: an Imported Mesh, a name, an identity transform, the parent, and no material override. That is how rb-architecture's scenes place props today. Groups that draw the same nodes of the same source inside one square become one set at the square's centre. Each group becomes a copy, keeping its position, yaw (or rotation) and uniform scale. Everything else stays as it was.

```text
python tools/prop_sets.py scenes/arch_aldermere.scene.json scenes/arch_aldermere.sets.scene.json --draw-distance 160 --collide-bounds
```

It never writes its input, and it prints what it did, for example `{"entities_before": 4741, "entities_after": 3985, "sets": 131, "copies": 247}`. Options:

- `--area` sets the square size (128 m by default).
- `--shadow-distance` sets the sets' shadow distance.
- `--min-copies` leaves groups as entities when a square has fewer of a kind than this.

A composer that writes props should write sets directly. Group the props by source and parts, cut them by area, and write one `somnium.PropSet` entity per group, as in the example above.

## What a set does not have

- Per-copy scripts, interaction, picking or vertex paint.
- Levels of detail: one draw distance per set. Make the far half a second set with its own source if you need one.
- Motion of its own: copies stand still, though wind in their materials still sways them.
- Back-face culling: copies draw two-sided, which costs closed meshes a little.
- The renderer's GPU-driven path is required. Without it, sets do not draw, though their colliders still work. With no renderer at all (a headless run), nothing is built.
