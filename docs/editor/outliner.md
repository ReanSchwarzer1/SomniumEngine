# Outliner & Details

## Outliner

The top-right list is every entity in the scene. Click a row to select it. The viewport gizmo and Details panel follow. A scrollbar on the right means the list is taller than the pane — wheel or drag the thumb to move.

Type icons sit next to names (mesh, light, terrain, water, and so on). Search filters the tree.

## Details

The panel under the Outliner is the inspector for the current selection.

- **Transform** — Position, Rotation, Scale. Drag the slider or type a number.
- **Light** — intensity, range, cone, colour, temperature. Area lights show **Half W** / **Half H**; disc lights show **Radius**; tube lights show **Radius** and **Half W** (half-length). Only shown for lights.
- **Camera** — **Frustum Cull** (default on). CPU AABB early-out for terrain vis draws. Independent of **F10** (GPU 15B). Hold RMB and look away from the tile to see `cpu-cull` rise. `SOMNIUM_CPU_FRUSTUM=0` forces it off. **Play** uses this entity's world transform (local `-Z` is look); parent it to a player later. Physical Camera is on Post Processing.
- **Post Processing** — exposure, bloom, tonemap, RT Direct / RT Indirect / **RT Reflections** / **RT Refraction**, World Cache, RT Specular, Path Tracer, Mesh SDF, Probes, Analytic Mips, Shaft Amt, and the other scene-wide toggles.
- **Terrain / Foliage / Water** — authoring fields for those components. Terrain has **LOD Morph** / **Morph** and **Dbg** 0–31. Foliage has **Cull** / **LOD** / **Impostor** (horizontal metres: drop leaves, then keep solid parts — not a billboard), and the brush's own **Density** / **Radius** / **Max slope** / **Kind** / **Scale min** / **Scale max** / **Min layer**. **Min layer** is the one that can make a kind place nothing at all: set it to `0` to paint a kind on ground its palette entry would otherwise refuse (Help → Terrain). Water defaults are frozen; **SSR**, **RT Reflect**, and **Reflect Debug** are the reflection knobs (Help → Water).
- **UI Canvas** — create one from **Create → UI Canvas**. Its Details panel exposes screen/world/overlay placement, logical resolution, world pixel density, billboarding, and layer. Hello Engine supplies a visible starter widget tree through the public runtime canvas API; the entity is the authored attachment, not an editor-only widget tree.

Search at the top of Details hides sections that do not match. New component types add rows here as they ship — the inspector is not a closed list.

## Reading a row

Selection is always two cues, never one colour: the row takes a translucent
indigo fill **and** a 2 px indigo rail down its left edge, and its label steps
up a weight. Hovering washes the row without moving anything — a hover that
changed a border would reflow the list under your cursor.
