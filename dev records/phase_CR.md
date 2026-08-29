# Phase CR — Crysis

> *“Can it run Crysis” was always “spend the hardware you already bought.”*
> Somnium’s problem was the inverse: Task Manager **1.5% CPU** while shading
> still cost **~50 ms** at maximized Native. The GPU was busy; other cores were
> idle because the CPU work was serial — and a lot of it was setting up terrain
> the camera could not see.

> **Codename:** Crysis (2007)  
> **Status:** IN ENGINE (2026-08-14) — CR-A–G in the tree. CPU frustum **default
> on**. GPU 15B stays **F10**.  
> **Record:** this file + [`phase CR/CR-A_occupancy.md`](phase%20CR/CR-A_occupancy.md)  
> **Do not copy source** from UE5 / CryEngine / Frostbite. Patterns cited in
> `ATTRIBUTION.md` §1.9.

Frozen (unchanged by this phase): water datum 16.1 m / optical 18.6 m / Gerstner
0.85; XV 32-layer look; foliage LOD; rustc 1.88; wgpu 29 **single queue**; no
per-pixel sample LOD; clipmap default **off**.

## 1. What already existed

GPU instance frustum + Hi-Z is **Phase 15B**, default on, A/B on **F10**.
Terrain chunks already have AABBs. Shadows **intentionally** ignore 15B
(off-screen casters still shadow into view). `select_lods` was distance-only.
CPU still enqueued all 256 chunks on the default tile.

Foliage already CPU-culls via `cull_distance` because “the GPU cull cannot do
this, because a draw has to exist before it can be rejected.”

`rayon` was a workspace dep; only voxel meshing used it.

## 2. What shipped

| ID | Work | Exit |
|---|---|---|
| **CR-A** | Occupancy truth vs DF-A + Task Manager | [`phase CR/CR-A_occupancy.md`](phase%20CR/CR-A_occupancy.md): GPU-bound |
| **CR-B** | CPU AABB frustum early-out for terrain vis | Off-screen chunks absent from `draw_queue`. Tests: behind-camera / straddling |
| **CR-C** | Camera Details **Frustum Cull** + `SOMNIUM_CPU_FRUSTUM=0` | Toggle in Details; default on. F10 = GPU 15B |
| **CR-D** | Job pool | `jobs.rs` rayon at **512+** items. 256-chunk default stays serial (CR-A) |
| **CR-E** | Cascade-volume shadow caster cull | Never camera-only. `shadow_only_queue` keeps off-screen ground that hits a cascade. `SOMNIUM_CASCADE_CULL=0` |
| **CR-F** | Persistent CPU buffers | `draw_queue` / `cull_aabbs` / `shadow_caster_scratch` / `rebuilt_chunks` keep capacity. No extra clipmap RAM |
| **CR-G** | Docs, ATTRIBUTION, Help, tests | This file |

### Vis vs shadows

CPU camera frustum skips **visibility** (`draw_queue`). A rejected chunk that
still overlaps a cascade is pushed to `shadow_only_queue` and occupies instance
slots **after** the vis draws, so the shadow pass can find its transform.
GPU 15B still runs on vis draws only.

```text
camera frustum ──► draw_queue ──► GPU 15B + Hi-Z ──► vis
                 ↘ shadow_only ──► cascade AABB test ──► shadow atlas
draw_queue ─────────────────────► cascade AABB test ──► shadow atlas
```

## 3. Inspector

There is no fly-cam component. **Camera** is a singleton entity (same pattern as
Post Processing) with **Frustum Cull**, default on. Physical Camera (aperture /
shutter / ISO) stays on Post Processing.

## 4. Non-goals (held)

- wgpu multi-queue / async compute
- “Use 100% of all cores” or RAM without a measured cache
- Per-pixel live/clipmap mix; DF clipmap audit
- Retune Great Lakes water, XV look, foliage LOD
- Nanite / geometry clipmaps / rewriting 25C morph
- Copying UE5 / CryEngine / Frostbite source
- Camera-frustum killing shadow casters

## 5. Tests

- `culling.rs`: behind-camera chunk culled; straddling kept; `aabb_in_any_frustum`; default landscape look-away drops vis
- `jobs.rs`: serial path; parallel path matches serial above the threshold

Profiler **terrain chunks** shows `vis / cpu-cull`. `[off]` means the Camera checkbox is clear; `[forced-off]` means `SOMNIUM_CPU_FRUSTUM=0`. The default coastal vista often keeps all 256 on a wide window — hold RMB and look at empty sky.
