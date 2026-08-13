# XV-Zeta — 32-layer landscape identity (plan)

**Status:** IN ENGINE — 2026-08-13. XV-J is next.  
**Sits between:** XV-I (done) and XV-J (verification).  
**Parent:** [`phase_XV.md`](../phase_XV.md).

Runtime inspection of the default Great Lakes landscape after XV-A–I: close-up
photogrammetry is present; from the preset camera the land reads as one
desaturated brown. Inspector palette clicks select a layer but do not paint.
This plan is the response.

## 1. What the live scene actually showed

- Close-up: tiled PBR (sand/mud ripple, packed normals/height) is working.
- Landscape view: ridges, water, and lighting vary; **hue does not**. Valleys
  and peaks share one ochre. A thin pale shore fringe is the only material
  identity that survives minification.
- Inspector: Paint = 5 (Mud, `>` on the palette). Foliage **Paint Mode is on**.
  Terrain has no paint-mode toggle. Wet = 0, Dbg = 0.

## 2. Channel confirmation (already in the engine)

Packed format is unchanged. The shader **does** consume the photogrammetry
channels. It does **not** tessellate or displace vertices.

| Source map | Packed | Shader use |
|---|---|---|
| Diffuse RGB | albedo RGB | PBR albedo; perceptual (`sqrt`) blend; GI mean albedo |
| Displacement | albedo A | Height-blend between layers; dominant-layer POM + POM self-shadow. **Not** geometry displacement (XV non-goal). |
| Normal DX XY | surface RG | Tangent normal, Z reconstructed; surface-gradient composition; hex counter-rotation on packed XY |
| ARM G (rough) | surface B | Roughness; Toksvig/Godot-style mip fixture |
| ARM R (AO) | surface A | Material AO, multiplied with GTAO |
| ARM B (metal) | dropped | Terrain is dielectric |
| Separate AO/Rough files | unused at runtime | Packer reads `arm` |

If a material “only looks good as a flat albedo swatch,” that is a **kit / splat
/ macro** failure, not a missing AO or height sample.

## 3. Why inspector clicks do not paint

Palette buttons only emit `SetTerrainPaintLayer`. Painting the splat requires
all of:

1. `terrain_edit_active` (F6, or tools 1–6),
2. `BrushMode::Paint` (key **6**; default brush is **Raise**),
3. drag on the viewport, not a click on the palette,
4. foliage paint **off** — `ToggleFoliagePaint` sets `terrain_edit_active = false`.

The captured UI has foliage Paint Mode checked, so viewport drags never reach
`apply_paint`. Clicking `Mud` only changes which layer *would* paint if the
terrain paint brush were armed.

Zeta must make the Terrain palette the paint tool: click a name → select that
layer, switch to `BrushMode::Paint`, turn terrain edit on, turn foliage paint
off. Add a Terrain `[Paint Mode]` toggle mirroring foliage. Viewport drag then
paints. Palette click itself does not stamp a texel (Fyrox/Unreal: palette
selects, stroke applies).

## 4. Why the landscape is brown

Ranked, from live code + the 16-layer roster — not from a missing texture.

1. **The kit is ochre.** Layers 0–15 are aerial grass-rock, forest duff, brown
   mud, sand, dry earth, red clay, gravel, pebbles, mossy rock. Even “Grass”
   (`aerial_grass_rock`) is mossy stone from 15 m, not lawn. At 400 m those
   scans minify to one dirt colour. Adding more brown scans will not fix this.
2. **Biome coverage is soil-heavy.** Inland `cover` splits grass / forest /
   meadow (three similar browns). Mid-elevation also gets dry earth, sparse
   grass, red clay, mud. Cool gray cliff (14) and snow (3) occupy steep/high
   bands that the preset camera mostly sees edge-on.
3. **Macro is not material identity.** Default overlay strength 0.45. On the
   Great Lakes heightmap the engine tries `assets/terrain/great_lakes/macro_color.png`
   (baked from the source **Diffuse Map** — a satellite-style brown continent).
   If that file is missing, a **landform-derived** 512² map (altitude / slope /
   hollow / noise, centred on 0.5) still does not inject green vs gray vs tan.
   Overlay on already-brown detail keeps brown.
4. **Detail fade.** Full strongest-four out to 60 m, then a fall to dominant
   layers by 400 m. Distant pixels keep the winner’s **mean**, which for this
   kit is tan.
5. **Layers 0–7 still tile at 0.25 / m** (4 m repeat) so old scenes do not
   retile. At landscape scale that is high-frequency noise that averages away.

O3DE’s macro material, Frostbite’s unique colour layer, and Unreal Landscape
colormaps exist so distant pixels show **which material won**, not which
heightfield hollow they sit in. Somnium already stores `layer_albedo[32]` mean
colours on the GPU; Zeta should drive the macro (or a splat-weighted unique
colour) from those, not from the satellite diffuse.

## 5. Thirty-two materials — architecture

Keep **four locally active** (strongest-four, hex ≤24 taps, steep biplanar ≤36).
Raise the **global** palette to 32.

| Choice | Decision |
|---|---|
| Encoding | **Eight RGBA splatmaps** (direct weights). Do not switch to O3DE-style indexed IDs. |
| Sparsity | Still at most four non-zero stored channels per texel; fifth decays. |
| Sidecar | **v4.** v3 copies 0–15, zeros 16–31, no four-nonzero on migrate. |
| WGSL | `array<vec4<T>, 8>` for per-layer scalars, never `array<f32, 32>`. Four more `i32` splat bindless indices. |
| `GpuTerrainMaterial` | Expect ~1600 bytes (16-wide blocks double; `layer_albedo` 256→512). Layout tests must move with it. |
| Paint / inspector | Palette 32 named buttons (two banks of 16 if the panel overflows). Keys `,`/`.` already wrap `TERRAIN_LAYER_COUNT`. |
| Cliff | Stay on layer 14 unless a cooler gray wall scan earns the slot. |

### Memory (this is the real gate)

XVI budgets: BC7 2K ≤ 200 MiB, RGBA8 2K ≤ 700 MiB, never both resident.

| Pack | 32 layers × 2 maps, mips included |
|---|---|
| RGBA8 2K | ~1365 MiB — **fails** 700 |
| RGBA8 1K | ~341 MiB — fits 700, not 200 |
| BC7 2K | ~341 MiB — **fails** 200 |
| BC7 1K | ~85 MiB — fits both |

Existing 0–7 are 4K on disk, runtime default 2K. Sixteen 2K RGBA8 pairs are
already ~683 MiB — at the RGBA8 ceiling **before** adding 16 layers.

**Zeta default:** keep 0–15 at `SOMNIUM_TERRAIN_RES` (2048). Pack 16–31 at 2K
sources but **load them at 1024** until a BC7 encoder exists. Log projected
residency before allocation. If 16–31 at 1K still blows the budget on a given
adapter, drop runtime 0–15 to 1024 with a one-line log (do not silently replace
the committed 4K files). RVT stays deferred.

## 6. Layers 16–31 — hue roles, not more dirt

Do **not** download until a first-party audit (same gate as XV-A): CC0, packer
quartet (`diff`, `nor_dx`, `arm`, `disp`) at 2K, DirectX normals, physical size,
no Quixel/AI. Substitutions go in `rejected_for_role`. Compatibility-locked
0–7 stay.

Intended **hue classes** (IDs are candidates; audit may substitute):

| Idx | Role (must read at 400 m) | Candidate (audit) | Fallback |
|---:|---|---|---|
| 16 | Lush green ground (not aerial grass-rock) | Poly Haven lawn/meadow scan TBD | ambientCG `Grass001` / `Grass005` |
| 17 | Dark conifer duff | `leaves_forest_ground` | ambientCG `Ground037` |
| 18 | Cool gray aerial rock | `aerial_rocks_01` or `aerial_rocks_02` | ambientCG `Rock029` |
| 19 | Dark wall / slate | `rock_face_01` / `02` if wall-tagged | ambientCG `Rock063` |
| 20 | Green moss carpet | PH moss ground TBD (not `mossy_rock`) | ambientCG moss ground |
| 21 | Pale limestone / chalk | PH audit | — |
| 22 | Dark wet loam (cooler than `brown_mud`) | PH audit | — |
| 23 | Pine-needle litter | PH audit | — |
| 24 | Bright meadow / wildgrass | PH audit | `Grass005` |
| 25 | Wetland / peat | PH audit | — |
| 26 | Gray granite talus | PH audit | — |
| 27 | Light dune (cooler than `aerial_sand`) | PH audit | — |
| 28 | Lichen rock | PH audit | — |
| 29 | Autumn leaf litter | `forest_floor` | — |
| 30 | Packed pale path | `grassy_cobblestone` only if it stays path, not dirt | — |
| 31 | Hard snow / wind crust (distinct from `snow_02`) | PH audit | — |

Reject any candidate whose 256² downscale is still the same ochre as layers
0–11. The acceptance test for a new ID is **hue ΔE against the current mean
ground colour**, not a pretty 4K crop.

## 7. Biome and macro (the diversity work)

- Rebuild the Appalachia preset so **flat-to-rolling inland** is green (16/24)
  and forest (1/17), not mud/earth/sand. Keep sand/mud in the waterline bands.
  Put cool gray (18/19/26) on steep faces with cliff 14. Snow stays high.
- **Splat-weighted unique colour** at 512²: for each macro texel, strongest-four
  (or all non-zero) × `layer_albedo`, written as the macro RGB. Overlay or lerp
  toward it with distance (detail fade already known). Optional: stop loading
  `great_lakes/macro_color.png` as the default overlay, or multiply it at a much
  lower strength so satellite brown cannot own the continent.
- Expose Macro strength next to Wet in the inspector (already a GPU scalar).
- Debug: 21 dominant albedo already exists; add a landscape-scale “macro RGB”
  view if 21 is too close-up.

Histogram-preserving blend and extra RNM maps stay out unless Zeta evidence
asks. Painted wetness channel still later.

## 8. Subphase slices (implement in this order)

| Slice | Work | Exit |
|---|---|---|
| **Zeta-A** | Terrain Paint Mode + palette arms paint; foliage paint cannot steal the stroke; current layer name visible | Click Mud, drag ground, mud appears. F6/6 still work. |
| **Zeta-B** | Splat-derived unique colour / macro; Great Lakes satellite overlay no longer dominates; biome retune of 0–15 toward green/gray | Overview camera is not one brown. Shore/cliff/snow readable without zoom. |
| **Zeta-C** | `TERRAIN_LAYER_COUNT = 32`, eight splatmaps, sidecar v4, 880→~1600 layout tests, inspector second bank | Old v3 scenes keep 0–15; 16–31 zero until packed. |
| **Zeta-D** | Audit + fetch + pack layers 16–31 (skip overwrite of 0–15). Fail closed on license/hash/maps. | 30 photographed layers; 16 and 24 stay procedural (`grass_path_*` failed ΔE). |
| **Zeta-E** | 32-weight biome; Create → Terrain / startup only. Bump `DEFAULT_LANDSCAPE_VERSION`. | Same seed → bit-identical weights. Landscape kit matrix rows for new hues. |

Then **XV-J**. Do not start J until Zeta-A–E are in engine (GPU evidence still
belongs to J).

## 9. Inspiration (pattern study, no code lift)

| Source | Take |
|---|---|
| O3DE Terrain | Many global materials, few local; **macro material** is unique colour, not a second detail set. Local source stores top-two IDs; Somnium keeps direct weights. |
| Frostbite / DICE | Low-frequency unique colour + tiled photogrammetry. Close terrain needs coherent scans; distant terrain needs the unique layer. |
| Unreal Landscape | Weightmaps + optional colormap. Colormap is why a landscape reads as fields from the air. |
| Terrain3D (Godot, MIT, not in `example_repo`) | Up to 32 texture sets; autoshade vs paint; debug views. |
| Far Cry AVT / CoD super-terrain | World-scale VT checklist — **out of Zeta**. Gate remains profiler-backed. |
| Poly Haven Verdant Trail | Cohesive green biome scans — use as a **role** reference when picking 16–31, not as a bundle import. |

## 10. Non-goals

- RVT, clipmaps, tessellation, true geometry displacement.
- Quixel / AI / non-CC0.
- Replacing the Great Lakes heightfield or water (`WaterComponent::great_lakes` stays frozen).
- Keeping an eight-layer “old look” as default (already removed).
- XV-J captures, adapter freeze, `context.md` close-out.
