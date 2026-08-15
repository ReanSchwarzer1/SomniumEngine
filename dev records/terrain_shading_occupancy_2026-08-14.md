# Terrain shading occupancy — 2026-08-14

> Session notes for the next model. **Not** an invitation to retune Great Lakes
> water, shrink the 32-layer GPU format, or turn Clipmap on as the Coastal
> default. Help: [`docs/editor/terrain.md`](../docs/editor/terrain.md),
> [`docs/editor/lighting.md`](../docs/editor/lighting.md). Start-here:
> [`post_halcyon_audit_handoff.md`](post_halcyon_audit_handoff.md).

**Worktree:** this landed in the same window as Maps (Coastal / Island). Check
`git status` — the compact shading PSO and related WGSL may still be
uncommitted.

---

## Frozen (do not change)

| Item | Value |
|---|---|
| Water | datum **16.1 m**, optical `max_depth` **18.6 m**, Gerstner `wave_speed` **0.85**. Island uses `WaterComponent::ocean` with the **same look numbers**. Do not retune Great Lakes / XV water to “match Coastal.” |
| GPU splat format | **32 layers** / 8 splatmaps, `GpuTerrainMaterial` **2032** bytes. Do **not** shrink `TERRAIN_LAYER_COUNT`. Island leaves slots 16–31 unpublished; the struct size stays 32. |
| Per-pixel sample-count LOD | **Forbidden.** `close` / `use_maps` / `layer_budget` compiled three paths into one shader and walking went 20→27 ms (XV-Zeta §11.1). |
| Clipmap | In engine, inspector default **off**. Cheap shade path for a 1 km tile. Do **not** enable it as the Coastal default. Audit is [`phase_DF.md`](phase_DF.md) §12. |

---

## Maps (look signed off)

Recipes: `assets/Maps/Coastal.somnium`, `assets/Maps/Island.somnium`. Content
Drawer: **Game / Maps**, double-click to load.

| | Island | Coastal |
|---|---|---|
| Tile | 512 m, 8×8 chunks (64) | 1024 m, 16×16 chunks (256) |
| GPU layers | hero bank **16**, splatmaps 4–7 unbound, layers 16–31 not published (`hero_bank_only`) | **32** layers, 8 splatmaps, extra bank bound |
| Terrain options | Hex **off**, Parallax **off** | Hex / Parallax as XV walking defaults |
| Water | `WaterComponent::ocean`, same optical freeze | Great Lakes |
| Camera | `(0, 28, 115)` | XV overview / walk |

Island GPU budget: `create_terrain_hero_bank` + `apply_hero_bank_gpu_budget`.
User: **“perfect yeah works”** for the island look. Do not retune the recipe
for fps.

---

## Measurements (Island, terrain selected, before compact PSO)

Approximate, same machine as XV-J (RTX 5080 Laptop, Vulkan):

- Island ~**20 fps**, Coastal ~**18 fps** — looked like the same cliff.
- Profiler: **Shading ~41.2 ms** of ~**52.8 ms** total. Water ~**3.6 ms**.
  Geometry / vis reconstruct were cheap on Island.

After compact PSO (Island starts on `ShadingSpec::COMPACT`): Island **30+ fps**.

Coastal with Hex / Parallax / Soft Shadows **unchecked**: still ~**20 fps on
the ground**. Same compact *lighting* shader, different scene (table above).

---

## What did not move the needle

Runtime uniforms do **not** delete WGSL from the compiled shading shader.
Naga/DXC occupancy stays at the **union of every path** that is still in the
module.

1. **Parallax checkbox** — ~2–3 fps, likely noise. CPU already wrote
   `parallax_scale = 0` / `parallax_steps = 0`, but the shader gated POM on a
   **per-pixel** `allow_pom && fade` test, so DXC flattened the march.
2. Uniform POM skip (`if tm.parallax_steps >= 4` first; zero
   `parallax_shadow_steps` when scale is 0) — **still no drop**. Necessary so
   a future compact PSO is honest; not sufficient by itself.
3. **Soft Shadows** (this is **PCSS**; there is no “PCSS” label) and
   **Contact Shadows** — no drop. ReSTIR DI already writes sun vis
   (`alpha = 1` every pixel). Shading skipped PCSS with `if traced.a > 0.5`,
   a **varying texture test**; DXC flattened it so the 16+24 PCSS filter +
   12-step contact march still ran.
4. Uniform `shading_mode` **bit 4** when `restir_pass.active() && tlas().is_some()`
   — **still no drop**. Same story: uniform skip is necessary; occupancy is
   compile-time.

**Do not spend another session flipping those checkboxes expecting Shading ms
to fall.** If they are already off and the compact PSO is live, the remaining
cost is scene size / layer count / pixels that hit terrain.

---

## What did work

**Pipeline overrides** that actually delete unused code:
`ShadingSpec` + `ShadingPass::ensure_pipeline`.

WGSL `override`s (defaults in the shader source are the *full* path; compact
PSO writes zeros):

| Override | Compact | Full |
|---|---|---|
| `enable_hex` | false | live terrain `hex_tiling` |
| `enable_pom` | false | `parallax_scale > 0` |
| `enable_pcss` | false | Soft Shadows **and** not ReSTIR sun vis |
| `enable_contact` | false | Contact Shadows **and** not ReSTIR sun vis |
| `enable_clipmap` | false | any queued terrain has clipmap enabled |
| `enable_debug` | false | `shading_debug != 0` |
| `terrain_scan` | **16** | **32** if any queued terrain is not `hero_bank_only` |

Island (hex off, POM off, 16-layer scan) stays on `ShadingSpec::COMPACT` and
never pays the recreate hitch.

Also in that PSO work (correctness / occupancy, not a second feature):

- Analytic UV neighbour barycentrics sit **inside** `if analytic_grad` (they
  used to always run).
- Debug views gated by `enable_debug` via
  `let dbg = select(0.0, light._pad2_z, enable_debug)`.

---

## Why Coastal stays slower

Same compact lighting shader once Hex / Parallax / Soft Shadows are off.
Different occupancy **inside** `evaluate_terrain_material` plus a bigger vis
buffer:

- `terrain_scan` is **32** because `hero_bank_only` is false (Coastal publishes
  the extra bank).
- Almost every pixel is ground. Island is a small islet + water/sky (water is
  ~3.6 ms, not 40 ms).
- Vis reconstruct + shadow maps walk **4×** the chunks (256 vs 64).

If the profiler now shows Shading near Island and **Vis / Shadow** are the extra
~15 ms, that is the 256-chunk tile, not leftover POM.

The designed cheap shade path for a big landscape is **Clipmap** (Terrain
details). Keep the default **off**. Do not flip it on to “win” Coastal fps
inside the Halcyon audit.

---

## Code (verify; lines drift)

| Area | Path |
|---|---|
| Compact PSO | `crates/somnium_renderer/src/pass/shading.rs` (`ShadingSpec`, `ensure_pipeline`) |
| Spec from live terrain | `crates/somnium_renderer/src/renderer.rs` (before the Shading pass) |
| Terrain material | `crates/somnium_renderer/src/shaders/terrain_material.wgsl` |
| Lighting / PCSS / ReSTIR vis | `crates/somnium_renderer/src/shaders/shading.wgsl` |
| `shading_mode` bit 4 | `global_pool.wgsl`, `cluster.rs`, `renderer.rs` |
| Uniform POM / hex skip | `terrain_material.wgsl`; `terrain/mod.rs` `gpu_material()` |
| Island budget | `create_terrain_hero_bank`, `apply_hero_bank_gpu_budget`, `hero_bank_only` |
| UI | `crates/somnium_ui/src/lib.rs` — **Soft Shadows** = PCSS; **Hex Tiling** / **Parallax** / **Clipmap** on Terrain details |

`shading_mode` bits (do not renumber):

| Bit | Meaning |
|---|---|
| 0 | Cel |
| 1 | PCSS (inspector **Soft Shadows**) |
| 2 | Contact Shadows |
| 3 | Analytic Mips |
| 4 | ReSTIR already wrote sun vis — sample `restir_vis.r`, do not `sample_shadow` |

---

## What the next session should not do

- Do not treat unchecking Hex / Parallax / Soft Shadows as a shading-cost fix.
  The compact PSO already does that when they are off.
- Do not reintroduce per-pixel `close` / `use_maps` / `layer_budget`.
- Do not shrink 32-layer GPU format so Coastal “becomes Island.”
- Do not enable Clipmap as the Coastal default.
- Do not retune water numbers.
- Do not change Island recipe look (signed off).
- Occupancy remaining on Coastal is a **Daggerfall / tile-size** problem, or a
  second *aerial* PSO that drops unique-colour / two-layer maps **without** a
  per-pixel sample-count branch — not another uniform gate.
