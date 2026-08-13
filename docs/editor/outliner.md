# Outliner & Details

## Outliner

The top-right list is every entity in the scene. Click a row to select it. The viewport gizmo and Details panel follow. A scrollbar on the right means the list is taller than the pane — wheel or drag the thumb to move.

Type icons sit next to names (mesh, light, terrain, water, and so on). Search filters the tree.

## Details

The panel under the Outliner is the inspector for the current selection.

- **Transform** — Position, Rotation, Scale. Drag the slider or type a number.
- **Light** — intensity, range, cone, colour, temperature. Area lights show **Half W** / **Half H**; disc lights show **Radius**; tube lights show **Radius** and **Half W** (half-length). Only shown for lights.
- **Post Processing** — exposure, bloom, tonemap, RT Direct / RT Indirect / **RT Reflections** / **RT Refraction**, World Cache, RT Specular, Path Tracer, Mesh SDF, Probes, Analytic Mips, Shaft Amt, and the other scene-wide toggles.
- **Terrain / Foliage / Water** — authoring fields for those components. Terrain has **LOD Morph** / **Morph** and **Dbg** 0–31. Foliage has **Cull** / **LOD** / **Impostor** (horizontal metres: drop leaves, then keep solid parts — not a billboard). Water defaults are frozen; **SSR**, **RT Reflect**, and **Reflect Debug** are the reflection knobs (Help → Water).

Search at the top of Details hides sections that do not match. New component types add rows here as they ship — the inspector is not a closed list.
