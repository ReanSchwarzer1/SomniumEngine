# Phase DF — Daggerfall

> *“The Iliac Bay is not a tile set. It is a country you can walk.”*
> TES II *Daggerfall* (1996) put a continent on screen when the hardware could
> not hold a continent’s worth of unique textures. This phase is the same
> problem at photogrammetry resolution: **keep the ground under the player’s
> feet looking like XV, while the rest of an open world stops costing like XV
> on every pixel.**

> **Codename:** Daggerfall, after *The Elder Scrolls II: Daggerfall*  
> **Status:** IN ENGINE (2026-08-14) — DF-A–G are in the tree. Inspector
> **Clipmap** stays **default off**. Walk luminance vs clipmap-off has **not**
> passed the 1% gate (DF-A +35.6% on an earlier capture). **A dedicated audit
> is required** before treating the path as “runs better” or turning it on by
> default — see §12. Do not start that audit in the same session that shipped
> the look/hitch fixes.  
> **Start-here for the engine:** [`post_halcyon_audit_handoff.md`](post_halcyon_audit_handoff.md)  
> **Clipmap audit start-here:** this file §12 + [`phase DF/DF-A_timings.md`](phase%20DF/DF-A_timings.md).  
> **Depends on:** Phase XV-A–J (32-layer PBR, strongest-four, unique-colour
> macro, aerial hex/POM cut). FSR 3 is complementary (fewer shaded pixels) and
> is **not** a substitute for this work.  
> **Target:** rustc 1.88, wgpu 29, winit 0.30  
> **Do not copy source** from UE5, O3DE, Far Cry, or CoD. Patterns below are
> studied from `C:\Users\adhir\Downloads\GE\example_repo` and papers; cite in
> `ATTRIBUTION.md` as they are used.

The current Great Lakes tile is **1024 m × 1024 m**, 32 global layers, eight
splatmaps, strongest-four, hex-tiling, 24-step POM. That is already more unique
PBR than many shipped games had in 2010. Open-world titles that *look* like they
use “dozens of textures” do **not** evaluate dozens of tiled materials at every
shaded pixel. They **cache a composited landscape** at screen-scaled density and
only pay the full blend where the player can resolve it.

---

## 1. Executive decision

**Ship an O3DE-style nested material clipmap** (macro + detail stacks, camera-
centred, toroidal update) as the shading fast path, with **UE5 Runtime Virtual
Texture’s split** (cache *camera-independent* data; keep *camera-dependent*
relief only near the camera).

**Near the ground, visual fidelity is a hard constraint.** Walking pixels must
not lose hex-tiling, height-blend, or POM self-shadow relative to today’s
`evaluate_terrain_material`. The way to do that is **not** a per-pixel
`if (close) { expensive } else { cheap }` inside one WGSL entry point — XV-Zeta
already proved that compiles three programs into one and made walking *slower*.
The way is:

1. **Bake** strongest-four + hex + height-blend into a **finest detail clipmap**
   whose texel density matches (or beats) a 2K layer at 4 m tile
   (≈ **512 texels/m**).
2. **March POM** against that **single** baked height field (already the
   dominant-layer idea), not against four packed arrays.
3. **Farther rings** of the same stack cover more metres at lower texels/m.
   Shading samples the stack (a handful of taps) instead of 4× hex (up to 24
   material-map taps) + POM steps.
4. Aerial view already zeros hex/POM via `gpu_material_for_camera` (> 80 m AGL).
   Daggerfall should make mid-range *walking toward the horizon* cheap too.

Far Cry **Adaptive Virtual Texturing** and CoD **Super Terrain** are the
blueprint for when the world grows past one kilometre. They are **DF-H
research**, not the first ship. Geometry clipmaps / a CDLOD rewrite are **out of
scope** except where 25C morph already exists (default off).

---

## 2. Goals

1. Cut **terrain shading time** at landscape and horizon views without a
   measurable loss at **eye level** (1.6 m AGL, looking along the ground).
2. Make **layer count** a residency/authoring problem, not a per-pixel cost:
   32 → 64 global layers should not multiply shading taps if only four are
   composited into the clipmap.
3. Keep FSR, vis-buffer, GTAO, ReSTIR, and Halcyon water working. Terrain stays
   in the visibility buffer and the TLAS.
4. Update incrementally as the camera moves (toroidal), not a full-screen blit
   every frame.
5. Inspector A/B: **Clipmap** on/off, debug visualization of which ring a pixel
   used, profiler timings vs XV-J.

---

## 3. Non-goals

- Per-pixel sample-count LOD / `close` flag in `terrain_material.wgsl`
  (explicitly forbidden; see XV-Zeta).
- Replacing the chunk mesh / CPU stitch LOD with geometry clipmaps (Losasso &
  Hoppe). 25C morph is enough geometry LOD for this phase.
- Hardware sparse residency / Vulkan `SPARSE_RESIDENCY` (wgpu does not expose a
  portable path; software clipmap + indirection is the WebGPU-native answer).
- UE Virtual Heightfield Mesh, Nanite landscape, or World Partition streaming.
- Baking POM *view rays* into the clipmap (view-dependent; wrong from any other
  angle).
- Retuning Great Lakes water, biome v3, snow `relief * 0.48`, or unique-colour
  lerp as a “performance trick”.
- Foliage LOD changes (signed off 2026-08-14).
- Copying O3DE `TerrainClipmapManager.cpp` or UE `VirtualTextureMaterial.usf`.

---

## 4. Why the current path does not scale

Live contract: [`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md).
Shader: `crates/somnium_renderer/src/shaders/terrain_material.wgsl`.

Every visible terrain pixel currently:

1. Samples **8 splatmaps**.
2. Picks **strongest four** of 32 weights.
3. Optionally hex-tiles each layer (**6** `textureSampleGrad` vs **2**).
4. Optionally POM-marches the dominant height (**~24** steps) plus self-shadow.

XV-J (RTX 5080 Laptop, Vulkan, 1280×720, release):

| View | Shading |
|---|---:|
| Overview (~150 m up, hex/POM off) | **3.951 ms** |
| Walk / eye (1.7 m AGL, hex on) | **5.532 ms** |
| Forest close (hex+POM) | **8.036 ms** |
| Plan budget | **1.10 ms** (explicit miss) |

FSR reduces *how many* pixels run this shader; it does not reduce *work per
terrain pixel*. A 4K window at Native still pays forest-close cost on a huge
fraction of the screen. Open-world cameras (horizon + feet in one frame) are
the worst case: feet need XV quality, horizon currently still almost does too
until the 80 m AGL uniform kicks in — and 80 m AGL is “flying”, not “standing
on a ridge looking 400 m”.

**Unique colour** (`macro_map`, 512²) already exists and is the coarsest
“clipmap”. Daggerfall is the missing *detail* stack between that and the tiled
arrays.

---

## 5. Repository and literature audit

Verified against local trees and public papers on 2026-08-14. No engine code
was copied.

### 5.1 Already in Somnium (do not rebuild)

| Piece | Where | Use in DF |
|---|---|---|
| Strongest-four + height-blend | `terrain_material.wgsl` | **Generator** for clipmap pages |
| Hex-tiling | `hextile.wgsl` | Bake into finest ring (view-independent) |
| POM in world XZ metres | `terrain_parallax_*` | Keep for near; march **clipmap height** |
| Unique-colour macro 512² | `terrain/macro_map.rs` | Coarsest colour; do not replace, composite |
| Aerial uniform hex/POM cut | `gpu_material_for_camera` > 80 m | Keep; clipmaps handle *distance along ground* |
| Chunk LOD + stitch | `terrain/mesh.rs`, CDLOD-style ranges | Geometry stays |
| 25C vertex morph | vis instance `_padding` | Optional; default off |
| BC7 packs | `encode_terrain_bc7` | Clipmap *sources* stay compressed; clipmap *cache* is runtime |

### 5.2 O3DE Terrain clipmaps (primary architecture)

**Local:** `C:\Users\adhir\Downloads\GE\example_repo\o3de-development\o3de-development\Gems\Terrain\Code\Source\TerrainRenderer\TerrainClipmapManager.h`

- Nested stacks, same texel count per level, **scale base 2**, camera-centred.
- **Macro:** max radius 2048 m, finest **2 texels/m** (colour + large variation).
- **Detail:** max radius **256 m**, finest **2048 texels/m** (full PBR).
- Image size 512 / 1024 / 2048. Margins so updates are infrequent. Toroidal
  addressing; extra margin so adjacent logical texels are not bilinear-blended
  across a wrap seam.
- Clipmap *images*: macro color/normal; detail color, normal, height, roughness,
  specular F0, metalness, occlusion.
- Docs: expensive blend runs **once into the clipmap**, not every frame on every
  pixel. [O3DE Terrain World Renderer — Clipmap](https://www.docs.o3de.org/docs/user-guide/components/reference/terrain/world-renderer/)

Somnium mapping: we are dielectric terrain (metal dropped). Cache **albedo,
normal, height, roughness, AO**. Skip metalness/F0 or store constants.

### 5.3 Unreal Engine 5 — Runtime Virtual Texture

**Local:** `...\UnrealEngine-release\Engine\Shaders\Private\VirtualTextureMaterial.usf`  
**Docs:** [Runtime Virtual Texturing](https://dev.epicgames.com/documentation/unreal-engine/runtime-virtual-texturing-in-unreal-engine)

Pattern to take, not the page table:

- Graph split: **camera-independent** layer blend → RVT output; **camera-
  dependent** (or cheap) work samples the RVT.
- Landscape with many layers is the textbook client; BasePass complexity drops
  because blend is cached.
- Optional: stream **low mips** (SVT) and keep **high mips** runtime (hybrid).
- Virtual Heightfield Mesh is a *geometry* consumer of height RVT — out of DF
  v1.

Somnium has no material graph. The “RVT output” is our clipmap generate compute
shader calling the same helpers as `evaluate_terrain_material` minus POM.

### 5.4 Far Cry 4 Adaptive Virtual Texture (Chen, GDC 2015)

[GDC Vault](https://www.gdcvault.com/play/1021761/). Summarized in ATVI 2023
notes: world in **64 m sectors**; each sector is a virtual image whose **mip
count scales with camera distance**; ~10 texels/cm near; 10 km world; 255 active
sectors + one default whole-world image.

**Too much machinery for a 1 km Great Lakes tile**, exactly right when DF-H
grows the world. Do not start here.

### 5.5 Call of Duty Super Terrain (Hooker / Etienne, 2021–2023)

[Boots on the Ground](https://research.activision.com/publications/2021/09/boots-on-the-ground--the-terrain-of-call-of-duty);
[Advances 2023 PDF](https://advances.realtimerendering.com/s2023/Etienne%28ATVI%29-Large%20Scale%20Terrain%20Rendering%20with%20notes%20%28Advances%202023%29.pdf).

Virtual texturing for artist blending; quadtree mesh; later titles moved toward
**one material index per vertex** plus color for distant. Somnium should **not**
drop to one-ID-per-vertex at the player’s feet (that is a fidelity cut). Distant
rings of the clipmap are allowed to look like a unique-colour + low-frequency
normal, which we already have.

### 5.6 Clipmaps and CDLOD (geometry — background only)

| Work | Takeaway for DF |
|---|---|
| Tanner et al. 1998, *The Clipmap* (SIGGRAPH) | Nested windows into a huge texture; toroidal update |
| Losasso & Hoppe 2004, *Geometry Clipmaps* | Nested **height** grids; not this phase’s mesh |
| Asirvatham & Hoppe, GPU Gems 2 | GPU clipmap as textures |
| Strugar 2010, CDLOD | Already cited for 25C; keep chunk meshes |

### 5.7 Sparse / MegaTexture

id Tech MegaTexture and Vulkan sparse residency are **not** the wgpu plan.
Software page cache + indirection (Toni Sagrista’s SVT write-up, 2023) is the
portable shape if we ever leave nested clipmaps for a full VT. DF v1 is
clipmaps, which *are* a specialized VT with an implicit page table (the ring
index).

### 5.8 Frostbite / Battlefield 3 (near-ground split)

**Widmark, GDC 2012** — [Terrain in Battlefield 3](https://media.contentapi.ea.com/content/dam/eacom/frostbite/files/gdc12-terrain-in-battlefield3.pdf).
**Andersson, SIGGRAPH 2007** — procedural shader splatting (masks in a sparse
quad-tree; compute instead of store).

BF3 is the closest published split to our fidelity constraint:

- Splat **diffuse / normal / specular / smoothness into a virtual texture**
  (they used clipmap *indirection*, 6×64² layers, ~**32 samples/m**). Full-screen
  sample of that cache was **2.5–3 ms on PS3**.
- Virtual texture has a practical ceiling; **detail shader splatting** then
  fills to **500–1000 samples/m**, typically limited to **50–100 m** view
  distance.
- Indirection updates dirty rects only; recentering wraps (same toroidal idea
  as O3DE).

Somnium mapping: O3DE nested images are simpler on wgpu than a page atlas +
indirection clipmap. The **BF3 lesson we must keep** is the two-rate shading:
cached composite everywhere, extra near-field only where the player can resolve
it — and that extra work is **POM on baked height**, not re-running hex×4 in
the pixel shader. Do not copy Frostbite shaders.

Decima / Horizon (van Muijden GDC 2017, GCAP visibility) is **procedural
placement and GPU scene queries**, not a material clipmap. Out of DF v1
(foliage LOD is already signed off).

### 5.9 Other local trees (skimmed, not adopted as the DF spine)

| Tree | Note |
|---|---|
| `CDLOD-master` | Morph already in 25C |
| `o3de-development` Gems/Terrain | **Spine** |
| `UnrealEngine-release` VT / Landscape | RVT split + docs |
| `terra-main` | Noise planets; not splat PBR |
| `godot-4.7.1-stable` / Terrain3D (web) | Paint UX only; XV already closed that |
| `bgfx-master` hextile | Already ported |
| `fyrox` terrain | Chunk mesh ancestor of Phase 14 |

---

## 6. Architecture (Somnium)

### 6.1 Two clipmap stacks

**Macro stack** (extends unique colour):

- Finest ~2–8 texels/m, radius kilometres.
- RGB albedo (unique colour + distant splat) + packed normal.
- Shading far from the camera: 1–2 samples.

**Detail stack** (the expensive one):

- Finest **≥ 512 texels/m** (match 2048² layer at 0.25 repeats/m). Stretch to
  O3DE’s 2048 texels/m only if memory allows.
- Radius **~128–256 m** (O3DE default 256 m).
- Layers: albedo RGB, packed tangent/world normal, height, roughness, AO.
- Format: `Rgba16Float` or BC-compressed if we generate then transcode; start
  with **RGBA8 packed** (albedo RGB+height A, normal XY + rough + AO) to match
  existing packs.

Each stack: **N rings**, same `clipmap_size` (start **1024**), scale base **2**,
toroidal origin stored in a uniform (`origin_xz`, `texels_per_meter` per ring).

### 6.2 Generate pass (compute in the plan; **fragment in the tree**)

Plan: one compute dispatch per dirty rectangle. **Shipped:** one fragment
draw per dirty rectangle into array-layer color attachments (see §11).
Do not put generate back in compute without a Vulkan Dbg-32 proof.

One pass per dirty rectangle (not the whole 1024² every frame; cap is
`MAX_GEN_TEXELS`).

For each texel in world XZ:

1. Unpack splat, strongest-four, height-blend — **same functions as today**.
2. Hex-tile if this ring’s texels/m is above a threshold (finest 1–2 rings).
3. Write packed G-buffer. **No POM. No view vector.**

Sculpt / paint / wetness: mark rings dirty (full refresh of affected world
AABB). Unique-colour lerp is a generate input, not a shading mix that fights
the cache.

### 6.3 Shading (`evaluate_terrain_material`)

**Forbidden:** `if (pixel_distance < X) { full blend } else { clipmap }` in the
**same** shader as hex/POM/full blend. That is the XV-Zeta footgun.

**Allowed:**

- **A. Uniform camera policy** (like aerial 80 m): when the camera is high,
  bind “clipmap-only” constants (already zeros hex/POM). Extend with “use
  detail stack”.
- **B. Two PSOs** (vis shading already has two-sided). `pipeline_terrain_clipmap`
  vs current. Split on a **uniform** (camera AGL or a debug toggle), not a
  per-pixel branch that still compiles both bodies.
- **C. Clipmap-only shading for all pixels**, with finest ring dense enough that
  walking *is* the XV look. POM becomes a march through **one** height clipmap.
  This is the preferred end state: one shader, cheap taps, fidelity in the
  cache.

Recommended ship order: **C for distance rings + POM-on-clipmap-height for the
finest ring** (still one program: POM loop bound by a **uniform** step count,
already how `parallax_steps` works). Hex cost moves to generate, not shade.

Cliff biplanar: still view/slope dependent. Keep current steep path when
`cliff_blend` is high **or** bake a second projected clipmap later (DF-F).
v1: cliffs may stay on the live path (small screen area).

### 6.4 Near-ground fidelity contract (acceptance)

Capture harness like XV-J (`SOMNIUM_TERRAIN_EYE=1`, forest, shore, walk):

| Metric | Gate |
|---|---|
| Eye-level mean luminance vs clipmap-off | **≤ 1%** (same spirit as 25H POM A/B) |
| Hex lattice / tiling | No new grid at feet; hex remains in **generate** |
| POM self-shadow on gravel/mud | Present at eye; may fade with existing `fade` |
| CIEDE2000 vs strongest-four reference (offline) | Reuse XV fixture on clipmap generate, not a new cheat |
| Walk shading ms | **≤** clipmap-off walk (must not regress) |
| Overview / ridge-looking-out shading ms | **Target ≤ 50%** of clipmap-off (stretch: approach 1.10 ms) |

If generate is too coarse, **raise finest texels/m**, do not skip hex at feet.

### 6.5 Memory (order of magnitude)

O3DE: 1024² × ~9 detail images × stack depth. Somnium packed 2 maps × 1024² ×
4 bytes × ~6 rings ≈ **48 MiB** for detail — small next to 213 MiB BC7 sources.
Budget **≤ 128 MiB** GPU for both stacks. Never keep RGBA8 source packs **and**
clipmaps if over the XV residency cap; sources stay BC7.

### 6.6 Interaction with FSR / jitter / vis buffer

- Generate in **world XZ**, not clip space — jitter does not apply.
- Shading still runs at **render res** (FSR input). Clipmap helps that pass.
- GTAO / ReSTIR read vis depth; clipmap must not change geometry.
- Hi-Z / cull unchanged.

### 6.7 Wetness / snow / unique colour

Generate reads the same uniforms as today (global wetness, snow cap). Changing
Wet in the inspector dirties the stack (or a cheap modulate in shading if it
stays a uniform multiply — prefer dirty if it alters blend weights).

---

## 7. Milestones

| ID | Work | Est. | Exit |
|---|---|---|---|
| **DF-A** | Measure at **maximized Native** + resolution sweep; FSR on/off; eye / overview / ridge-look. | 0.5 | **MEASURED** — [`phase DF/DF-A_timings.md`](phase%20DF/DF-A_timings.md). **Stale vs current look** (see §12); remeasure after audit. |
| **DF-B** | `TerrainClipmap` GPU images + uniforms; CPU toroidal origin; generate (no POM). Debug 32 albedo. | 2 | **IN ENGINE** — generate is a **fragment** pass (compute bindless wrote black). |
| **DF-C** | Shading samples detail stack; hex not in shade. Inspector Clipmap default **off**. | 2 | **IN ENGINE** — POM-on-clipmap-height **not** shipped (smear); see remaining. |
| **DF-D** | Incremental dirty rects; sculpt/paint invalidate; extended margin wrap. | 1 | **IN ENGINE** |
| **DF-E** | Macro stack; distance rings; profiler scope. Default on **after** maximized-window gates. | 1 | Macro **in engine**. Default **still off** (walk luminance gate). |
| **DF-F** | Cliffs stay live (biplanar in shade). | 1 | **IN ENGINE** |
| **DF-G** | Docs, ATTRIBUTION §1.8 as used, Help Terrain, tests. | 0.5 | **IN ENGINE** — this file + Help. `shaders_validate` + `terrain::clipmap` tests. |
| **DF-H** | Research spike only: AVT sectors / world > 2 km. **No ship** unless user expands the map. | 0 | Note in this file |

Kill switch: `SOMNIUM_TERRAIN_CLIPMAP=0` and inspector toggle.

---

## 8. Inspector / Help

Terrain Details:

- **Clipmap** toggle (default off until DF-E).
- **Detail m** finest texels/m (readout).
- **Dbg** extra modes: clipmap albedo, ring index, generate heat.

Help `docs/editor/terrain.md`: clipmaps cache blend; POM is **not** marched
on the cache (smear); aerial 80 m cut remains. Default off.

---

## 9. Must not do

1. Per-pixel material LOD branch in `evaluate_terrain_material`.
2. Retune water / biome / snow / unique-colour lerp to hide cost.
3. Drop hex at **eye level** to hit a timing number.
4. Put clipmap generate in the fragment shader of the vis pass.
5. Depend on Vulkan sparse images.
6. Change foliage LOD.
7. Copy O3DE/UE source; transliterate ideas into original WGSL/Rust.

---

## 10. Bibliography (cite in ATTRIBUTION as used)

1. O3DE `TerrainClipmapManager.h` / World Renderer clipmap docs (Apache-2.0 OR MIT).
2. Epic, *Runtime Virtual Texturing* (UE 5.x docs); `VirtualTextureMaterial.usf` (EULA — **study only**).
3. Ka Chen, *Adaptive Virtual Texture Rendering in Far Cry 4*, GDC 2015.
4. JT Hooker, *Boots on the Ground: The Terrain of Call of Duty*, Treyarch 2021.
5. Etienne / ATVI, *Large Scale Terrain Rendering*, Advances in Real-Time Rendering 2023.
6. Tanner, Migdal, Jones, *The Clipmap*, SIGGRAPH 1998.
7. Losasso & Hoppe, *Geometry Clipmaps*, SIGGRAPH 2004.
8. Filip Strugar, *CDLOD*, 2010 (already §13.20 / 25C).
9. Heitz & Neyret hex-tiling / bgfx 49-hextile (already 25F).
10. XV-Zeta + XV-J compile gate (Somnium first-party).
11. Mattias Widmark, *Terrain in Battlefield 3*, GDC 2012 (Frostbite 2 VT +
    50–100 m detail splat).
12. Johan Andersson, *Terrain Rendering in Frostbite using Procedural Shader
    Splatting*, SIGGRAPH 2007.

---

## 11. As shipped (2026-08-14) — divergences from §6

Intent in §6 still stands. The tree does **not** match every sentence:

| Plan | Shipped |
|---|---|
| Generate **compute** | **Fragment** MRT (albedo + surface). Compute `textureSampleGrad` on the bindless layer array wrote **black** (Dbg 32 silhouettes) even after storage copies. UE5 RVT / this engine’s G-buffer pattern. |
| Camera-centred stacks | **Look-at** XZ, clamped to **8 m**, snapped to 0.5 m. Looking down keeps the centre under the camera. |
| Shade `textureSampleGrad` + aniso Repeat | **Toroidal bilinear `textureLoad`**. Repeat + anisotropy smeared the wrap into streak bands. |
| Square Chebyshev ring pick | **Circle** contains + **256-texel** blend to the next ring. |
| POM on baked clipmap height | **Off.** Marching `world_xz` off the ring / across wrap smeared UVs. |
| Finest-first generate | Ring **3** first (8 m at 64 t/m), then 2→0 (sharpen), then 4–7, then macro. Shade **skips unready** rings (`clipmap_detail_ready` / `clipmap_macro_ready`). |
| Full-stack generate | **`MAX_GEN_TEXELS` = 1M** (one 1024² / frame). More hitchs. Cheap coarse rings in the same frame as a hex ring hitchs. |
| `GpuTerrainMaterial` | **2032** bytes. Ready bitmasks occupy the old `_clipmap_pad`. XV 1664-byte **body** is unchanged; DF fields follow. |

Files: `terrain/clipmap.rs`, `pass/terrain_clipmap.rs`, `shaders/clipmap_gen.wgsl`, `shaders/clipmap_shade.wgsl`, `shaders/terrain_material.wgsl`, `pass/shading.rs` group 2, `renderer.rs` generate-before-shade.

## 12. Audit (required)

**Do not treat clipmap as default-on or “done” until this audit runs.** The look
and hitch work landed in one session; a different model (Claude Opus 5) must
read the architecture and defect-hunt before anyone spends more implementation
time “making clipmap run better.”

**This is a read-only audit unless the user asks for fixes.** Frozen: Great
Lakes water (datum 16.1 m, optical max_depth 18.6 m, Gerstner `wave_speed`
0.85), XV look, foliage LOD, no per-pixel live/clipmap mix (XV-Zeta).

### 12.1 Read first

1. This file (especially §3 non-goals, §6.3 forbidden branch, §6.4 gates, §9, §11).
2. [`phase DF/DF-A_timings.md`](phase%20DF/DF-A_timings.md) — maximized Native
   numbers. **Walk luminance +35.6% is from before the fragment/look/sampling
   rewrite.** Do not cite it as the current look; remeasure if the user wants
   gates.
3. Help [`docs/editor/terrain.md`](../docs/editor/terrain.md).
4. Then the files in §11.

### 12.2 What to hunt

- Generate vs shade UV (framebuffer Y, toroidal `origin`, `textureLoad` wrap).
- Ready-bit vs dirty vs `fill_gpu` order (same-frame generate then shade).
- 1M texel cap vs look-at slides vs `mark_full` while flying (~150 m/s).
- Ring-3-first + unready skip: holes, popping, sampling a ring whose centre
  has not slid yet.
- Group 2 array views vs layer `RENDER_ATTACHMENT` views of the **same**
  textures (the storage/copy path was a dead end; do not reintroduce it).
- Hitch sources: more than one 1024² hex pass, per-frame bind-group create,
  `MAX_CHEAP_GEN_TEXELS` (removed).
- Default-off is correct until §6.4 eye luminance ≤ 1% **and** walk shading
  does not regress, measured at **maximized Native**.

### 12.3 Remaining DF work (after the audit, not instead of it)

| Item | Status |
|---|---|
| **Audit** | **Required next.** Not started. |
| **DF-E default-on** | Blocked on §6.4 gates at maximized Native with the **current** look. |
| **POM on clipmap height** | Planned (§1 / DF-C). **Off** — smear. Revisit only if the audit says the cache UV is stable. |
| **CIEDE2000 vs strongest-four** | Gate in §6.4; no offline fixture run on generate. |
| **Dbg generate heat** | Listed in §8; not shipped. Dbg 32 albedo / 33 ring index exist. |
| **DF-H AVT / > 2 km** | Research only if the map grows. Not v1. |

Do **not**: per-pixel live/clipmap mix; drop hex at feet to chase luminance;
put generate back in compute without a Vulkan Dbg-32 proof; raise
`MAX_GEN_TEXELS` above 1M; retune water / foliage / unique-colour.

## 13. Next-session start

1. If the user asked for a **clipmap audit**: this file §12. Do not implement
   “improvements” until the audit is delivered unless they explicitly redirect.
2. If the user asked to **turn Clipmap default on**: refuse until DF-E gates
   are remeasured at maximized Native.
3. Frozen: Great Lakes water, XV look, no per-pixel sample LOD, rustc 1.88.
