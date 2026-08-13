# Somnium Engine — Project Context

> **Last updated:** 2026-08-13 evening  
> **Current phase:** Phase IV complete (IV-A through IV-K); Phase XV (Appalachia) **XV-A–J complete** (1.10 ms shading remains an explicit exception; BC7 encoder + local packs 2026-08-13); Phase 26 (Metaphor) **26-A–I shipped, phase remains open** (immersive play, ComboBox overlay, drawer tiles; 26-J not started); Phase VV (Halcyon) **VV-A–H in tree** (RT water reflections; live miss-rate capture still open)  
> **Start-here:** `dev records/halcyon_context_handoff.md`  
> **Toolchain:** Rust 1.85, wgpu 29, winit 0.30  
>
> Phase IV-K, the ocean fidelity pass against
> [GodotOceanWaves](https://github.com/2Retr0/GodotOceanWaves), closed on
> 2026-08-13. The three-cascade 1024² FFT, the Jacobian whitecap model, and the
> GDC 2019 *Atlas* surface lighting all ship; the record of what was
> implemented, what was deliberately deferred, and where Somnium departs from
> the reference is in `dev records/phase_IV.md` section 14. Two IV-K items were
> **not** delivered and are stated there rather than buried: the ocean clipmap
> body kind (K-1) and the HDRI/Filmic environment (K-7) were deferred, and GPU
> sea spray (K-6) was abandoned after two failed emitters.
>
> **Phase XV live contract** (do not silently retune): 32 global layers /
> strongest-four local; sidecar v4; `GpuTerrainMaterial` 1664 bytes; unique
> colour from splat (512²); biome v3 / landscape v4; snow cap `relief * 0.48`;
> aerial hex/POM off when the camera is > 80 m above the heightfield
> (`gpu_material_for_camera`). Do not reintroduce a per-pixel terrain
> sample-count LOD. Water: `WaterComponent::great_lakes` stays frozen.
> BC7: `encode_terrain_bc7` writes gitignored `assets/terrain/bc7/`; runtime
> loads them when complete (~213 MiB hero 2048 + extra 1024).
> `SOMNIUM_TERRAIN_FORCE_RGBA8=1` for A/B. Canonical write-up:
> `dev records/phase XV/XV-Zeta_plan.md`.
>
> Remaining work (independent tracks):
> - **Phase VV — Halcyon** — VV-A–H in tree (ray-traced water reflections).
>   **Start-here:** `dev records/halcyon_context_handoff.md`. Plan:
>   `dev records/phase_VV.md`. Kill switch `SOMNIUM_RT_REFLECT=0`. Live SSR
>   miss-rate capture still open. Do not re-implement A–H.
> - **Phase 26 — Metaphor** — 26-A–I plus the 2026-08-13 UX polish (including
>   immersive play, ComboBox overlay, 80 px drawer tiles) are in the tree.
>   **The UI phase is not closed:** later engine work keeps needing inspector
>   fields, menus, drawers, and Help pages. Queued: 26-J reflection inspector
>   (only if requested), 26-H SDF text, 26-D2 drag-drop. Contract:
>   `dev records/phase_26.md`. Do not restart at 26-A inside a Halcyon session.

---

## Table of Contents
2. [Repository Layout](#2-repository-layout)
3. [High-Level Architecture](#3-high-level-architecture)
4. [Crate Dependency Graph](#4-crate-dependency-graph)
5. [somnium_core — Lifecycle & Events](#5-somnium_core--lifecycle--events)
6. [somnium_renderer — Visibility Buffer Pipeline](#6-somnium_renderer--visibility-buffer-pipeline)
7. [somnium_ecs — Entity Component System](#7-somnium_ecs--entity-component-system)
8. [somnium_ui — Native Editor UI](#8-somnium_ui--native-editor-ui-phase-12--metaphor-chrome-still-growing)
9. [somnium_physics — Jolt Integration](#9-somnium_physics--jolt-integration)
10. [somnium_audio — Kira Integration](#10-somnium_audio--kira-integration)
11. [somnium_asset — Asset Pipeline](#11-somnium_asset--asset-pipeline)
12. [Frame Execution Order](#12-frame-execution-order)
13. [GPU Buffer & Shader Data Layout](#13-gpu-buffer--shader-data-layout)
14. [Rendering Passes — Detailed](#14-rendering-passes--detailed)
15. [Camera System](#15-camera-system)
16. [UI Messaging — Direct Rust API](#16-ui-messaging--direct-rust-api-phase-12-complete)
17. [Phase History & Roadmap](#17-phase-history--roadmap)
18. [Known Issues & Active Bugs](#18-known-issues--active-bugs)
19. [somnium_voxel — Voxel World](#19-somnium_voxel--voxel-world-phase-14-complete)
20. [Heightmap Terrain System](#20-heightmap-terrain-system-phase-14-sss-complete)
21. [Phase 15 — GPU-Driven Rendering](#21-phase-15--gpu-driven-rendering-plan--progress)

---

## 1. Project Summary

**Somnium Engine** is a from-scratch 3D game engine built in Rust targeting desktop platforms (Windows primary, Linux/macOS secondary via wgpu backends). It is designed from first principles around three architectural commitments:

| Commitment | Consequence |
|---|---|
| **Visibility Buffer rendering** | No overdraw bandwidth penalty; deferred shading reads only live pixels |
| **Archetype ECS** | Cache-coherent component iteration; no hash-map per-component lookup |
| **Native UI** | Editor chrome rendered by wgpu UiPass (no wry WebView dependency) |

The engine is **not** a wrapper around Unity or Unreal. It studies those codebases (in `example_repo/`) for architectural patterns only — no source code is reused.

---

## 2. Repository Layout

```
GE/
├── Cargo.toml                  Workspace root; all version pins live here
├── context.md                  This file — living architecture document
├── ATTRIBUTION.md              Reference-architecture provenance
│
├── assets/                     Runtime assets (not compiled into the binary)
│   ├── LICENSE.md              Asset license and attribution
│   └── test_scene.glb          glTF 2.0 test scene (DamagedHelmet or similar; see assets/LICENSE.md)
│
├── crates/
│   ├── somnium_core/           App lifecycle, events, timing, config, ECS re-exports
│   ├── somnium_renderer/       wgpu backend, Visibility Buffer, shading passes
│   ├── somnium_ecs/            Archetype ECS (no external deps)
│   ├── somnium_ui/             Native wgpu widget tree, Nocturne editor chrome, UiPass
│   ├── somnium_physics/        Jolt Physics high-level wrapper
│   ├── somnium_physics_sys/    Raw FFI bindings to libjolt
│   ├── somnium_audio/          Kira audio engine wrapper
│   ├── somnium_asset/          Vertex type, glTF 2.0 loader (load_gltf → LoadedScene)
│   └── somnium_voxel/          Voxel world: chunks, block_mesh meshing, async gen, LOD (Phase 14)
│
├── examples/
│   └── hello_engine/           Runnable demo: glTF scene (falls back to procedural cubes)
│
└── example_repo/               Reference codebases (NOT compiled, excluded from workspace)
    ├── UnrealEngine-release/
    ├── The-Forge-master/
    ├── DirectXShaderCompiler-main/
    ├── swiftshader-master/
    └── ...
```

---

## 3. High-Level Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│                          User Game Code                              │
│                    impl GameApp for HelloGame                        │
│   on_init()  on_event()  on_update()  on_render()  on_shutdown()     │
└────────────────────────────┬─────────────────────────────────────────┘
                             │  EngineContext<'a>  (zero-copy borrows)
┌────────────────────────────▼─────────────────────────────────────────┐
│                   somnium_core :: Engine<G>                          │
│           winit ApplicationHandler  ─  state machine                │
│  Uninitialized → Running → Suspended → ShuttingDown                  │
├──────────┬──────────────┬──────────────┬──────────────┬─────────────┤
│  TimeState│  EngineConfig│  ECS World   │ PhysicsWorld │ AudioEngine │
│  (timing) │  (config)    │  (archetypes)│ (Jolt FFI)   │ (Kira)      │
└──────────┴──────────────┴──────────────┴──────────────┴─────────────┘
                 │                              │
   ┌─────────────▼──────────────┐   ┌──────────▼───────────────────┐
   │   somnium_renderer          │   │   somnium_ui :: UiManager    │
   │   SomniumRenderer           │   │   native wgpu widget tree    │
   │  ┌──────────────────────┐   │   │  ┌────────────────────────┐  │
   │  │  VisibilityBufferPass│   │   │  │ Grid: menu bar 28 px   │  │
   │  │  (Pass 1: R32Uint)   │   │   │  ├────────────────────────┤  │
   │  ├──────────────────────┤   │   │  │ toolbar│viewport│panel │  │
   │  │  ShadingPass         │   │   │  │ 40 px  │stretch │280 px│  │
   │  │  (Pass 2: PBR+Sky)   │   │   │  ├────────────────────────┤  │
   │  ├──────────────────────┤   │   │  │ Output Log 192 px      │  │
   │  │  GlobalResourcePool  │   │   │  └────────────────────────┘  │
   │  │  (bindless arrays)   │   │   │   UiPass (wgpu, alpha blend) │
   │  ├──────────────────────┤   │   └──────────────────────────────┘
   │  │  GeometryPool  64 MB │   │
   │  │  InstancePool  16 MB │   │
   │  │  MaterialPool        │   │
   │  │  TexturePool         │   │
   │  └──────────────────────┘   │
   └─────────────────────────────┘
                 │
   ┌─────────────▼──────────────┐
   │   wgpu (Vulkan/DX12/Metal) │
   │   GPU hardware             │
   └────────────────────────────┘
```

---

## 4. Crate Dependency Graph

```
hello_engine
  ├── somnium_voxel              (chunk streaming; no GPU deps)
  │     ├── somnium_asset        (Vertex type)
  │     ├── block-mesh 0.2      (visible_block_faces face culling)
  │     ├── ndshape 0.3         (linearized 3-D array indexing)
  │     └── rayon               (chunk generation worker pool)
  └── somnium_core
        ├── somnium_ecs          (no external deps beyond std)
        ├── somnium_renderer
        │     ├── somnium_asset  (Vertex type, future glTF)
        │     └── wgpu 29
        ├── somnium_ui
        │     └── fontdue 0.7  (glyph atlas)
        ├── somnium_physics
        │     └── somnium_physics_sys  (cc FFI → libjolt)
        └── somnium_audio
              └── kira 0.9
```

**Key workspace-level version pins** (`Cargo.toml`):

| Dependency | Version | Role |
|---|---|---|
| `wgpu` | 29.0 | GPU abstraction (Vulkan/DX12/Metal/WebGPU) |
| `winit` | 0.30 | Window creation & input events |
| `glam` | 0.29 | SIMD math (Vec3, Mat4, Quat) |
| `bytemuck` | 1.21 | Safe byte casting for GPU uploads |
| `fontdue` | 0.7 | CPU glyph rasterizer for native UI font atlas |
| `kira` | 0.9 | Audio engine |
| `gltf` | 1 (names, utils, images) | glTF 2.0 / GLB asset loading |
| `rayon` | 1.12 | Voxel chunk generation workers (Phase 14); parallel ECS (future) |
| `block-mesh` | 0.2 | Voxel face-culling mesher (Phase 14) |
| `ndshape` | 0.3 | Linearized 3-D voxel array indexing (Phase 14) |
| `serde` / `serde_json` | 1.0 | Scene serialization (`.somnium` JSON format) |

---

## 5. somnium_core — Lifecycle & Events

### 5.1 Application State Machine

`Engine<G>` implements `winit::application::ApplicationHandler`. The lifecycle is a four-state machine:

```
               winit::EventLoop::run_app()
                        │
                        ▼
              ┌─────────────────┐
              │  Uninitialized  │
              └────────┬────────┘
                       │ resumed() — first call
                       │  create window
                       │  RenderContext::new()
                       │  SomniumRenderer::new()
                       │  UiManager::new()
                       │  game.on_init()
                       ▼
              ┌─────────────────┐◄────────────────────────────┐
              │    Running      │                             │
              │                 │  about_to_wait() per frame  │
              │  ┌───────────┐  │◄────────────────────────────┘
              │  │time.tick()│  │
              │  │physics    │  │
              │  │on_update  │  │
              │  │begin_frame│  │
              │  │on_render  │  │
              │  │r.render() │  │
              │  └───────────┘  │
              └────────┬────────┘
                       │ suspended() / close requested
                       ▼
              ┌─────────────────┐
              │   Suspended     │
              └────────┬────────┘
                       │ resumed() again
                       ▼ (back to Running)
              ┌─────────────────┐
              │  ShuttingDown   │
              └─────────────────┘
```

### 5.2 EngineContext

`EngineContext<'a>` is a single-frame borrow bundle passed to every `GameApp` callback. It holds zero-cost borrows (no Arc, no clone) of:

| Field | Type | Mutable? | Description |
|---|---|---|---|
| `time` | `&TimeState` | no | Frame timing, FPS, dt |
| `config` | `&EngineConfig` | no | Window/FPS config |
| `world` | `&mut World` | yes | ECS world |
| `physics` | `&mut PhysicsWorld` | yes | Physics simulation |
| `audio` | `&mut AudioEngine` | yes | Audio playback |
| `render_ctx` | `Option<&RenderContext>` | no | Raw wgpu device/queue/surface |
| `renderer` | `Option<&mut SomniumRenderer>` | yes | High-level draw API |
| `selected_entity` | `&mut Option<Entity>` | yes | Editor selection state |
| `ui` | `&mut UiManager` | yes | IPC send/recv |
| `should_exit` | `bool` | yes | Set to request shutdown |

### 5.3 Event Translation

Raw `winit::WindowEvent` is translated to `EngineEvent` by `translate_window_event()`. Game code never imports `winit` directly.

```
WindowEvent::KeyboardInput  →  EngineEvent::KeyInput { key: KeyCode, state: InputState }
WindowEvent::MouseInput     →  EngineEvent::MouseButton { button, state }
WindowEvent::Resized        →  EngineEvent::WindowResized { width, height }
WindowEvent::CloseRequested →  EngineEvent::WindowCloseRequested
DeviceEvent::MouseMotion    →  EngineEvent::MouseMotion { delta_x, delta_y }
```

**Design rule:** `DeviceEvent::MouseMotion` is dispatched separately in `device_event()` because it is raw-device input — it fires regardless of window focus, giving smooth camera look at all times. Standard keyboard input arrives via `WindowEvent::KeyboardInput` in `window_event()` with no focus workarounds needed (wry WebView child HWNDs have been removed in Phase 12C).

### 5.4 TimeState — Hybrid Frame Limiter

When `target_fps` is set, the frame budget enforcer uses a two-phase strategy:

```
Frame start ──► work ──► wait_for_frame_budget()
                              │
                              ├─ if remaining > 1 ms: thread::sleep(remaining - 1ms)
                              │    (coarse, OS timer, ~15 ms granularity on Windows)
                              │
                              └─ spin-wait until exact deadline (sub-microsecond accuracy)
```

FPS readout is an exponential moving average (α = 0.1), seeded on frame 1.

---

## 6. somnium_renderer — Visibility Buffer Pipeline

### 6.1 Philosophy

Traditional forward/deferred renderers suffer from **overdraw** — pixels behind opaque surfaces are shaded and then thrown away. The Visibility Buffer (pioneered at UbiSoft/EA, popularized by The Forge) avoids this:

```
Pass 1 — Rasterize all geometry → write (InstanceID, PrimitiveID) per pixel
Pass 2 — Fullscreen shader → look up the exact triangle → shade ONCE per pixel
```

No pixel is shaded more than once. Bandwidth scales with the framebuffer, not scene complexity.

### 6.2 GPU Resource Pools

```
GlobalResourcePool (@group 0 — bindless, one bind group for everything)
┌───────────────────────────────────────────────────────────────────────┐
│ binding 0  STORAGE  vertex_buffer   64 MB  (all mesh verts)           │
│ binding 1  STORAGE  index_buffer    32 MB  (all mesh inds)            │
│ binding 2  STORAGE  instance_buffer 16 MB  (per-frame data)           │
│ binding 3  STORAGE  view_buffer    224 B   (camera/matrices, P13)     │
│ binding 4  TEXTURE  textures[1024]         (bindless array)           │
│ binding 5  STORAGE  material_buffer        (all materials)            │
│ binding 6  STORAGE  light_buffer   320 B   (GpuDirectionalLight)      │
│ binding 7  STORAGE  local_lights    16 KB  (array<GpuLocalLight>, P13)│
│ binding 8  STORAGE  light_indices    1 MB  (array<u32>, P13)          │
│ binding 9  STORAGE  cluster_offsets 1.5MB  (array<ClusterOffset>, P13)│
│ binding 10 STORAGE  cluster_params  32 B   (ClusterParams, P13)       │
└───────────────────────────────────────────────────────────────────────┘

ShadowPass @group(1) — per-cascade uniform
┌─────────────────────────────────────────────────────────────────────┐
│ binding 0  UNIFORM  cascade_index  16 B   (u32 cascade index 0..3)  │
└─────────────────────────────────────────────────────────────────────┘

ShadingPass @group(1) — pass-local resources
┌─────────────────────────────────────────────────────────────────────┐
│ binding 0  TEXTURE  vis_buffer    R32Uint (from visibility pass)    │
│ binding 1  SAMPLER  default_sampler                                  │
│ binding 2  TEXTURE  shadow_atlas  Depth32Float DepthOnly aspect     │
│ binding 3  SAMPLER  shadow_sampler  (comparison, LessEqual, Linear) │
└─────────────────────────────────────────────────────────────────────┘
```

Geometry and materials are **uploaded once** at init. Instances are rebuilt every frame. The shaders index into all of these from a single `@group(0)` bind group.

### 6.3 Pass 1 — Visibility Buffer

| Property | Value |
|---|---|
| Input | ECS draw queue (`Vec<DrawCommand>`) |
| Output texture | `R32Uint`, same resolution as window |
| Depth attachment | `Depth32Float` |
| Clear value | `0` (sky/background sentinel) |
| Vertex shader | `visibility.wgsl::vs_main` |
| Fragment shader | `visibility.wgsl::fs_main` |
| Vertex pulling | Programmable (no vertex buffer binding; reads from storage) |

**Packed pixel encoding:**

```
  31       22 21                  0
  ┌──────────┬──────────────────────┐
  │ inst+1   │      prim_id         │
  │ (16 bits)│     (16 bits)        │
  └──────────┴──────────────────────┘
  
  0x00000000  = sky / background (clear value, never written by shader)
  0x00400000  = instance 0, primitive 0 (minimum mesh value)
  0xFFFFFFFF  = instance 65534, primitive 65535 (max; 65 535 draw limit)
```

Instance index is stored as `inst_idx + 1` so that 0 is permanently reserved as the sky sentinel. The clear value of `0` is reliable on all GPU backends because `0.0f32` bit-casts to `0x00000000u32` (correct for both DX12's `ClearRenderTargetView` float path and Vulkan's uint path).

**Vertex shader data flow:**

```
inst_idx (builtin) ──► instances[inst_idx].index_offset
                            │
v_idx (builtin) ────────────┤
                            ▼
                    indices[index_offset + v_idx]  →  vertex_id
                            │
                            ▼
                    vertices[vertex_offset + vertex_id]  →  pos
                            │
                            ▼
                    view_proj * model * vec4(pos, 1.0)  →  clip_pos
```

### 6.4 Pass 2 — Shading

| Property | Value |
|---|---|
| Input | vis_buffer (R32Uint from Pass 1) |
| Output | Swapchain surface (sRGB or native) |
| Clear value | `(0.07, 0.07, 0.07, 1.0)` dark gray (overwritten by fullscreen triangle) |
| Vertex shader | `shading.wgsl::vs_main` — generates fullscreen triangle from vertex_index |
| Fragment shader | `shading.wgsl::fs_main` — sky OR PBR BRDF |

**Fragment shader decision tree:**

```
textureLoad(vis_buffer, pixel_coords, 0).r
        │
        ├─ == 0u  ──────────────────────────────────────────────►  Procedural Sky
        │          inv_view_proj * (ndc, 0,1) → ray_dir          (horizon gradient
        │          sky = zenith/horizon/ground blend              + sun disk
        │          sun = pow(dot(ray_dir, sun_dir), 1024)          + glow)
        │
        └─ != 0u  ──────────────────────────────────────────────►  PBR Surface
                   instance_id = (vis_data >> 22) - 1
                   prim_id     = vis_data & 0x3FFFFF
                   i0,i1,i2    = indices[index_offset + prim*3 + 0..2]
                   v0,v1,v2    = vertices[vertex_offset + i0..i2]
                   bary        = perspective-correct barycentrics
                   normal      = normalize(interpolated, world-transformed)
                   surface     = { albedo, roughness, metallic, f0 }
                   result      = evaluate_brdf(surface, sun_dir) * light_color
```

### 6.5 PBR BRDF (`brdf.wgsl`)

Cook-Torrance specular + Burley (Disney) diffuse:

```
evaluate_brdf(surface, l):
  angular = get_angular_info(normal, view_dir, l)
  D  = D_GGX(n_dot_h, roughness)          — distribution
  V  = V_SmithGGX(n_dot_v, n_dot_l, α²)  — visibility / geometry
  F  = F_Schlick(f0, v_dot_h)             — fresnel
  Fr = D * V * F                          — specular
  kS = F
  kD = (1 - kS) * (1 - metallic)
  Fd = Diffuse_Burley(albedo, α, n·v, n·l, v·h)
  return (kD * Fd + Fr) * n_dot_l
```

### 6.6 View Buffer Layout (208 bytes, Phase 11)

```
Offset  Size  Field
──────  ────  ──────────────────────────────────────────────────────────
   0     64   view_proj (mat4x4<f32>)         — projection × view
  64     64   inv_view_proj (mat4x4<f32>)     — for sky ray casting
 128     64   view (mat4x4<f32>)              — world→camera (Phase 11)
 192     12   camera_pos (vec3<f32>)          — world space
 204      4   _padding (f32)                  — cascade debug flag (1.0 = on)
```

The raw `view` matrix at offset 128 is read by the shading pass to compute per-pixel view-space depth for cascade selection. The `_padding` field at offset 204 is repurposed as a cascade debug overlay toggle — when `> 0.5`, the shader tints pixels red/green/blue/yellow by cascade index. `visibility.wgsl` only reads `view_proj` at offset 0.

### 6.7 DrawCommand & Sort Key

```rust
DrawCommand {
    sort_key:     SortKey  — 64-bit key for state-change minimization
    vertex_offset: u32     — into global vertex buffer
    index_offset:  u32     — into global index buffer
    index_count:   u32
    material_id:   u32     — into material pool
    transform:     Mat4    — world matrix
}

SortKey bit layout:
  63..56  pass_id      (8 bits)  — 0 = opaque, later = transparent
  55..32  material_id  (16 bits) — minimize pipeline / bind-group changes
  31..0   mesh_id      (32 bits) — minimize vertex buffer changes
```

---

## 7. somnium_ecs — Entity Component System

### 7.1 Core Concepts

| Concept | Rust type | Description |
|---|---|---|
| Entity | `Entity { index: u32, generation: u32 }` | Lightweight handle, generational to detect stale refs |
| Component | `trait Component: Send + Sync + 'static` | Any `Copy` + `'static` struct |
| Archetype | `Archetype` | Group of entities with identical component sets; data in parallel dense arrays |
| ComponentSet | `ComponentSet` | Bitmask of component IDs; used for archetype matching |
| World | `World` | Owns all archetypes and the entity allocator |

### 7.2 Storage Layout

Each archetype stores components as parallel **Struct-of-Arrays**:

```
Archetype { components: [Transform, MeshComponent, MaterialComponent] }

  entity slot:    [  0  |  1  |  2  |  3  ]
  Transform col:  [  T0 |  T1 |  T2 |  T3 ]  ← contiguous f32 × 10 per entity
  MeshComponent:  [  M0 |  M1 |  M2 |  M3 ]  ← contiguous u32 × 3 per entity
  MaterialComp:   [  C0 |  C1 |  C2 |  C3 ]  ← contiguous u32 × 1 per entity
```

Iterating all `(Transform, MeshComponent, MaterialComponent)` entities in `on_render` walks three contiguous slabs — cache-coherent, vectorizable.

### 7.3 Archetype Query

```rust
let required = ComponentSet::from_ids(vec![
    ComponentId::of::<Transform>(),
    ComponentId::of::<MeshComponent>(),
    ComponentId::of::<MaterialComponent>(),
]);

for archetype in world.query_archetypes(&required, &excluded) {
    let t_col = archetype.column_index(ComponentId::of::<Transform>()).unwrap();
    for row in 0..archetype.len() {
        let t = unsafe { archetype.column(t_col).get::<Transform>(row) };
        // ...
    }
}
```

The query filters archetypes whose `ComponentSet` is a superset of `required` and has no overlap with `excluded`.

### 7.4 Built-in Components (defined in somnium_core)

| Component | Fields | Description |
|---|---|---|
| `Transform` | `translation: Vec3`, `rotation: Quat`, `scale: Vec3` | Local-space TRS; `to_matrix()` → `Mat4` |
| `WorldTransform` | `matrix: Mat4` | Cached world-space matrix; propagated each frame via hierarchy traversal |
| `MeshComponent` | `vertex_offset`, `index_offset`, `index_count: u32` | Points into global geometry buffers |
| `MaterialComponent` | `id: u32` | Index into `MaterialPool` |
| `Name` | `[u8; 64]` null-terminated UTF-8 | Display name for the entity. Fixed-length so it satisfies `Copy`. Use `Name::new(str)` / `.as_str()`. |
| `LightComponent` | `light_type: LightType`, `color: Vec3`, `intensity: f32` | Marks an entity as a light source. Phase 11. `light_type` is one of `Directional / Point / Spot`; only Directional is functional. |
| `Parent` | `entity: Entity` | Back-reference to this entity's parent; `Entity::DANGLING` = root |
| `Children` | `list: Vec<Entity>` | Ordered child entity list; maintained by `ReparentCmd` |
| `MeshKind` | `Cube \| Sphere \| Plane \| Cylinder` | Procedural mesh tag; used by scene serializer to reconstruct geometry |

---

## 8. somnium_ui — Native Editor UI (Phase 12 + Metaphor; chrome still growing)

### 8.1 Architecture

The editor UI is rendered entirely by the wgpu backend — no OS WebView dependency. `UiPass` composites the widget tree over the 3D viewport each frame using an alpha-blending screen-space render pass.

```
OS Window (HWND, undecorated)  ← wgpu 3D scene, then UI overlay
  │
  └── UiPass (wgpu, alpha blend, LoadOp::Load)
        │
        └── UserInterface widget tree  (outer_grid, 7 rows)
              ┌──────────────────────────────────────────────┐  Row 0  36 px  title bar
              │ mark  Somnium Engine              fps  _ □ × │
              ├──────────────────────────────────────────────┤  Row 1  menu
              │ File Edit Create View Window Help            │
              ├──────────────────────────────────────────────┤  Row 2  toolbar
              │ Save  Select Landscape Foliage  ▶ ⛶ ❚❚ ■       │
              ├──────────────────────────────────────────────┤  Row 3  26 px  viewport bar
              ├────────┬──────────────────────┬──────────────┤  Row 4  *  main
              │ Sculpt │  3D Viewport         │ Outliner     │
              │        │  (transparent)       │ Details      │
              ├────────┴──────────────────────┴──────────────┤  Row 5  220 px
              │ Content Drawer (tiles)  or  Output Log       │
              ├──────────────────────────────────────────────┤  Row 6  status
              │ Content Drawer   Output Log    status text   │
              └──────────────────────────────────────────────┘
```

### 8.2 Key types

| Type | File | Role |
|---|---|---|
| `UserInterface` | `ui.rs` | Widget tree, two-pass layout (measure/arrange), hit-test, message queue, draw dispatch |
| `UiPass` | `pass.rs` | wgpu render pass: ortho proj, vertex/index buffers, font atlas, scissor, alpha blend |
| `UiManager` | `lib.rs` | Entry point: `new()`, `end_frame()`, `build_editor_layout()`, outliner/inspector rebuilds |
| `FontAtlas` | `font.rs` | fontdue 0.7, 1024×1024 Rgba8, shelf packing, `measure_text`, `ascent` |
| `DrawingContext` | `draw.rs` | Command list: `push_rect`, `push_text`, clip stack |

### 8.3 Widget Library

All widgets port the Fyrox UI architecture (see ATTRIBUTION §13.13–13.17):

| Widget | File | Description |
|---|---|---|
| `Canvas` | `widgets/canvas.rs` | Absolute positioning container |
| `StackPanel` | `widgets/stack_panel.rs` | Linear layout (Horizontal / Vertical) |
| `Border` | `widgets/border.rs` | Background fill + per-side stroke |
| `Button` | `widgets/button.rs` | Click via `ButtonMessage::Click`; hover / press / `SetSelected` fills |
| `Text` | `widgets/text.rs` | fontdue-rendered label; optional wrap + newlines (`with_wrap`) |
| `Grid` | `widgets/grid.rs` | WPF-style rows/columns: Strict / Auto / Stretch; `SetRowSize` for the docked drawer |
| `ScrollViewer` | `widgets/scroll_viewer.rs` | Clipped vertical scroll; always-visible right gutter, wheel + thumb drag |
| `WrapPanel` | `widgets/wrap_panel.rs` | Left-to-right wrapping tiles (Content Drawer) |
| `TextBox` | `widgets/text_box.rs` | Single-line keyboard text input |
| `NumericField` | `widgets/numeric_field.rs` | f32 numeric input with live `ValueChanging` / commit `ValueChanged` |
| `UiCanvas` | `runtime.rs` | Game HUD/pause canvas without editor chrome (26-G) |
| `ColorSwatch` / `ColorPicker` | `widgets/color_picker.rs` | Iris colour property + HSV popup (26-F) |
| `CommandPalette` | `widgets/command_palette.rs` | Ctrl+P command search (26-I) |
| `ToastHost` | `widgets/toast.rs` | Transient status toasts (26-I) |
| `Splitter` | `widgets/splitter.rs` | Two-pane resizable container (Phase 26-A) |
| `CheckBox` | `widgets/check_box.rs` | Real checkbox; replaces `[x]`/`[ ]` buttons (26-B) |
| `ComboBox` | `widgets/combo_box.rs` | Header in the inspector; list is a root `Popup` + `ComboDropdown` (26-B, overlay fix 2026-08-13 evening). Replaces foliage/tonemapper cyclers |
| `TreeView` | `widgets/tree_view.rs` | Hierarchical outliner / content tree (26-B/E) |
| `TabControl` | `widgets/tab_control.rs` | Header strip + one visible page (26-B) |
| `Image` / `Icon` | `widgets/image.rs` | Icon-atlas textured quad (26-A) |
| `SearchBox` / `Breadcrumb` / `Tooltip` | `widgets/search_box.rs` | Filter, path crumbs, hover hint (26-B) |
| `ContextMenu` | `widgets/context_menu.rs` | Right-click action list (26-B) |
| `Popup` | `widgets/popup.rs` | Anchored overlay; File/Create follow their buttons on resize (26-A) |

### 8.4 Editor Layout (Phase 26-A–I Metaphor — chrome still growing)

`UiManager::build_editor_layout()` constructs the Nocturne editor tree on init.
The OS window is **undecorated**; row 0 is a custom title bar (engine mark,
“Somnium Engine”, fps, min/max/close). Font is bundled Inter
(`crates/somnium_ui/assets/fonts/Inter-Regular.ttf`), rasterized with 1.5×
supersampling and window DPI (26-H SDF slipped).
Popups size to their content; File/Create follow their buttons. Columns are
nested `Splitter`s with persisted widths. Inspector numerics include a slider
beside the typed field. Native cursors follow splitter/slider/button hit tests.
FPS is written every frame via `UiManager::set_fps`.

**Metaphor is not closed.** 26-A–I plus the 2026-08-13 UX polish are the
baseline shell. Later features (animation, cooking, 25J terrain material UI,
networking debug, …) must add inspector sections, menus, drawer types, and
`docs/editor/*.md` pages rather than one-off panels. Help includes **Water**
(`docs/editor/water.md`: SSR / RT Reflect / Reflect Debug). 26-J (reflection
inspector) is still out unless requested.

```
outer_grid (7 rows: 36 title | menu | toolbar | 26 vp-bar | * | 220 drawer | 24 status)
├── title bar — EngineMark, “Somnium Engine”, fps, Minimize / Maximize / Close
├── menu_bar — File/Edit/Create/View/Window/Help
├── main toolbar — Save, Select, Landscape, Foliage, Play, Immersive play, Pause/Stop (selected fill)
├── viewport toolbar — camera speed, profiler
├── tools_split | content_split | details_split (resizable, persisted)
│     ├── left Sculpt (named Raise/Lower/Smooth/Flatten/Noise/Paint, selected fill)
│     ├── viewport (transparent passthrough)
│     └── Outliner TreeView + Details (CheckBox/Combo/ColorSwatch/slider; visible scrollbars)
├── bottom row — Content Drawer (WrapPanel tiles, default on) or Output Log (same slot)
└── status bar — labeled Content Drawer / Output Log buttons, status text
```

Overlays (root children): compact File/Edit/Create/View/Window/Help menus,
F1 Help (`docs/editor/*.md`, wrapped + TOC including **Water**), command palette (Ctrl+P),
unsaved-changes modal, colour picker, toasts, **ComboBox dropdowns** (Type /
Tonemap). Click-away closes those transients; it does **not** close the docked
drawer. Evidence PNGs were not invented — capture from a live session into
`dev records/phase 26/` if needed.

**Keyboard:** F1 Help, Ctrl+Space toggles the docked Drawer, Esc closes the
top overlay **or exits immersive play**, then falls through to quit. RMB over
chrome can hit the UI; RMB over the viewport is still fly-cam.

**UI event routing** (`app.rs::window_event`):
1. `ui_consumed = ui.process_os_event(&event)` — routes mouse/keyboard to widget tree
2. Widgets emit `UiMessage` records during `handle_routed_message` (e.g., `ButtonMessage::Click`)
3. `UiManager` maps widget handles → `EditorEvent` variants, queued internally
4. `about_to_wait()` drains `ui.poll_editor_event()` → `handle_editor_event(ev)` → ECS / undo / create

**Keyboard shortcuts** (Phase 12E-partial, `app.rs`):
- `winit::ModifiersChanged` → `ctrl_held: bool` field on `Engine`
- `Ctrl+Z` → `EditorEvent::Undo`, `Ctrl+Y` → `EditorEvent::Redo`, `Delete` → `EditorEvent::DeleteSelected`

### 8.5 Layout Engine — Bugs Fixed (Phase 12)

| Bug | Symptom | Root Cause | Fix |
|---|---|---|---|
| **RootControl infinity** | Right panel at x=40 instead of x=screen−280 | `RootControl::measure_override` passed `Vec2::INFINITY` to children → Grid stretch columns sized to content (320px), not screen width | Pass `available` (screen_size) to children in `measure_override` |
| **Invalidation not propagating** | Outliner buttons zero-size on frame 2+ | `add_node`/`remove_node` only invalidated immediate parent; ancestors kept stale `measure_valid`/`arrange_valid` cache | `invalidate_ancestors()` walks from parent to root, clearing both flags on every ancestor |
| **Log panel overlap** | Log header and scroll view drawn in same rect | `Border` passes inner_rect identically to all children | Replaced outer Border with inner Grid: row 0 = 22px header, row 1 = stretch ScrollViewer |

### 8.6 Active Regressions

No active regressions. All major UI rendering, resizing, click detection, outliner selection, gizmo updates, and camera/WASD controls have been fully stabilized and fixed.

---

## 9. somnium_physics — Jolt Integration

`somnium_physics` wraps the Jolt Physics C++ library via `somnium_physics_sys` (raw FFI). The high-level API:

```
PhysicsWorld::new(PhysicsConfig::default())
  │
  ├── create_body(RigidBodyDescriptor { shape, position, motion_type, layer })
  │    → BodyId
  │
  ├── step(dt: f32)   — advances simulation by dt seconds
  │
  ├── get_position(BodyId) → Vec3
  │
  └── optimize_broad_phase()   — call once after all static bodies are added
```

**Layers:**
- `LAYER_NON_MOVING` (0) — static geometry (floors, walls)
- `LAYER_MOVING` (1) — dynamic bodies (players, physics objects)

Physics results are synced to ECS `Transform` components in `on_update` via a query over `(Transform, PhysicsBody)` archetypes.

---

## 10. somnium_audio — Kira Integration

`somnium_audio::AudioEngine` wraps the Kira audio library. Current API surface:

```
AudioEngine::new()    — initializes the audio device
  │
  ├── (future) play_sound(handle, settings)
  ├── (future) set_listener(position, orientation)
  └── (future) create_bus(settings) → BusHandle
```

The engine currently creates an `AudioEngine` on startup and holds it alive, but no sounds are played in `hello_engine`. The scaffolding is ready for Phase 8+ audio work.

---

## 11. somnium_asset — Asset Pipeline

### 11.1 Vertex Type

```rust
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],   // 12 bytes
    pub normal:   [f32; 3],   // 12 bytes
    pub uv:       [f32; 2],   //  8 bytes
}                             // = 32 bytes per vertex
```

### 11.2 LoadedScene (Phase 10)

`load_gltf(path) -> Result<LoadedScene, String>` is the only public entry point. No `gltf::` types escape it.

```
LoadedScene {
    meshes:    Vec<LoadedMesh>       // one per glTF primitive
    materials: Vec<LoadedMaterial>   // PBR metallic-roughness
    textures:  Vec<LoadedTexture>    // RGBA8, decoded from embedded/external images
    nodes:     Vec<SceneNode>        // world-space flattened (parent chain pre-multiplied)
}
```

**LoadedMesh** — `vertices: Vec<Vertex>`, `indices: Vec<u32>`. If the primitive has no normals, flat face normals are generated. If no UVs, defaults to `(0.0, 0.0)`.

**LoadedMaterial** — `base_color [f32;4]`, `roughness f32`, `metallic f32`, `albedo_map / normal_map / metallic_roughness_map: Option<usize>` referencing `LoadedScene.textures`.

**LoadedTexture** — `data: Vec<u8>` (RGBA8, row-major), `width`, `height`. All glTF image formats (R8G8B8, R8G8B8A8, BGR variants, R8, R8G8) are converted manually without a second image-crate dependency.

**SceneNode** — `name: String`, `mesh_index: Option<usize>`, `material_index: Option<usize>`, `transform: Mat4` (world-space). One node is emitted per (glTF node, primitive) pair. The node graph is traversed depth-first; each node's world transform = parent_world * local.

### 11.3 Upload path (in somnium_renderer)

`SomniumRenderer::upload_scene(ctx, &scene) -> Vec<UploadedNode>` handles:
1. Create `wgpu::Texture` (Rgba8UnormSrgb) per `LoadedTexture`; pad rows to `COPY_BYTES_PER_ROW_ALIGNMENT`; write via `queue.write_texture`.
2. Register each texture view in `TexturePool` / `GlobalResourcePool` (bindless array).
3. Upload each `LoadedMaterial` → `GpuMaterial` with texture indices wired into `albedo_map / normal_map / metallic_roughness_map` fields.
4. Upload each `LoadedMesh` via `GeometryPool::upload_mesh`.
5. Return one `UploadedNode { entity_name, vertex_offset, index_offset, index_count, material_id, transform }` per renderable node.

---

## 12. Frame Execution Order

Every frame, `about_to_wait()` runs in this exact sequence:

```
1. time.tick()
      Updates delta_time, elapsed, frame_count, EMA FPS

2. physics.step(dt)
      Advances Jolt simulation by one delta

2.5. Gizmo drag update  (Phase 11.5B)
      If gizmo_drag is Some, reproject cursor ray onto constrained axis/plane,
      write new Transform directly to ECS world, update renderer.gizmo_world_pos

3. game.on_update(ctx)
      ├── Sync physics → ECS transforms  (query PhysicsBody + Transform)
      ├── EditorCamera.update(dt)        (WASD movement if RMB held)
      └── log_timer update

4. Process queued editor events
      while let Some(ev) = ui.poll_editor_event() → handle_editor_event(ev)
      Dispatches: entity select/create/delete, gizmo mode, grid toggle, undo/redo
      (Events enqueued by UI button callbacks in window_event via process_os_event)

5. (Native UI has no begin_frame — widget tree rebuilt during end_frame each frame)
   Note: UI OS events (clicks, keyboard) are processed in window_event(), not here.

6. game.on_render(ctx)
      ├── Compute view + projection matrices
      ├── renderer.set_view(view, proj, camera_pos)
      ├── Query (Transform, LightComponent) → renderer.set_directional_light()
      └── Submit DrawCommands for all (Transform, MeshComponent, MaterialComponent)
          also calls ui.set_fps(), ui.rebuild_outliner(), ui.update_inspector()

7. renderer.render(ctx, ui, window)
      ├── Write view buffer to GPU (208 bytes: VP, invVP, view, cam_pos, debug_flag)
      ├── Compute 4 cascade view-projection matrices (PSS splits + bounding sphere)
      ├── Write light buffer to GPU (320 bytes: GpuDirectionalLight)
      ├── Build instance buffer from draw queue
      ├── Sort draw queue by SortKey
      ├── Acquire swapchain texture
      ├── [Shadow Pass]       4 cascades → Depth32Float 4096×4096 atlas
      ├── [Visibility Pass]   write R32Uint vis_buffer
      ├── [Shading Pass]      read vis_buffer + shadow_atlas → PBR+PCF → Rgba16Float HDR
      ├── [Grid Overlay]      fullscreen ray march → XZ plane grid → Rgba16Float HDR (if enabled)
      ├── [Water prepass]     G-buffer (normal / roughness / coverage) → HDR MRT
      ├── [Water reflection]  half-res RT compute (skipped if no ray query / SOMNIUM_RT_REFLECT=0)
      ├── [Water shade]       SSR + RT + env cube on confidence → HDR
      ├── [PostProcess Pass]  ACES tone map + vignette → swapchain (Rgba16Float → sRGB)
      ├── [Gizmo Pass]        procedural arrow/ring/cube axes → swapchain (if entity selected)
      ├── [Light Gizmo Pass]  batched world-space LineList light bounds → swapchain (Phase 13E, if enabled)
      ├── ui.end_frame()      (rebuild outliner/inspector → layout → draw → UiPass::prepare + render)
      ├── queue.submit()
      └── output.present()

8. time.wait_for_frame_budget()   (hybrid sleep + spin-wait)

9. window.request_redraw()
```

---

## 13. GPU Buffer & Shader Data Layout

### Vertex (32 bytes, matches `somnium_asset::Vertex`)

```
WGSL struct Vertex {
  pos_x, pos_y, pos_z: f32   offset  0  (12 bytes)
  norm_x, norm_y, norm_z: f32 offset 12  (12 bytes)
  u, v: f32                   offset 24  ( 8 bytes)
}
```

### GpuInstanceData (80 bytes, matches `somnium_renderer::instance::GpuInstanceData`)

```
WGSL struct Instance {
  model: mat4x4<f32>    offset  0  (64 bytes)
  material_id: u32      offset 64  ( 4 bytes)
  vertex_offset: u32    offset 68  ( 4 bytes)
  index_offset: u32     offset 72  ( 4 bytes)
  _padding: u32         offset 76  ( 4 bytes)
}
```

### GpuMaterial (48 bytes, matches `somnium_renderer::material::pool::GpuMaterial`)

```
WGSL struct Material {
  base_color: vec4<f32>          offset  0  (16 bytes)
  roughness: f32                 offset 16  ( 4 bytes)
  metallic: f32                  offset 20  ( 4 bytes)
  albedo_map: i32                offset 24  ( 4 bytes)  -1 = none
  normal_map: i32                offset 28  ( 4 bytes)  -1 = none
  metallic_roughness_map: i32    offset 32  ( 4 bytes)  -1 = none
  _padding: i32                  offset 36  ( 4 bytes)
}                                           = 40 bytes (+ align pad = 48)
```

### View (208 bytes, Phase 11)

```
WGSL struct View (shading.wgsl) {
  view_proj: mat4x4<f32>      offset   0  (64 bytes)
  inv_view_proj: mat4x4<f32>  offset  64  (64 bytes)
  view: mat4x4<f32>           offset 128  (64 bytes)   ← Phase 11: world→camera
  camera_pos: vec3<f32>       offset 192  (12 bytes)
  _padding: f32               offset 204  ( 4 bytes)   ← cascade debug flag
}
```

### GpuDirectionalLight (320 bytes, Phase 11)

```
WGSL struct DirectionalLight (shading.wgsl @group(0) @binding(6)) {
  direction: vec3<f32>                  offset   0  (12 bytes)
  _pad0: f32                            offset  12  ( 4 bytes)
  color: vec3<f32>                      offset  16  (12 bytes)
  _pad1: f32                            offset  28  ( 4 bytes)
  view_proj: array<mat4x4<f32>, 4>      offset  32  (256 bytes)  ← 4 cascade VPs
  cascade_splits: array<f32, 4>         offset 288  (16 bytes)   ← view-space Z splits
  shadow_map_size: f32                  offset 304  ( 4 bytes)
  _pad2: array<f32, 3>                  offset 308  (12 bytes)
}                                                    = 320 bytes total
```

### Clustered Lighting Structs (Phase 13C)

```
WGSL struct GpuLocalLight (64 bytes, binding 7) {
  position_ws: vec3<f32>
  range: f32
  color: vec3<f32>
  light_type: u32
  direction_ws: vec3<f32>
  spot_cos_outer: f32
  spot_cos_inner: f32
  _pad: array<f32, 3>
}

WGSL struct ClusterOffset (8 bytes, binding 9) {
  offset: u32
  count: u32
}

WGSL struct ClusterParams (32 bytes, binding 10) {
  grid_width: u32
  grid_height: u32
  num_slices: u32
  tile_size: u32
  near: f32
  far: f32
  shading_mode: u32      ← 0 = PBR, 1 = Cel-shaded
  num_local_lights: u32
}
```

---

## 14. Rendering Passes — Detailed

### Pass 1: Visibility Buffer

```
Attachments:
  Color[0]  R32Uint  vis_buffer    CLEAR(0) / STORE
  Depth     D32Float depth_buffer  CLEAR(1.0) / STORE

Pipeline:
  VS  visibility.wgsl::vs_main
        in:  @builtin(vertex_index) v_idx
             @builtin(instance_index) inst_idx
        out: clip_pos, instance_id (flat)
  FS  visibility.wgsl::fs_main
        in:  @builtin(primitive_index) prim_idx
             instance_id (flat)
        out: @location(0) u32  ← packed vis_data

Draw call per mesh:
  rpass.draw(0..index_count, inst_id..inst_id+1)
```

### Pass 2 (new, Phase 11): Shadow

```
Attachments:
  Depth  Depth32Float  shadow_atlas 4096×4096  CLEAR(1.0) / STORE

Loop: 4 cascades
  set_viewport(cascade_quadrant)  ← 2048×2048 quadrant in 4096×4096 atlas
  set_bind_group(1, cascade_bind_group[i])  ← cascade index uniform

Bind groups:
  @group(0)  GlobalResourcePool   (vertices, indices, instances, view, textures, materials, light)
  @group(1)  ShadowPass per-cascade  (cascade_index u32 uniform)

Pipeline:
  VS  shadow.wgsl::vs_main — programmable vertex pulling → light.view_proj[cascade.index] * world_pos
  FS  None — depth writes only
  Cull: Back  Depth bias: constant=2, slope_scale=2.0
```

### Pass 3: Shading → HDR (Phase 11.5K)

```
Attachments:
  Color[0]  Rgba16Float HDR target  CLEAR(dark gray) / STORE

Bind groups:
  @group(0)  GlobalResourcePool   (vertices, indices, instances, view, textures, materials, light)
  @group(1)  ShadingPass          (vis_buffer R32Uint, default_sampler, shadow_atlas, shadow_sampler)

Pipeline:
  VS  shading.wgsl::vs_main — fullscreen triangle (no buffers)
  FS  shading.wgsl::fs_main — sky or PBR + PCF shadow → HDR linear radiance

Draw call (single):
  rpass.draw(0..3, 0..1)
```

### Pass 3.5: Grid Overlay → HDR (Phase 11.5H, optional)

```
Attachments:
  Color[0]  Rgba16Float HDR target  LOAD / STORE  (alpha-blended on top of shading)

Bind groups:
  @group(0)  GridView  (view buffer 208 bytes: binding 0)

Pipeline:
  VS  grid.wgsl::vs_main — fullscreen triangle
  FS  grid.wgsl::fs_main
        Reconstructs world ray via inv_view_proj
        Intersects with XZ plane (y=0)
        Derivative-based AA grid lines, 1m minor / 10m major / axis highlights
        Distance fade 50m–100m, alpha blending
  Blend: ALPHA_BLENDING (src_alpha over 1-src_alpha)
```

Enabled only when `renderer.grid_enabled == true`. Toggle via `"toggle_grid"` IPC or `G` key.

### Pass 3.6: Water — prepass, reflection, shade (Phase IV + VV Halcyon)

Water is **not** in the visibility buffer. After opaque shading (and the HDR
scene copy used for refraction/SSR), the pass splits:

```
Water prepass   water.wgsl::fs_prepass   G-buffer: normal, roughness, coverage, velocity
Water reflection  water_reflection.wgsl  half-res compute: GGX/mirror ray query via rt_hit.wgsl
Water shade     water.wgsl::fs_main      SSR (trace_ssr) + RT texture + env cube on confidence
```

Profiler scopes: `"Water prepass"`, `"Water reflection"`, `"Water shade"`.
Kill switch `SOMNIUM_RT_REFLECT=0` (or no `EXPERIMENTAL_RAY_QUERY`) skips the
compute pass and restores SSR + sky cube. Water and transparents stay out of
the TLAS. FFT displacement cascades are vertex-only so the reflection sampled
texture fits `max_sampled_textures_per_shader_stage` (16). Inspector: **SSR**,
**RT Reflect**, **Reflect Debug**; Post FX **RT Reflections**. Help:
`docs/editor/water.md`. Plan: `dev records/phase_VV.md`.

### Pass 4: PostProcess — HDR → swapchain (Phase 11.5K)

```
Attachments:
  Color[0]  Swapchain (sRGB)  CLEAR(black) / STORE

Bind groups:
  @group(0)  PostProcessPass
    binding 0  TEXTURE  hdr_texture   Rgba16Float
    binding 1  SAMPLER  linear clamp
    binding 2  UNIFORM  params_buffer  16 bytes { exposure f32, vignette f32, _pad×2 }

Pipeline:
  VS  postprocess.wgsl::vs_main — fullscreen triangle
  FS  postprocess.wgsl::fs_main
        sample hdr_texture → apply exposure
        ACES filmic: (x*(2.51x+0.03))/(x*(2.43x+0.59)+0.14)  clamp [0,1]
        Radial vignette: smoothstep(0.35, 0.75, dist_from_center)
  Blend: REPLACE (overwrites swapchain)
```

`set_params(exposure, vignette_strength)` uploads to params_buffer. Default: 1.0, 1.0.  
`resize(width, height)` recreates the Rgba16Float texture + bind group.

### Pass 4.5: Gizmo (Phase 11.5B, conditional)

```
Attachments:
  Color[0]  Swapchain (sRGB)  LOAD / STORE  (draws on top of tone-mapped image)

Bind groups:
  @group(0)  GizmoPass
    binding 0  STORAGE  view_buffer    208 bytes (view_proj + camera_pos)
    binding 1  STORAGE  params_buffer   64 bytes (model mat4x4)

Pipeline:
  VS  gizmo.wgsl::vs_main  — vertex buffer pull, per-vertex color
  FS  gizmo.wgsl::fs_main  — returns vec4(color, 1.0), no depth test
  Blend: REPLACE  CullMode: None  Topology: TriangleList  Depth: disabled
  IndexFormat: Uint32

Geometry (pre-built, stored in vertex+index buffers):
  Translate: 3 × arrow (shaft prism + 8-sided cone), index range t_start..t_end
  Rotate:    3 × ring (36-segment torus, inner=0.80, outer=0.88), range r_start..r_end
  Scale:     3 × arm (shaft + cube handle), range s_start..s_end

Model matrix:
  model = Mat4::from_translation(entity_world_pos) * Mat4::from_scale(Vec3::splat(dist*0.15))
  (screen-size-constant: dist = camera_pos.distance(gizmo_pos).max(0.5))

Rendered only when renderer.gizmo_world_pos is Some.
Mode selection (Translate/Rotate/Scale) by index range in draw_indexed().
```

**Picking (CPU-side, Phase 11.5B):**

```
On LMB press:
  1. Cast world-space ray from camera through NDC cursor
  2. Transform ray to gizmo local space via inv(model)
  3. gizmo_hit_test(local_origin, local_dir, mode) → Option<GizmoAxis>
     Uses AABB slabs for each axis handle

On LMB drag (each frame in about_to_wait step 2.5):
  Translate: ray_axis_param → axis-line parameter → delta translation
  Rotate:    ray-plane intersection → atan2 angle → delta rotation via Quat::from_axis_angle
  Scale:     ray_axis_param ratio → scale along one axis

On LMB release:
  push_silent(SetTransformCmd { old: start_transform, new: final_transform })
  (already applied; push to undo history without re-executing)
```

### Pass 4.7: Selection Outline (Phase 11.5I, conditional)

```
Attachments (both sub-passes share this layout):
  Color[0]  Swapchain (sRGB)       LOAD / STORE  (composite over gizmo/post result)
  DS        Depth24PlusStencil8    owned by OutlinePass (separate from vis-pass depth)

Bind group (OutlinePass-owned BGL):
  binding 0  UNIFORM   outline_uniform_buf  160 bytes
             — view_proj (mat4×4), model (mat4×4), outline_color (vec4),
               outline_width (f32), vertex_offset (u32), index_offset (u32)
  binding 1  STORAGE   global vertex buffer  (reused from GeometryPool)
  binding 2  STORAGE   global index buffer   (reused from GeometryPool)

Sub-pass 1 — stencil write:
  VS  outline.wgsl::vs_stencil  — vertex pull + model/view_proj transform
  FS  outline.wgsl::fs_stencil  — returns vec4(0) (write_mask = NONE)
  Stencil: CompareFunction::Always, pass_op = Replace (writes ref=1)
  Depth: disabled (Always), no depth write
  CullMode: None  (fill entity silhouette completely)
  LoadOp stencil: Clear(0)

Sub-pass 2 — extruded outline:
  VS  outline.wgsl::vs_outline  — same vertex pull, then:
       clip_norm_dir = (view_proj * model * vec4(normal, 0)).xy
       safe_norm = safe normalize of clip_norm_dir
       offset = safe_norm * outline_width * clip.w   (perspective-correct)
       return vec4(clip.xy + offset, clip.zw)
  FS  outline.wgsl::fs_outline  — returns outline_color (orange #FA9412)
  Stencil: CompareFunction::NotEqual (ref=1) — skips entity interior
  Depth: disabled (Always), no depth write
  Blend: ALPHA_BLENDING  CullMode: Back
  LoadOp stencil: Load (preserves sub-pass 1 result)

Color: [0.98, 0.58, 0.07, 1.0]  width_ndc: 0.007
Rendered only when renderer.outline_entity is Some (entity with MeshComponent selected).
SomniumRenderer::set_outline_entity() / clear_outline() controlled by select_entity IPC.
```

**Texture sampling in the shading pass (Phase 10+):**

```
After barycentrics → interpolated uv + geo_normal:

TBN matrix (no vertex tangents, derived from geometry):
  edge0 = p1-p0, edge1 = p2-p0 (world space)
  duv0  = uv1-uv0, duv1 = uv2-uv0
  tangent   = normalize((edge0 * duv1.y - edge1 * duv0.y) / det)
  tangent   = Gram-Schmidt(tangent, geo_normal)   ← keep TBN orthogonal
  bitangent = cross(geo_normal, tangent)
  tbn       = mat3x3(tangent, bitangent, geo_normal)

Albedo:     material.base_color.rgb * textureSample(albedo_map, uv)  (if map ≥ 0)
Normal:     geo_normal  OR  normalize(tbn * (normal_sample * 2 - 1))  (if map ≥ 0)
MR map:     glTF spec — green channel → roughness, blue channel → metallic (if map ≥ 0)
```

---

## 15. Camera System

The `EditorCamera` in `hello_engine/src/main.rs` implements UE5-style fly-through navigation:

| Control | Behavior |
|---|---|
| **RMB hold** | Activates mouselook; reclaims keyboard focus via `focus_window()` |
| **RMB + mouse** | Pitch (clamped ±89°) and Yaw (free rotation) |
| **RMB + WASD** | 3D translation along the look direction / right vector |
| **RMB + QE** | Move up/down along world Y |
| **RMB + Shift** | 3× speed boost |
| **ESC** | Shutdown |

**View matrix computation:**

```rust
forward = Vec3(cos(yaw) * cos(pitch), sin(pitch), sin(yaw) * cos(pitch)).normalize()
view    = Mat4::look_at_rh(position, position + forward, Vec3::Y)
proj    = Mat4::perspective_rh(fov=45°, aspect, near=0.1, far=1000.0)
view_proj = proj * view
```

Camera position at engine start: `Vec3::new(0.0, 2.0, 8.0)` — pulled back so a glTF scene centered at the origin is visible on first launch.

---

## 16. UI Messaging — Direct Rust API (Phase 12 Complete)

The wry IPC protocol (`IpcMessage`, `poll_ipc`, `send_message` via `evaluateScript`, `handle_ipc_command`) was removed in Phase 12C. All editor communication now flows through direct Rust API calls.

**Event flow (Phase 12D-full):**
1. `app.rs::window_event()` calls `ui.process_os_event(&event)` — routes mouse/keyboard into the `UserInterface` widget tree (hit-test, focus, message dispatch)
2. Widgets emit `UiMessage` records via the `emit: &mut Vec<UiMessage>` parameter in `handle_routed_message` (e.g., `ButtonMessage::Click`, `NumericFieldMessage::Value`)
3. `UiManager::process_os_event` maps widget handles to `EditorEvent` variants and pushes them onto an internal queue
4. `about_to_wait()` drains the queue via `ui.poll_editor_event()` → `handle_editor_event(ev)` which calls into ECS / undo stack / scene serializer

**EditorEvent variants** (defined in `editor_event.rs`):
- `SelectEntity(Option<Entity>)`, `CreateEntity(CreateKind)`, `DeleteSelected`
- `Undo`, `Redo`
- `SetGizmoMode(GizmoMode)`, `ToggleGrid`
- `SetInspectorValue { handle: Entity, field: InspectorField, value: f32 }`

Editor commands in `editor_commands.rs` and `scene_serial.rs` are wired to native button callbacks.

All editor events (button clicks, keyboard shortcuts, gizmo interactions) flow through the native Rust widget tree — no IPC or WebView involvement since Phase 12C.

---

## 17. Phase History & Roadmap

| Phase | Status | Description |
|---|---|---|
| 1 | ✅ Complete | Project skeleton, Cargo workspace, winit window |
| 2 | ✅ Complete | wgpu initialization (`RenderContext`), surface, adapter selection |
| 3 | ✅ Complete | Visibility Buffer Pass 1 (R32Uint texture, programmable vertex pulling) |
| 4 | ✅ Complete | Shading Pass 2 (fullscreen triangle, PBR BRDF, sky shader) |
| 5 | ✅ Complete | `GlobalResourcePool` bindless arrays (vertices, indices, instances, materials) |
| 6 | ✅ Complete | `somnium_ecs` archetype world, entity allocator, component storage |
| 7 | ✅ Complete | Static mesh upload, `GeometryPool` / `MaterialPool`, cube demo in hello_engine |
| 8 | ✅ Complete | Advanced shading: normal maps (TBN from UV derivatives), metallic-roughness map sampling |
| 9 | ✅ Complete | Hybrid UI: wry panels, IPC, WASD focus bugs fixed (BUG-002, BUG-004) |
| 10 | ✅ Complete | glTF 2.0 asset loader (`somnium_asset`): `load_gltf` → `LoadedScene`, `upload_scene`, `Name` ECS component, real entity names in outliner |
| 11 | ✅ Complete | Cascaded shadow maps (CSM): 4-cascade PSS shadow atlas, `GpuDirectionalLight`, PCF sampling, `LightComponent` ECS, cascade debug overlay (C key) |
| 11.5A | ✅ Complete | Hierarchy: `Parent`/`Children`/`WorldTransform` ECS components, propagation, Outliner IPC with depth/parent |
| 11.5B | ✅ Complete | Transform gizmos: procedural arrow/ring/cube geometry, `GizmoPass`, ray picking (AABB), LMB drag (translate/rotate/scale), undo on release |
| 11.5C | ✅ Complete | Editor HTML overhaul: inspector with TRS fields, entity hierarchy, dropdown menus, console |
| 11.5D | ✅ Complete | Entity creation/deletion/duplication: Create menu, procedural meshes (cube/sphere/plane/cylinder) |
| 11.5E | ✅ Complete | Undo/redo: `EditorCommand` trait, `UndoStack`, `SetTransformCmd`, `SetNameCmd`, `SetLightCmd`, `ReparentCmd`, `DeleteEntityCmd`, `CreateEntityCmd` |
| 11.5F | ✅ Complete | Scene serialization: `save_scene`/`load_scene`/`new_scene` JSON `.somnium` format |
| 11.5H | ✅ Complete | Editor grid overlay: ray-XZ-plane shader, derivative AA, distance fade, axis highlights, alpha blend into HDR |
| 11.5K | ✅ Complete | HDR pipeline: `Rgba16Float` intermediate target, ACES filmic tone mapping, radial vignette, `PostProcessPass` |
| 11.5I | ✅ Complete | Selection outline: two-subpass stencil (write entity footprint → draw extruded halo), `OutlinePass`, clip-space normal extrusion with perspective correction, orange (#FA9412) highlight |
| BUG-005 | ✅ Fixed | Toolbar dropdown menus clipped by WebView: `expand_toolbar`/`collapse_toolbar` via `menu_opened`/`menu_closed` IPC |
| 11.5J | ✅ Complete | GPU particle system: `ParticleEmitter` ECS component, CPU simulation (`simulate_particles`), `ParticlePass` instanced billboard rendering, soft radial alpha, color/size lerp, LCG pseudo-random spawn direction |
| 11.5L | ✅ Complete | Toolbar & menu polish: Q/W/E/R/G/C/F/P hotkeys, Ctrl+Z/Y/S/D shortcuts, Spot Light + Particle Emitter in Create menu |
| 11.5M | ✅ Complete | Output log: `LogCaptureLayer` (`tracing_subscriber::Layer`), `mpsc::channel`, installed alongside `fmt` layer in `registry().with(...)`, drained every 5 frames via `UiManager::send_message("append_log", ...)` |
| 11.5N | ✅ Complete | Study & document: bevy_water (FBM wave noise, dual-direction crossfade), bevy_voxel_world (32³ padded chunks, block_mesh, async LOD), bevy_wind_waker_shader (1D ramp quantization, rim highlight), UE5 headers (Nanite cluster limits, LightDefinitions bit layout, FroxelDefinitions 8×8 tiles, InstanceCulling flags, IVT uniform) — all documented in ATTRIBUTION.md §13.9–13.12 |
| 12A-1 | ✅ Complete | Native UI migration: ported Fyrox generational arena — `Handle<T>` (index+generation+PhantomData), `Pool<T>` (records+free_stack), `Handle::transmute<U>()` bridge. `crates/somnium_ui/src/pool.rs`. Cited in ATTRIBUTION.md §13.13. |
| 12A-2 | ✅ Complete | Native UI core types: ported Fyrox widget/message/draw/control architecture. `types.rs` (Rect, Thickness, alignment), `draw.rs` (Vertex, DrawCommand, DrawingContext+clip-stack), `message.rs` (UiMessage, MessageDirection, WidgetMessage), `widget.rs` (Widget, WidgetBuilder), `node.rs` (Control trait, UiNode), `ui.rs` (UserInterface: two-pass layout, hit-test, message queue, draw). Cited in ATTRIBUTION.md §13.14. |
| 12A-3 | ✅ Complete | Layout engine per-widget overrides: Canvas (infinite-space / absolute arrange), StackPanel (Vertical/Horizontal, sequential layout), Grid (SizeMode::Strict/Auto/Stretch, 4-group measurement algorithm). LayoutCtx extended with `row()`/`column()` accessors. Cited in ATTRIBUTION.md §13.15. |
| 12A-4 | ✅ Complete | Font atlas: `fontdue` 0.7, `crates/somnium_ui/src/font.rs` — `FontAtlas` (512×512 RGBA8, shelf packing, `get_or_rasterize`, `measure_text`, `ascent`). `DrawingContext::push_text` emits glyph quads at `texture_id=0`. `LayoutCtx::measure_text` calls atlas metrics (no rasterization). `UserInterface::add_font(bytes) -> u8`. `text.rs` now drives real font rendering. |
| 12A-5 | ✅ Complete | Widget library: `theme.rs` (UE5 dark palette), `widgets/canvas.rs`, `widgets/stack_panel.rs`, `widgets/border.rs`, `widgets/button.rs` (ButtonMessage::Click via `emit` Vec), `widgets/text.rs` (real fontdue rendering), `widgets/grid.rs`. `handle_routed_message` updated with `emit: &mut Vec<UiMessage>` parameter. Cited in ATTRIBUTION.md §13.15. |
| 12B-1 | ✅ Complete | `UiPass` wgpu render pass (`pass.rs` + `ui_pass.wgsl`): vertex/index upload with doubling resize, ortho uniform, white-1×1 + font-atlas bind group variants, alpha blend, per-DrawCommand scissor rect, lazy BG1 switch. `prepare(device, queue, draw_ctx, w, h)` + `render(encoder, surface_view)`. Cited in ATTRIBUTION.md §13.17. |
| 12D-skel | ✅ Complete | Native UI wired: `UiManager` gains `native_ui: UserInterface` + `ui_pass: UiPass`; `build_editor_layout()` builds 3-row Grid (menu bar 28 px, main area stretch, log 192 px) with inner 3-col Grid (toolbar 40 px, viewport transparent, right panel 280 px). `end_frame()` runs layout→draw→prepare→render. `clip_bounds` init bug fixed. |
| 12C | ✅ Complete | Removed wry: deleted `editor.html`; stripped wry from `somnium_ui/Cargo.toml` and workspace `Cargo.toml`; removed `IpcMessage`, `poll_ipc`, `handle_ipc_command` (400-line dispatcher), `window_focused`, `focus_window()` RMB hack, `DeviceEvent::Key` fallback, log drain `send_message`, `list_directory` from `app.rs`; `send_message` kept as no-op stub; zero warnings introduced. |
| 12D-full | ✅ Complete | Full editor layout: 3-row outer Grid + 3-col inner Grid; menu bar Grid with FPS right-aligned; Outliner (ScrollViewer + StackPanel rebuilt per-frame); Inspector (9 NumericField TRS handles, `SetInspectorValue` events); Output Log (22px header row + stretch ScrollViewer); Create popup (8 entity kinds); app.rs fully wired: UI event routing, EditorEvent loop, selection/create/delete/undo/redo. New widgets: ScrollViewer, TextBox, NumericField. |
| 12E-partial | ✅ Complete | Keyboard shortcuts: `Ctrl+Z` (undo), `Ctrl+Y` (redo), `Delete` (delete selected). `winit::ModifiersChanged` tracks `ctrl_held: bool`. |
| 12-bugfix | ✅ Complete | Layout bugs fixed: `RootControl::measure_override` now passes `available` instead of `Vec2::INFINITY`; `invalidate_ancestors()` propagates layout invalidation up the full ancestor chain on every `add_node`/`remove_node`; log panel rebuilt as inner Grid (22px header + stretch ScrollViewer) to fix overlapping children. |
| BUG-006 | ✅ Fixed | UI buttons respond perfectly to clicks, and events are correctly dispatched |
| BUG-007 | ✅ Fixed | Viewport WASD + RMB movement works smoothly, and keyboard events are only consumed when a text-input widget has focus |
| 13 | ✅ Complete | Water shader (PBR textures, Beer's law, FBM waves, dual-panning UVs, analytic GGX specular) |
| 13C-perf | ✅ Complete | Clustered-light assignment rewritten: counting sort into reusable flat buffers (was `Vec<Vec<u32>>`, one heap allocation per froxel a light touched — ~23 ms/frame in a debug build with 2 lights, capping the frame rate at ~43 FPS). Tile size 16 → 32 px (4× fewer froxels, offset upload 1.5 MB → 382 KB/frame), and the whole pass is skipped when no local lights exist. 21× faster in release, 5× in debug; a unit test asserts the new output is byte-identical to the old implementation. |
| 13E | ✅ Complete | Light gizmos: `LightGizmoPass` (`pass/light_gizmo.rs` + `light_gizmo.wgsl`) draws editor wire bounds per light type — point = sphere at range, spot = inner/outer cones (`height = range·cos θ`, `radius = range·sin θ`) + aim line, directional = arrow + parallel rays; every light also gets an origin cross. All lights batch into one world-space `LineList` draw over the swapchain (no depth test); selected light draws at full brightness, others dimmed to 45%. `L` toggles. Submitted engine-side by `app.rs::submit_light_gizmos()`. The inspector gained an editable **Light** section (intensity, range, inner°, outer°) that is shown only when the selection has a `LightComponent`, with edits routed through `SetLightCmd` so they undo; inner/outer angles are clamped to stay ordered. |
| 13E-b | ✅ Complete | Editor cleanup: voxel terrain is no longer auto-spawned — it is created from **Create > Voxel Terrain**, backed by a `VoxelTerrainComponent` marker entity so it appears in the outliner and can be selected/deleted (the game-layer streaming driver is built/torn down to follow that entity, freeing chunk allocations on delete). The menu-bar FPS counter was removed. |
| 14 | ✅ Complete | Voxel world (`somnium_voxel` crate): 32³ chunks padded to 34³, `block_mesh::visible_block_faces` meshing, FBM heightmap terrain, async generation (rayon + mpsc), 3 LOD levels via nearest-neighbour downsample, `set_voxel` edit overlay with version-guarded remeshing, `GeometryPool` free-list for chunk mesh recycling, palette-texture material; chunks rendered as direct DrawCommands (not ECS entities). See §19. |
| 14 SSS | ✅ Complete | Heightmap terrain system: chunked heightmap (`somnium_renderer::terrain`), 5 LOD levels with CPU-side block-fan stitching (no T-junction cracks), splatmap PBR (4 procedural layers, height-based blending, triplanar cliffs), `TerrainPass` into HDR target with CSM shadows + clustered lights, sculpt brushes (Raise/Lower/Smooth/Flatten/Noise) + splat painting with undo, editor terrain mode (F6, toolbar palette, in-shader brush cursor ring), Create > Terrain, scene-save sidecar binaries. See §20. |
| 15A | ✅ Complete | GPU-driven indirect draw: `indirect.rs` builds a `DrawIndirectArgs` buffer (`INDIRECT \| STORAGE \| COPY_DST`) from the sorted draw queue each frame, and the visibility pass submits the whole scene with one `multi_draw_indirect` instead of one `draw()` per object. `multi_draw_indirect` is core in wgpu 29; only `INDIRECT_FIRST_INSTANCE` is feature-gated (each draw's `first_instance` is its instance-buffer slot), so it's requested optionally and the renderer falls back to the per-draw CPU loop when absent. `F9` A/B-toggles the two paths — they must render identically. |
| 15A1 | ✅ Complete | Post-processing moved into the scene: `PostProcessComponent` on a selectable **Post Processing** entity drives exposure, vignette, and a new chromatic-aberration effect. **The vignette no longer defaults on** — it darkened the viewport edges permanently with no way to disable it; all effects now start off and the renderer's defaults are 0. The inspector gains a Post FX section with checkbox-style toggles (a Button whose label carries `[x]`/`[ ]`, since the UI has no Checkbox widget) plus strength fields. CA splits R/B along the vector from screen centre, written branch-free so `strength = 0` is exactly the un-aberrated image. |
| 15A2 | ✅ Complete | **FXAA anti-aliasing** (`pass/fxaa.rs` + `fxaa.wgsl`), ported from Timothy Lottes' FXAA 3.11 console preset. Runs on the tone-mapped LDR image: post-processing renders into an intermediate target, FXAA resolves it to the swapchain, and the editor overlays (gizmos, outline, particles, UI) draw *after* so text and thin lines stay pixel-sharp. Every tap uses `textureSampleLevel`, not `textureSample` — the contrast early-out is a data-dependent branch, and implicit-derivative sampling is illegal in non-uniform control flow in WGSL. Toggled from the Post Processing entity; defaults **on** (it's an image-quality feature, and the visibility-buffer pipeline has no MSAA). Disabled = pass skipped entirely, post-processing writes straight to the swapchain. |
| 15B | ✅ Complete | **GPU instance frustum culling.** `GeometryPool` records a local AABB per mesh at upload (keyed by `vertex_offset`, so no `DrawCommand` call site changed); a compute pass (`pass/cull.rs` + `cull.wgsl`) transforms each instance's AABB to world space and tests it against the six frustum planes, writing `instance_count = 0` into the Phase 15A indirect args for failures. Nothing is removed — indices stay stable and a culled draw costs nothing. Plane extraction (Gribb–Hartmann, near = `row2` for wgpu's `z ∈ [0,1]`) and the AABB test are mirrored in `culling.rs` with 13 unit tests; the shader is a transliteration. Conservative: boxes straddling a plane are kept. `F10` A/B-toggles it — a correct cull is invisible. Shadows are unaffected (the shadow pass draws directly), which is right: off-screen casters still shadow into view. |
| 15C | ✅ Complete | **Instance cap raised 1022 → 65 535.** `vis_data` repacked from a 10/22 split to 16/16, so the scene is no longer capped at 1022 draws. The trade is 65 536 triangles per *draw*; `GeometryPool` warns at upload if a mesh exceeds it rather than silently wrapping the primitive index, and Phase 15D's meshlets make the limit moot. Draw compaction was dropped as unnecessary — with the cap lifted, its only remaining benefit was keeping instance IDs dense. |
| 15D–15F | ⬜ Planned | Meshlet generation → Hi-Z occlusion culling → meshlet rendering. See §21. |
| 19 | ✅ Complete | **Image-based lighting.** `pass/ibl.rs` + `ibl_gen.wgsl` capture the engine's own procedural sky into an `Rgba16Float` cubemap (256², 6 mips), then GGX-prefilter mips 1-5 by roughness — the prefiltered-environment half of Karis' split-sum approximation. `shading.wgsl` replaces the flat `0.03 * albedo` ambient with real environment lighting: diffuse irradiance sampled along N from the roughest mip, plus split-sum specular along the reflection vector using an **analytic** BRDF fit (Lazarov), so no 2-D LUT is needed. Metals now reflect the sky instead of reading flat. Capturing the sky procedurally means reflections always match the drawn background and stay correct when the sun moves; regeneration is skipped unless the sun actually changes. Cel-shading keeps its flat ambient deliberately. |
| 20 | ✅ Complete | **Model import.** The menu bar's inert "File" text became a real menu with **Import Model…**, which opens a native file picker (`rfd`) and imports a glTF/GLB through the existing `load_gltf` → `upload_scene` path. One entity per renderable node, named from the glTF node so it is identifiable in the outliner, and the last one is auto-selected so the gizmo lands on the import. Node transforms are relative to the glTF scene origin, so a normally authored model appears at world (0,0,0) while multi-part models keep their relative layout. Placeholder for the real content browser (Phase 21). |
| 20B | ✅ Complete | **Editor camera speed control.** New viewport toolbar row between the menu bar and the viewport (UE5-style) with a **Camera Speed** slider and live readout. Speed is mapped **exponentially** over 0.5–500 m/s: imported scenes vary by orders of magnitude, and a linear slider would waste most of its travel on unusable speeds. **RMB + scroll wheel** adjusts it in multiplicative steps (UE5 muscle memory) and drives the slider back. Required two pieces of UI infrastructure: a `Slider` widget, and **mouse capture + `MouseMove` routing** in `UserInterface` — `captured_ih` existed but was never used, so no widget could be dragged, and a Button pressed then released elsewhere stayed stuck in the pressed state. Both fixed. The camera no longer owns its speed; it reads `EngineContext::camera_speed`. |
| 19-fix | ✅ Complete | **Texture sampling + ambient fixes.** Three separate issues, two long-standing: (1) the shading sampler used wgpu's default `ClampToEdge` while glTF's default wrap is `REPEAT`, so any UV outside 0-1 smeared the edge texel across the surface — present since Phase 10 and the cause of the streaked look on imported models; now `Repeat`. (2) Imported glTF textures uploaded with `mip_level_count: 1`, so the trilinear sampler had nothing to filter between and minified textures aliased; a CPU box-filter mip chain is now built at import. (3) Phase 19's IBL replaced a 3% flat ambient with full sky irradiance, but with no ambient occlusion that light reaches shadowed surfaces unattenuated and washed shadows out — indirect is scaled by `IBL_INTENSITY = 0.35` until SSAO or a glTF occlusion map lands. |
| 21 | ✅ Complete | **Alpha-blended materials** (`pass/transparent.rs` + `transparent.wgsl`). The visibility buffer resolves one triangle per pixel and structurally cannot show through, so glTF `alphaMode: BLEND` materials previously rendered fully opaque — imported car glass became solid grey panels and a soft shadow-plane decal became a hard black rectangle. Blended draws now go to a forward pass after opaque shading, terrain and water: depth-tested against the opaque depth but **never writing** it, sorted back-to-front per object, `cull_mode: None` with the normal flipped on back faces (blended glTF geometry is nearly always thin and double-sided). `SomniumRenderer::submit` routes by material, so **no call site changed**; blended instances are appended to the same instance buffer after the opaque ones and the visibility pass simply draws the opaque range. Shading is deliberately lighter than `shading.wgsl` — sun plus an IBL reflection with a Fresnel term, no clustered-light loop — since glass reads mostly as reflection and tint. The loader now imports `alphaMode`, `alphaCutoff` and `doubleSided`, which it previously discarded. **Known limit:** sorting is per object, so two blended surfaces of the same object that interpenetrate can still composite wrongly; `AlphaMode::Mask` is imported but not yet cut out in the shader. |
| 21-fix | ✅ Complete | **Instance buffer built before the draw sort.** The residual "fan of mirror-like shards" on imported models. `build_frame` filled the instance buffer from `draw_queue`, then sorted `draw_queue` — but instance `i` is exactly what draw `i` pulls its model matrix, `vertex_offset` and `index_offset` from, so after the sort almost every draw was paired with a different mesh's geometry offsets while still drawing its own `index_count`. Programmable vertex pulling then read unrelated regions of the geometry pool and stretched triangles between them. Introduced by Phase 21, which had to build the instance buffer early to compute `transparent_base`; the sort now runs first. It only showed on imports because a sort is a no-op below a handful of draws — the single-mesh helmet demo could never expose it, the 47-node car did. `GeometryPool::upload_mesh` also gained the index-range check it never had, which ruled out corrupt geometry as the cause and now traps the same symptom at its other possible source. |
| 22 | ✅ Complete | **Water surface rewrite.** The water lit itself from a hardcoded `light_dir`, never sampled the shadow map, had a constant `vec3(0.2,0.3,0.4)` ambient and **no environment reflection at all** — Fresnel was computed but only fed the specular lobe. It also drove its colour from the opaque depth buffer via Beer's law, so with nothing under the surface the far-plane reading made `beers_law` 0 and every pixel collapsed to a constant `base_color * 0.2 + deep_color * 0.3`: that, not the lighting, is why open water looked flat and importing a huge car appeared to "fix" it — the car simply gave the water something to be shallow over. Now: the pass binds the real sun, the CSM atlas (same 3x3 PCF as `shading.wgsl`), the Phase 19 environment cubemap and a pre-water copy of the HDR target. Shading is split into **transmitted** (screen-space refraction, per-channel Beer-Lambert absorption, subsurface scattering tint) and **reflected** (prefiltered environment + GGX sun glint), combined by a **view-angle Fresnel** — see-through looking down, mirror at grazing incidence. The BRDF gained its missing Smith geometry term, which let the `* 2.0` fudge go. A far-plane depth reading is now tested explicitly as "no backdrop" (zero transmission, all scattering) instead of being clamped, because clamping still let enough sky through the blue channel to read as a swimming pool. |
| 22C | ✅ Complete | **Sun and indirect light are editable.** Light colour (linear RGB, per channel) joined intensity in the inspector — the sun's colour is the main lever on a scene's mood and was previously unreachable from the editor. Range and cone-angle rows now hide entirely for a directional light instead of sitting there holding meaningless zeroes. `IBL_INTENSITY`, the hardcoded `0.35` stopgap duplicated across three shaders, became `PostProcessComponent::ibl_intensity` with an **IBL** row on the Post Processing entity; it rides in the directional-light buffer's former padding, so every pass that lights anything already binds it. Until SSAO lands, the trade between flat-and-bright and contrasty-and-dark is the artist's to make. |
| 22D | ✅ Complete | **Inspector fields were effectively uneditable.** Every part of the chain worked — click focused the field, keys reached it, the committed value mapped to an `InspectorField` and reached the ECS — but focusing pre-filled the edit buffer with the current value and typing **appended** to it. Clicking a field reading `0.000` and typing `7` committed `0.0007`; exposure `1.000` plus `2` committed `1.0002`. Every edit produced a number visually indistinguishable from the original, which is indistinguishable from a field that does nothing. `NumericField` now has select-all-on-focus: the buffer is still pre-filled so a value can be amended, but the first accepted character (digit, `.` or `-`) replaces it, backspace clears the selection, and the selection is drawn as a highlight so the state is visible. Commits are also suppressed when the parsed value is unchanged, so clicking through fields no longer spams the undo stack. Separately, the transform gizmo only re-synced on selection or its own drag, so typing a position left it stranded at the object's old location — it now follows inspector edits. |
| 22E | ✅ Complete | **Drag-to-scrub on inspector fields** (UE-style: drag right increases, left decreases). Press-and-move on any numeric field scrubs it; a 3-pixel threshold keeps an ordinary click a click, and crossing it drops the text-edit state the press handed over so the field never shows a caret mid-drag. Steps are computed against the value and cursor x captured at press rather than accumulated, so nothing drifts if the app writes to the field mid-gesture. Rates are per-field because inspector values span orders of magnitude — 0.05/px for positions and angles, 0.005 for light colour channels and IBL, 0.0002 for chromatic aberration, whose default is 0.004. Undo: a scrub emits `ValueChanging` per step, which the engine applies straight to the component with **no** undo entry, and one closing `ValueChanged` on release, which pushes a single command rewinding to the pre-drag value — otherwise a 200-pixel drag would leave 200 undo entries. Two supporting fixes: `NumericField::is_text_input` now reports the live edit state instead of a constant `true`, so keys reach the game again once a scrub ends the edit session; and `Focus` is re-sent on every press rather than only when the focused node changes, since a scrubbed field stays the focused node while dropping its edit state and could otherwise never be clicked back into typing. |
| 15D | ✅ Complete | **Meshlet/cluster generation at mesh upload** (`somnium_renderer/src/meshlet.rs`). Every static mesh is split into runs of **128 triangles**, matching UE5 Nanite's `NANITE_MAX_CLUSTER_TRIANGLES`, so 15E and 15F can cull and draw below whole-object granularity. Clusters are stored as an offset and count into the mesh's index range rather than a triangle list, which only works if a cluster's triangles are contiguous — so `build_meshlets` returns a **permuted index buffer** and `upload_mesh` uploads that instead. Triangle order within a draw does not affect the image, so the reorder is free; verified on the imported helmet. Clustering is a **Morton sort of triangle centroids cut into fixed-size runs**, not Nanite's METIS graph partition: the space-filling curve keeps spatial neighbours adjacent in the sequence, which is all the bounding volumes need, and it stays O(n log n), allocation-light and deterministic. Each cluster carries a bounding sphere (AABB-centred, conservative rather than minimal) and a **normal cone** — axis plus cosine cutoff — for rejecting a back-facing cluster whole. Clusters whose normals span more than a hemisphere get a cutoff of `-1.0`, which never culls. Pooled (voxel) uploads are deliberately **not** clustered: chunks are remeshed continuously, so the sort would cost more than the culling saves, and a chunk is already small enough to cull as a unit. Malformed input degrades rather than panics — out-of-range indices, trailing partial triangles and NaN positions drop the affected triangles. 14 unit tests, including a permutation check that the reorder preserves every triangle, a containment check on every bounding sphere, and a locality check that two distant point clouds do not end up in one cluster. |
| 15E1 | ✅ Complete | **Hi-Z depth pyramid + occlusion math** (`pass/hiz.rs`, `shaders/hiz.wgsl`, `culling.rs`). An R32Float mip chain the size of the viewport, rebuilt each frame straight after the visibility pass — the moment the depth buffer holds exactly the opaque geometry that can occlude. Every texel holds the **furthest** depth of the region below it: wgpu depth runs 0 near to 1 far, so the reduction is `max`, and taking the max can only make an occluder look nearer than it is, which errs toward drawing. Level 0 is a compute copy rather than a blit because depth textures cannot be bound as storage. **Odd mip sizes** are the trap — halving 5 gives 2 and the trailing row would vanish from the pyramid, letting a real occluder go unrecorded — so the reduction widens to 3 texels on whichever axis is odd. On the CPU side: `project_aabb_to_screen` (returns `None` when the box crosses or sits behind the eye, where the perspective divide is meaningless — treated as visible rather than guessed at), `hiz_mip_level` (picks the level where the footprint spans at most 2×2 texels, which is what keeps the lookup constant-time regardless of on-screen size), and `is_occluded`. Every ambiguous case resolves toward drawing: a cleared pyramid at 1.0 occludes nothing, and equal depths count as visible so an object cannot cull itself on the following frame. 18 unit tests. **Not yet consumed** — the two-phase cull is 15E2. |
| 15E2 | ✅ Complete | **Two-phase occlusion culling.** The frame graph is now cull(1) → visibility(clear) → Hi-Z → cull(2) → visibility(load) → Hi-Z. Phase one tests frustum then occlusion against the *previous* frame's pyramid; phase two re-tests only what phase one rejected **on occlusion** against the pyramid just rebuilt from phase one's depth, which is what catches geometry that became visible this frame. Reprojecting the previous frame alone would drop geometry the moment the camera moves — the second phase is what makes that safe, since anything wrongly rejected gets a look at fresh depth within the same frame. Two details are easy to get wrong and are handled explicitly: frustum rejects are **not** recorded in the phase-two set (they are still off-screen, and resurrecting them would draw outside the view), and phase two **zeroes** the instance count of everything it is not re-testing, or phase one's draws would be submitted twice. `cull.wgsl` gained a transliteration of the `culling.rs` occlusion math, and `CullPass` gained a per-draw flag buffer, a Hi-Z binding, and one params uniform per phase — both dispatches are encoded before either runs, so a single uniform could not carry two `phase` values. Occlusion is held off until a pyramid exists (`hiz_ready`), because wgpu zero-fills a new texture and zero is the near plane, which would read as "everything is occluded"; a resize resets the flag for the same reason. A layout test pins `GpuCullParams` to 192 bytes with per-field offsets, since a Rust/WGSL uniform mismatch does not fail validation — the shader just reads the wrong words and culls the wrong things. |
| 15E-verify | ✅ Complete | **Occlusion culling measured on a real scene.** The demo has one opaque mesh, so it can neither exercise nor benefit from occlusion culling; the imported `car_scene` (47 nodes, heavily self-occluding) can. Three diagnostics made this measurable: `SOMNIUM_CULL_STATS=1` copies the indirect args back after each cull phase and logs how many draws survived (`instance_count` doubles as the verdict, so counting non-zero entries *is* the submitted-draw count); `SOMNIUM_NO_OCCLUSION=1` keeps frustum culling but skips the Hi-Z half, so the two can be told apart; and `SOMNIUM_IMPORT=<path>` imports a model at startup, since the File → Import dialog cannot be scripted. **Result**, camera against the car body: frustum alone drew 24 of 35, frustum + occlusion drew **17 of 35** — 7 more draws removed, **29% fewer** than frustum alone — and the two screenshots are pixel-identical, which is the correctness half: everything occlusion removed was genuinely invisible. `phase2_drawn` was non-zero on some frames during camera motion, confirming the second phase does catch real disocclusions rather than being dead weight. A useful incidental check: `total=35` rather than 48 because 13 of the car's meshes are `alphaMode: BLEND` and route to the transparent queue, which is not indirect-drawn. |
| 23 | ⏸ Deferred | **GPU culling for the transparent pass.** Alpha-blended draws bypass culling entirely — they go to the Phase 21 forward queue, which is CPU-submitted per object with no indirect args and so no frustum or occlusion test. On the imported `car_scene` that is 13 of 47 meshes, roughly a quarter of the model. Give the transparent queue its own indirect args and cull dispatch, keeping the back-to-front sort (sort first, then build args, so argument `i` still lines up with instance `i` — the same ordering trap that produced the shard artifact). Occlusion has to stay conservative here: blended geometry can be hidden by opaque occluders, but must never occlude itself. |
| 24A | ✅ Complete | **Physical light units and exposure.** `somnium_core::light_units` — directional lights carry **illuminance in lux**, point/spot **luminous power in lumens** (converted to candela at upload), cameras **EV100** from aperture/shutter/ISO with `exposure = 1/(1.2·2^EV100)`. Presets for both (`lux::DIRECT_SUNLIGHT` = 100 000, `lux::FULL_MOON` = 0.05, `lumens::BULB_60W` = 800). **Auto-exposure** (`pass/auto_exposure.rs` + `auto_exposure.wgsl`): a 256-bin log-luminance histogram built with per-workgroup atomics, reduced to a weighted mean, converted to a target EV and adapted per *second* rather than per frame so the rate does not follow the frame rate; the result stays on the GPU and the post-process pass reads it directly. Three separate copies of the sky gradient turned out to exist — `ibl_gen.wgsl`, the HDR clear colour, and `shading.wgsl`'s background branch — and **all three had to become luminances** scaled by sun illuminance (~0.08 cd/m² per lux), or the background sat five orders of magnitude below the scene and rendered pure black. That scaling is also what finally makes **night work**: verified at 100 000 lux (daylight) and 0.05 lux (full moon), the second producing a dark blue moonlit scene rather than an unchanged bright one. 9 unit tests pin the photometry, including that moonlight metered for daylight is black and metered for night is visible. |
| 24B | ✅ Complete | **AgX tone mapping**, implemented analytically in `postprocess.wgsl` rather than as the 3-D LUT the reference ships — a closed-form curve does not justify shipping and binding a KTX2 asset. Rec.709→AgX inset matrix, log2 encode over [−12.474, +4.026] stops, sixth-order contrast sigmoid, outset matrix, then an inverse-sRGB step so AgX's display encoding does not compound with the sRGB target's own. The tone mapper is selectable (AgX / ACES / Reinhard) through `Tonemapper` on `PostProcessComponent`. ACES was fine while the sun was an arbitrary 3.0 and stops being fine at 100 000 lux: it pushes bright saturated light toward the primaries and clips, where AgX desaturates into the highlight the way film does. |
| 24C | ✅ Complete | **Atmospheric scattering (Hillaire 2020).** `shaders/atmosphere.wgsl` + `pass/atmosphere.rs`. A transmittance LUT (256×128, Bruneton's horizon-concentrating parameterisation) and a multiple-scattering LUT (32×32, second order plus a geometric series for every remaining order) are built once at startup — neither depends on the sun or the camera. The sky is then a real ray-march through Rayleigh, Mie and ozone: 32 steps with analytic per-segment integration rather than a Riemann sum. **The three duplicated sky gradients are now one.** `ibl_gen.wgsl` marches the atmosphere into the environment cubemap, and `shading.wgsl`'s background samples that same cubemap, so background, ambient and reflections cannot disagree. Sharp features — sun disc with limb darkening, moon disc, stars — are drawn analytically over the background at screen resolution instead of being baked in: at 256² per face a texel spans ~0.35°, so a half-degree disc smeared into a blob (observed, then fixed). Keeping the sun disc out of the cubemap also removed a double-count, since the shading pass already computes its specular highlight from the analytic light. |
| 24D | ✅ Complete | **Night sky.** Moon disc (0.53°, ~2 500 cd/m²) with a scattered halo, a procedural star field placed one-per-cell so density stays even, and an airglow floor so a moonless night is dark but never identically zero. Night fades in on the sun's **illuminance, not its elevation** — dimming a light and moving it below the horizon are different things, and intensity is the dial the inspector actually exposes; keying off elevation left a starless sky when the sun was turned down to moonlight (observed, then fixed). The environment cubemap already regenerates whenever the sun changes, so ambient tracks it for free. |
| 24AD | ✅ Complete | **Velocity buffer for TAA.** 24F reprojects from depth, which is exact for camera motion and wrong for moving objects: geometry that moves while the camera is still ghosts, limited only by the neighbourhood clip. Needs previous-frame per-instance transforms carried through the visibility pass and a velocity target written alongside. Also unlocks motion blur (24Z). |
| 24W | ✅ Complete | **Water in physical units.** Two faults, both left over from before 24A. The sun was treated as a **point source**, which drives GGX toward a singularity on a near-mirror surface — an unbounded spike across a few pixels, which is what made sunlit water blow out; the lobe is now widened by the sun's angular radius so the spike becomes a glitter path, with an energy term keeping total reflected light unchanged. And a leftover `min(…, 40.0)` on the glint, written when the sun was an arbitrary ~5, was crushing it to nothing against a sky that now measures thousands of cd/m² — removed, since the disc widening bounds the peak on physical grounds instead of by an arbitrary ceiling. The diffuse and scatter terms also had hand-tuned 0.25/0.5 coefficients replaced by the actual Lambert normalisation. Output is clamped below `Rgba16Float`'s finite limit: water is the most mirror-like surface in the scene and therefore the likeliest to overshoot, and an Inf here would reach TAA's blend as NaN. |
| 24X | ✅ Complete | **Screen-space contact shadows.** A shadow map cannot resolve contact: its texels cover centimetres at best, and 24H's normal-offset bias deliberately pushes samples off the surface, erasing precisely the darkening where two surfaces meet. A short ray marched through the depth buffer toward the light fills that gap — visible as grass tufts now sitting on the ground rather than floating on flat colour. A **thickness limit** is what makes it usable: without one every thin object casts an infinitely deep shadow volume behind itself, because the march cannot distinguish a leaf from a wall receding from the camera. The start offset is jittered per pixel so the step pattern becomes noise for TAA to resolve rather than visible banding, and the result only ever *darkens* the shadow-map term, which stays authoritative at its own scale. Parameters follow Bend Studio's screen-space shadows; their wavefront scheduling is **not** ported, only the sampling behaviour. |
| 24Y | ✅ Complete | **Colour grading.** White balance on the orange–blue and green–magenta axes, ASC CDL (slope / offset / power — the standard film grades with), contrast pivoting around middle grey rather than black, and saturation. Applied **after** tone mapping, in display space: grading beforehand fights the curve, whose job is to fit scene luminance into a display's range. Exposure and the tone curve decide how bright the image is and how it rolls off; grading decides what it *feels* like, and no amount of the former substitutes. **File-based 3-D LUTs are not included** — that needs a `.cube` loader and an asset path, and is worth its own sub-phase rather than a stub here; the controls are the part that is usable today. |
| 24Z | ✅ Complete | **Lens realism: depth of field, film grain, dithering.** DoF is driven by the *same* aperture the exposure model already uses, because in a real camera they are one number — opening to f/1.4 both brightens the frame and throws the background out; a renderer that separates them tells a small lie in every shot. Thin-lens circle of confusion against a 36 mm sensor, gathered on a per-pixel-rotated Vogel disk, with a **neighbour test** that only accepts a sample blurred enough to reach this pixel — without it a sharp foreground bleeds over blurred background, the classic tell of a gather-based DoF. Runs **before** bloom so out-of-focus highlights bloom as discs. Grain scales with darkness, because sensor noise lives in shadows and flat grain reads as dirt on the lens. **Dithering is not cosmetic now that exposure is physical**: smooth dark gradients band visibly at 8 bits, and half a bit of noise costs nothing to hide it. **Motion blur landed with 24AD's velocity buffer** — Jimenez's depth and spread weights, Wicked's cheap configuration, before TAA and on HDR. See §17.12. |
| 24AA | ⏸ Deferred | **Cloud shadows.** A scrolling noise mask over the sun's contribution. Cheap, and one of the strongest cues that an outdoor scene is a place rather than a render, because it puts the sky in motion without any volumetric cost. Reference: Spartan's `cloud_shadow.hlsl`. |
| 24AB | ⏸ Deferred | **Lighting debug views.** Per-light-type heatmaps, cluster occupancy, exposure histogram readout, a luminance false-colour view. GI is nearly impossible to debug by eye, and every engine surveyed ships these. Reference: O3DE's `LightCullingHeatmap.azsl`, UE's Lumen visualisation modes. |
| 24AC | ✅ Complete | **FidelityFX SPD and CAS.** Single-pass downsample for the Hi-Z pyramid and bloom chain (one dispatch instead of a pass per mip), and contrast-adaptive sharpening to recover the softness TAA introduces. Reference: Spartan's `spd.hlsl`, `cas.hlsl`. |
| 24E | ✅ Complete | **Sun as a physical disc.** 0.53° angular diameter drives `evaluate_brdf_area`, which widens the specular lobe by the source's angular radius and normalises its energy (Karis' sphere-light approximation). A point source gives a one-pixel highlight on anything smooth, which is among the clearest tells that an image is rendered. The correction is **specular-only** — a first attempt scaled the whole BRDF and would have darkened every lit surface, since diffuse does not care how large a source is. Lights also gained **colour temperature in Kelvin**, one physically meaningful dial replacing three coupled RGB channels; the Planckian fit is sRGB and is decoded to linear before use, which left warm lights far too saturated when skipped. `sun_angular_radius` rides in the light buffer's remaining padding. |
| 24F | ✅ Complete | **Temporal anti-aliasing + specular AA** (absorbs the old Phase 18). Halton-jittered projection; depth-based reprojection; 9-tap Catmull-Rom history sampling (bilinear compounds and goes visibly soft over ~100 frames); Playdead `clip_aabb` neighbourhood clipping with Salvi variance clipping. Blending happens in a **tone-mapped space** — averaging HDR directly lets one bright sample dominate, so a glint flickers rather than resolving, which is the artefact the pass exists to remove. History buffers ping-pong because wgpu forbids binding one texture as both read and write. **Limitation:** reprojection is depth-based, so it handles camera motion exactly but objects that move while the camera is still will ghost until a velocity buffer exists (24AD). Specular AA folds Toksvig normal-map variance back into roughness so mipped detail widens the lobe rather than aliasing. |
| 24G | ✅ Complete | **Sampling infrastructure.** Interleaved gradient noise, Vogel disk (chosen over Poisson tables, which must be shipped and indexed, and over grids, which alias into rings), cosine-weighted hemisphere with Frisvad's branchless basis, R2 and Halton sequences. Shared so the patterns are chosen once — white noise clumps, and clumps survive filtering as blotches. |
| 24H | ✅ Complete | **Shadow quality: PCSS, normal-offset bias, cascade blending.** Blocker search then Vogel-disk filtering, both rotated per pixel by gradient noise, with the search and filter radii driven by the sun's 24E angular size — so a shadow hardens at its contact point and softens with distance from the caster, which a fixed kernel cannot express. Normal-offset bias replaces constant depth bias: offsetting along the surface normal avoids the acne/peter-panning trade entirely. Cascades blend over the last 10% of each range instead of switching, since an abrupt switch shows as a line where resolution and filter width change together. Reference: Spartan's `shadow_mapping.hlsl`. **Contact shadows landed with 24X.** **Completed later**: the normal-offset bias was dimensionally wrong — it used `2.0 / shadow_map_size`, NDC-per-texel across the whole atlas rather than a world distance over one cascade, so about a third of a texel. Half of every surface's samples self-shadowed, showing not as acne stripes but as a uniform ~0.5 shadow factor that flattened real shadows into the wash. Replaced with a slope-scaled depth bias. Cascades are also now fitted from the **un-jittered** matrix; with the jittered one their frusta moved every frame and every shadow edge crawled. |
| 24I | ✅ Complete | **GTAO with bent normals.** `pass/gtao.rs` + `gtao.wgsl`. Phase 17I applied only *baked* occlusion, so terrain, procedural meshes and all foliage received sky light unattenuated — the reason contact points stayed flat and shaded bark read sky-blue. GTAO (Jimenez 2016) rather than classic SSAO: it searches each screen-space slice for its **horizon angles** and integrates the visible arc analytically, producing a real visibility fraction rather than a darkening heuristic — which matters because this term will later feed the GI gather, not just tint the image. Normals are reconstructed from depth, taking the *closer* neighbour per axis: a naive central difference straddles silhouettes and yields normals facing nowhere real. Two slices with per-pixel and per-frame rotation, then a depth-weighted denoise; the residual noise is what TAA is for, which is precisely why 24F was its prerequisite. The **bent normal** is the part that changes indirect light's colour rather than only its amount — the irradiance gather uses it, so a surface in a crevice collects light from the opening rather than from the wall beside it. Screen-space AO **multiplies** the baked term rather than replacing it: the two know different things, and taking the minimum would discard whichever is more informative. Reference: Spartan's `ssao.hlsl`. |
| 24J | ✅ Complete | **Ray-tracing scene: BLAS/TLAS via wgpu acceleration structures.** A bottom-level structure per uploaded mesh and a top-level structure rebuilt each frame from the *same draw queue the raster path uses*, so the traced scene and the drawn one cannot drift apart. Positions are the first 12 bytes of the 32-byte vertex, so `BLAS_INPUT` on the existing pools lets the build read geometry in place — no second copy. The plan's claim held: **the feature gap really was just ray query**, since the binding arrays and non-uniform indexing Solari also needs were already mandatory for the bindless pool. Four things beyond the feature bit were needed and none are obvious: an `unsafe` **experimental-features token** (wgpu asks the caller to acknowledge that these APIs may contain soundness bugs), the `max_blas_*`/`max_tlas_instance_count` **limits**, the `max_acceleration_structures_per_shader_stage` **binding limit** — all three default to zero — and `enable wgpu_ray_query` in the shader. Ships with a ray-traced shadow **acceptance test** (`SOMNIUM_RT_DEBUG=1`), because a correctly built acceleration structure and a silently broken one look identical until something traces against it; it showed the helmet self-shadowing, which is what confirmed the build. Degrades cleanly: every entry point checks whether the device granted ray query. |
| 24K | ✅ Complete | **ReSTIR DI — resampled direct lighting.** The shadow ray from 24J plus the thing that makes rays affordable: *resampling*. Eight unshadowed candidates are drawn across the sun's disc, one is kept in proportion to its contribution by weighted reservoir sampling, and the single expensive ray confirms only that one — the estimator stays unbiased because the kept sample carries the weight of everyone it beat. **Temporal reuse** then combines each pixel's reservoir with its own history, capped at `M_CAP` so a reservoir keeps responding to change rather than fossilising (an uncapped `m` keeps a switched-off light visible for as long as the history has been accumulating). Sampling across the sun's angular disc gives a **real penumbra** rather than PCSS's filtered approximation of one, with no cascades, no depth bias and no peter-panning. Enabled by `SOMNIUM_RESTIR=1`; shading prefers the traced result and falls back to the shadow map when alpha is 0, which is also what an unsupported device produces since wgpu zero-fills the target. **Remaining: spatial reuse** (neighbour reservoirs) and a **multi-light set** — the target function currently evaluates only the sun, where a full implementation would weigh every light's intensity and falloff. **On by default.** Verified against the shadow map on a cube over a plane: 3.0 against 3.1 in shadow, 110.9 lit either way. It was switched off for a while on the reading that it "returned lit" and erased shadows — that was wrong. The missing shadows were the `GpuMaterial` layout bug, which zeroed the sun term so nothing could darken whether traced or mapped. **Remaining, moved to 24L's scope:** spatial reuse and a multi-light target function. **Known limit:** it shadows only visibility-buffer geometry, because terrain and water write depth in their own later passes — Phase 25A/25B closes that. |
| 24L | ✅ Complete | **ReSTIR GI — ray-traced indirect diffuse.** The feature that makes indirect light look like Lumen rather than a constant ambient term: real coloured bounce, contact darkening and light leaking through openings, all fully dynamic with no bake. Reference: `bevy_solari/src/realtime/restir_gi.wgsl`. |
| 24AE | ✅ Complete | **Shadow caster culling.** The shadow pass issued every draw four times, once per cascade: 24.5 ms of a 42 ms frame, nearly all of it grass whose shadow is a sub-pixel speckle. Two independent cuts. Unreal's `r.Shadow.RadiusThreshold` (`ShadowSetup.cpp`) culls a caster when its **projected screen radius** falls below a threshold — a *size* test, not a distance cut, and measured from the camera rather than the light, so a tree keeps casting at 200 m where a tuft stops at 30. And an authored `FoliageComponent::foliage_shadow_distance` (**Sh Dst** in the Foliage inspector, default 40 m), because the size test only rescues you once the camera is far from the grass, which is not how anyone plays. **Measured** at eye level: casters 7 166 → 1 873, Shadows **23.769 → 6.158 ms**, frame 26.893 → 9.545 ms, with every other pass unchanged to the third decimal. See §17.9. |
| 24M | ⬜ Planned | **World-space radiance cache for multi-bounce.** A hashed/clipmapped world cache that rays terminate into, so a single traced bounce still resolves to many bounces of energy across frames, and distant geometry costs a lookup instead of a long trace. Reference: `bevy_solari/src/realtime/world_cache_{query,update,compact}.wgsl`; UE's equivalent is `LumenRadianceCache.usf`. |
| 24N | ⬜ Planned | **Ray-traced reflections with a denoiser** (general specular GI, not water). Screen-space trace first, ray traced where the screen has no answer, radiance cache beyond that, then spatial + temporal denoising. **Water** already has this blend as **Phase VV Halcyon** (VV-A–H in tree: SSR + half-res RT + env cube). This row is still the scene-wide path. Reference: `bevy_solari/src/realtime/specular_gi.wgsl`, `bevy_pbr/src/ssr/`. |
| 24O | ⬜ Planned | **Offline path tracer for validation.** A slow, unbiased, accumulate-over-many-frames reference renderer sharing the 24J scene bindings. Not shipped in the frame loop — its whole job is to be *ground truth*, so “does the real-time GI actually converge to the right answer” becomes a comparison rather than an opinion. Bevy ships exactly this alongside Solari and it is the single best idea taken from studying it. Reference: `bevy_solari/src/pathtracer/`. |
| 24P | ⏸ Deferred | **Software fallback: mesh SDFs + global distance-field clipmap.** For GPUs without ray query. Bake a signed distance field per mesh at upload and composite into a camera-centred clipmap, then cone-trace it for GI and AO. This is Lumen's software path (`LumenMeshSDFCulling.usf`, `LumenSoftwareRayTracing.ush`) and is the more portable but substantially larger implementation. **Deliberately sequenced after the hardware path**, not before it — see §22.2. |
| 24Q | ⏸ Deferred | **Baked light probes: irradiance volumes and reflection probes.** The cheapest fallback tier and still the right answer for static scenes on weak hardware: a grid of SH irradiance probes plus localised reflection cubemaps, blended per object. Reference: `bevy_pbr/src/light_probe/{irradiance_volume,environment_map}.rs`. |
| 24R | ⬜ Planned | **Area lights (LTC).** Rect, disc and tube lights via Linearly Transformed Cosines — analytic, no sampling noise, correct soft shadows and elongated highlights. Softboxes, windows and strip lights are most of what makes an interior read as photographed rather than rendered, and no amount of point-light tuning substitutes. Reference: `bevy_pbr/src/ltc/`, `bevy_light/src/rect_light.rs`. |
| 24S | ✅ Complete | **Transmission and subsurface scattering.** Frostbite's approximation (Barré-Brisebois & Bouchard) rather than a real subsurface solve: light leaving the *far* side of a thin surface, spread by scattering, brightest looking almost straight into the source through the material. **This is what the foliage was missing all along.** Leaves lit only by reflection stay flat and dark regardless of how correct the albedo is — the symptom the grass has shown since Phase 17, and which no amount of albedo or occlusion work could have fixed. Transmitted light is tinted by albedo, which is why backlit foliage reads more saturated than the same leaf lit from the front, and it is deliberately **not** multiplied by the shadow factor: the entire point is light arriving through the surface from the side the shadow map calls dark. Materials take `transmissionFactor` from `KHR_materials_transmission` where present; foliage assets do not set it, so a sidecar cutout mask is taken as evidence of thin geometry and infers 0.5 — the same convention-over-metadata rule the alpha masks and ARM packing already use. `GpuMaterial` grew from 48 to 64 bytes (WGSL rounds the array stride to the 16-byte alignment `base_color` forces); the layout test caught this and was updated rather than deleted. |
| 24T | ✅ Complete | **Emissive materials and physical bloom.** Materials carry `emissiveFactor` and an emissive texture from glTF, added to shading independently of every light in the scene — a screen is as bright in a dark room as a lit one. Bloom is **deliberately not threshold-based**: a threshold asks "which pixels count as bright?", a question with no physical answer whose meaning changes the moment exposure does — a scene metered for night would bloom everything, one metered for noon nothing. Real bloom is light scattering inside the lens, which happens to *all* light in proportion to how much there is. So a progressive 13-tap downsample builds a mip chain and a 9-tap tent upsample sums it back additively (Jimenez, SIGGRAPH 2014); bright regions dominate naturally because they carry more energy. Added **before** exposure and tone mapping, since it is scattering on the way to the sensor rather than a filter over the picture, and built **after** TAA, because a blur of unstable input broadcasts that instability everywhere it reaches. `GpuMaterial` grew 64 → 80 bytes; the layout test was updated again. |
| 24U | ✅ Complete (shafts still unseen) | **Volumetric fog, aerial perspective and light shafts.** A froxel volume accumulating in-scattering per depth slice, fed by 24C's aerial-perspective LUT so distant hills desaturate correctly and the sun throws real shafts through the canopy. Among the highest perceived-realism-per-line-of-code in the whole phase. Reference: `bevy_pbr/src/volumetric_fog/`. |
| 24V | ✅ Complete | **Local lights in physical units, with source radius.** The photometric half landed with 24A-1 — point and spot lights carry lumens converted to candela, and `smooth_distance_attenuation` already divides by distance squared, so illuminance was correct. What was missing is that they were still **point** sources. Lights now carry a `source_radius` in metres (distinct from `range`, which is reach): a 5 cm bulb a metre away subtends a real angle, and feeding that through `evaluate_brdf_area` is what stops its highlight being a single pixel on anything polished. **IES profiles are not included** — that is an asset-pipeline job and is better as its own sub-phase than half-done here. |
| 15F | ✅ Complete | **Meshlet rendering path.** A draw is now one indirect argument per **cluster**, so frustum, Hi-Z and backface tests all work at 128-triangle granularity instead of per object — 530 cull units where there were 35. `first_vertex` carries the cluster's index offset within its mesh, because the vertex shader adds `instance.index_offset` itself; `first_instance` carries the owning instance, which is also what the cull shader now reads to find the model matrix, since the draw index no longer *is* the instance index. Meshes with no clusters (voxel chunks) stay a single whole-mesh argument, so one pipeline serves both. **The subtle break:** the fragment shader keyed the visibility buffer on `@builtin(primitive_index)`, which restarts at 0 every draw call. Splitting a mesh across many draws would have sent the shading pass to the wrong triangle in every cluster after the first. The triangle id now comes from `vertex_index / 3` in the vertex shader — `vertex_index` includes `first_vertex`, so it is mesh-relative, and all three vertices of a triangle divide to the same value. Cone culling rejects a whole cluster when every triangle in it faces away; it is only sound because the visibility pass culls back faces, and it is skipped for mirroring transforms whose negative determinant would flip the stored axis. **Measured** on the imported car at a fixed viewpoint: whole-mesh draws submitted 21 782 triangles, clusters **16 220** — 25.5% fewer — with opaque geometry pixel-identical (0.00% on the car body, 0.06% on the helmet silhouette; the rest of the frame differs only where the time-animated water is). |
| 15F-fix | ✅ Complete | **Cluster bounds use the box, not the sphere.** The first 15F measurement showed the cluster path submitting **2.1% *more*** geometry than whole-mesh draws. Cause: `push_cluster_args` culled against the bounding *sphere's* AABB, which is up to √3 wider per axis than the cluster's real box and can reach outside the parent mesh's bounds — so boundary clusters survived frustum tests their whole mesh failed, and cluster culling was not the strict refinement it should be. `Meshlet` now stores the local AABB alongside the sphere and culling uses the box. Same viewpoint, same scene: 174 clusters drawn → 127, and a 2.1% regression became a 25.5% improvement. |
| skipped-frame-fix | ✅ Complete | **Double submission after a dropped frame.** Found while reading cull statistics: exactly one frame in 3 914 reported twice the expected draw count. The surface-acquisition failure path returned early without emptying the per-frame queues, so the next frame appended to them and submitted everything twice. Invisible for opaque geometry — same pixels, same depth — which is why it went unnoticed, but it double-blends the transparent pass and wastes a whole frame of work. Queue clearing moved into `clear_frame_queues`, called on every path out of `render`. |
| 17A | ✅ Complete | **Foliage scattering** (`terrain/foliage.rs`). Placement is a **jittered grid**: the terrain is cut into cells sized so one instance per cell hits the requested density, and each cell contributes one candidate placed randomly inside it. That is stratified sampling — even coverage without the clumps and bald patches of independent uniform sampling, at a fraction of the cost of Poisson-disc. Every candidate is derived by hashing its cell coordinate with the seed, so nothing depends on iteration order or carried RNG state: re-scattering gives an identical layout, which matters because the list is rebuilt on every sculpt stroke and foliage that reshuffled mid-edit would be unusable. Candidates are rejected on **slope** and on the **splat layer** beneath them, so grass follows the paint and stops at cliffs. The instance ceiling is enforced by **coarsening the grid**, never by stopping partway through it — truncation would pile everything into the first corner visited and leave the rest bare (there is a test for exactly that). Instances are deliberately not ECS entities, for the same reason voxel chunks are not: thousands of them, regenerated constantly, would flood the outliner and undo stack. They go out as ordinary draw commands, which also means they inherit the whole Phase 15 pipeline — indirect draws, frustum, Hi-Z and per-cluster culling — without foliage knowing any of it exists. `TerrainData` gained an `edit_revision` counter so the scatter is rebuilt only when settings or the terrain actually change. Mesh is a solid tapered-prism tuft (`generate_foliage_tuft`) rather than the usual alpha-tested crossed billboards: the visibility pass culls back faces, so a flat quad would vanish from one side, and `alphaMode: MASK` is imported but not yet cut out in the shader. **F8** toggles foliage on the selected terrain until the layer UI lands. Verified in the editor: 10 876 instances over a 1024x1024 m terrain. Two look fixes came out of that first render — albedo had to drop to ~0.05 because a sun of intensity 5 pushes anything brighter past 1 and tone-maps it to white, and the blade normals had to lean outward instead of nearly straight up, or every blade caught identical light and the tuft read as a flat smudge. 19 unit tests. |
| 17B | ✅ Complete | **Terrain colliders** (`terrain/collider.rs`, `jph_heightfield_shape_create`). Jolt's `HeightFieldShape` needs a square power-of-two sample grid; a Somnium terrain is `chunks * cells + 1` vertices per side (513 by default), so the heightmap is resampled rather than handed over. Resolution rounds **down** to a power of two so the collider is never finer than the mesh it approximates, and caps at 512 — 262 144 samples, which still resolves every 2 m over a kilometre, while 1024 would quadruple the rebuild cost for detail no rigid body can feel. Rebuilds are gated on `TerrainData::edit_revision`, so a sculpt stroke costs one tree build, not one per frame. Two robustness fixes came with it: `PhysicsWorld::create_body` now returns `BodyId::INVALID` when a shape fails to build instead of handing Jolt a null pointer and tripping an assert inside the body interface, and non-finite height samples are replaced before they can poison the tree build. Verified by a stepped simulation, not a mock: a sphere dropped from 8 m rests on a flat field, a sphere on a ramp's high end rests above 5 m (so the height data is genuinely honoured), and malformed fields — non-power-of-two, or a sample buffer shorter than declared — are refused rather than read past the end of. |
| 17C | ✅ Complete | **Terrain layer + foliage inspector.** Two new sections, shown only when the selection has the matching component. **Terrain**: active paint layer plus per-layer texture tiling, which reach into renderer-side `TerrainData` and so bypass the undo stack exactly as sculpting already does. **Foliage**: an Enabled toggle plus density, seed, max slope, layer, and scale range — editing any of them lets the scatter cache notice the component changed and re-scatter on the next frame, with no explicit invalidation. Drag rates are per field again: density lives well under 1 per square metre, and layer indices step in whole numbers. Density is clamped to 4/m² because a square-kilometre terrain at more than that is millions of candidates. F8 stays as a shortcut for the Enabled toggle. |
| 17D | ✅ Complete | **Alpha cutout and double-sided materials** — the two things the engine lacked before real foliage assets could render. **Cutout** happens in the *visibility* pass, not shading: the visibility buffer decides what exists at each pixel, so a leaf's cut-away corners have to be discarded before the depth buffer records a solid quad. That meant giving the pass the albedo textures and a sampler it never had. Derivatives are taken at top level and fed to `textureSampleGrad`, because sampling inside the per-material branch would break WGSL's uniformity rule and dropping to LOD 0 instead would make distant foliage crawl. Only `MASK` cuts out — `OPAQUE` alpha channels are routinely meaningless and clipping `BLEND` would leave hard edges through glass — and a `MASK` material with an unusable cutoff falls back to glTF's 0.5 rather than 0, which everywhere else means "no cutout". **Double-sided** adds a second visibility pipeline with culling off; a leaf card is one flat quad and vanishes entirely from one side otherwise. Draws are partitioned single-sided-first with the boundary recorded, and the pass issues one `multi_draw_indirect` per range. Argument order no longer needs to match the draw queue, because `first_instance` carries the instance explicitly and the cull shader reads it from there. The shading pass flips the geometric normal toward the viewer for flagged materials only — doing it unconditionally would light the inside of closed geometry. **glTF `TANGENT` import turned out to be unnecessary**: the shading pass already derives tangents from triangle edges and UV deltas, so normal maps work on any mesh without vertex tangents. |
| 17E | 🟡 Partial | **Real foliage assets** — four CC0 Poly Haven models (~101 MB, 1.53 M triangles) in `assets/foliage/`, chosen on measured size and triangle count: `fir_tree_01` is 486 MB / 7 M tris and `pine_tree_01` 937 MB / 17 M, both larger than the entire geometry pool was. Engine work this forced out: the pool grew from 64/32 MB and now **sizes itself to the device's `max_storage_buffer_binding_size`**, since these are storage buffers for vertex pulling and exceeding the limit is a validation error at *bind-group* creation, so it surfaces as a first-frame crash rather than a clean failure; a **capacity guard** refuses an oversized mesh with an error instead of moving the bump pointer past the end and corrupting every later mesh; the **shadow pass gained a fragment stage** purely so alpha-tested geometry can `discard`, as it was depth-only and cast the shadow of every card's whole quad; and foliage `BLEND` materials are **re-tagged to `MASK`** so they take the opaque path — left blended, thousands of instances go through the sorted forward pass with no depth write, no shadows and no GPU culling. Scattering also became a **disc that follows the camera**: blanketing a square kilometre at a believable density is millions of instances, while a 45 m disc gives 18 000. Cell indices stay absolute, so instances do not reshuffle as the camera moves — there is a test for that. **Open issue:** the grass geometry, scatter, placement and shadows are all correct, but it shades grey-blue rather than green. Ruled out: shadow-quad self-shadowing (fixed, no change), the double-sided normal flip (now flips toward the sun, no change), and alpha cutout (these models are modelled blades with JPEG textures and carry no alpha at all). It looks like a material-channel problem — next step is to output albedo, normal and the ARM channels directly from the shading pass rather than guessing again. | **Three reconstruction bugs found and fixed while trying to make this look right, none of them in the foliage code**: (1) the visibility buffer packed instance and primitive ids into one `R32Uint`, capping meshes at 65 536 triangles — the island tree's 714 000-triangle leaf mesh wrapped and shaded from unrelated triangles, which is the shattered-atlas look; now `Rg32Uint` with a channel each and no cap. (2) The TBN guard `1.0 / (tbn_det + 1e-7)` does not rescue a degenerate triangle, it manufactures a huge `inv_det` and `normalize` returns NaN — a NaN normal reflects the environment map, hence flat facets of sky blue. Degeneracy is now detected and the normal map skipped when the frame is arbitrary. (3) The bitangent had no handedness term, so mirrored UV islands — routine on bark — inverted the normal map's green channel, giving hard-edged dark patches following UV seams. **Still to do for photorealism**: hemispherical leaf normals so cards stop shading like flat plates, transmission (24S) reaching foliage materials for backlit leaves, and bark roughness.
| 17F | ✅ Complete | **Foliage painting** (`terrain/foliage_paint.rs`). 17A filled the whole terrain the moment foliage was switched on, which is the wrong model for authoring — an artist wants grass in the meadow and trees on the ridge, not everywhere at once. A terrain now starts **bare**, and instances are painted. The key design point is **spacing, not per-dab count**: a stroke fires many times a second over the same ground, so "add N per dab" would pile thousands on one spot. Every candidate must instead clear a minimum spacing from what is already there, derived from the density so the two cannot disagree (`1/sqrt(d)` for a packed layout). Painting over covered ground becomes a no-op and a held brush converges instead of growing without bound — there is a test that holds the brush for 40 dabs and asserts the count stays under the area limit. Spacing is **per palette entry**, so grass can be painted under a tree. Candidate radii are `sqrt`-distributed or the brush paints a hot spot in the middle, which is also tested. **Single** mode places one instance at the cursor, which is how trees go down, and repeated clicks do not stack. Erase can target one entry or clear everything. Placement is hash-driven off a stroke counter rather than a live RNG, so a recorded stroke sequence replays identically — what undo and scene reload will need. The palette is four fixed CC0 entries for now, each loaded lazily the first time it is painted, since loading four scanned models up front would add seconds to startup for meshes that may never be placed; a content drawer replaces this later, which is why the brush stores a palette *index* and nothing about the mesh. UI is a Foliage section with Enabled / Paint Mode / Erase / Single toggles and brush size, density, type, scale range and slope limit. 14 unit tests. |
| 17F-fix | ✅ Complete | **Strokes landed a terrain-width away, no brush ring, no type picker.** `TerrainData::raycast` marches in terrain-local space but transforms the result back to **world** before returning; painted instances are stored **local**, because `submit_foliage` composes them with the terrain's transform. Storing the world hit as if it were local applied the terrain's `-512` offset a second time and dropped every stroke off the mesh entirely. The sculpt brush was unaffected because it feeds the same world-space hit to the shader, which also works in world space — so the two conventions had been quietly coexisting. Diagnosed by logging the hit and reading `(-1.2, 3.3)` where a local coordinate has to be in `0..1024`. The **brush ring** now shows for foliage too (amber, `brush.w = 3`): the cursor updater bailed out unless *sculpt* mode was active, so the foliage brush painted blind — which is what let the offset go unnoticed. The **type picker** is a named popup mirroring the Create menu, replacing a numeric index nobody should have to decode; selecting a tree also flips **Single** on, since trees want one-per-click and ground cover wants a spread. The old numeric row is kept for field routing but its whole row is hidden — hiding just the field left a stray "Type" caption, because the label lives in the row. |
| 17G | ✅ Complete | **Foliage performance, distance culling, and a usable type picker.** The slowdown was self-inflicted by Phase 15F: a 6 422-triangle grass tuft expands to **51 indirect arguments**, so 2 000 painted instances meant 102 000 draws and 6.3 MB of arguments and cull bounds uploaded *every frame* — to cull sub-parts of things a few pixels across. Clustering pays for a large mesh drawn once, and is backwards for a small mesh drawn thousands of times. The renderer now counts how often each mesh appears in the frame and drops back to a single whole-mesh argument past 8 copies, cutting a painted field's draw count by ~51x. **Distance culling** rejects instances beyond `cull_distance` (120 m) while the submission list is built, so they never reach the instance buffer or the indirect arguments at all — the GPU cull cannot do this, because a draw has to exist before it can be rejected. The test is horizontal distance, so flying up does not make ground cover vanish from under the camera. The per-frame submission vector is also reused rather than reallocated. **UI:** the Foliage section moved to the top of the inspector, since at the bottom it fell behind the output log and the type button was literally unclickable. The type control became an in-place cycler (`Type: Fir Sapling >`) rather than a popup — this UI has no anchoring, so a floating popup has to be hand-positioned and kept landing somewhere unhelpful; for a handful of entries, cycling cannot be occluded or mispositioned. Picking a tree still switches Single on automatically. |
| 17H | ✅ Complete | **Cutout foliage: alpha masks, alpha-weighted mips, and the island tree.** Three reported faults, three unrelated causes. (1) *Everything looked blue-grey.* Poly Haven ships vegetation as **alpha-cutout cards** — the diffuse atlas carries blade colour only where the mask is opaque (78% of `grass_medium_01` is near-black) and the cutout lives in a **separate `_alpha_` map their glTF never references**. Trusting the glTF meant rendering the black background as if it were the plant, leaving ambient sky as the brightest thing on screen. The loader now folds a sibling `X_alpha_2k.png` into `X_diff_2k.jpg`'s alpha channel by filename convention and promotes the material to `MASK` + double-sided; a missing sidecar is not an error. (2) *Saplings had no trunk.* `ensure_palette_mesh` kept the largest **primitive**, but a glTF node is usually several — the sapling is `branches` + `twigs`, the island tree is trunk + branches + leaves. Primitives are now grouped by node transform, the heaviest **node** wins, and all of its primitives are kept as `FoliagePart`s with a local transform. (3) *The island tree painted nothing.* Not the triangle cap, as assumed: the file lists `KHR_texture_transform` in `extensionsRequired` and the `gltf` crate rejected the import outright. Enabling the feature fixed it; failed imports are now cached so a broken model no longer retries — and stalls — on every brush dab. **Mip generation** was also wrong for cutouts: a plain box filter averages blade colour with the transparent background, so foliage darkened with distance, and averaging a binary mask drops texels under the 0.5 cutoff so coverage erodes until distant grass vanishes. Colour is now averaged **weighted by alpha**, and each level's alpha is rescaled to preserve coverage (Castaño). Both reduce exactly to the old behaviour for opaque textures. |
| 17I | ✅ Complete | **Ambient occlusion reaches indirect light.** The IBL term had a standing note that nothing attenuated sky light, and it showed on foliage: grass albedo is a dark olive, so an unoccluded sky reflection's 4% Fresnel sheen was a large share of each blade's colour — and the sky is blue. `Surface` now carries an `occlusion` term applied to indirect diffuse, and to indirect specular through Lagarde's specular-occlusion fit, never to the sun (which already has shadow maps). Sourcing it needed two attempts: reading AO from the metallic-roughness map's red channel rendered the damaged helmet **pitch black**, because glTF leaves that channel undefined and models with a separate AO texture leave it at zero. Occlusion now comes from the material's own `occlusionTexture` — plus one narrow inference: exporters that pack ARM (AO/Roughness/Metallic) have no way to declare it and simply leave `occlusionTexture` unset, so an `_arm` filename is taken as stating the packing. That is the same convention-over-metadata rule the `_alpha_` sidecars use, and it is scoped to the filename so a plain metallic-roughness map is never misread. `occlusion_map` took over the material struct's padding word, so the GPU layout is unchanged. |
| 16 | ⏸ Deferred | Scripting (Rhai or Lua) |
| 25A | ✅ Complete | **Terrain into the visibility buffer.** Terrain records at `renderer.rs:1516`, *after* the visibility pass (1386/1408), GTAO (1458) and ReSTIR (1443). It therefore misses every one of them, and `terrain.wgsl` carries its own duplicated copies of the shadow and cluster helpers — so each lighting improvement in Phase 24 had to be written twice or silently skipped terrain. This is the same failure as 24C's sky-in-three-places, and the fix is the same: one source. Terrain writes depth and visibility IDs in the pre-pass like everything else, and the duplicated helpers are deleted. **Unblocks GTAO, contact shadows, ReSTIR and correct TAA on terrain in one change, and is what makes 24K verifiable at all.** Reference: O3DE keeps a dedicated `Terrain_DepthPass.azsl` feeding the shared depth buffer rather than a self-contained terrain pass. |
| 25B | ✅ Complete | **Terrain chunks in the TLAS.** 25A-2 already put chunks in the draw queue the acceleration-structure build reads, so they reached the TLAS loop and were skipped for having no BLAS; 25B registers one per chunk. The architectural half came from `bevy_solari/src/scene/blas.rs`, which builds a bottom-level structure only for geometry that was **added or modified** and rebuilds the top level alone each frame — Somnium had been reissuing *every* BLAS every frame, invisible with a handful of meshes and untenable at 256 chunks. `RaytracePass` gained a `pending_blas` list, stores the size descriptor and offsets each BLAS was built with, and gained `mark_geometry_dirty` for the case Bevy has no equivalent of: a chunk's *contents* changing under a stable allocation when sculpted. BLAS geometry is always the full-detail unstitched `(lod 0, mask 0)` range, never the frame's LOD — a BLAS is sized once at creation, and a traced shadow that changed shape as chunks swapped LOD would be worse than one finer than what is drawn. **Verified** with `SOMNIUM_RT_TERRAIN=0/1`: 6 945 terrain pixels move by 17.998 mean absolute luminance, TLAS instances go 1 → 17, and mesh and sky come back bit-identical — the only new occluder is terrain, so this is terrain shadowing terrain. That is 24K's acceptance test, which is why 24K closes with it. See §25.3d. |
| 25M-2 | 🟡 Automated complete | **Sunset & Night Sky Visual Fixes, audited 2026-08-11.** **(A)** The authoritative `PostProcessComponent` default is now `ibl_intensity = 1.0`. **(B)** CSMs include a 1 km caster-depth extension patterned after Flax's extended CSM culling range; receiver bias uses the true triangle plane, and contact-shadow thickness is compared in linear view-space metres instead of nonlinear NDC depth. **(C)** Stars use 3×3×3 neighbour evaluation, smooth angular falloff, a magnitude distribution and Milky Way concentration. **(D)** The lunar orbit has a 29.53-day synodic period and 5.14° inclination; the disc uses the real 0.2666° angular radius, tangent-plane sphere normal, phase lighting and limb darkening, with default illuminance tuned to 0.010 lux. **(E)** Palette vegetation retains its foliage/double-sided/transmission semantics, faces its geometric normal toward the viewer, uses wrapped backside transmission without an ambient albedo glow, and has a roughness floor. Moon BRDF evaluation no longer applies N·L twice, and ReSTIR GI invalidates materially changed light history, rejects unsupported emissive hits, and falls back to night IBL when sunlight is zero. Automated tests pass; night appearance is user-confirmed, while the daytime shadow correction awaits the same on-screen confirmation. |
| 25C | ⬜ Planned | **CDLOD vertex morphing.** `terrain/mesh.rs` builds discrete per-LOD index topology with edge stitching, so an LOD switch swaps geometry in one frame and pops — most visible exactly where it is least wanted, on a ridge line against the sky. CDLOD morphs vertices toward the coarser level's positions across the last part of each range, so the transition is continuous and the switch happens when the two meshes already agree. Reference: `CDLOD-master/source/BasicCDLOD/Shaders/CDLODTerrain.vsh` — `morphVertex`, `g_morphConsts`. |
| 25D | ✅ Complete (macro tier + detail budget; no toroidal clipmap — see §25.13) | **Macro + detail clipmaps.** One splatmap over the whole terrain sets a hard ceiling on texture detail: enough resolution close up means an impossible texture far away. O3DE's answer is two tiers — a *macro* clipmap covering the entire terrain at low frequency for colour and large-scale variation, and a *detail* clipmap of a few rings centred on the camera carrying full-rate PBR, composited per pixel. Detail cost then scales with screen area rather than world area. Reference: `TerrainMacroClipmapGenerationPass.azsl`, `TerrainDetailClipmapGenerationPass.azsl`, `ClipmapComputeHelpers.azsli`. |
| 25E | ✅ Complete | **Height-weighted material blending.** The current shader sharpens splat weights, which is halfway there. O3DE's `AppendHeightToWeight` adds each material's own height map into its weight before normalising, so gravel settles *into* the cracks of rock instead of being averaged across it — the difference between two textures cross-faded and two materials meeting. Reference: `TerrainDetailHelpers.azsli`. |
| 25F | ✅ Complete | **Stochastic hex-tiling.** **On by default since 25K.** Shipped off at first because against procedural layers there was no repetition to remove and it only showed its own lattice; with photographed layers it removes the banding outright. Ported from `bgfx-master/examples/49-hextile/fs_hextile.sc` into `shaders/hextile.wgsl` — simplex grid, hashed per-vertex offsets, three `textureSampleGrad` taps with per-tap derivatives, luminance-modulated sharp weights — plus one thing the reference does not need: **counter-rotating each tap's tangent-space normal**, since a normal map stores its vector in the texture's UV frame and each tap read that frame rotated. Rendered side by side, the plain path shows *no findable grid* while the hex-tiled one shows its own lattice faintly: the four layers are procedural, tileable, low-contrast noise, so there is no repetition to remove. Re-judge once **25D**/**25J** bring photographed layers. Two traps recorded: naga's SPIR-V backend **segfaults** if a texture is pulled out of a binding array and passed across a function boundary, and the reference's own default rotation strength is **0** — at 1.0 the lattice showed as hard triangular seams. See §25.3e. |
| 25G | ⬜ Planned | **Biplanar upgrade for cliffs.** Triplanar projection already runs on steep slopes, but it costs three sample sets per map. Biplanar takes the two dominant axes instead of three, at close to the same quality for two thirds of the taps — which matters once 25D and 25F have multiplied the sample count. Reference: `bevy-plugins/bevy_triplanar_splatting-main/src/shaders/biplanar.wgsl`. |
| 25I | ✅ Complete | **Aerial perspective on terrain.** 24C builds the LUT and terrain does not sample it, so distant hills stay saturated while everything else desaturates correctly — which reads as a matte painting behind a rendered scene. Cheap once 25A has terrain in the shared shading path. |
| 25J | ⬜ Planned | **Terrain material UI and colliders** (absorbs the old Phase 17 remainder). Per-layer tiling, tint, roughness and height-blend strength in the inspector, plus a collider built from the committed heightmap so gameplay and physics agree with what is drawn. |
| 25M | 🟡 Mostly complete | **Night, twilight and the sun below the horizon.** Rotating the sun below the horizon turns the terrain red with black blotches and bleaches the foliage. Confirmed cause: `ray_intersects_ground` exists nowhere in the engine, so a sun below the horizon still samples the transmittance LUT and clamps to its reddest row instead of switching off. Port the guard from `bevy_pbr/src/atmosphere/functions.wgsl`, gate the direct term on `max(mu_sun, 0)`, add a twilight ramp, then re-check exposure and ReSTIR fireflies against measurement. Also gives 24U the low-sun scene its light shafts have never been verified in. See §25.14.
| 25N | ⬜ Planned | **Analytic gradients for visibility-buffer shading.** Foliage is blurry and aliased at once because `shading.wgsl` samples mesh textures with `textureSample`, whose implicit derivatives are taken across a 2×2 quad that routinely straddles different triangles and instances — so the mip level is arbitrary per pixel. Terrain escapes it by already using `textureSampleGrad`. Fix: evaluate the triangle’s barycentric at the neighbouring pixels analytically and difference the UVs, as Wicked’s `surfaceHF.hlsli` does with `bary_quad_x`/`bary_quad_y`. See §25.14.
| 25P | ⬜ Planned | **Foliage instancing and LOD.** A scene with trees and grass submits **9 047 draws / 90.9 M triangles**, with Visibility (phase 1) at 9.25 ms and Shading at 7.44 ms of a 23.5 ms frame. `submit_foliage` pushes one draw per part per instance and there is no foliage LOD at all. Batch identical parts into instanced draws first (a submission change, no shaders), then mesh LODs by projected screen radius reusing 24AE’s ratio test, then impostors. See §25.14.
| XV | ✅ A–J complete | **Phase XV — Appalachia.** 32 global photogrammetry PBR layers, eight splatmaps, strongest-four, unique-colour macro, full-PBR biplanar cliffs, Terrain Paint vs Foliage Paint, biome v3 / landscape v4, aerial hex/POM LOD (`gpu_material_for_camera`, 80 m). Live look signed off 2026-08-13. **XV-J** closed the same day: compile gate + `phase_XV-J_*.png` corpus + wgpu freeze (RTX 5080 Laptop, Vulkan, driver 610.74). Release overview shading **3.951 ms**, walk **5.532 ms** (1.10 ms budget is an explicit exception). BC7 encoder ships (`encode_terrain_bc7`); local packs load at 2048+1024 (~213 MiB, `compressed=true`). Visual A/B: `dev records/phase XV/evidence/XV-BC7_visual_check.md`. Plan: `dev records/phase_XV.md`. Live contract: `dev records/phase XV/XV-Zeta_plan.md`. Evidence: `dev records/phase XV/evidence/XV-J_compile_gate.md`. Do not rewrite §20 (Phase 14) as if it were XV. |
| 26 | 🔧 open | **Phase 26 — Metaphor.** 26-A–I shipped 2026-08-13 (toolkit, Nocturne shell, docked Content Drawer tiles, Details/Outliner, Iris, `UiCanvas`, palette/toasts/HiDPI/layout persist/unsaved, custom title bar, wrapped Help, button hover/press, visible scrollbars). Evening polish: immersive play, 80 px drawer tiles, ComboBox root-popup overlay, toolbar Select/Landscape/Foliage wiring. 26-H SDF slipped (supersampled bitmap Inter). **Phase remains open:** new engine features keep needing new UI/UX. 26-J not started. Contract: `dev records/phase_26.md`. |
| VV | 🔧 A–H + VV+1 | **Phase VV — Halcyon: ray-traced water reflections.** **Start-here:** `dev records/halcyon_context_handoff.md`. Water G-buffer prepass + half-res RT compute + shade blend with SSR on confidence. Shared `rt_hit.wgsl` (GI wraps `rt_trace`). Kill switch `SOMNIUM_RT_REFLECT=0`. Inspector: water **RT Reflect** / **Reflect Debug**; Post FX **RT Reflections**. **VV+1 refraction** in the same compute pass (array layer 1), **default off** (Post FX **RT Refraction**; `SOMNIUM_RT_REFRACT=0`). Live SSR miss-rate capture not yet in `dev records/phase VV/`. Plan: `dev records/phase_VV.md`. |

---

## 17.5 Phase 25 — Photorealistic Terrain (plan)

### 25.1 Why terrain is the weakest surface in the engine

Terrain already does more than it gets credit for: four PBR layers blended from a
splatmap, height-sharpened weights, triplanar projection on cliffs, CSM shadows
and clustered local lights. On its own that is a respectable terrain shader.

The problem is not the shader, it is *where it runs*. `TerrainPass::record` is at
`renderer.rs:1516` — after the visibility pass (1386 and 1408), after the
acceleration-structure build and ReSTIR (1419–1443), after GTAO (1458). Terrain
is therefore invisible to all of them, and `terrain.wgsl` keeps its **own copies**
of `sample_shadow`, the cascade selection and the cluster lookup.

Two consequences, and the second is the expensive one:

1. Terrain receives no GTAO, no contact shadows, no traced visibility, and gets
   TAA reprojection from a depth buffer it never wrote to.
2. **Every lighting improvement in Phase 24 either had to be implemented twice or
   quietly skipped terrain.** That is not a terrain bug, it is a structural tax on
   all future lighting work.

This is precisely the fault 24C fixed for the sky, where the same constants lived
in three files and night was impossible until they became one. The resolution is
the same: **one shading path, one source of truth**, which is why 25A leads.

### 25.2 What the reference engines actually do

| Engine | Taken | Where |
|---|---|---|
| **O3DE** | Macro + detail **clipmaps** — whole-terrain low frequency plus camera-centred high frequency, so detail cost scales with *screen* area, not world area. Also `AppendHeightToWeight`, which folds a material's own height into its blend weight. | `Gems/Terrain/Assets/Shaders/Terrain/` |
| **O3DE** | A dedicated terrain **depth pass** feeding the shared depth buffer — the architecture 25A adopts. | `Terrain_DepthPass.azsl` |
| **CDLOD** | `morphVertex` — continuous LOD morphing so a level switch happens only once both meshes agree. | `BasicCDLOD/Shaders/CDLODTerrain.vsh` |
| **bgfx** | **Hex-tiling** (Heitz–Neyret): three lattice-offset samples with randomised rotation, blended barycentrically, to destroy visible tiling. | `examples/49-hextile/fs_hextile.sc` |
| **Unreal** | The same hexaplanar idea, independently, in TextureGraph — confirmation this is the standard answer rather than a trick. | `Plugins/TextureGraph/Shaders/Layer/AdjustHexaplanar*.usf` |
| **Bevy** | **Biplanar** projection: two dominant axes instead of three, near-identical quality for two thirds of the taps. | `bevy_triplanar_splatting-main/src/shaders/biplanar.wgsl` |
| **bevy_terrain** | Compute-driven tile refinement into indirect draws — noted, not adopted; the existing chunk/LOD system already fills this role and replacing it would be churn, not quality. | `src/shaders/tiling_prepass/refine_tiles.wgsl` |

### 25.3 Sequencing, and why 25A and 25B come first

25A and 25B are not photorealism features. They are sequenced first because:

- **They unblock 24K.** ReSTIR reads the visibility-buffer depth and traces the
  TLAS. Terrain is in neither, so there is currently no surface in the demo that
  can *show* a traced shadow — the ground filling the frame is the water plane,
  which shades in its own pass and never samples `restir_vis`. A hill shadowing
  the valley beside it is a test that cannot pass by accident, unlike the
  cube-on-a-plane scenes that failed twice.
- **They stop the duplication tax** before 25D–25H add substantially more shader
  surface that would otherwise have to be written twice.
- **They are worth more than they look.** GTAO, contact shadows and correct TAA
  arriving on terrain at once is a visible change on its own, before any new
  texturing work lands.

Ordering by visible gain per unit of work after that: **25F** (repetition is the
loudest artefact and hex-tiling is a self-contained shader change), then **25E**
(materials meeting instead of cross-fading), then **25D** (the resolution
ceiling), then **25C** (popping), then **25H**/**25G**/**25I** as refinement.

### 24.9 Phase 24 scope

Narrowed to a near-term set plus the GI chain. Everything dropped is deferred,
not cancelled — the plans stay in the table above.

**Near-term, in dependency order:**

| | Why it is in scope |
|---|---|
| **24AD** velocity buffer | The last genuine gap in 24F. Depth reprojection is exact for camera motion and wrong for moving objects, which ghost. Also the only thing blocking 24Z. |
| **24Z** motion blur | Everything else in 24Z shipped; motion blur was always waiting on 24AD. |
| **24U** volumetrics | Fog, aerial perspective and light shafts. The highest perceived realism per line of code left in the phase. |
| **24R** area lights (LTC) | Rect, disc and tube lights. Self-contained, no dependencies. |
| **24AC** SPD + CAS | Single-pass downsample for Hi-Z and bloom, plus contrast-adaptive sharpening to recover what TAA softens. Small and independent. |

**Also in scope, the GI chain** — large and strictly sequential, so it is its own
run rather than something to interleave:

| | |
|---|---|
| **24L** ReSTIR GI | Ray-traced indirect diffuse. Real coloured bounce, contact darkening, light leaking through openings, fully dynamic with no bake. Builds directly on 24K's reservoirs. |
| **24M** world radiance cache | Rays terminate into a hashed/clipmapped world cache, so one traced bounce still resolves to many bounces of energy across frames. |
| **24N** RT reflections + denoiser | Screen-space first, traced where the screen has no answer, cache beyond that, then spatial + temporal denoising. |
| **24O** reference path tracer | Slow, unbiased, offline. Not in the frame loop — its job is to make "does the real-time GI converge to the right answer" a comparison rather than an opinion. |

**Outside Phase 24, in scope:** the **17E** remainder (hemispherical leaf normals,
transmission reaching foliage, bark roughness) and **17F** (foliage painting).

**Deferred:** the fallback tiers (**24P** mesh SDFs, **24Q** light probes) — they
exist for hardware without ray query and are the portable-but-larger path, so
they wait until the hardware path is settled. Also **24AA** cloud shadows,
**24AB** lighting debug views, **Phase 23** (GPU culling for transparents) and
**Phase 16** (scripting).

**Sequencing note.** 24L wants terrain inside the visibility buffer — Phase 25A —
before light starts bouncing off the largest surface in the scene. Running the GI
chain before 25A means writing it against a world where terrain is invisible to
it, then revisiting.

---

### 25.3b 25A implementation plan (from reconnaissance, not yet started)

**What the code actually looks like.** `TerrainPass::record` (`pass/terrain.rs:265`)
opens its own render pass and writes colour into the HDR target plus depth into
`vis_pass.depth_view`, taking `&[&TerrainData]` — chunk vertex/index buffers it
owns. It does **not** use the instance buffer or the programmable vertex pulling
that `record_visibility` (`renderer.rs:966`) drives. So "terrain into the
visibility buffer" is not one change; the two paths do not currently share a
vertex model.

**Split 25A in two.** The reconnaissance says most of the value is in the
smaller half:

**25A-1 — terrain depth prepass. DONE, and it does *not* do what this plan
originally claimed.** Terrain now writes depth into the shared buffer before the
acceleration-structure build, ReSTIR and GTAO, via a fragment-less pipeline.

Measured immediately afterwards: `SOMNIUM_GTAO=0/1` over the terrain region moved
the image by a mean of **0.71**, with 92 of 27 000 sampled pixels changing by more
than 12 — the noise floor. The acceptance test recorded below **fails**.

The reasoning behind this sub-phase was wrong, and worth writing down because it
is an easy mistake to repeat: GTAO, contact shadows and ReSTIR are all consumed
in `shading.wgsl`, and terrain shades in `terrain.wgsl`, which samples none of
them. Depth is what those passes *read to compute* their result; it is not what
delivers the result to a surface. So the prepass lets GTAO compute occlusion
around terrain — terrain now correctly occludes meshes, which is a real if small
gain — while terrain itself still cannot receive any of it.

**Keep it** (it is correct, cheap, and a prerequisite for 25B), but it is not the
shortcut it looked like. **25A-2 is required, not optional.**

**25A-2 — one shading path (larger, do only if 25A-1 leaves something wanted).**
Terrain writes visibility IDs and is reconstructed in `shading.wgsl` like
everything else, and `terrain.wgsl`'s duplicated `sample_shadow`, cascade
selection and cluster lookup are deleted. This is what actually retires the
duplication tax — the reason each Phase 24 improvement had to be written twice.
It needs an instance-id namespace for terrain chunks and an attribute
reconstruction path for a mesh that is not in the global vertex pool.

**25A-2 design — upload chunks into the global vertex pool, do not add a
namespace.** The earlier plan assumed terrain would need its own instance-id
range and a bespoke attribute path in `shading.wgsl`, because chunks own their
vertex buffers. That is the harder of the two options and it is no longer the
better one.

The visibility buffer became `Rg32Uint` during the 17E work, with instance id in
`.r` and primitive id in `.g` and **no cap on either**. That removes the reason
to invent a namespace: terrain chunks can simply be uploaded through
`geometry::upload_mesh` like every other mesh and submitted as ordinary
`DrawCommand`s. They then carry a normal `vertex_offset`, and `shading.wgsl`
reconstructs their attributes with the code path it already has — no terrain
branch, no second decode, nothing to keep in sync later.

Consequences to design for, in order of risk:

1. **Sculpting re-uploads a chunk.** `upload_mesh` is currently append-only and
   built for load-time use, so it needs either free-list reuse or a
   per-chunk-stable allocation that can be rewritten in place. This is the real
   work of 25A-2 and should be settled first.
2. **Chunk LOD switching changes index counts**, so the draw and its cluster
   bounds have to be rebuilt with it — the same path `rebuild_dirty_chunks`
   already drives.
3. **Terrain then flows through GPU culling and meshlet clustering for free**,
   which is a gain but means chunk AABBs must be correct or terrain will vanish
   exactly the way cubes did when the cone sentinel was wrong.
4. **`terrain.wgsl` keeps only its splat/triplanar material work**; its
   `sample_shadow`, cascade selection and cluster lookup are deleted, since
   shading now happens once in `shading.wgsl`.

The acceptance test is unchanged and still fails today: `SOMNIUM_GTAO=0/1` must
differ **on terrain**. Also re-check `SOMNIUM_SHADOW_DEBUG=7`, which reports
sun-versus-ambient dominance — terrain reading ambient-dominant after this change
would mean the material path did not survive the move.

**Sequencing note (revised).** The measurement above settles what was left open:
25A-2 has to happen for terrain to receive anything. It is a maintainability
change *and* the visual one — there is no cheaper path to lighting on terrain.

### 25.3c 25A-2 — what shipped, and the three bugs it uncovered

**The chunk-reallocation question is settled: a stable per-chunk span, rewritten
in place. No free list.** The deciding fact is that a chunk's vertex count is
`(chunk_cells + 1)²` and never changes — sculpting rewrites height *values*, and
a coarser LOD skips vertices through the index buffer rather than rebuilding the
grid. So `GeometryPool` gained `reserve_vertices` / `reserve_indices` and
`write_vertices` / `write_indices`: reserve once at terrain creation, rewrite
forever. Free-list churn (the voxel path) was the alternative and is worse here,
because `vertex_offset` is the key for the AABB map, the meshlet map and 25B's
per-mesh BLAS — churning it would invalidate all three on every brush dab and
leave the stale entries behind, since `free_mesh` does not remove them.
`write_vertices` refreshes the AABB, which is what keeps GPU culling honest
after a sculpt. Index data is *chunk-relative*, so one span per `(lod,
edge_mask)` is shared by every chunk: at most 5 × 16 spans, ~2 MB.

Terrain is now expanded into ordinary `DrawCommand`s before the draw sort, so it
flows through the instance buffer, the indirect args, GPU frustum and Hi-Z
culling, the shadow pass (**terrain casts shadows for the first time**) and the
visibility buffer. `shading.wgsl` reconstructs its attributes with the path it
already had; the only terrain branch is the *material*, which is what
`terrain_material.wgsl` still holds — splat weights, height blending, triplanar
cliffs and the brush ring. `terrain.wgsl`'s `sample_shadow`, cascade selection
and cluster lookup are deleted along with the whole of `pass/terrain.rs`,
including 25A-1's depth prepass, which the visibility pass now makes redundant.

**Verified**: 36 469 terrain pixels in the visibility buffer at 1280×720, albedo
0.139 (a real grass/rock value, so the bindless layer maps resolve), shadow
factor 0.9995, plausible shading normals. 208 tests green, and shader modules
are now naga-validated in `cargo test` — WGSL previously only compiled at
device-creation time, so a mistake surfaced as a first-frame crash and nowhere
else.

**Three bugs found on the way, none of them in terrain:**

1. **`ShadingPass::resize` rebuilt its bind group from stale texture views.** It
   took only the new visibility view and reused the GTAO, depth and ReSTIR
   clones captured at construction — all of which are recreated on resize. So
   after the first window resize (the demo does three during startup) `gtao.a`
   read 0, which zeroes `surface.occlusion`, which zeroes *both* terms of
   `evaluate_ibl`: **no surface in the scene received any indirect light**,
   contact shadows marched a dead depth buffer, and ReSTIR visibility read as
   "not run". This had been the state of every session since 24I.
2. **TLAS instances were written at the draw-queue index, not densely.** The two
   were the same number while every draw had a BLAS; terrain chunks have none
   until 25B, so the moment one is skipped the writes go sparse while the
   stale-slot clear loop assumes density. The symptom was not a missing shadow
   but an *unstable* one — two runs of one build had the terrain fully lit and
   fully shadowed.
3. **GTAO occludes a grazing surface with its own tangent plane.** Terrain read
   **0.029** visibility on open ground, which is what made it render near-black
   the first time it consumed GTAO. A sample lying in the surface's own plane is
   the surface, not an occluder; requiring one to sit measurably above it took
   terrain to 0.548. This is a 24I defect that terrain merely exposed — it is
   the first large surface in the scene seen at a grazing angle.

**Measurement tooling** (`capture.rs`): `SOMNIUM_CAPTURE=<file>` writes the HDR
target back at a fixed frame index — before tone mapping, exposure and TAA — and
labels every pixel terrain / mesh / sky from the visibility buffer.
`SOMNIUM_CAPTURE_COMPARE=<file>` diffs against one and reports mean absolute
luminance and changed-pixel counts *per class*. This exists because §26.4 is
right that screen-grabbed frame deltas are unusable, and because "the image
changed" is not the same claim as "terrain changed". `SOMNIUM_GTAO=0` is the
switch the acceptance test always named and which did not exist; it is seeded
into `PostProcessComponent`, not `GtaoPass`, because the component is copied
into the pass every frame and a pass-side default never survives to frame one.
`SOMNIUM_SHADOW_DEBUG` gained 8 = occlusion, 9 = albedo, 10 = shading normal,
11 = terrain flag.

4. **Terrain chunk winding was backwards, and `cull_mode: None` had been hiding
   it since Phase 14.** The block-fan ring is built +X then +Z, which traces
   *clockwise* in the XZ plane seen from above, so emitting `[center, a, b]`
   made every terrain triangle a back face. The old terrain pass drew with
   culling off — its comment said the winding was "uniform but unverified" —
   so a fully back-facing surface still rendered. The visibility pass back-face
   culls, and the moment terrain moved into it **a flat terrain rendered zero
   pixels** and a sculpted one showed only the slopes whose underside faced the
   camera. This is what "Create > Terrain draws nothing" was. Wound the other
   way, with a test that takes the cross product of every emitted triangle at
   every LOD and stitch mask and asserts `n.y > 0`.

   Two process notes, because both cost real time. The bug was invisible in the
   `SOMNIUM_TERRAIN=1` smoke test, whose sculpted hill happens to be seen from
   beneath — so `SOMNIUM_TERRAIN=flat` now reproduces **Create > Terrain**
   exactly (default 16×16 descriptor, no sculpting, spawned at y = 0) and is the
   variant to verify against. And the first run after the fix still reported
   zero, because `cargo test` builds the *test* profile: the binary
   `Start-Process` launches is only rebuilt by `cargo build`.

**Acceptance test: passing.** On the flat terrain, `SOMNIUM_GTAO=0` against
`SOMNIUM_GTAO` unset, same build, same frame index:

| class | pixels | mean abs Δ luminance | changed |
|---|---|---|---|
| **terrain** | 843 729 | **27.88** | **329 792 (39%)** |
| mesh | 16 431 | 0.0000 | 0 |
| sky | 61 440 | 0.0000 | 0 |

Both runs render the identical view (843 729 terrain pixels either side), and
sky and mesh come back bit-identical — sky is the control, since GTAO cannot
touch it, and the helmet carries its own `occlusionTexture`, which overwrites
the screen-space term. Only terrain moves, which is the thing 25A-1 could not
make happen. Terrain pixels also appear in the visibility buffer, the plan's
other clause. Sun-versus-ambient dominance is answered by the same measurement
rather than by `SOMNIUM_SHADOW_DEBUG=7`: terrain reads 2813 cd/m², against the
~150 that ambient alone would give at `ibl_intensity` 0.35, so the material
survived the move sun-dominant.

The earlier nondeterminism in this A/B was the TLAS slot bug above, compounded
by a terrain that filled 4% of the frame; with both fixed the pair repeats to
the digit.

### 25.3d 25B — terrain chunks in the TLAS

25A-2 already put terrain chunks in the draw queue the acceleration-structure
build reads, so they arrived at the TLAS loop and were skipped for having no
BLAS. 25B registers one.

**The architecture came from the reference, and it is the part that mattered.**
`bevy_solari/src/scene/blas.rs` builds a bottom-level structure only for meshes
that were *added or modified*, and `binder.rs` then rebuilds the **top** level
alone each frame with an empty BLAS slice. Somnium had been reissuing **every**
BLAS every frame — affordable with a handful of meshes, and not once a terrain
contributes 256 chunks of 8 192 triangles. `RaytracePass` now keeps a
`pending_blas` list, `register_mesh` stores the size descriptor and offsets it
was built with so a rebuild needs no caller state, and `mark_geometry_dirty`
covers the case Bevy has no equivalent for: terrain chunk *contents* changing
under a stable allocation when sculpted. `rt_geometry` is gone.

**Always the full-detail, unstitched `(lod 0, mask 0)` geometry**, never the
frame's LOD. A BLAS is sized once at creation, so its index range cannot follow
a per-frame LOD — and it should not: a traced shadow whose shape changed as
chunks swapped LOD would be worse than one slightly finer than what is drawn.
`ensure_index_blocks` therefore reserves `(0, 0)` unconditionally. Chunks
register on the frame their heights are first written and re-dirty on every
sculpt, which is exactly the set `rebuild_dirty_chunks` now reports.

**Verification.** `SOMNIUM_RT_TERRAIN=0` holds terrain out of the acceleration
structures and changes nothing else, which isolates what the sub-phase added:

| class | pixels | mean abs Δ luminance | changed |
|---|---|---|---|
| **terrain** | 791 798 | **17.998** | **6 945** |
| mesh | 27 186 | 0.0000 | 0 |
| sky | 102 616 | 0.0000 | 0 |

TLAS instances go 1 → 17 (sixteen chunks plus the helmet). About 0.9% of terrain
pixels move — the hill's shadowed strip and its contact areas, which is what a
low sun over a mostly convex hill should change — while the helmet, already in
the TLAS, and the sky come back bit-identical. **Only terrain changed, and the
only new occluder is terrain**, so this is terrain shadowing terrain: the 24K
acceptance test, which is why 24K moves to ✅ with it.

The `SOMNIUM_RT_DEBUG=1` view remains the qualitative check, but it is not the
evidence here — it writes into the HDR target after the frame capture point, and
a screen grab of it depends on window focus. The A/B above is the measurement.

**25B is unchanged** and depends only on 25A-1: once terrain depth is in the
frame before the acceleration-structure build, terrain chunk geometry can be
added as BLAS entries at the committed LOD, rebuilt on sculpt.

**First check either way** — `SOMNIUM_GTAO=0/1` must differ *on terrain*, which
it cannot today.

### 25.3e 25F — hex-tiling, and why it ships switched off

Ported from `example_repo/bgfx-master/examples/49-hextile/fs_hextile.sc`
(Mikkelsen's hextile-demo, after Heitz & Neyret) into `shaders/hextile.wgsl`:
skew UV into a simplex grid, give each grid vertex a hashed offset, sample three
times and blend by barycentric weight. `terrain_material.wgsl` uses it for the
layer albedo and normal maps.

Three things the port had to get right, and one the reference does not cover:

- **`textureSampleGrad` with per-tap derivatives.** Each tap reads a different
  part of the texture, so implicit derivatives would be taken across a
  discontinuity and collapse mip selection into noise. The derivatives of world
  position are taken in `shading.wgsl` where control flow is uniform and scaled
  per layer.
- **Sharp weights and luminance modulation**, which are what stop the blend
  reading as a wash.
- **Not passing textures across function boundaries.** Pulling a
  `texture_2d<f32>` out of the binding array into a local and passing it as a
  parameter is legal WGSL and **segfaults naga's SPIR-V backend** — the process
  dies during pipeline creation with no diagnostic whatsoever. `hex_sample`
  takes a bindless *index*, like every other sampling site in the engine.
- **Normals need counter-rotating** (not in the reference, which tiles colour
  only). A normal map stores its vector in the texture's UV frame, and each tap
  read that texture through a different rotation; blending the raw samples
  averages three normals that disagree about which way "along U" points.

**The measurement says do not enable it yet.** Rendered side by side on the flat
terrain, the plain path shows *no findable grid*, and the hex-tiled path shows
its own lattice faintly. The cause is content, not code: the four layers are
**procedural, tileable, low-contrast noise** generated in `textures.rs`, so there
is no repetition to remove, while the technique's own tile boundaries do shift
each patch's mean slightly. Two rounds of tuning improved but did not remove
that — first dropping rotation to the reference's own default (`hextile.cpp`
ships `m_tileRotationStrength = 0.0f`; at 1.0 the simplex lattice was plainly
visible as hard triangular seams), then softening the weight exponent, gain and
luminance bias.

So it is implemented, cited, validated and **off by default**, behind
`SOMNIUM_HEXTILE=1`. The A/B confirms it is wired correctly: 805 993 of 845 018
terrain pixels change with mesh and sky bit-identical, and mean terrain
luminance moves 0.08% (2815.9 → 2813.6) — the blend redistributes detail without
raising the mean, which is the check that the luminance modulation is not
washing the texture out. Turn it on when the layers are photographed rather than
generated; **25D**'s detail clipmap and **25J**'s file-based layers are what
bring that, and 25F should be re-judged then.

### 25.5 17E remainder — status

- **Transmission reaching foliage: already done**, in 24S rather than in a
  separate change, and the 17E note above is stale on this point. `load_gltf`
  infers `transmission = 0.5` when a sibling `*_alpha_*` cutout mask exists
  (`somnium_asset/src/lib.rs`), `upload_scene` carries it into `GpuMaterial`, and
  `shading.wgsl`'s `transmitted_light` consumes it. The chain is complete.
- **Hemispherical leaf normals: not done.** The cheap form — blending the
  geometric normal toward the direction from the instance origin to the hit
  point, which needs no new data since `instance.model` is already in the shader
  — was left unwritten deliberately: the demo has no way to *show* foliage
  without painting it by hand in the editor, so the change could not be
  verified, and an unverifiable shader change to foliage is exactly what this
  phase has been paying for. It needs either a scripted foliage scene (the
  natural companion to `SOMNIUM_TERRAIN=flat`) or the editor.
- **Bark roughness: not done**, same reason.

### 25.6 25K — photographed terrain materials

Content, not code, was the blocker. 25F had nothing to fix and 25E had no height
map, because the four layers were procedural tileable noise generated in
`textures.rs`.

**Eight CC0 materials from Poly Haven** (`aerial_grass_rock`, `leafy_grass`,
`forrest_ground_01`, `brown_mud`, `aerial_rocks_04`, `snow_02`,
`coast_sand_rocks_02`, `gravel_floor`) at 4K, fetched by
`tools/fetch_terrain_textures.sh` and channel-packed by
`somnium_asset --example pack_terrain`. `aerial_rocks_04` is deliberately the
texture the bgfx hex-tile example ships with, so 25F can be judged against the
material its own reference was tuned on.

**Four source maps become two textures**, which is the decision everything else
follows from:

| packed texture  | R        | G        | B         | A      |
|-----------------|----------|----------|-----------|--------|
| `*_albedo.png`  | albedo R | albedo G | albedo B  | height |
| `*_surface.png` | normal X | normal Y | roughness | AO     |

Memory is the obvious reason — a 4K RGBA8 array with mips is ~350 MB, and two
arrays is half of four. The one that matters more is **sample count**: the
terrain shader samples every layer for every pixel, and 25F triples whatever it
samples. Normal Z is reconstructed as `sqrt(1 - x² - y²)`, exact for a unit
normal and what BC5 would force anyway; metalness is dropped because terrain
layers are dielectric.

Three things came out of the O3DE reference (`Gems/Terrain/Assets/Shaders/`):
`DetailMaterialData`'s shape of per-map bindless indices plus per-map factors;
`AppendHeightToWeight`, which is 25E and needs the real height map this phase
delivers; and **`MaxAnisotropy = 16`** on its detail sampler — ground is the one
surface always seen at a grazing angle, where an isotropic mip is chosen for the
shorter axis and smears everything along the longer one. That is now on the
shading sampler and is a large part of why detail survives to the horizon.

Also here: the layer arrays are built **with a mip chain**. They were
`mip_level_count: 1`, which was survivable for smooth noise and is pure aliasing
for photographed detail. The mip filter is a plain box, deliberately *not* the
alpha-weighted one `renderer.rs` uses for glTF — alpha there is cutout coverage,
alpha here is a height map, and weighting albedo by it would darken every layer
toward its own crevices.

**Load resolution defaults to 2K** (`SOMNIUM_TERRAIN_RES=4096` for full detail):
4K across two arrays is ~700 MB of VRAM. The committed assets are 4K so the
detail is there when BC compression makes it affordable. Codec crates are pinned
to `opt-level = 3` in the dev profile — `image` in debug is ~2 orders slower and
decoding the layers exceeded a 90-second timeout before that.

`assets/terrain/_source/` is git-ignored and re-derivable; only the packed result
is committed. The procedural generator is kept as a fallback, because a clone
without ~650 MB of assets must still start.

**This settled 25F.** With procedural layers there was no repetition to remove
and hex-tiling only showed its own lattice, so it shipped off. With photographed
layers the tiling grid is immediately visible as bands marching to the horizon,
and hex-tiling removes them. Same code, same parameters, opposite verdict,
decided entirely by the content — so **25F is now on by default**.

**Not done:** the eight materials are loaded and packed but only **four are
wired**, because the splatmap is RGBA8 and hard-caps the layer count. Going to
eight needs a second splatmap and touches `Splatmap`, the paint brush,
`auto_splat`, the `TerrainEditCmd` undo payloads in `somnium_core`, foliage layer
filtering and the inspector — a change across three crates that wants the editor
to verify, so it is its own step rather than a rushed tail on this one.

### 25.7 25L — eight layers, and a real heightmap

**Eight materials, two splatmaps.** Fyrox gives every layer its own mask texture
(`scene/terrain/mod.rs`, `Layer::mask_property_name`), which has no ceiling at
all; packing four masks per RGBA texture is the same idea at a quarter of the
bindings, and two textures is where the cost stops being free. All eight of
25K's materials are now wired: grass, forest floor, rock, snow, meadow, mud,
sand, gravel.

The splat texel became a named type — `textures::SplatTexel` — because it
crosses a crate boundary: the editor's `TerrainEditCmd` undo payloads carry
blocks of them, and a bare `[u8; 4]` in `somnium_core` is exactly the kind of
thing that silently disagrees after a widening. The `.somnium` terrain sidecar
gained a version check for the same reason; a v1 file's splat block is a
different size and is now refused rather than misread.

**Layers are weight-gated, so eight are cheaper than four were.** Splat weights
are sparse — two or three materials meet at any texel and the rest are zero — so
`LAYER_WEIGHT_EPSILON` skips sampling a layer that cannot change the result. A
fixed 16 samples (48 with hex-tiling) becomes the four or six that contribute.
This is only legal because the terrain path samples with explicit derivatives
throughout: `textureSampleGrad` has no uniformity requirement where
`textureSample` inside that branch would be undefined.

**Auto-splat assigns all eight** from altitude, slope and a low-frequency noise.
The noise is what keeps the two grasses and the forest floor from laying down as
horizontal altitude bands — a real hillside does not change species on a contour
line.

**Heightmaps load.** `terrain/heightmap.rs` reads 16-bit PNG, any other image
the engine decodes, and CDLOD's `.tbmp`; `SOMNIUM_HEIGHTMAP=<path>` in the demo,
with procedural FBM relief as the default so terrain is landscape rather than a
plain even with no asset. Terrain has had a heightmap field since Phase 14 and
no way to fill it except the sculpt brush.

Two bugs found on the way, both in the same function, and both worth recording:

1. **The resample point-sampled.** CDLOD's dataset is 4096×2048 against a 1025²
   grid, so bilinear taps landed four source texels apart. That is the same
   mistake as a texture without mips and far more destructive on a heightmap,
   because the aliasing becomes *geometry*. Now area-averaged over the footprint
   each destination vertex owns.
2. **`.tbmp` is a *tiled* bitmap, and I decoded it as linear rows.** The header
   is `[pixelFormat, width, height, version, blockDim]`; word 4 is 256, and
   `256 + 4096 × 2048 × 2` happens to equal the file length exactly, so "word 4
   is the header size" fitted the evidence perfectly and was wrong — it is
   `blockDim`, and the header is always 256 bytes. The file stores 256×256
   tiles (`TiledBitmap::GetBlockStartPos`). Read row-wise it produced regular
   horizontal terraces separated by black walls. **A size check is not a format
   check**, and the answer was in `TiledBitmap.cpp` the whole time — the name of
   the class was the warning.

The procedural-relief render is what isolated it: same mesh, LOD and material
path, smooth landscape, so the fault had to be in the loader rather than
anywhere downstream.

**Known artefact:** the CDLOD render carries small black speckles across the
surface — most likely shadow acne, since terrain began casting shadows in 25A-2
and this dataset has metre-scale relief that the cascades cannot resolve.
`SOMNIUM_SHADOW_DEBUG=1` will confirm or rule it out in one run; it does not
appear on the smoother FBM relief.

### 25.8 Terrain self-shadowing — three causes, only one of them the shadow map

The CDLOD render came out stippled with black patches. `SOMNIUM_SHADOW_DEBUG=1`
confirmed they were `shadow_factor` reaching zero, and the obvious reading —
shadow acne — was wrong twice over.

**The shadow map was not the source.** A normal-offset bias was added first,
fixing the recovery 24H got wrong: `ndc.x` is `dot(row0, world)`, so the
world-per-NDC scale is the reciprocal of **row** 0's length, where the earlier
attempts took *column* 0 of `proj * view`, which mixes the x, y and depth
scales. That is a real correction and it is kept — but pushing the offset to 24
texels, far past peter-panning, changed the image **not at all**, which is what
proved the shadow map innocent.

The two actual causes were both surfaces intersecting themselves, and both
reached terrain for the first time in 25A-2:

1. **ReSTIR's shadow ray started inside the terrain.** Its origin is
   reconstructed from the depth buffer and its `t_min` was a flat 5 cm — far
   below the world footprint of a pixel at any distance, so the ray re-hit the
   surface it left. `t_min` now scales with that footprint, measured by
   reconstructing the neighbouring pixel, which needs no new uniforms and is
   exactly the scale of both the reconstruction error and the slope offset.
   This is what the large elongated patches were; disabling ReSTIR removed them
   and left something different behind, which is how the two were separated.
2. **The contact-shadow march started on the surface.** Its first steps sit
   within `CONTACT_THICKNESS` of the depth buffer's own value on ground seen at
   a grazing angle, so terrain shadowed itself as a fine stipple everywhere.
   The march now starts one step along the surface normal.

Worth recording as a method point: three plausible mechanisms, and the fix for
the most plausible one was ruled out by making it absurdly large and seeing
nothing change. Toggling ReSTIR was what actually split the remaining two.

### 25.9 The default scene

**Create > Terrain** builds the same thing: a new terrain arrives with relief
and its eight materials already assigned, rather than as a flat plain that has
to be sculpted before it looks like anything. The source of that relief lives in
`TerrainData::apply_default_relief` — env override, then the shipped CDLOD
dataset, then procedural FBM — and **not** in either caller, because a fallback
chain written twice across two crates is one that will disagree. Choosing a
heightmap at creation is a UI-phase job; this is the default that dialog will
start from.

`hello_engine` now spawns terrain **by default** — the editor's own
Create > Terrain geometry, with `assets/terrain/heightmap.tbmp` (CDLOD's
dataset, MIT, attributed in `assets/LICENSE.md`) and all eight materials
auto-splatted by altitude and slope. `SOMNIUM_TERRAIN` selects a variant rather
than enabling it: `1` is the legacy sculpted 4x4 smoke test that exercises the
brush paths, `0`/`none` disables terrain. `SOMNIUM_HEIGHTMAP` overrides the
file, and procedural FBM relief is the fallback when it is missing, so a clone
without assets still gets landscape rather than a plain.


### 25.10 17E remainder — closed

Three items were outstanding. **Two were already done and the note was stale**,
which is worth recording as a pattern: transmission reached foliage back in 24S
via the `*_alpha_*` sidecar inference, and bark roughness is data-driven —
every Poly Haven foliage material wires its ARM map as
`metallicRoughnessTexture`, so roughness has always come from the green
channel. Checking the glTF JSON took a minute and saved implementing both.

But checking *why* bark still looked wrong found a real bug, and a much larger
one than bark:

**Every imported glTF texture was uploaded as `Rgba8UnormSrgb`** — including
normal, metallic-roughness/ARM and occlusion maps, none of which are colour.
The sRGB decode bent all of them: an authored roughness of 0.5 arrives as
~0.21, so *every imported material in the engine read glossier than it was
made*, and normal maps were skewed the same way, weakening all surface detail.
That is what the 17E note recorded as "bark roughness", and it applied equally
to the helmet and to every model imported since Phase 10. Texture usage is now
collected from the materials — glTF images carry no colour-space flag, so how a
texture is *referenced* is the only thing that says what it means — and only
albedo and emissive are sRGB.

**Hemispherical leaf normals** are ported from
`SpartanEngine-master/data/shaders/g_buffer.hlsl` ("foliage curved normals"):
rotate the normal about the axis running along the card, by an angle taken from
how far across the card's width the pixel sits, so a flat leaf shades as a
curved one instead of a flat plate. Spartan carries a `width_percent` vertex
attribute for this; Somnium needs no such attribute, because on a foliage card
`uv.x` **is** the distance across the blade. Gated on a new
`MATERIAL_FLAG_FOLIAGE` rather than on `transmission`, since glass is
transmissive too and must not be bent into a leaf.

**`SOMNIUM_FOLIAGE=1` scatters foliage without the editor**, which is what
unblocked the whole item. Painting by hand was the only way to get a plant on
screen, so the foliage shading work could not be seen, let alone measured — the
reason this sat open across two sessions. Strokes are deterministic, so an A/B
of a foliage shading change is now like-for-like. The scene also needs an
enabled `FoliageComponent` on the terrain entity and a camera at eye level:
foliage is culled past 120 m (17G) and a tuft is sub-pixel from the landscape
camera, which is why the first two attempts scattered 25 733 instances and
showed nothing.

**Honest limit:** the curved normals are implemented, flagged and active, but
their visual delta at tuft scale is subtle and was not isolated with an A/B —
there is no runtime toggle for them. The sRGB fix is the change with the
objectively verifiable mechanism.


### 25.11 24U + 25I — one froxel volume for aerial perspective and fog

Taken together because they are the same integral. A 3-D table indexed by
(screen x, screen y, distance) holds the light scattered *into* the view ray up
to that distance and the transmittance surviving it; shading applies both with
one fetch:

    colour = colour * transmittance + inscattering

They differ only in what scatters — the atmosphere's Rayleigh and Mie terms, or
a fog medium — and in whether the sun is shadow-tested per step. Bevy keeps
them apart (a 3-D LUT for aerial perspective, a screen-space march for fog);
folding them into one volume means distant hills desaturate and a shaft crosses
a valley by the same code, and there is no second definition of what the air is
made of. That is the same argument 25A-2 made for terrain shading.

**The plan assumed a LUT that did not exist.** 24U was written as "fed by 24C's
aerial-perspective LUT" — 24C built transmittance and multiple-scattering
tables and a sky march, and no aerial LUT. Building it was most of this work.

Details taken from `bevy_pbr/src/atmosphere/aerial_view_lut.wgsl` that are easy
to drop and visible when dropped: **log-space storage**, so hardware filtering
between slices interpolates an exponential correctly; the **half-slice offset**
when sampling, because each texel is the integral over its whole slice; and a
**linear fade over the first slice**, without which the full first slice of
scattering is applied at zero distance and fog appears on the lens.

Two things worth recording:

- **Units.** The atmosphere model is in kilometres — extinction km⁻¹, scale
  heights of 8 km and 1.2 km — and the scene marches in metres. The air terms
  are converted per-metre at the sample; without that the air is a thousand
  times denser and reads as fog going opaque a metre from the camera.
- **The shadow lookup is deliberately not the surface one.** `shading.wgsl`'s
  PCSS exists to make a shadow *edge* look right and costs 40 taps; a froxel
  needs a yes/no answer at a thirtieth of the screen's resolution and the
  volume's own filtering smooths it. Reusing the surface path here would be
  slower and wrong — a different algorithm for a different job, not a copy.

**Verification.** `SOMNIUM_VOLUMETRICS=0/1` on the default scene:

| class | pixels | mean abs Δ luminance | changed |
|---|---|---|---|
| **terrain** | 784 523 | **279.74** | **694 948 (89%)** |
| sky | 137 077 | 0.0000 | 0 |

Mean terrain luminance falls 4554.7 → 4313.2, which is the correct *sign*: at
this sun angle extinction removes more than in-scattering adds, so distant
ground loses contrast toward the sky rather than brightening. **Sky comes back
bit-identical**, which is the control that matters — the sky's radiance already
comes from a full march through the same atmosphere, and applying the volume to
it as well would count the air twice.

**Status.** 25I is complete: aerial perspective reaches terrain, meshes and
foliage alike, because it is applied once at the end of the shared shading path
— the 25A-2 payoff again. 24U is **partial**: the froxel volume, the fog medium
with height falloff and a Henyey-Greenstein phase, and the per-froxel shadow
test for shafts are all implemented and on by default, but **light shafts have
not been visually confirmed** — that needs a low sun behind an occluder, which
the current scene does not have. The code path is exercised; the picture is
not. Also absent: temporal reprojection of the volume, which is what lets the
step count come down without the fog crawling.


### 25.12 25E — height-weighted material blending

Splat weights say how much of each material is at a texel. They say nothing
about which one is **on top**, and normalising them cross-fades: at a seam every
pixel is half of each, which is a colour that exists in neither material. The
gravel in the demo scene was the proof — pale, low-contrast, its pebbles ghosted
into the grass beside it, because most of the gravel's screen area was being
averaged with something else.

Ported from O3DE's `TerrainDetailHelpers.azsli` (`AppendHeightToWeight` and the
depth-blend loop in `GetDetailSurface`), which is a two-part algorithm:

1. **Height into weight**, clamped by coverage:
   `w += h * min(1, (1/min_weight) * w)`. The clamp is the part that is easy to
   drop and load-bearing — without it a 4% sliver of a material with a tall
   height map out-ranks the 96% material that is actually painted there, and the
   height map becomes a second splatmap nobody authored. `blend.rs` has that as
   a pair of tests: one asserting the sliver loses, one asserting it *wins* when
   the clamp is removed, so the parameter cannot quietly stop mattering.
2. **Depth blend**: only materials within their own `blend_width` of the winner
   contribute, renormalised across that band. Because the band is measured on
   weights that already carry each layer's relief, the boundary follows the
   rock's crevices instead of a contour of the splatmap.

**The parameters are per layer, and that is the point.** A single global
sharpness makes every transition the same transition. `blend::LAYER_BLENDS`
authors `height_scale` / `blend_width` / `min_weight` against what each of the
eight photographed materials physically is: rock and gravel deep-relief and
hard-edged (`0.15` bands), snow and wet mud shallow and soft (`0.55`–`0.60`).
O3DE's own defaults are `heightBlendFactor 0.5` / `heightWeightClampFactor 0.1`,
and it uploads the reciprocal of the second — so do we, in `blend::weight_clamp`.

Also here, from the same reference: **albedo is blended in an approximately
perceptual space** (`sqrt` in, squared out). A weighted mean of *linear* albedo
between two materials of different luminance sits below both once it is read
through the display transform, which is why a seam that should be a texture
boundary showed as a dark band along it.

**Measured**, `SOMNIUM_TERRAIN_HEIGHT_BLEND=0` vs `1`, eye level over the
gravel/grass boundary: **776.8** mean absolute luminance over 921 600 terrain
pixels, 319 812 of them past the 1% threshold, with mesh and sky bit-identical.
From the landscape camera the same change is 135.0 over 302 041 pixels — real
but nearly invisible, which is the finding as much as the number is.

**`SOMNIUM_TERRAIN_EYE=1`** came out of that. Every terrain texturing phase
since 25F has been judged on a hillside a kilometre away, where a material
transition is a few pixels wide; the features live at metres and the demo camera
did not. It is the foliage phase's eye-level stance without the foliage, and
25H's parallax will need it too.

**Two tests were added that would have caught real bugs.** `blend.rs` mirrors
the algorithm in Rust and pins its properties — sums to one, degrades to a plain
blend when heights are equal, relief flips two evenly-matched materials both
ways. And `shaders_validate.rs` now asks naga for the WGSL layout of
`TerrainMaterial` and compares it to `GpuTerrainMaterial`'s `repr(C)` offsets;
`material/pool.rs` had only ever proved the Rust half, with the WGSL half left
as a comment. It caught this phase's own trailing `vec3<u32>` pad, which aligns
to 16 in WGSL and to 4 in Rust and would have given the struct a 272-byte stride
against Rust's 256 — invisible with one terrain, silent corruption with two.

### 25.13 25D — the macro tier, and the clipmap that was not built

**Scope decision first, because it is most of the phase.** O3DE's answer to the
resolution ceiling is two toroidally-addressed clipmap stacks: a macro pair and
seven detail arrays, generated by compute into rings centred on the camera,
sampled trilinearly across levels, with incremental region updates as the centre
moves (`ClipmapBounds.h`, `TerrainDetailClipmapGenerationPass.azsl`,
`ClipmapComputeHelpers.azsli`). The **detail** half of that is a cache: it
composites the layered PBR once per clipmap texel instead of once per pixel.

Somnium composites per pixel, at full rate, with explicit derivatives. Caching
that into a clipmap would **lower** close-range quality — the innermost ring
would have to hold millimetre texels to match what 25K and 25E just bought, and
it cannot — in exchange for a cost win on a 1 km bounded heightfield that is not
the streamed, unbounded world the machinery exists for. So the detail clipmap is
deliberately **not** built, and the phase delivers the two things it was actually
for:

**1. The macro tier.** Eight materials describe a texel of ground but not a
landscape: every patch of grass is the same patch of grass, and at distance the
layers converge to their own mean and the terrain goes uniform. O3DE's macro
material is authored imagery over the terrain with the detail composited on top
(`TerrainMacroHelpers.azsli`). Somnium has nothing to author with, so
`terrain/macro_map.rs` **derives** it from the landform — altitude, macro-scale
grade, how much of a hollow a point sits in, and two octaves of large-scale
noise — which is better than an authored map would have been at this stage,
because the variation then *correlates* with the terrain instead of floating
over it. Ridges come out drier and paler, hollows darker and greener.

The composite is O3DE's `ApplyTextureBlend`, all four modes, defaulting to
**overlay** so the detail keeps its own light and dark structure and takes only
the macro's colour and level. It happens **between 25E's `sqrt` and its
squaring** — O3DE performs overlay and linear-light in a display-referred space
for the same reason, and it is what makes a macro texel of 0.5 the exact
identity. That in turn is what makes "no macro map bound" and "strength 0" the
same picture, which `a_flat_terrain_stays_near_the_neutral_value` pins.

**2. A detail budget that scales with screen area.** The per-pixel layer gate
rises with camera distance, from `LAYER_WEIGHT_EPSILON` to `FAR_LAYER_EPSILON`
(0.2, which admits at most four layers and in practice one or two). Not higher:
at 0.5 only one layer can ever survive, so a genuine 51/49 boundary snaps and
the seam crawls as the camera moves.

**Measured.** Debug mode 12 writes layer taps as a fraction of the 48-tap worst
case straight to the HDR target before exposure, so the capture harness's mean
terrain luminance × 48 *is* the mean taps per pixel:

| view | fade off | fade on |
|---|---|---|
| landscape camera | 16.74 taps/px | **11.44** (−32%) |
| eye level (`SOMNIUM_TERRAIN_EYE=1`) | 12.00 taps/px | 12.00 (unchanged) |

Close up it costs exactly nothing, which is the property that matters: the
budget only ever removes layers a pixel could not resolve.

**Re-measured in milliseconds once Phase 29's profiler existed:** the shading
pass goes **0.973 → 0.883 ms**, −9.2%, with every other pass identical to the
third decimal. So −32% of the texture reads buys −9.2% of the pass — reads were
not the whole of its cost. See §29.1.

Macro tier A/B (`SOMNIUM_TERRAIN_MACRO=0/1`): **127.5** mean absolute luminance
over 784 523 terrain pixels, 601 598 of them past the 1% threshold, sky
bit-identical. At eye level the same switch moves 18.5 (0.4%) and **zero** pixels
past the threshold — not a failure but the definition of the tier: a
hundreds-of-metres signal is nearly constant across a 30 m view.

**The thing that did not work, and why it is worth recording.** The budget first
dropped **hex-tiling** past the fade as well, on the reasoning that three taps
per map exist to hide a repetition already below a pixel at that range. That
measured beautifully — 16.74 → 6.20 taps, −63% — and put a hard lattice across
the entire mid-ground, visible in both sides of the macro A/B, which is what
gave it away. At distance a 4 m tile is a *few pixels* wide, so the repetition
does not vanish; it beats against the pixel grid and becomes **more** visible
than it is close up. Hex-tiling earns its taps furthest away, which is the
opposite of the intuition. The layer gate was doing nearly all of the saving
anyway, and −32% artefact-free is the number that ships.

**Still open from the original row:** a macro *normal* map (Somnium's terrain
mesh is one vertex per metre, so the geometric normal already carries that
frequency), authored macro imagery in place of the derived map, and the detail
clipmap itself, which stays unbuilt on purpose.

### 25.4 Verification plan

Terrain makes the lighting work testable, so each sub-phase states its own check:

- **25A** — ✅ passing. `SOMNIUM_GTAO=0/1` on `SOMNIUM_TERRAIN=flat`: terrain
  moves by 27.88 mean absolute luminance over 843 729 pixels (39% of them past
  the 1% threshold) while sky and mesh come back bit-identical. See §25.3c.
- **25B** — ✅ passing. `SOMNIUM_RT_TERRAIN=0/1` moves 6 945 terrain pixels by
  17.998 mean absolute luminance while mesh and sky stay bit-identical, with the
  TLAS going 1 → 17 instances. The only new occluder is terrain, so this is
  terrain shadowing terrain — the 24K acceptance test, and 24K is ✅ with it.
  See §25.3d.
- **25C** — fly a ridge line against the sky and record; no popping frame to frame.
- **25M** — ✅ passing for the reported bug. `SOMNIUM_SUN_ELEVATION=-10` now
  renders black-with-stars instead of red; +2° renders a golden hour. HDR
  terrain luminance 4362 (day) → 137.7 (dusk) → 0.0001 (night). The night
  specular look and the exposure/firefly hypotheses are still open. See §17.16.
- **25H** — ✅ passing. `SOMNIUM_TERRAIN_PARALLAX=0/1` at eye level: 1729.8
  mean absolute luminance over 921 600 terrain pixels, mean luminance moving
  only −0.4% — detail redistributed, not darkened. Shading 0.898 → 1.022 ms.
  See §17.15.
- **24AC (SPD)** — ✅ passing. Hi-Z 0.045 → 0.029 ms with the frame
  **bit-identical** (`mean_abs = 0.0000, changed = 0`), which is the real
  acceptance test for something feeding occlusion culling. See §17.14.
- **24AD / 24Z** — ✅ passing. Both passes dispatch and are profiled:
  Velocity 0.010 ms, Motion Blur 0.034 ms moving / 0.001 ms static.
  See §17.11–17.12.
- **24U** — 🟡 partial. Temporal reprojection and jitter land and cost
  0.071 → 0.093 ms, but **light shafts have still not been seen** — the demo
  scene has no low sun behind a hard occluder. See §17.13.
- **24AC (CAS)** — ✅ passing. `SOMNIUM_CAS=0/1` on the swapchain: mean absolute
  difference 13.6 of 765 across the viewport, max 210, 64.5% of pixels changed;
  cost 0.018 ms. The HDR capture harness cannot see it — see §17.10.
- **24L** — ✅ passing. `SOMNIUM_RESTIR_GI=0/1` at eye level: 51.2 mean
  absolute luminance over 921 600 terrain pixels, 342 748 past the 1% threshold.
  Cost 0.788 ms on the profiler. See §17.8.
- **24AE** — ✅ passing. Eye-level foliage camera: casters 7 166 → 1 873 and
  Shadows 23.769 → 6.158 ms, with every other pass unchanged to the third
  decimal. See §17.9.
- **25D** — ✅ passing. Debug mode 12 (`SOMNIUM_SHADOW_DEBUG=12`) reports mean
  layer taps per pixel: 16.74 → 11.44 with the detail budget on, unchanged at
  eye level. `SOMNIUM_TERRAIN_MACRO=0/1` moves 127.5 over 784 523 terrain
  pixels with sky bit-identical. See §25.13.
- **25E** — ✅ passing. `SOMNIUM_TERRAIN_HEIGHT_BLEND=0/1` with
  `SOMNIUM_TERRAIN_EYE=1`: 776.8 mean absolute luminance over 921 600 terrain
  pixels, mesh and sky bit-identical. See §25.12.
- **25F** — a flat plain from a high camera: the tiling grid must not be findable.
- Every sub-phase keeps `cargo test --workspace` green, currently 209 tests.

---

## 17.6 Phases 26-33 — the systems Somnium does not have (plan)

### 26.1 What the survey actually showed

Surveying `example_repo/New_Engines` (Flax, Wicked, Esoterica, Stride, NeoAxis,
Overload, Falco, rbfx) against Somnium's nine crates — `core`, `ecs`,
`renderer`, `physics`, `physics_sys`, `audio`, `asset`, `ui`, `voxel` — the
useful finding is not a list of rendering features to copy.

Somnium's renderer is, feature for feature, already close to Flax's. Comparing
`FlaxEngine/Source/Engine/Renderer/` against Phase 24 turns up the same passes
solving the same problems: ambient occlusion, atmosphere pre-compute, colour
grading, contrast-adaptive sharpening, depth of field, eye adaptation, histogram,
motion blur, shadows, screen-space reflections, volumetric fog. Nearly all of it
is either shipped or already planned in 24.

The gap is everywhere else. Flax ships 37 engine modules; Somnium has nine
crates. What is missing are not renderer features — they are the systems that
turn a renderer into an engine:

| Flax module | Somnium equivalent |
|---|---|
| `Animations` (+ `AnimationGraph`) | **nothing** |
| `UI` / `Render2D` (canvas, controls, text) | `somnium_ui`, editor panels only |
| `Content` / `ContentImporters` / `Streaming` | `somnium_asset`, direct glTF load |
| `Profiler` | **nothing** |
| `Navigation` | **nothing** |
| `Particles` | a CPU emitter inside the renderer |
| `Networking` / `Online` | **nothing** |
| `Localization` | **nothing** |
| `Input` | ad-hoc keycode matching in `hello_engine` |
| `Video` | **nothing** |

That imbalance is the finding worth acting on. Phase 24 has spent its length
deepening a renderer that was already competitive, while nine whole subsystems
sit at zero.

### 26.2 Proposed phases

**Phase 26 — UI framework.** `somnium_ui` draws the editor's own panels and
nothing else; there is no way for a *game* built on Somnium to have a UI at all.
Needs a retained widget tree with dirty tracking, a layout pass (flex / anchor /
grid), SDF text with real shaping and kerning, input routing with focus and
capture, 9-slice and styling, and canvases in both screen and world space. **The
editor should then be rebuilt on top of it** — dogfooding is the only thing that
keeps a UI framework honest, and it would retire the inspector's hand-positioned
popups and the cycler that replaced them in 17G. References: Flax
`Engine/UI/UICanvas` and `GUI/`, Wicked `wiGUI` / `wiFont`, Stride `Stride.UI`,
rbfx's Urho-derived UI.

> **Shipped (2026-08-13):** 26-A–I plus UX polish (immersive play, ComboBox
> overlay, 80 px drawer tiles) are in the tree. **The UI phase is not over** —
> later features still need chrome. 26-J is out unless requested; 26-H SDF
> remains slipped. Phase VV (Halcyon) VV-A–H is in the tree
> (`dev records/halcyon_context_handoff.md`); remaining Halcyon work is
> evidence captures, not a UI rebuild. Contract:
> [`dev records/phase_26.md`](dev%20records/phase_26.md). The paragraph
> above is the original gap statement; do not treat it as the implementation
> order, and do not restart at 26-A.

**Phase 27 — Skeletal animation.** GPU skinning with a joint palette, clip
sampling, blend trees, a state machine, IK, root motion, and animation events.
Esoterica is the reference to study hardest: its animation system is the most
serious of the eight surveyed, built around a graph with compile-time validation.
Flax's `AnimationGraph` shows the authoring side through Visject. Skinning has to
reach the visibility buffer, which is the one place this touches Phase 24's work.

**Phase 28 — Asset pipeline: cooking, hot reload, streaming.** Assets load
directly from source files at startup today — 101 MB of foliage re-parsed on
every run, and 17H had to cache *failed* glTF imports to stop the paint brush
stalling on a retry. Needs an offline cook to an engine-native format, content
hashing, a runtime streaming budget with LOD residency, and hot reload.
References: Flax `Content` / `ContentImporters` / `Streaming`, Stride
`Stride.Assets`.

**Phase 29 — Profiler and debug tooling. ✅ Complete (GPU half; see §29.1).** There is no way to answer "why is
this frame slow" beyond guessing. That cost real time in 17G, where a 51x
draw-call regression was found by reasoning rather than measurement. Needs CPU
zones, GPU timestamp queries per pass, a frame graph view, and counters for
draws, triangles, instances and memory. **Absorbs 24AB** (lighting debug views),
which belongs in a tooling phase rather than a lighting one. References: Flax
`Profiler`, Wicked `wiProfiler`, Esoterica's debug views.

**Phase 30 — Navigation and AI.** Navmesh generation from level geometry,
A* with funnel smoothing, agent steering and avoidance, off-mesh links.
References: Flax `Navigation`, Esoterica `Navmesh` — both Recast/Detour-derived.

**Phase 31 — GPU particles and VFX.** The current emitter simulates on the CPU
and draws billboards; Phase 11.5J said outright that this was a starting point.
Needs compute-driven simulation, sorting, depth collision, ribbons and trails,
and mesh particles. Wicked is the standout reference here — `wiEmittedParticle`
and `wiHairParticle` are among the better open implementations, and the hair one
is directly relevant to Phase 25's grass.

**Phase 32 — Networking.** Replication, client prediction, rollback, transport.
References: Flax `Networking` / `Online`, Wicked `wiNetwork`. Lowest priority of
the eight unless a multiplayer target appears; listed for completeness.

**Phase 33 — Input, localization, video.** Grouped because none justifies a
phase alone: an input abstraction with action maps and rebinding (keycodes are
currently matched inline in `hello_engine`), string tables with runtime locale
switching, and video playback for cutscenes. References: Flax `Input`,
`Localization`, `Video`.

### 26.2b Second pass — O3DE, Falco, NeoAxis, Overload

The first pass leaned on Flax and Wicked and only skimmed these four. Going back
through them changes the plan in three places and adds five systems that were
missed entirely.

**O3DE is the richest reference of all of them, and was under-used.** Its ~80
Gems are a catalogue of what a full engine contains, and several are systems
Somnium has no equivalent of and that were absent from the first plan:

| Gem | What it covers |
|---|---|
| `EMotionFX` + `MotionMatching` | Production animation, including **motion matching** — well beyond a blend tree |
| `Prefab` | **Prefabs / nested instancing.** Somnium has no prefab concept at all |
| `Maestro` | **Cinematics and a sequencer** — track view, keyframed properties |
| `Vegetation` + `GradientSignal` + `SurfaceData` + `LandscapeCanvas` | **Rule-driven procedural scattering.** Far past Phase 17's brush: gradients, surface tags, exclusion, a node graph to author it |
| `NvCloth`, `AtomTressFX` | **Cloth and hair/fur simulation** |
| `ScriptCanvas` + `ScriptEvents` + `GraphCanvas` + `GraphModel` | Visual scripting, and a reusable node-graph editor framework |
| `LyShine` + `UiBasics` + `TextureAtlas` | Runtime UI with atlasing |
| `RecastNavigation`, `Multiplayer`, `Profiler`, `Streamer` | Confirm Phases 30, 32, 29, 28 |
| `WhiteBox` | Level blockout modelling in-editor |
| `SaveData`, `LocalUser`, `GameState` | The game-framework layer — save slots, profiles, state stack |
| `DiffuseProbeGrid`, `Meshlets`, `SkyAtmosphere`, `Stars` | Already covered by Phase 24 |

**Falco** is a complete worked example of the thing Phase 16 has been vague
about: a **C# scripting API over a C++ engine**. `FalcoEngine/API/` mirrors the
engine as managed `Components`, `Assets`, `Input`, `Math`, `UI`, `PostProcessing`
namespaces, and `FalcoEngine/Editor/` carries a matching `*Editor2.cpp` per
component type. It is Unity's architecture, small enough to read end to end —
worth studying for the binding boundary and the per-component inspector pattern
rather than for any single feature.

**NeoAxis** ships `RoslynPad`, meaning **C# compiled and hot-reloaded inside the
editor**, plus a component/property model driving generated UI. The relevant idea
for Somnium is the reflection-driven inspector: properties declare themselves and
the editor draws them, instead of every component needing hand-written panel code
the way Somnium's inspector does today.

**Overload** is the smallest and cleanest of the eight — `OvRendering`,
`OvCore`, `OvUI`, `OvWindowing`, `OvPhysics`, `OvAudio`, `OvTools`, `OvDebug`,
`OvEditor`, `OvGame`, `OvMaths`. No standout feature; its value is as a model of
**module boundaries** for an engine this size, which is a fair comparison point
for how Somnium's nine crates are drawn.

### 26.2c Revisions to the plan

Three changes, and five additions:

- **Phase 26 (UI) gains a reflection-driven inspector.** From NeoAxis and Falco:
  components describe their own properties and the editor generates panels. This
  is not cosmetic — every new component in Somnium currently needs inspector
  code written by hand, which is why the Foliage panel ended up with a cycler
  rather than a popup in 17G.
  **Metaphor v1 (2026-08-13) did not ship this.** 26-E still hand-builds
  Details on Checkbox/Combo/PropertyRow so the chrome rewrite is not blocked on
  a reflection system. Tracked as optional 26-J in
  [`dev records/phase_26.md`](dev%20records/phase_26.md). Metaphor itself stays
  open: new components still need inspector UI until 26-J exists.
- **Phase 27 (animation) gains motion matching** as a later sub-phase, from
  O3DE `MotionMatching`. EMotionFX joins Esoterica as the primary reference.
- **Phase 30** is confirmed as Recast/Detour-based by three independent engines.

**New phases:**

- **Phase 34 — Prefabs and scene composition.** Nested prefab instances,
  overrides, propagation. Somnium can save a flat scene and nothing else. This is
  arguably more urgent than several planned phases: without it there is no way to
  reuse anything built in the editor. Reference: O3DE `Prefab`.
- **Phase 35 — Procedural scattering, rule-driven.** Replaces the brush-only
  model of 17A/17F with gradients, surface tags, slope and altitude rules, and
  exclusion volumes. Reference: O3DE `Vegetation` + `GradientSignal` +
  `SurfaceData`, authored through `LandscapeCanvas`. Pairs naturally with
  Phase 25 terrain.
- **Phase 36 — Cinematics and sequencer.** Keyframed properties, camera cuts,
  event tracks. Reference: O3DE `Maestro`.
- **Phase 37 — Cloth and hair.** Reference: O3DE `NvCloth`, `AtomTressFX`,
  Wicked `wiHairParticle`.
- **Phase 38 — Game framework.** Save slots, user profiles, a game state
  stack. Small, and the difference between an engine and a tech demo. Reference:
  O3DE `SaveData`, `LocalUser`, `GameState`.

**Revised order:** 26 (UI, now including the reflection-driven inspector), 29
(profiler), **34 (prefabs)**, 27 (animation), 28 (asset pipeline), 35
(scattering, alongside Phase 25), then 31, 30, 36, 37, 38, 33, with 32 last.

Prefabs move early on the same reasoning as UI: both decide whether the engine
can be *used* to build something, as opposed to what it can display.

### 26.3 Renderer items still worth taking from Flax

Four things Flax has that Phase 24 does not, all in deferred-GI territory:

- **DDGI** (`Renderer/GI/DynamicDiffuseGlobalIllumination`) — probe-based
  dynamic GI. A cheaper and more robust tier than 24L's ReSTIR GI, and the right
  answer on hardware where ray query is slow rather than absent. Sits beside
  **24Q**.
- **Global Surface Atlas** (`Renderer/GI/GlobalSurfaceAtlasPass`) — a surface
  cache in the Lumen sense, letting a ray resolve against cached shading instead
  of re-shading its hit. Relevant to **24M**.
- **SMAA** (`Renderer/AntiAliasing/SMAA`) — a better spatial AA than the FXAA
  Somnium ships, which matters whenever TAA is switched off.
- **Lightmap baking** (`Engine/ShadowsOfMordor`) — Flax's offline lightmapper.
  The static-scene tier below every dynamic GI option, and the one that runs on
  anything.

### 26.4 Suggested order, and why

**26 (UI) first.** It is the largest capability gap and the one that decides
whether anything can be *built* with the engine rather than merely shown by it.

**29 (profiler) second.** It is small, and it makes every phase after it
verifiable with numbers instead of screenshots. This matters more than it sounds:
a full session was lost to a screen-capture frame-delta metric that turned out to
vary from 0.776 to 2.018 across three runs of an identical build. A GPU timestamp
would not have done that.

Then **27 (animation)** and **28 (asset pipeline)**, which are what a real
project hits first. Then 31, 30, 33, with 32 last.

---

## 17.7 Phase 29 — the profiler

**What it is.** One `wgpu::QuerySet` of timestamps, written from the encoder
*around* each pass, so no pass had to be modified to become measurable. Results
are read one or more frames later through a three-deep ring of mapped readback
buffers and smoothed over thirty frames. `crates/somnium_renderer/src/profiler.rs`.

Two things came out of reading the references rather than from the phase
description:

- **Wicked `wiProfiler.cpp`** — the deferred readback is the whole design.
  Waiting on a resolve would make the profiler the most expensive thing in the
  frame and change the number it exists to report. Its guard against nonsense
  timestamps is ported too: one frame of driver garbage would otherwise poison a
  thirty-frame window, and `end - begin` the wrong way round on `u64` is not a
  small error, it is roughly six centuries.
- **Flax `ProfilerGPU.h` / `RenderStats.h`** — events carry a **depth**, which
  is what turns a list of passes into a frame graph, and the timings travel with
  **counters** (draws, triangles, instances, TLAS instances). A pass time says
  how long something took and never why; "why" is nearly always one of the
  counters.

**`TIMESTAMP_QUERY_INSIDE_ENCODERS`, not just `TIMESTAMP_QUERY`.** The plain
feature only permits timestamps declared in a pass descriptor's
`timestamp_writes`, which would have meant threading query indices through every
pass in the engine. The encoder form lets the profiler bracket a pass from
outside. Detected, never demanded — the same pattern as GPU-driven rendering and
ray tracing; an adapter without it still runs, with counters and no timings.

**The bug that made it silent.** Nothing in the engine polls the device per
frame — the only two `poll` calls are blocking waits for a specific readback —
so `map_async` callbacks never fired, `ready` never flipped, and the profiler
reported nothing at all while looking entirely healthy. `after_submit` now polls
with `PollType::Poll`. Worth recording because the failure mode is *no output*,
not a wrong number, and there is nothing in the code to point at.

**What it says about this scene** (debug build, 1280×720, terrain scene):

```
Frame                  2.354 ms
  Shadows              0.324    Visibility (phase 1) 0.053    Hi-Z    0.044
  GTAO                 0.214    Volumetrics          0.105    Shading 0.869
  Water                0.012    Transparent          0.001    TAA     0.133
  Bloom                0.231    Post + present       0.050
unattributed           0.317 ms
257 draws / 288 460 tris / 256 terrain chunks / 257 TLAS instances
```

`unattributed` is printed instead of a total, which would only repeat the `Frame`
row. It is the passes not yet bracketed — culling, the second visibility phase,
ReSTIR, IBL, the editor overlays — and it is the honest statement of how much of
the frame the profiler still cannot see.

**Its first real job was 25D**, which had to express its cost win in *texture
reads* through a debug shader because there was no clock on the GPU.
`SOMNIUM_TERRAIN_DETAIL_FADE=0/1`, same viewpoint:

| pass | fade off | fade on |
|---|---|---|
| Shading | 0.973 ms | **0.883 ms** (−9.2%) |
| Shadows | 0.324 | 0.324 |
| GTAO | 0.214 | 0.215 |
| unattributed | 0.317 | 0.316 |

Every pass the change should not touch is identical to the third decimal, which
is the control. So 25D's −32% in texture reads buys −9.2% of the shading pass —
texture reads were not the whole cost of it, and that is a thing the tap counter
could not have said.

**In the editor**: a `[x] Profiler` toggle on the viewport toolbar and an
overlay panel pinned to the top-left of the viewport, showing the same tree with
live numbers. The toggle drives collection as well as visibility — a hidden
profiler that keeps writing timestamps is paying for a measurement nobody reads.
Headless, `SOMNIUM_PROFILE=1` prints the table every `SOMNIUM_PROFILE_EVERY`
frames (default 120), which is how the 25D table above was produced.

**Not done, from the phase description:** CPU zones (only GPU work is timed),
memory counters, and a frame-graph *view* beyond the indented tree. 24AB's
lighting debug views were absorbed into the phase and already exist as
`SOMNIUM_SHADOW_DEBUG` modes 1–12.

---

## 17.8 Phase 24L — ReSTIR GI

24K resampled *direct* light: which of the sun's samples a pixel can see. 24L
resamples the other half of the rendering equation — light that arrived by
bouncing off something else. The estimator is the same; the sample space is not.
**A DI reservoir holds a direction to a light; a GI reservoir holds a point in
the world** — where the ray landed, its normal, and the radiance leaving it
toward us. That difference is the whole of ReSTIR GI, and it is what makes a
neighbour's sample reusable: two pixels a few centimetres apart see the same lit
patch from slightly different angles, and a Jacobian converts between them.

Ported from `bevy_solari/src/realtime/restir_gi.wgsl`. Three things came out of
reading it that were not in the phase description:

- **The reconnection-shift Jacobian and its rejection threshold.** Reusing a
  neighbour's *point* means looking at the same patch from a different position;
  the solid angle it subtends and the cosine at its surface both change, and the
  estimator is only unbiased if that is divided out. Bevy rejects above 1.2 —
  past it the shift is a bad approximation and the sample adds variance instead
  of removing it.
- **Fixed buffer roles, not a ping-pong.** The spatial pass reads its
  *neighbours'* reservoirs, so reading and writing one buffer would be a data
  race and a double-count at once. `gi_a` is the previous frame's finished set,
  `gi_b` the handoff between the two dispatches.
- Bevy's `NO_WORLD_CACHE` path lights the sample point directly, which is what
  Somnium does — there is no world cache here.

**`global_pool.wgsl`** came out of this phase and matters beyond it. The
`@group(0)` scene bindings used to live at the top of `shading.wgsl`; they are
now their own module, concatenated into both passes. A **ray hit resolves through
the same `instances` array a visibility-buffer hit does** — one description of
the scene, not two that could disagree. The TLAS's `instance_custom_data`
changed from `vertex_offset` to the instance-buffer index to make that lookup
exact: `instance_index` on an intersection is the TLAS *slot*, and instances
without a BLAS are skipped during the build, so the two drift apart the moment
one mesh is missing.

**Terrain bounce albedo.** A ray landing on terrain would otherwise take
`base_color`, which is white, and the ground would bounce colourless light into
everything above it. Evaluating the real eight-layer composite per bounce is out
of the question, so `GpuTerrainMaterial::layer_albedo` carries each layer's
**mean linear albedo** and the hit blends them by the splat weights — two
texture reads. Averaged in linear space, because the mean of sRGB bytes is not
the sRGB of the mean.

**Measured.** `SOMNIUM_RESTIR_GI=0/1` at eye level: **51.2** mean absolute
luminance over 921 600 terrain pixels, 342 748 past the 1% threshold, mean
luminance 4527 → 4476 — slightly *darker*, which is the right direction for
replacing a constant ambient with a real occluded bounce. Cost, from the Phase 29
profiler: **0.788 ms** for two full-res dispatches at 1280×720.

**Honest limit:** the effect is conservative in the demo scene, an open hillside
under a bright sky where a constant ambient and a real bounce largely agree. GI
earns its keep in enclosed and occluded geometry, and the scene has none. That
is a scene limitation, not a pass limitation.

**Not done:** specular GI (the cubemap still supplies the specular lobe), a
world cache, a denoiser, and multi-bounce — the sample point is lit by the sun
directly, so this is a one-bounce solution.

---

## 17.9 Phase 24AE — shadow caster culling

The profiler's first real finding on a working scene: **Shadows at 24.5 ms of a
42 ms frame**, with 8 599 draws and 52.9 million triangles issued *four times*,
once per cascade. Most of it was grass whose shadow is a sub-pixel speckle. The
main view has stopped drawing distant foliage since 17G; the shadow pass never
learned to.

**Two cuts, deliberately independent.**

**1. Projected screen radius** — Unreal's `r.Shadow.RadiusThreshold`
(`ShadowSetup.cpp`):

```
draw = radius² > threshold² · distance²        i.e.  radius / distance > threshold
```

Two things about it are easy to get wrong. The distance is from the **camera**,
not the light — the question is whether anyone would see the shadow, which is a
screen-space question. And it is a **size** test, not a distance cut: a tree
keeps casting at 200 m because its radius is metres, a tuft stops at 30 m
because its radius is centimetres. One rule that scales itself to the object,
which is why UE uses it in place of a per-asset shadow distance. `casts_shadow`
in `pass/shadow.rs`, five tests, `SOMNIUM_SHADOW_RADIUS` to tune (UE ships 0.01;
0 disables).

**2. An authored foliage shadow distance** — `FoliageComponent::foliage_shadow_distance`,
default 40 m, editable as **Sh Dst** in the Foliage inspector. The size test is
the right general rule but only rescues you once the *camera* is far from the
grass, which is not how anyone plays. At eye level the field fills the frame and
every tuft is still large enough to pass the radius test. This is the dial for
that case, and it is nearer than the draw distance on purpose.

**Measured** (eye-level foliage camera, `SOMNIUM_FOLIAGE=1`):

| | casters | Shadows | Frame |
|---|---|---|---|
| neither cut | 7 166 / 7 166 | 23.769 ms | 26.893 ms |
| both | 1 873 / 7 166 | **6.158 ms** | **9.545 ms** |

Every other pass is unchanged to the third decimal — Visibility 0.042/0.045,
GTAO 0.482/0.478, Shading 0.895/0.902 — which is the control. From a distant
camera the radius test alone takes it further: 362 of 1 069 casters, Shadows
0.669 ms.

`shadow casters N of M draws` is now a profiler row and an overlay row, because
a `Shadows` time that has grown is nearly always this ratio having grown.

**One structural change:** `ShadowPass::record` takes
`&[ShadowCaster { instance_index, index_count }]` rather than the draw queue.
The pass used a draw's *position in the queue* as its instance index, so
filtering into a new `Vec<DrawCommand>` would have renumbered them and paired
every draw with another mesh's transform.

**Not done:** per-cascade frustum culling of casters (a caster is in or out for
all four cascades), and a fade rather than a hard cut at the foliage distance.

---

## 17.10 Phase 24AC (CAS half) — contrast adaptive sharpening

TAA resolves a jittered history into a stable image and charges softness for it:
every frame is a weighted average of several sub-pixel offsets, so the highest
frequencies the renderer produced are exactly the ones it loses. An unsharp mask
gives them back and gives the noise back with them, haloing every high-contrast
edge on the way.

CAS is AMD's answer: sharpen by an amount **derived per pixel from the local
contrast**. Where the neighbourhood already spans most of the available range
there is nothing safe to add, so it adds nothing; where it is flat there is
headroom, so it sharpens hard. Detail without ringing, which no fixed-strength
filter achieves at any setting.

Ported from `SpartanEngine-master/data/shaders/amd_fidelity_fx/` — `cas.hlsl`
and the `CasFilter` no-scaling path in `ffx_cas.h`. Spartan compiles it with
neither `CAS_BETTER_DIAGONALS` nor `CAS_SLOW`, so `shaders/cas.wgsl` follows the
same configuration: a cross-shaped soft min/max and the green channel's weight
applied to all three, which is AMD's own default. The kernel is

```
0 A 0
A 1 A     A = sqrt(headroom) · peak,   peak = -1 / lerp(8, 5, sharpness)
0 A 0
```

`A` is **negative** — a ring of negative lobes around a centre of 1.0, divided
by the sum of the weights.

Three things came out of reading it rather than from the phase line:

- **The headroom is relative**, divided by `mx`. A dark region and a bright one
  with the same contrast *ratio* get the same treatment, which is what stops CAS
  over-sharpening shadows.
- **The `sqrt` shaping matters.** The raw ratio falls away too fast and leaves
  mid-contrast detail — most of an image — barely touched.
- **`APrxLoRcpF1` / `APrxLoSqrtF1` are deliberately not ported.** They are
  bit-trick approximations for hardware where reciprocal and square root are
  slow; the exact forms are what AMD's own `CAS_GO_SLOWER` selects, and this is
  one full-screen pass at the end of the frame. The approximate reciprocal also
  hides a trap: exact `1/0` on a fully black neighbourhood is infinity, and
  `0 * inf` is NaN. `max(mx, 1e-5)` guards it, and a test pins it.

**Where it runs.** On the tone-mapped LDR image, immediately after
post-processing (or after FXAA when that is active), writing the swapchain —
the same handoff `FxaaPass` uses, so the two chain without either knowing about
the other. CAS measures headroom before clipping, which only means anything once
the signal is in the 0..1 range it will be displayed in.

Placed **before** the gizmo, outline and UI passes on purpose. Those draw into
the surface afterwards, and sharpening a one-pixel gizmo line or a font glyph
would only ring it.

**Measured.** Cost **0.018 ms** at 1280×720, reported by the Phase 29 profiler
as a nested scope under `Post + present`. Effect, from two swapchain grabs of
the same static view with `SOMNIUM_CAS=0/1`: mean absolute difference **13.6 of
765** (1.8%) across the viewport, **max 210**, **64.5%** of pixels changed by
more than 2/255 — a small mean with large peaks at edges, which is the signature
of a sharpening filter rather than a level shift.

**Caveat on that measurement:** the capture harness reads the HDR target
*before* tone mapping, so it is blind to CAS by construction and reports a
bit-identical frame. The numbers above come from desktop grabs of the swapchain
instead, which means they carry whatever frame-to-frame variation a converged
TAA image still has. The cost figure and the unit tests are exact; the pixel
diff is indicative.

**Not done:** the SPD half of 24AC — single-pass downsample for the Hi-Z pyramid
and the bloom chain. `ffx_spd.h` and `spd.hlsl` are the reference and the phase
row stays open for it.

---

## 17.11 Phase 24AD — the velocity buffer

Where each pixel was on the previous frame, in UV space, written once and read
by anything that walks backwards through time. `shaders/velocity.wgsl`.

Reconstructing the world position from this frame's depth and projecting it with
the previous frame's matrix gives the exact motion of a **static** point under a
**moving camera**. Two details from
`WickedEngine-master/shaders/visibility_velocityCS.hlsl` are what make it usable
rather than nearly right:

- **The background gets a velocity too.** A pixel with no geometry is treated as
  a point just inside the far plane along its own ray. Returning zero there
  would leave the sky the one part of a whip pan that does not blur, which reads
  as a hole punched through the motion. (Just *inside* 1.0, because exactly 1.0
  un-projects to infinity and its previous-frame projection divides by ~0.)
- **The result is clamped to ±1 screen.** A pixel that reprojects far off-screen
  would otherwise hand motion blur a gather direction hundreds of screens long,
  and every tap would read the same clamped edge texel.

Wicked subtracts the TAA jitter from both ends; Somnium's matrices are
un-jittered instead, which is the same correction one level up —
`TaaPass::record` had already established that both ends of a reprojection must
be un-jittered, having measured 51 000 of 51 000 pixels reprojecting wrongly
with a still camera when they were not.

**Not covered: objects that move on their own.** That needs the previous frame's
model matrix to travel with the instance, and there is nowhere to put it — the
draw queue is re-sorted every frame, so instance `i` is not the same object it
was last frame, and there is no stable per-object id to key a history on.
Nothing in the engine currently moves independently of the camera: there is no
skinning, no wind, and rigid bodies do not write transforms. Phase 27 is when
that changes. The note is in the shader so the gap is found by reading rather
than by watching a smear stay still.

**Cost: 0.010 ms.**

---

## 17.12 Phase 24Z — motion blur, and 24Z closed

The half that had been waiting on 24AD since the phase was written. A real
shutter is open for a slice of each frame and smears whatever moved during it; a
renderer that samples one instant produces a sequence of sharp frames, which
reads as strobing.

From `WickedEngine-master/shaders/motionblurCS.hlsl` (Jimenez, *Next Generation
Post Processing in Call of Duty: Advanced Warfare*, SIGGRAPH 2014), two weights
are ported and one piece is deliberately not:

- **`DepthCmp`** classifies each tap as in front of or behind the centre. A
  moving foreground should bleed *over* a static background; a static foreground
  must not be smeared by a background moving behind it. Without it a fast pan
  drags the silhouette of everything static in the frame.
- **`SpreadCmp`** asks whether the tap's own blur is long enough to reach the
  centre at all, which stops a still object from picking up colour from a fast
  one merely because it is nearby.
- **Tile-max / neighbourhood-max is not ported.** Reducing velocity to tiles and
  gathering along the tile maximum is what lets a fast *object* blur outside its
  own silhouette. It costs two more reduction passes and only pays off for
  object motion, which 24AD does not produce; under camera motion the whole
  frame moves together and the centre velocity and the neighbourhood maximum
  agree almost everywhere. This is Wicked's own `MOTIONBLUR_CHEAP`
  configuration, taken for the reason it offers it.

Also ported: the dithered start offset (a fixed one bands, because every pixel's
taps land on the same grid) and the `sum.rgb + (1 - sum.a) * centre` fallback,
so coverage no tap could legitimately account for keeps its own colour instead
of fading toward black.

**Placed before TAA and on the HDR image.** Before TAA because TAA's history is
what stabilises the gather's dither, and blurring the resolved image would smear
a frame already blended with its own past. On HDR because a blur after tone
mapping smears clipped highlights as flat white — the difference between a
headlight trail and a grey smudge.

**Off by default** (`SOMNIUM_MOTION_BLUR=1`, or the inspector). It is the one
effect that makes a still screenshot of a moving camera look broken rather than
better. **Cost: 0.034 ms** with the camera moving; it early-outs below half a
pixel of motion, so a static frame pays 0.001 ms.

---

## 17.13 Phase 24U — closed with temporal reprojection

The froxel volume shipped in the earlier session with fog, height falloff, a
Henyey-Greenstein phase and a per-froxel shadow test. What it lacked was the
thing that makes a low step count affordable.

**Two halves, and only together are they worth anything:**

- **A per-frame jitter of the sample position within each step.** A fixed
  midpoint samples the same points every frame, so a thin medium is either
  always hit or always missed and the error is a *stationary* pattern — banding
  that sits still while the camera moves, which is the most visible kind. The
  offset is golden-ratio rather than random, because over a short window the
  samples should spread evenly through the step and a random sequence clumps.
- **Reprojection through world space.** The froxel grid is attached to the
  camera, so a froxel does not keep its identity across a move; reprojecting the
  froxel centre through the previous view-projection is what makes the history
  mean the same piece of air. Blended at 0.05, about twenty frames — a third of
  a second at 60 Hz, below where a fog change reads as lag.

Jitter without reprojection is just noise; reprojection without jitter only
smears the same bias. Together they turn the error into something the temporal
filter can average away.

**History is a copy, not a ping-pong.** 32×32×32 RGBA16F is 256 KB, so
`copy_texture_to_texture` at the end of the pass costs less than the alternative
— a ping-pong pair would force the shading pass to rebuild its bind group every
frame, since it binds the volume by view.

**Cost: 0.071 → 0.093 ms**, which is the copy and the extra fetch.

**Still not visually confirmed: light shafts.** The code path is exercised every
frame and the shadow test is in the integral, but the demo scene has no low sun
behind a hard occluder, so nobody has *seen* a shaft. That needs a scene, not a
change — and this note stays until someone has looked at one.

---

## 17.14 Phase 24AC — SPD, and 24AC closed

The Hi-Z pyramid cost one dispatch per mip: **eleven** at 1280×720, each a
pipeline barrier behind the last, each reading a texture the previous one had
just written. The arithmetic is trivial; the *dependency chain* is the cost.

SPD's observation is that a workgroup owning a 64×64 tile of the source can
compute six mip levels of that tile entirely in its own shared memory — after
the first reduction the whole tile fits there. A dispatch boundary is only
needed when a level draws on more than one tile, and by then the image is 64×
smaller. Ported from `SpartanEngine-master/data/shaders/amd_fidelity_fx/` —
`ffx_spd.h`'s `SpdDownsampleMips_0_1_LDS`, the no-wave-operations path, which is
the right one because WGSL has no subgroup quad swizzles.

**The last-workgroup trick is deliberately not ported, and this is the
interesting part of the phase.** SPD does the whole pyramid in one dispatch: a
global atomic counter elects the workgroup that finishes last, and that one
reads mip 6 — written by *other* workgroups — and carries on. That requires
`globallycoherent` storage images. **WGSL has no such qualifier**, and its memory
model offers no way to make one workgroup's texture writes visible to another
inside a dispatch; `storageBarrier` is workgroup-scoped. Porting it anyway would
be a data race that happens to pass on one driver.

So the same shader is dispatched **twice**, separated by a real barrier: once
over the whole image for six mips, once with a single workgroup to finish the
tail from the sixth. Eleven dispatches become three — the depth copy, then these
two — the structure is SPD's, and nothing rests on undefined behaviour.

Two details carried over from the original per-mip shader because the pyramid
feeds occlusion culling:

- The reduction is **`max`** — a texel holds the *furthest* depth of its region,
  so the test can never make an occluder look further away than it is.
- **Odd sizes widen to three.** Halving 5 gives 2, and a plain 2×2 reduction
  drops source column 4 — a real occluder vanishing from the pyramid, which is
  the one error direction that rejects visible geometry.

**Measured.** Hi-Z **0.045 → 0.029 ms** (−36%), and the rendered frame is
**bit-identical**: `SOMNIUM_SPD=0/1` through the capture harness gives
`mean_abs = 0.0000, changed = 0` over all 921 600 pixels, with the same 257
draws and 288 460 triangles surviving culling. For something feeding occlusion
culling that identity is the acceptance test, not the speed.

**One device requirement.** Six storage textures in one stage, where wgpu's
default ceiling is four. `context.rs` asks the adapter for up to eight in the
"detect, do not demand" style the file already uses, and `HiZPass` checks the
*granted* limit before building anything — a device that cannot manage six logs
a line and keeps the per-mip chain. `SOMNIUM_SPD=0` forces that path anyway,
which is the A/B.

**Not done:** SPD is not yet used for the bloom chain, which still downsamples a
level per pass. The reduction there is a filtered average rather than a max, so
it needs a second variant of the shader; the pyramid was the one with eleven
barriers in it.

---

## 17.15 Phase 25H — parallax occlusion on terrain

Terrain is the surface most often seen at a grazing angle, and that is exactly
where a normal map stops working. It shades a flat plane as though it had
relief, but the relief never *moves* against the surface, so the ground reads as
a photograph lying on glass. Parallax fixes the one thing a normal map cannot:
it displaces where each texel appears, so a pebble occludes the crack behind it
and the surface gains depth as the camera moves.

**Marching in metres, not UV.** The textbook formulation walks tangent-space UV.
Somnium's terrain has a world-aligned tangent frame and **eight layers with
different tiling**, so a UV offset would mean something different for each of
them. Marching in world XZ metres gives one offset that is correct for all
eight: every layer converts it with its own tiling exactly as it converts the
position. It also means `LayerBlend::parallax_depth` is authored in metres and
does not silently change meaning when a layer's tiling is edited.

**One height field, not eight.** Marching a *blended* height would mean sampling
every contributing layer at every step — eight times the cost of the most
expensive loop in the frame. The march runs against the dominant layer's height
map and the resulting offset is shared, which is right: the layers are all lying
on the same piece of ground. O3DE's `MultilayerParallaxDepth.azsli` does blend
per step; that is the more correct answer and it is not worth eight times the
taps here.

**References.** `bevy/crates/bevy_pbr/src/render/parallax_mapping.wgsl` for steep
parallax plus the single-lookup POM refinement — and for the reason every fetch
is `textureSampleLevel`: a `textureSample` inside a loop needs derivatives,
which forces the compiler to unroll a loop whose bound is dynamic.
`o3de/.../ParallaxMapping.azsli`'s `AdvancedParallaxMapping` for the second
half.

**That second half is what sells it: parallax self-shadowing.** From the point
the view ray actually hit, march *toward the sun* through the same height field;
every step that ends up under the surface darkens the result, weighted by how
far along the march it is so a nearby occluder casts a harder edge than a
distant one. Without it a pebble moves correctly and is still lit as though
nothing were beside it. It is folded into `shadow_factor`, not into `occlusion`,
because that is what it is — a second occluder between the point and the sun,
one far too small for the shadow map to have ever resolved. Occlusion is an
indirect quantity and mixing them would darken the sky's contribution with a
shadow the sun casts.

**It rides 25D's budget.** The step count is `parallax_steps * (1 - fade)`,
using the distance fade Phase 25D already computes, so parallax reaches zero at
the same range the layer count does — one budget, not two. The self-shadow fades
with it, or it would pop off at the distance the steps run out.

**Measured** at eye level, `SOMNIUM_TERRAIN_PARALLAX=0/1`: **1729.8** mean
absolute luminance over 921 600 terrain pixels, 898 589 of them past the 1%
threshold. Mean luminance moves only 4475.3 → 4458.9 (−0.4%), which is the
useful part of that pair: the effect *redistributes* detail rather than
darkening the frame, which is what displacement should do and what a
self-shadow term applied too broadly would not.

**Cost:** shading **0.898 → 1.022 ms** (+14%) at eye level, where nothing is
faded out. GTAO is identical to the millisecond either way, which is the control.

**Controls.** `Relief` in the Terrain inspector multiplies every layer's
authored depth, so one dial covers the terrain without flattening the difference
between gravel and mud; 0 switches it off. `SOMNIUM_TERRAIN_PARALLAX=0` is the
same switch for the A/B.

**Not done:** silhouette clipping. The march displaces texture but the geometry's
edge is still the mesh's edge, so relief does not break the outline of a ridge
against the sky. O3DE handles that with a pixel depth offset written from the
parallax result (`CalcPixelDepthOffset`), which needs the depth output of the
visibility pass to move — a change to a pass every other feature reads.

---

### 25.14 Three new sub-phases, from looking at a scene with foliage in it

Planned, not started. Each names what the screenshots showed, what the code
actually does, and what the reference does instead.

---

## 25M — Night, twilight, and the sun below the horizon

**What the screenshots show.** Rotating the sun gizmo below the horizon turns the
terrain deep red with black blotches and bleaches the foliage white. With fog and
shafts disabled the terrain goes near-black with white speckles and the sky keeps
a bright band at the horizon. Both are wrong in a way that says "the maths left
its valid range", not "it is night now".

**Confirmed cause.** `ray_intersects_ground` appears **nowhere** in Somnium's
shaders (`grep` over all 35 of them returns zero). Bevy's atmosphere applies it
at the point the sun's contribution is gathered:

```wgsl
let transmittance_to_light = sample_transmittance_lut(local_r, mu_light);
let shadow_factor = transmittance_to_light * f32(!ray_intersects_ground(local_r, mu_light));
```
(`bevy_pbr/src/atmosphere/functions.wgsl`)

Without that factor, a sun below the horizon still samples the transmittance LUT.
The LUT is parameterised on `mu = sun.y`, and below the horizon the lookup clamps
to its last valid row — **the reddest one**, because at grazing angles Rayleigh
has scattered out everything but red. So the engine keeps lighting the world with
the reddest possible sunlight instead of switching the sun off. That is the red.
Bevy also guards the sun disc itself with `max(mu_light, 0.0)`
(`functions.wgsl:501`), which Somnium does not.

**Plan.**

1. Port `ray_intersects_ground(r, mu)` into `atmosphere.wgsl` and apply it
   wherever the sun's transmittance is fetched: the sky, the aerial-perspective
   integral, and the volumetric pass's per-step `sun_transmittance`.
2. Gate the direct sun term on `max(mu_sun, 0)` so `light.color` stops lighting
   surfaces from below the world.
3. Give the sun a real twilight ramp rather than a hard cut — attenuate through
   the last few degrees so sunset is a transition, which is the whole reason to
   sample a transmittance LUT at all.
4. **Then re-check the two remaining artefacts against instrumentation**, because
   they are hypotheses until measured:
   - *White foliage.* Likely auto-exposure: as the scene darkens the meter drives
     EV up with no night-appropriate floor. Test with `SOMNIUM_AUTO_EXPOSURE`
     off and a fixed EV; if it disappears, the fix is exposure limits.
   - *White speckles at night.* Likely ReSTIR GI fireflies — a bounce that finds
     a bright sliver has nothing clamping its radiance, and the effect only shows
     once the frame is dark enough for one pixel to dominate. Test with
     `SOMNIUM_RESTIR_GI=0`; if it disappears, add a luminance clamp on the
     initial candidate the way every production ReSTIR does.
5. **Acceptance:** a sun rotated from noon to below the horizon produces a
   believable day → dusk → night, which is already written down as Phase 24's
   own definition of done (§22.4) and has never been checked.

This also finally gives light shafts (24U) the scene they need: a low sun behind
a ridge is exactly the case that has never been rendered.

---

## 25N — Analytic gradients for visibility-buffer shading

**What the screenshots show.** Foliage is simultaneously blurry and aliased —
some patches mushy, neighbouring ones crawling with sharp speckle, and the
character of it changes as the camera moves. Terrain in the same frame is clean.

**Confirmed cause.** The shading pass reconstructs UVs per pixel from the
visibility buffer and then samples with **implicit derivatives**:

```wgsl
surface.albedo *= textureSample(textures[material.albedo_map], default_sampler, uv).rgb;
```
(`shading.wgsl:672`, and five more like it)

`textureSample` takes its mip level from `dpdx/dpdy` across the 2×2 quad. In a
full-screen resolve, a quad routinely straddles **different triangles and
different instances**, so the difference between neighbouring UVs is not a
derivative at all — it is the gap between two unrelated surfaces. The mip that
comes out is arbitrary: too high on one pixel (mush), zero on the next (alias).
Foliage is worst hit because it has many tiny triangles and a high-contrast
cutout texture. Terrain escapes it because the terrain path already computes
`world_ddx/world_ddy` explicitly and uses `textureSampleGrad` — the fix is to do
for meshes what terrain already does.

**Reference.** Wicked's `surfaceHF.hlsli` (`SURFACE_LOAD_QUAD_DERIVATIVES`)
evaluates the *same triangle's* UVs at the neighbouring pixels' barycentrics and
differences them:

```hlsl
uvsets_dx = uvsets - attribute_at_bary(uv0, uv1, uv2, bary_quad_x);
uvsets_dy = uvsets - attribute_at_bary(uv0, uv1, uv2, bary_quad_y);
```

No quad ever crosses a triangle boundary, because the neighbour is evaluated
analytically rather than read from a neighbouring lane.

**Plan.**

1. `shading.wgsl` already builds barycentrics analytically from the triangle's
   NDC positions. Evaluate that same expression at `target_ndc` offset by one
   pixel in x and in y, giving `bary_quad_x` / `bary_quad_y`.
2. Interpolate UV at all three and difference: `uv_ddx`, `uv_ddy`.
3. Replace every mesh `textureSample` in the shading and transparent paths with
   `textureSampleGrad`.
4. **Acceptance:** a still frame of foliage at a fixed camera, captured with the
   harness, must show the mip-level debug view changing smoothly across a leaf
   rather than in per-pixel jumps; and the A/B must be visibly sharper without
   raising aliasing. Wicked also derives a *ray-cone* LOD for its ray-traced
   path — worth noting as the follow-up for ReSTIR GI's texture fetches, which
   currently guess a fixed mip 4.

---

## 25P — Foliage instancing and LOD

**What the screenshots show.** With trees and grass painted: **9 047 draws,
90 963 841 triangles**, Visibility (phase 1) **9.25 ms** and Shading **7.44 ms**
of a 23.5 ms frame. The two most expensive things in the frame are the geometry
prepass and the material resolve, in that order.

**Confirmed cause (draws).** `submit_foliage` pushes **one `DrawCommand` per
part per instance**. Every tuft of grass and every tree part is its own draw,
its own instance-buffer entry, and its own `rpass.draw` in both the visibility
pass and (until 24AE culled most of them) the shadow pass. The engine already
has an instance buffer and the visibility pass already draws by instance
range — nothing is *batched* into it.

**Confirmed cause (triangles).** There is no foliage LOD at all. The palette
meshes are the full-detail glTF (~1.5 M triangles across the four assets) and
every instance draws every triangle at every distance. 17G's distance cull only
decides whether an instance exists, not how detailed it is.

**Plan, in the order the profiler says to do it.**

1. **Batch identical parts into instanced draws.** Group the foliage batch by
   `(vertex_offset, index_offset, material_id)`, write their transforms
   contiguously into the instance buffer, and issue one draw per group with an
   instance range. ~8 790 draws should collapse to roughly the number of palette
   parts. This is a submission change only — no shader work — and it is the same
   mistake 17G found and fixed once already at a different layer.
2. **Mesh LODs per palette entry.** Pick by projected screen radius, reusing
   `pass::shadow::casts_shadow`'s ratio test so the engine has one definition of
   "how big is this on screen".
3. **Impostors for the far band.** Neither Wicked nor Flax ships a generic
   impostor system in the copies here, so this is the one part with no reference
   to lean on and should be scoped last, after 1 and 2 have been measured.
4. **Shading cost.** 7.44 ms is largely the terrain material — eight layers, hex
   tiling and now a 24-step parallax march — over a much larger viewport than
   the 1280×720 the earlier numbers were taken at. Before adding anything,
   measure with debug mode 12 (taps) and with `SOMNIUM_TERRAIN_PARALLAX=0` to
   split the terrain material's cost from the foliage's, then consider scaling
   parallax steps by screen-space footprint rather than by distance alone.

**Acceptance:** draws below 500 with foliage painted, and a frame-time
comparison from the Phase 29 profiler for each step, since each of the three is
independently measurable.

---

**Sequencing.** 25M first — it is a correctness bug, it is small, and it unblocks
24U's light-shaft verification. Then 25N, which is a contained shader change with
a large visual payoff. Then 25P, whose first step is cheap and whose later steps
should be judged on measurements taken after the first.

---

## 17.16 Phase 25M — the sun below the horizon

**The plan's first claim was wrong, and finding that out was the phase.** §25.14
said `ray_intersects_ground` appeared nowhere in the engine. It appears
everywhere — under the name `ray_hits_ground`. The grep had been for Bevy's
spelling. Two of the three places that sample the sun's transmittance
(`atmosphere.wgsl:209`, `atmosphere_lut.wgsl:89`) were already guarded.

**The real cause was one level up, on the CPU.** `LightComponent::photometric_color`
returns intensity × tint and nothing else, so a sun authored at 100 000 lux
stayed at 100 000 lux when the gizmo rotated it below the horizon. The engine
went on lighting the world with full noon sunlight arriving from underground.
The atmosphere shaders were behaving correctly; nothing had told the *direct*
light that the sun had set.

**Fix: the sun's illuminance is what survives the trip through the air.**
`somnium_core::sun::transmittance(sun_up, altitude_km)` integrates the same
Rayleigh, Mie and ozone profile `atmosphere.wgsl` uses — the same constants, in
kilometres — along the ray toward the sun, and returns zero once that ray would
have to pass through the planet. Applied where the directional light is uploaded,
which is the one value every consumer reads: shading, shadows, ReSTIR DI and GI,
the froxel volume, and the sky's own `sun_illuminance` and moon blending. There
is nowhere for them to disagree about whether the sun has set.

Two things fall out of integrating rather than fading:

- **Sunset colour is physics, not a gradient.** Rayleigh removes blue first, so a
  low sun comes out orange on its own. `a_low_sun_comes_out_orange` pins the
  blue/red ratio at 0.05 elevation against noon.
- **The horizon crossing is soft.** The cut is at −0.86°, not zero: the sun's
  disc is about half a degree across and refraction lifts it by roughly another
  half, so light keeps arriving after the geometric centre has set. A hard cut
  at zero steps visibly in the one moment anybody is watching.

Six tests, including that brightness falls **monotonically** all the way down —
a sunset that brightens anywhere flashes as it crosses the step.

**Second fix: the froxel volume's missing guard.** `volumetric.wgsl` was the one
place that sampled `sample_transmittance` without `ray_hits_ground`. Below the
horizon the LUT lookup clamps to its last valid row — the reddest one, because at
grazing angles Rayleigh has taken out everything but red — so every froxel went
on being lit by the reddest possible sunlight. That is the red frame with fog
enabled.

**`SOMNIUM_SUN_ELEVATION` / `SOMNIUM_SUN_AZIMUTH`** place the sun in degrees at
startup. Reproducing a bug by rotating a gizmo by hand is not a test; this makes
dusk and night a capture like any other, and it is what finally gives **24U's
light shafts** the low sun behind a ridge they have never been verified against.

**Measured**, HDR terrain luminance at the landscape camera:

| sun elevation | terrain | sky |
|---|---|---|
| +35° (day) | 4362 | 18 447 |
| +2° (dusk) | 137.7 | 283.6 |
| −10° (night) | 0.0001 | 0.0004 |

Dusk renders as a real golden hour — a warm low sun raking across the terrain
with long shadows under a blue-grey sky — and night is black with stars instead
of red.

**What is not settled.** At night the terrain reads specular-dominated, like
foil. Diffuse is essentially zero while the environment cubemap still reflects a
faint sky, so whatever specular remains is *relatively* the whole image, and the
capture harness's PNG uses a fixed exposure that amplifies a near-black frame
enormously. That is very likely a property of how it is being *looked at* rather
than of what is rendered, and the honest next step is to judge it on screen with
auto-exposure running before changing anything. The two hypotheses §25.14 listed
— exposure with no night floor, and unclamped ReSTIR GI fireflies — are still
untested for the same reason: the red was drowning them, and now that it is gone
they need a fresh measurement rather than a guess.

---

### 25.15 Phase 25M-2 — what 25M left behind

Planned, not started. 25M stopped the sun lighting the world from underground.
Everything below is what became visible once it did. Four of the five have a
cause confirmed by reading the code, not a guess.

---

## A. Dusk is an explosion of orange and the shadows are black

**Confirmed cause, and it is a comment in our own source.** `shading.wgsl`:

> *"Physically this should be 1.0, but the engine has no ambient occlusion yet,
> so sky light reaches every surface unattenuated … At full strength that washes
> shadows out badly. Until SSAO (or a glTF occlusion map) lands, the indirect
> term is scaled back so shadow contrast survives."*

`ibl_intensity` is **0.35**, a fudge written before the engine had ambient
occlusion. It now has GTAO (24I), bent normals, per-material AO (25K) and
ray-traced indirect diffuse (24L). The condition the fudge was waiting on has
been met three times over and nobody went back to remove it.

At noon it barely shows, because the sun dominates. At dusk the sun is deep
orange and the *sky* is the blue fill that balances it — and that fill is being
run at a third strength. So the orange has nothing to balance against and the
shadows have almost nothing in them. This is one number, and it is the single
most likely cause of both halves of the complaint.

**Plan.**

1. Raise `ibl_intensity` to 1.0 and re-judge. Expect dusk shadows to fill with
   blue sky and the orange to stop dominating; expect noon to change very little.
2. If noon then looks flat, that is GTAO's strength to answer, not the sky's —
   the AO dials are already in the inspector.
3. **Check the tonemapper is doing its job on saturated colour.** AgX desaturates
   as it approaches white, which is exactly what stops a saturated orange from
   reading as a flat sheet; ACES does it differently and Reinhard barely at all.
   The A/B is a `CycleTonemapper` away and costs nothing.
4. Only then consider a sunset-specific saturation limit. It should not be
   needed, and reaching for it first would paper over 1–3.

---

## B. Blocky shadows on the terrain at a low sun

**Confirmed cause.** `shadow/cascade.rs` fits each cascade to the sub-frustum's
bounding sphere and then builds:

```rust
let light_eye = center + light_dir * radius * 2.0;
let near = 0.0;
let far  = 4.0 * radius;
```

The shadow map therefore only contains casters inside a slab `4 × radius` deep,
centred on the view slice. **A hill outside that slab casts nothing.** At noon
the slab is deep enough relative to how far shadows travel; at 2° elevation a
shadow runs tens of times its caster's height and the caster that should be
producing it is behind the near plane. That is what the large hard-edged
straight boundaries in the dusk screenshot are — not filtering, not resolution:
the caster is simply missing from the map.

The second, smaller part is texel footprint. `texel_size = 2 × radius /
resolution`, and a texel projected onto ground at elevation θ covers
`texel / sin θ` — at 2° that is 28× its noon size. Even a correct shadow map
looks stair-stepped there.

**References.** Unreal computes the caster extent along the light axis
separately from the receiver bounds (`ShadowSetup.cpp`, the subject-Z fitting in
`FProjectedShadowInfo`) rather than assuming a fixed multiple of the radius, and
exposes `r.Shadow.CSMSlopeScaleDepthBias` and `r.Shadow.TransitionScale` for what
is left. O3DE's `DirectionalLightShadowCalculator.azsli` scales its slope bias by
`tanTheta = sin/cos` of N·L, which grows exactly as the sun gets low — and its
own comment says the slope bias "exhibits noticeable artifacts" and that
**normal-offset bias is preferable**, which is what
`NormalOffsetShadows.azsli` implements: offset the lookup along the geometric
normal by a multiple of the shadow-map texel size.

**Plan.**

1. **Extend the near plane to include casters.** Compute the scene's extent along
   the light direction and push `light_eye` back by it, instead of `radius * 2`.
   This is the fix for the hard boundaries and it is a change to one function.
2. **Scale the normal offset by the real texel size and by grazing angle.**
   Somnium's `SHADOW_NORMAL_OFFSET_TEXELS` is a fixed 1.5. O3DE's formulation
   makes it a function of the map dimension; the grazing term is what low sun
   needs.
3. **Then re-judge whether traced shadows should simply take over at low sun.**
   ReSTIR DI (24K) already produces sun visibility with no texel grid at all, and
   its `RAY_BIAS_FOOTPRINTS = 6.0` bias — sized for a *pixel* footprint — is the
   thing to check before adding more cascade machinery. A traced shadow has none
   of this problem by construction.
4. **Acceptance:** the dusk capture at `SOMNIUM_SUN_ELEVATION=2` must show no
   straight-line shadow boundaries that do not correspond to terrain.

---

## C. Stars are rectangles

**Confirmed cause.** `atmosphere.wgsl::star_field` quantises the *direction* into
cubic cells, keeps one star per cell, and returns zero for any pixel whose cell
did not win:

```wgsl
let cell = floor(dir * cell_scale);
if h < 0.987 { return vec3<f32>(0.0); }
let falloff = pow(max(dot(dir, star_dir), 0.0), 40000.0);
```

Two things go wrong together. A star near a cell edge is **clipped by the cell
boundary**, because the neighbouring pixel belongs to a different cell that has
no star — so what should be a round dot is cut to the cell's quadrilateral. And
`pow(x, 40000)` in fp32 is a step function, not a falloff: values below about
`1 - 1e-5` underflow straight to zero, so there is no soft edge to hide it. A
cell-clipped step function is a rectangle.

**Plan.**

1. Evaluate the **3×3 neighbourhood of cells**, not one, so a star is never cut
   by the boundary of the cell that owns it.
2. Replace the `pow` with an explicit angular radius and a `smoothstep` against
   the **pixel's own angular footprint**, so a star is antialiased to its true
   sub-pixel size instead of being a hard dot the size of whatever `pow`
   survives. This is the same reasoning 25H used for `textureSampleLevel`: a
   quantity that must not depend on precision luck.
3. Give brightness a plausible magnitude distribution rather than a uniform
   `mix(0.002, 0.02)` — a few bright stars and many faint ones is what a sky
   looks like.
4. **Note:** TAA and CAS both act on sub-pixel points. Check the result with TAA
   off before blaming the star code for what a sharpening filter did to it.

---

## D. The moon

**Confirmed cause.** There is no moon. `night_sky_ambient` draws
`pow(dot(dir, moon_dir), 700) * 6` — a halo with no disc, no phase, no limb
darkening and no surface. The white blob in the screenshot is that halo clipped
by the tonemapper, with bloom around it.

**Plan.** Give it the same treatment the sun disc already gets in `sky_detail`:
a real angular radius (~0.26°, the same as the sun's, which is why eclipses
work), a phase term from the moon's direction relative to the sun, limb
darkening, and an intensity in the right units so bloom does not eat it. Keep
the halo — that part is real scattering — but as a separate, much dimmer term.

---

## E. Night specular: "lights shining on the foliage weirdly"

**Cause, partly confirmed.** With the sun gone, diffuse is essentially zero
(terrain HDR luminance 0.0001 against 4362 at noon), so the only surviving term
is image-based **specular** from the environment cubemap, which still holds a
faint sky. Anything with a low roughness then reads as wet metal, and foliage —
which has a 4% dielectric Fresnel at grazing angles — catches it worst. §17.16
already noted this and declined to chase it before looking at it on screen with
auto-exposure; the screenshots now show it is real and not a capture artefact.

**Plan.**

1. Check `specular_occlusion` is being applied to the night path at all — it
   exists (Lagarde & de Rousiers) and is exactly the term that should be killing
   this.
2. Check the environment cubemap is regenerated when the sun moves. If it holds
   a daytime sky, night specular is reflecting a sun that has set. **This is the
   first thing to measure**, because if true it also explains part of A.
3. Foliage specular at grazing angles wants a roughness floor; leaves are not
   mirrors. Related to the 17E remainder that has been open since Phase 17.

---

**Sequencing.** A (one number), then B-1 (one function), then E-2 (a measurement
that may explain part of A). C and D are self-contained and can go last, since
they are appearance rather than correctness. Each is independently capturable
with `SOMNIUM_SUN_ELEVATION`.

---

### 25.16 Phase 25M-2 audit and correction — 2026-08-11

The completion handover was checked against both call sites and the cited
reference implementations. The broad A–D work was present, but several details
meant the night result could still be black, green or unstable:

- The renderer's `ibl_intensity = 1.0` initializer was overwritten every frame
  by `PostProcessComponent::default()`, which was still 0.35. The component is
  the authoritative default and is now 1.0.
- `evaluate_brdf` already contains N·L, so the moon path's second N·L made its
  response N·L² and its attempted backside factor could never revive a backface.
  The moon now follows the same single-evaluation contract as the sun.
- Two-sided foliage was oriented toward the sun. Once the sun crossed the
  horizon this turned the geometric normal downward, also corrupting moonlight
  and shadow bias. It is now oriented toward the viewer, shadow lookup uses the
  unperturbed geometric normal, and the transmission lobe follows Unreal's
  wrapped backside-lighting shape without the old constant green ambient term.
- The foliage palette changed imported `BLEND` materials to `MASK` but did not
  preserve their vegetation semantics. Those materials now become foliage,
  double-sided and (when the source supplies no value) 50% transmissive; alpha
  sidecar detection also sets the foliage flag its own comment promised.
- ReSTIR GI reservoirs survived material changes to sun direction and colour,
  preserving daytime green bounce into a night frame. An accumulated 0.25° or
  2% colour change now invalidates GI history before temporal or spatial reuse;
  the threshold preserves reuse while the sun animates smoothly.
- The one-bounce GI estimator only samples direct sunlight at its bounce point.
  At night it nevertheless wrote an alpha-valid black result, replacing sky
  IBL, while rare emissive-mesh hits became the moving yellow/green fireflies.
  Zero-sun frames now emit an invalid traced result so IBL remains active, and
  emissive hits are rejected as in Bevy Solari until they can be importance
  sampled instead of discovered by chance.
- The earlier CSM change projected only receiver-frustum corners and therefore
  could not discover off-frustum casters. Cascades now reserve a 1 km depth
  extension, following Flax's extended directional-shadow culling range. The
  erroneous conversion of a world-space texel length directly into NDC depth
  was removed; world-space geometric-normal offset remains the grazing bias.
- The moon threshold represented a disc about six times too wide and its sphere
  normal was not tangent at the limb. It now uses a 0.2666° angular radius and a
  tangent-plane reconstruction with derivative antialiasing.
- The moonlight default is 0.010 lux at all three authoritative initialization
  layers (scene component, renderer state and GPU-light fallback), matching the
  user-accepted night capture.
- The remaining daytime polygons were not PCF resolution or terrain parallax.
  Receiver offset was using an interpolated vertex normal mislabeled as
  `geo_normal`, which can push a lookup behind its own coarse terrain triangle;
  it now uses the true face normal. Contact shadows also compared their 5 cm
  thickness against nonlinear NDC depth, turning that threshold into many
  metres across a landscape. They now reconstruct scene depth and compare
  linear view-space metres.
- Below-horizon solar transmittance again clamps the LUT direction to the
  horizon while the separate horizon fade switches direct sunlight off. This
  avoids integrating a ray pinned to the planet surface.

`cargo check --bin hello_engine` and the automated suites pass, including shader
validation and lunar full/new-moon direction tests. Workspace-wide
`cargo fmt --all -- --check` still reports pre-existing formatting drift in
unrelated files. A deterministic −10° capture before the GI correction showed
the reported moving yellow/green emissive blotches; the same capture afterward
has stable IBL fill without them. Its telemetry was `terrain px=921600, mesh
px=0`: although 18,278 foliage instances and both grass assets loaded, that
camera did not place foliage in the visibility buffer. Final foliage and +2°
dusk-shadow acceptance therefore remain and this phase is not marked visually
complete until those checks are recorded.

---

## 17.17 Phase IV-A–IV-E — Great Lakes landscape and finite water

**Completed 2026-08-11.** The default heightmap is now a deterministic
1025×1025 16-bit derivative of Motion Forge Pictures' FLOAT32 Great Lakes EXR.
The importer preserves floating-point EXR channels, verifies the audited source
range, area-resamples height, masks water out of the sRGB macro-colour map, and
bakes a 2048×2048 water mask, shoreline SDF, and 0–12 m synthetic bathymetry.
The baked dry terrain floor is 0.35 m above its original 15 m extraction datum.
The accepted default runtime water level is 16.1 m, which keeps the visible
surface above the residual terrain-grid intersection while the authored wet mask
still prevents water from spreading across dry land. This avoids coplanar
ground and water.

The daytime triangle-shaped terrain shadows were not present in the source
height field. Phase 25M-2 had made shadow receiver bias use a per-face normal for
all geometry, turning each terrain triangle into a different bias plane. Terrain
now uses its continuous interpolated geometric normal; ordinary meshes retain
the face-normal path. Debug modes 13–17 expose terrain LOD, triangle edges,
geometric and receiver-bias normals, shadow factor, and contact-shadow factor.

`WaterComponent` is now a small serializable ECS handle containing its terrain
relationship, preset, body kind, bounds, datum, maximum depth, enabled state,
and editable optics/wave settings. Heavy mask/depth/SDF CPU and GPU data lives
in the renderer's `WaterBodyRegistry`. Default startup and **Create → Terrain**
both create a `Terrain` and child `Water` hierarchy; composite create/delete
undo, duplication, inspection, serialization, and resource reconciliation are
tested. Asset provenance is in `assets/terrain/great_lakes/README.md`; reference
patterns are in `ATTRIBUTION.md` §13.31.

Validation: `cargo check --workspace --all-targets`; 202 renderer tests, 31 core
tests, and 3 UI tests pass in release mode. The importer was executed twice and
all output hashes matched. Current release-mode live wgpu evidence is organized
under `dev records/phase IV` rather than in the repository root.

**IV-D/E completed 2026-08-11.** The broad terrain-sized water plane is gone.
Each body builds a compact 2 m terrain-local mesh only over wet coarse cells,
then the full-resolution baked mask/depth/SDF performs exact fragment coverage.
The same deterministic four-band Gerstner contract drives the WGSL surface and
CPU surface-height/normal/depth/velocity/containment queries. Water writes
surface coverage plus motion vectors, and TAA uses those vectors only on water
while preserving opaque depth reprojection elsewhere.

Surface optics now use validated screen-space refraction, reconstructed
Beer–Lambert path length, RGB absorption and single scattering, dielectric
`F0 = 0.02037`, GGX sun/moon/environment lighting, bounded SSR with environment
fallback, and SDF shoreline foam. **Phase VV (Halcyon, 2026-08-13)** later
blends that SSR with half-res hardware ray tracing and the environment cube
(`SOMNIUM_RT_REFLECT=0` restores this IV-D/E fallback). Complete normal/ORM mip chains plus
pixel-footprint slope filtering prevent distant sparkle and Gerstner
cross-hatching. Wave and optical authoring values persist through ECS scene
serialization and their primary controls are available in the Water inspector.

Validation: `cargo check --workspace --all-targets`; 204 renderer tests, 31 core
tests, and 3 UI tests pass in release mode. Live wgpu post-TAA day and -20° sun
captures are `dev records/phase IV/IV-D-E/IV-D-E_day_post-TAA.png` and
`dev records/phase IV/IV-D-E/IV-D-E_night_post-TAA.png`.

**IV-F/G/H completed 2026-08-11.** The cinematic water tier now owns two
deterministic GPU inverse-FFT cascades (256² over 192 m and 512² over 53 m).
The compute chain evolves a wind spectrum, executes radix-2 ping-pong inverse
transforms, and writes displacement, gradients, horizontal-displacement
Jacobian, and temporally decayed foam history. Gerstner remains the deterministic
baseline and CPU-query contract. Serialized water authoring now includes
spectral blend, wind speed, foam decay/threshold, caustic strength, and an
underwater enable. Crest folding, shoreline SDF/depth, and wet-sand darkening
share one foam signal.

The post-TAA underwater HDR pass selects the finite body beneath the camera,
uses a smooth per-pixel near-plane submersion mask, reconstructs the submerged
ray segment, and applies RGB extinction, HG in-scattering, fog, sun/moon shafts,
and depth/turbidity-faded bed caustics. The surface renders two-sided with an
underside/TIR transition. Its transition and shaft WGSL is original and does
not translate the Shadertoy-cited helpers found in Wicked's underwater shader.

`DefaultLandscapePreset` v1 now owns the default terrain descriptor, Great
Lakes relief/material threshold, transforms, water datum, camera, and post
process. Normal startup and **Create → Terrain** both call
`create_default_landscape`; editor creation remains one undoable transaction
containing separate Terrain and Water entities. The old `WaterPlane` path is
removed. Release validation passed 33 core tests, 208 renderer tests, 9 shader
module tests, 3 UI tests, and every remaining workspace target. Live evidence
is `dev records/phase IV/IV-F-G-H/IV-F-G-H_surface_day.png`,
`IV-G_underwater_deep.png`, and `IV-G_waterline_transition.png` in that folder.

**IV-I/J completed 2026-08-11.** The default scene adds Opus Poly's CC BY 4.0
Gislinge Viking Boat as an unchanged 29,035-triangle multi-node GLB with its
embedded materials and a separate stable Jolt proxy hull. Fixed-step
environment simulation runs at 60 Hz in both Editing and Playing. Eight hull samples use the existing deterministic CPU
water query for distributed buoyancy, drag, and propulsion; the resulting
heading and speed drive analytic Kelvin-angle wake arms and prop-wash foam in
the water pass. The viewport toolbar now exposes Play, Pause/Resume, and Stop;
pausing freezes gameplay, physics, particles, and water time, while stopping
restores the vessel pose and clears velocities before live editor preview resumes.
From Play until Stop, including a paused play session, the renderer suppresses
the grid, transform and light gizmos, selection outline, and terrain/foliage
authoring cursors so the viewport contains only player-visible scene content.

Water's near-shore presentation now retains the 2048² source contour and uses a
bilinearly reconstructed, derivative-antialiased SDF boundary, a two-cell
raster guard ring, 1.5 m under-terrain dilation, a foam width in world metres,
scene-depth contact foam, noise-broken breakers, and a three-band rotated normal
detail stack with distance fade. Terrain chunks whose vertical range crosses
the water datum are held at LOD 0; neighbor relaxation preserves crack-free
transitions outside the shore band. The full-resolution terrain depth now hides
the dilated surface, so coarse distance-LOD facets cannot define the visible
shore. This visually softens the terrain/water intersection without changing
the licensed source elevation data or allowing visible water onto dry terrain.
Complete vessel provenance,
license, hash, scale, and render/physics separation notes live in
`assets/models/gislinge_viking_boat/README.md`; future screenshots belong in
`dev records/phase IV/IV-I-J/`, never the repository root. The current
post-TAA shoreline validation is
`dev records/phase IV/IV-I-J/IV-I-J_shoreline_lod_validation.png`.

**Phase IV ocean-spectrum refinement completed 2026-08-12.** The existing
two-cascade GPU inverse FFT remains Somnium's implementation, but its initial
Phillips-style spectrum is replaced by a finite-depth JONSWAP/TMA energy model
with Hasselmann directional spreading, swell shaping, high-frequency detail
control, and finite-depth dispersion. The authored `wind_speed` now rebuilds
the deterministic spectra when it changes instead of merely scaling animation
time, so calm and storm presets alter the actual wave-energy distribution.

The surface pass now mixes four-tap cubic B-spline and hardware-bilinear
gradient samples according to world-space pixel density, retaining close slope
detail while suppressing distant cascade aliasing. Jacobian compression is
preserved as an explicit crest mask. Foam history receives periodic spatial
feedback before exponential decay, making crest white water spread instead of
remaining sharp simulation texels, and compressed/back-lit crests add a small
shallow-water scattering contribution. These are adaptations of the published
GodotOceanWaves equations and Sea of Thieves rendering description; no
third-party textures were required. Naga validates all Phase IV water modules,
and the deterministic spectrum, wind response, parameter layout, and control
smoothing tests pass.

**Phase IV-K — ocean fidelity pass, completed 2026-08-13.** IV-K supersedes the
two-cascade simulation described immediately above. The full record, including
the mathematics and every deviation from the reference, is in
`dev records/phase_IV.md` section 14; the short version:

- **Three cascades at 1024²** (tile lengths 88 m, 57 m, 16 m) replace the two
  previous ones, with four complex spectra packed per Stockham inverse-FFT pass
  and butterfly factors precomputed once. Every spectrum parameter — wind,
  fetch, swell, spread, detail, whitecap, foam amount, seed — is now authored
  per cascade in `WaveCascadeParams` rather than shared globally.
- **Whitecaps come from the Jacobian.** Displacement, slope, horizontal
  stretch, and fold are unpacked in one pass; foam accumulates additively into
  an `r32float` history at a fixed 50 Hz step and decays exponentially. Foam is
  deliberately excluded from the Gerstner/spectral crossfade, so dialling
  `spectrum_blend` back cannot erase whitecaps that already formed.
- **Surface lighting follows the GDC 2019 *Atlas* model**, with four departures
  that a physically scaled deferred renderer forces. Diffuse is albedo-weighted
  and normalised by π rather than the reference's unitless `0.5 * ndotl`, which
  otherwise renders the entire lake white. *Two* Fresnel curves are evaluated:
  the reference's suppressed curve for the direct sun highlight, and a plain
  Schlick curve capped by reflection blur for the environment split, because
  one curve for both jobs makes water read as wet stone. Total internal
  reflection is gated on the camera being below the fragment, so a
  choppiness-folded wave is not mistaken for the Snell window and turned into a
  white shard. Both subsurface-scattering terms are gated on the viewer facing
  the sun, not just the height term.
- **Three items did not ship** and are recorded as such rather than left
  implied: the ocean clipmap body kind (K-1) and the HDRI/Filmic environment
  (K-7) are deferred, and GPU sea spray (K-6) was abandoned after two emitters
  placed particles incorrectly. The reference's spray texture and its
  attribution stay in the tree for a later attempt.

Evidence is in `dev records/phase IV/IV-K/`. The authored body that ships is
`WaterComponent::great_lakes`, captured in `ivk_authored_water_body.png`.

**Phase XV — Appalachia (XV-A through XV-J complete 2026-08-13).**
Thirty-two global materials so the ground can match IV-K water quality. Live
contract (32 layers, sidecar v4, 1664-byte GPU material, unique colour from
splat, biome v3, aerial hex/POM LOD, frozen Great Lakes water):
`dev records/phase XV/XV-Zeta_plan.md`. Verification record:
`dev records/phase XV/evidence/XV-J_compile_gate.md`. Plan:
`dev records/phase_XV.md`. IV/XV history:
`dev records/post_IV_context_handoff.md`. Current start-here (Halcyon):
`dev records/halcyon_context_handoff.md`. §20 below is still the Phase 14
heightmap record — do not treat it as the XV API. Explicit exceptions: 1.10 ms
shading budget (measured 3.951 ms overview / 5.532 ms walk, release 1280×720)
and BC7 packs (adapter supports BC; encoder ships, packs are local gitignored
artifacts — `dev records/phase XV/evidence/XV-BC7_visual_check.md`).

**Phase 26 — Metaphor (26-A–I shipped 2026-08-13; phase remains open).**
Nocturne editor chrome, docked Content Drawer, Iris colour pickers (26-F),
custom title bar, F1 Help, immersive play, ComboBox overlay. Later engine
features keep needing new UI/UX. 26-J (reflection inspector) not started.
Contract: `dev records/phase_26.md`. Independent of Phase VV except living
chrome for debug views.

**Phase VV — Halcyon (VV-A–H in tree, 2026-08-13).** Water G-buffer prepass,
half-res reflection compute (`pass/water_reflection.rs`, `shaders/water_reflection.wgsl`),
shared `rt_hit.wgsl` (GI wraps `rt_trace`), SSR/RT/env blend on confidence.
TLAS cap 8192; water and transparents stay out of the TLAS. Inspector: water
**RT Reflect** / **Reflect Debug**; Post FX **RT Reflections**. Help:
`docs/editor/water.md`. Start-here:
`dev records/halcyon_context_handoff.md`. Plan: `dev records/phase_VV.md`.
Kill switch: `SOMNIUM_RT_REFLECT=0`. Live SSR miss-rate capture still open.

## 18. Known Issues & Active Bugs

**RESOLVED — finite water coverage and query contract (IV-D).** The renderer
now consumes the Great Lakes bounds, wet/dry mask, depth map, and shoreline SDF
directly. A compact wet-cell mesh bounds raster work; full-resolution mask
sampling owns the exact shoreline. CPU gameplay queries share the shader's wave
parameters and shore attenuation.

**RESOLVED — shattered foliage (visibility-buffer id packing).** The visibility
buffer packed instance id and primitive id into one `R32Uint`, which forces a
trade-off with no good answer:

- **16/16** capped meshes at 65 536 triangles. The island tree's leaf primitive
  has ~714 000, so `prim_idx` wrapped and shading pulled an unrelated triangle's
  vertices — shattered facets showing random atlas fragments beside correctly
  drawn leaves. This had sat in these notes as a benign "warns, wraps".
- **12/20** fixed that and capped instances at 4 095. A densely painted foliage
  scene passes that easily, and then *every* mesh fetches another instance's
  vertices — far worse, and how it was found: the fix shattered the whole scene.

Now `Rg32Uint` with each id in its own channel. Costs 4 bytes per pixel and
removes both caps, so neither failure can return. `MAX_TRIANGLES_PER_DRAW` is no
longer a hardware bound.

**Lesson**: rebalancing bits between two fields that both need more is not a
fix, it is a choice of which cap to hit. The trade only looked acceptable
because the scene under test was sparse enough to hide the other side.

**RESOLVED — TAA shimmer (jittered shadow cascades).** `inv_view_proj` is taken
from the *jittered* matrix, which is correct for reconstructing world position
from a jittered depth buffer, and was then also being fed to
`compute_cascades`. So the shadow cascade frusta shifted by the sub-pixel jitter
every frame, moving every shadow-map texel in world space. TAA cannot average
that out — it is a real change in the scene, not a sampling difference — so it
read as everything vibrating. `jitter_ndc` returns zero when TAA is off, which
is exactly why switching TAA off removed the shimmer, and why the fault looked
like it lived in TAA rather than in what the jitter touched. Cascades are now
fitted from `view_proj_unjittered.inverse()`.

Confirmed by the user directly: shadows no longer jitter. **Do not trust the
frame-delta numbers originally recorded here** — screen-capture frame deltas were
later shown to vary from 0.776 to 2.018 across three runs of an *identical
build*, which is the whole range those comparisons were drawn from. The fix is
sound on its mechanism and on the user's observation, not on that measurement.

Two dead ends recorded so they are not retried: a floor on the variance clip box
(applied twice, measured twice, changed nothing — history sat at 234.1 against
current 250.9 either way), and the `tonemap_for_blend` round trip (its inverse
is exact, and the Catmull-Rom weights sum to 1, so neither loses energy).

**RESOLVED — meshes vibrating with TAA on.** Reconstructing world position with
the *jittered* inverse is geometrically exact but gives `prev_uv = uv - jitter`
for a still camera, so history was fetched from a location that moved every
frame. Measured with `SOMNIUM_TAA_DEBUG=8` (`|prev_uv - uv|` in pixels):
**51 000 of 51 000 pixels off**. Reprojecting entirely in un-jittered space, as
production TAA does, fixed it — confirmed by the user: jitter gone from foliage
and terrain, a little left on the helmet.

The matrices are now provably identical between frames with a still camera
(`SOMNIUM_TAA_MATDBG=1` logs `|unjittered - prev| = 0` from frame 1), so the
reprojection is mathematically identity.

**Mode 8 has a caveat**: it also counts the closest-depth dilation, which
deliberately reconstructs from a neighbour's position. Up to ~1px on detailed
geometry is correct behaviour. Judge a reprojection bug by whether *flat*
regions are off — sky never dilates.

**Second cause, also fixed: closest-depth dilation on smooth surfaces.** Taking
the nearest of the nine unconditionally decides the winner by depth differences
far below a smooth surface's own curvature, and the jitter perturbs exactly those
every frame — the chosen neighbour flips, `prev_uv` jumps a pixel, and history
comes from somewhere new each frame. Foliage was immune because its depth steps
between leaves are real and large, which is precisely the split the user
reported: foliage steady, plane/cube/helmet vibrating. (It was *not* specular
aliasing, which was the first guess and wrong — primitives are matte grey at
roughness 0.5.)

The gate is measured **against the local depth gradient** (`dpdx`/`dpdy` of the
centre depth), not an absolute value. The depth buffer is non-linear, so a fixed
epsilon means centimetres near the camera and tens of metres far from it: it
gated correctly up close and swallowed real silhouettes at distance, leaving
distant foliage shimmering. A multiple of the gradient is unitless and holds at
any distance — a smooth surface varies at roughly the gradient, an edge varies
far above it. `dilation_epsilon` is that multiple (4.0 by default).

Dilation is gated on the **neighbourhood depth spread**: inside a smooth
surface the spread is tiny and the pixel keeps its own depth; at a silhouette
the spread is large and the nearest sample wins outright. An earlier attempt
subtracted a fixed epsilon from every comparison instead — that fixed the smooth
case but suppressed dilation at real silhouettes too, and brought a little
foliage jitter back. The epsilon has to gate *whether an edge exists*, not bias
which neighbour wins. `SOMNIUM_TAA_DILATE_EPS=0` disables the gate entirely,
which the user confirmed is markedly worse.

**Note on measurement**: screen-capture frame deltas are useless here — 0.776 to
2.018 across three runs of one identical build. Every comparison drawn from them
this session was noise. Mode 8 has no run-to-run variance and is what found the
real bug.

**Foliage renders with wrong colours.** Trees show salmon/pink, grass white.
Not yet investigated.


**24K cannot be visually verified until Phase 25A/25B.** The pass dispatches and
shading consumes it, but no surface in the demo view can show the result: the only
visibility-buffer geometry is the helmet, and the ground filling the frame is the
water plane, which shades in its own pass and never samples `restir_vis`.

**Editor primitives spawned at `on_init` do not appear.** `SOMNIUM_SHADOWTEST`
spawns a ground plane and cube; the attach log confirms both get meshes uploaded
and clustered (Plane 6 indices, Cube 36, material 1) and the gizmo draws at the
right transform, but no geometry renders. Cause not yet found — unrelated to
ray tracing, and it is why the 24K test scene had to be abandoned.


| ID | Severity | Component | Description | Root Cause | Fix Status |
|---|---|---|---|---|---|
| BUG-001 | High | renderer | **White screen** — sky gradient not rendering | R32Uint clear with `u32::MAX as f64` bit-casts to `0x4F800000` on DX12 (`ClearRenderTargetView` float path), not `0xFFFFFFFF`; sky check fails | ✅ Fixed: clear to 0, encode inst+1, check `vis_data==0` |
| BUG-002 | Medium | core/ui | **WASD no movement** — camera rotation works (DeviceEvent), keyboard doesn't (WebView steals WindowEvent focus) | wry child HWNDs capture keyboard focus after UI panel click | ✅ Fixed: `focus_window()` on RMB + `DeviceEvent::Key` fallback when `!window_focused` |
| BUG-003 | Low | renderer | `visibility.wgsl` `View` struct missing `inv_view_proj` (80B vs 144B buffer) | Layout mismatch; harmless because `camera_pos` and `inv_view_proj` are unused in visibility pass | ⚪ Won't fix (harmless) |
| BUG-004 | Low | ui | `update_fps` IPC sends `data` field, JS reads `msg.value` | Struct field name mismatch in JS message handler | ✅ Fixed: JS now reads `msg.data` throughout (`update_fps` and `update_outliner`) |
| BUG-005 | High | ui | **Toolbar dropdowns clipped** — File/Edit/Create/View menus render behind the 3D viewport | wry toolbar WebView is 40 px tall; dropdown `overflow:visible` content is clipped by the WebView HWND boundary | ✅ Fixed: JS sends `menu_opened {height}` IPC on open → `UiManager::expand_toolbar()` resizes toolbar WebView to cover menu; `menu_closed` IPC → `collapse_toolbar()` restores 40 px |
| BUG-006 | Critical | ui | **UI buttons produce no response** — clicking any editor button has no effect | `widget.handle` was never set on `add_node`, causing all clicks to route to source `NONE` | ✅ Fixed: Spawning now assigns `widget.handle = handle_nh`. |
| BUG-007 | Critical | core | **Viewport camera movement broken** — WASD + RMB fly-cam navigation no longer responds | Keyboard event consumption was too aggressive or focus tracking got stuck | ✅ Fixed: Only consumes keyboard events if a text input currently has active focus. |
| BUG-013 | Medium | renderer | **Water plane texture seams** — straight lines sweeping across water | CPU mipmap wrapping downsample is leaving faint borders at tile edges on repeating UVs. | ⚪ Pending |

---

## 19. somnium_voxel — Voxel World (Phase 14 Complete)

Reference architecture: bevy_voxel_world (ATTRIBUTION.md §13.10). No GPU dependencies in the crate — meshes are plain `Vec<Vertex>` / `Vec<u32>` handed to the integration layer.

### 19.1 Pipeline

```
camera position                         set_voxel(world_pos, voxel)
      │                                          │
      ▼                                          ▼
VoxelWorld::update()                    edits: HashMap<IVec3, Voxel>
  1. drain finished mpsc results          + dirty flag / version bump on
  2. desired set: radius 5 chunks,          every chunk whose PADDED volume
     y ∈ [-1, 0], LOD by distance           contains the voxel (up to 8)
  3. despawn outside radius+margin
  4. queue tasks (nearest first,
     ≤ max_in_flight = 16)
      │
      ▼  rayon::spawn per chunk
  generate_padded(): sample TerrainConfig + edit overlay  (34³)
  mesh_chunk(): LOD downsample → block_mesh::visible_block_faces
      │
      ▼  mpsc channel
ReadyChunk { coord, lod, origin, Option<ChunkMeshData> }
      │
      ▼  hello_engine::VoxelTerrain (integration layer)
  GeometryPool::upload_mesh_pooled (free-list reuse)
  per-frame DrawCommand per non-empty chunk → visibility buffer
  (shadows / PBR / clustered lights come for free)
```

### 19.2 Key decisions

| Decision | Why |
|---|---|
| Chunks are **not ECS entities** | Streaming churns hundreds of chunks; entities would flood the outliner, undo stack, and serializer. Direct `DrawCommand` submission uses the same render path with none of the editor overhead. |
| Deterministic terrain fn + sparse edit overlay | No persistent voxel arrays. Workers regenerate any chunk from `TerrainConfig::voxel(pos)`; `set_voxel` writes only the overlay. Memory stays flat regardless of world size. |
| Version counter per chunk | An edit during in-flight meshing bumps the version; the stale result is discarded on arrival and the chunk requeues. No locks shared with workers. |
| `GeometryPool` free-list (`upload_mesh_pooled` / `free_mesh`) | The Phase-7 bump allocator never frees; remeshing would leak the 64 MB vertex pool in minutes. First-fit reuse of freed blocks caps growth. `MeshAllocation` now carries `vertex_count` + `*_capacity`. |
| Palette texture material | `Vertex` has no color attribute. Voxel type → texel index in a 6×1 RGBA palette; the mesher writes constant per-face UV at the texel center. One material, one draw per chunk. |
| LOD 0/1/2 = 32³/16³/8³ | Nearest-neighbour downsample of the padded grid before meshing, border kept aligned (bevy_voxel_world algorithm) so cross-chunk culling stays seam-free. Voxel size scales 1/2/4 m so every LOD spans the full 32 m chunk. |

### 19.3 Limits & future work

- Instance budget: the visibility buffer packs 16-bit instance IDs (max 65 535 draws/frame since Phase 15C; was 1022). Radius 5 ⇒ ~160 chunk draws, and off-screen chunks are frustum-culled on the GPU (Phase 15B).
- LOD transitions can show cracks at chunk borders between different LOD levels (known bevy_voxel_world artifact; alpha-fade blending is future work).
- No ambient occlusion baked into chunk meshes (`Vertex` has no color channel).
- Voxel physics colliders (Jolt heightfield/mesh shapes) are future work — terrain is render-only.
- The voxel world remains a separate, optional system; the **heightmap terrain (§20)** is the primary terrain.

---

## 20. Heightmap Terrain System (Phase 14 SSS Complete)

Reference architecture: Fyrox terrain + CDLOD (ATTRIBUTION.md §13.20). A professional
heightmap terrain with sculpting, splatmap texture painting, and chunked LOD —
deliberately **outside** the visibility-buffer pipeline (own vertex stream, own
specialized shader, and 256 chunks would eat a quarter of the 10-bit instance budget).

### 20.1 Architecture

```
ECS                              somnium_renderer
┌─────────────────────┐          ┌────────────────────────────────────────────┐
│ TerrainComponent     │ id ───► │ SomniumRenderer::terrains[id]: TerrainData │
│ (Copy: terrain_id +  │          │  heightmap: Vec<f32>   (CPU authority)    │
│  config mirror)      │          │  chunks: Vec<TerrainChunk>                │
└─────────────────────┘          │    per-chunk vertex buffer (65² Vertex)   │
                                  │  splatmap: RGBA8 tex + CPU copy           │
   app.rs (editor)                │  layer_textures: 3 × texture_2d_array    │
   ── F6 terrain mode             │  index_buffers: HashMap<(lod, edge_mask)>│
   ── brush stroke (LMB)          │  params/model uniforms + bind group      │
   ── undo restore queue          └────────────────────────────────────────────┘
   ── submit_terrains()                            │ render() 7.3
                                                   ▼
                            TerrainPass → HDR target, depth vs vis-pass buffer
                            (depth WRITE on, so the water pass tests against it)
```

- `somnium_renderer/src/terrain/mod.rs` — `TerrainDescriptor` (default 16×16 chunks
  × 64 cells × 1 m = 1024×1024 m), `TerrainData` (height accessors, bilinear
  `world_height_at`, LOD select, dirty-chunk rebuild, ray-march raycast with
  bisection refine, binary sidecar I/O).
- `terrain/mesh.rs` — vertex grid generation (central-difference normals sampled
  from the **global** heightmap so chunk borders shade identically) + LOD index
  buffers.
- `terrain/brush.rs` — `TerrainBrush` (mode/radius/strength/hardness), Fyrox
  falloff (`1 − d/r` then hardness remap), `apply_sculpt`, `apply_paint`
  (normalized RGBA weights), `auto_splat` by slope/height.
- `terrain/textures.rs` — 4 procedural PBR layers (grass/dirt/rock/snow; albedo
  alpha carries a noise "height" for height-based blending), `Splatmap` with
  dirty-row GPU upload.
- `pass/terrain.rs` + `shaders/terrain.wgsl` — splatmap-weighted 4-layer PBR,
  triplanar cliff projection on steep slopes, CSM shadow receive + clustered
  local lights (same algorithms as `shading.wgsl`), in-shader brush cursor ring.

### 20.2 LOD & stitching

- 5 LOD levels (step 1/2/4/8/16 over the 65² vertex grid); per-chunk LOD =
  `clamp(floor(log2(dist / lod_base_range)), 0, 4)`, then relaxed so adjacent
  chunks differ by ≤ 1 level.
- **Block-fan stitching** (CPU, original scheme derived from Fyrox + CDLOD):
  chunks are triangulated in 2×2-cell blocks as fans around the block center;
  the midpoint of a block edge lying on a chunk border with a coarser neighbor
  is omitted, which makes border vertices exactly match the neighbor's spacing
  — watertight, same triangle count as a regular grid. Index buffers are cached
  per `(lod, edge_mask)` and shared by all chunks.

### 20.3 Editing

- **Terrain edit mode**: F6 (or any toolbar terrain tool / `SetTerrainTool`
  event) while a terrain entity is selected; gizmos hidden, LMB applies the
  brush, cursor ring drawn in `terrain.wgsl` (green sculpt / blue paint).
- Brushes: Raise, Lower, Smooth (5×5 kernel over a pre-stroke snapshot),
  Flatten (levels toward the height under the initial hit), Noise, Paint.
  Keys: 1-6 tool, `[`/`]` radius, `-`/`=` strength, `,`/`.` paint layer,
  F7 auto-splat.
- **Undo**: full heightmap/splat snapshot at stroke start; on release the
  affected region's before/after data becomes a `TerrainEditCmd`
  (`push_silent`). Commands can't reach the renderer, so undo/redo pushes
  `TerrainRestoreOp`s onto a shared queue that `app.rs` drains (with renderer
  access) before the next render.
- **Persistence**: `.somnium` JSON stores the terrain config; heightmap (f32 LE)
  + splatmap (RGBA8) go to a `<scene>.terrain<id>.bin` sidecar (`save_binary` /
  `load_binary`).

### 20.4 Limits & future work

- Terrain **receives** CSM shadows but does not cast (not drawn in the shadow pass).
- Scene **loading** of terrain (and meshes) is still the pre-existing LoadScene TODO.
- No frustum culling of chunks yet (256 draws is cheap; AABBs are already tracked).
- Layer set is fixed at 4 procedural layers; file-based layer textures + layer
  management UI are future work (`TerrainLayer.tiling` is already data-driven).
- No terrain physics colliders (Phase 16+), no foliage scattering, no GPU
  tessellation / virtual texturing (explicit non-goals for this phase).
- Demo smoke test: run `hello_engine` with `SOMNIUM_TERRAIN=1` to spawn a
  pre-sculpted 4×4-chunk terrain (hill + valley + auto-splat).

---

## 21. Phase 15 — GPU-Driven Rendering (plan & progress)

Goal: move draw submission and visibility decisions onto the GPU, so frame cost
scales with what is *visible* rather than with what exists. Reference material is
UE5's `InstanceCullingDefinitions.h` / `NaniteDefinitions.h` (ATTRIBUTION §13.12).

Deliberately split into small, independently shippable steps — each one builds,
tests, and runs on its own, so the engine is never left half-converted.

| Step | Status | What it does |
|---|---|---|
| **15A2** | ✅ Complete | **FXAA.** Post-process anti-aliasing in the Post Processing entity, before the editor overlays. |
| **15A1** | ✅ Complete | **Post-processing volume.** Side quest: exposure/vignette/chromatic-aberration became a selectable scene entity, and the always-on vignette was defaulted off. |
| **15A** | ✅ Complete | **Indirect draw plumbing.** `DrawIndirectArgs` buffer built from the draw queue; visibility pass uses one `multi_draw_indirect` call. Feature-gated on `INDIRECT_FIRST_INSTANCE` with a CPU fallback. `F9` A/B-toggles the paths. |
| **15B** | ✅ Complete | **GPU frustum culling.** Per-mesh AABBs at upload; compute pass writes `instance_count = 0` for off-screen draws. Maths unit-tested in `culling.rs`; `F10` toggles. |
| **15C** | ✅ Complete | **Instance cap raised to 65 535.** `vis_data` repacked 10/22 → 16/16. Compaction dropped as unnecessary once the cap was lifted. |
| **15D** | ⬜ | **Meshlet generation.** Split meshes into ~128-triangle clusters at upload with per-cluster bounds + normal cone (UE5 `NANITE_MAX_CLUSTER_TRIANGLES = 128`). |
| **15E** | ⬜ | **Hi-Z occlusion culling.** Depth mip pyramid from the previous frame; two-phase cull so occluded clusters cost nothing. |
| **15F** | ⬜ | **Meshlet rendering.** Draw surviving clusters indirectly through the visibility buffer. |

### Notes for whoever picks up 15D

- Culling verdicts live in `instance_count`; the argument array is never
  compacted, so argument `i` is still instance `i`. Keep that invariant.
- The per-draw AABB array (`cull_aabbs`) is built parallel to the indirect args
  each frame. A mesh with no recorded AABB gets an infinite box and is never
  culled — that fallback is deliberate, since guessing bounds would pop geometry.
- 15C's 16/16 packing leaves **65 536 triangles per draw**. Meshlets (~128
  triangles each) sit comfortably inside that; when 15D lands, the per-draw
  triangle warning in `GeometryPool` should stop being reachable.
- Culling is camera-frustum only. The shadow pass deliberately does *not* use it
  — an off-screen caster still shadows into view. Per-cascade culling would need
  its own planes.

### 15A notes (kept for reference)

- Argument `i` corresponds to instance `i` — the indirect buffer is built **after**
  the draw-queue sort so the two stay aligned. Keep that ordering.
- `instance_count` is the keep/discard flag: culling writes `0`, never removes
  entries, so indices stay stable.
- The buffer is already `INDIRECT | STORAGE | COPY_DST`, so a compute shader can
  bind and write it without any allocation changes.
- What 15B still needs: per-mesh AABBs (compute them in
  `GeometryPool::upload_mesh*`, store alongside `MeshAllocation`), the 6 frustum
  planes uploaded in the view buffer, and a compute pipeline dispatched before
  the visibility pass.
- Verify culling actually happened by reading back the arg buffer (`COPY_SRC`) in
  a test, or by counting non-zero `instance_count` values — not by eyeballing the
  image, since a correct cull is invisible.

## 22. Phase 24 — Advanced Lighting (plan)

The goal is photorealism, and the honest summary of where the engine stands is that
its *materials* are physically based while its *lighting* is not. Shading runs a
correct GGX BRDF, but it is fed by an arbitrary sun multiplier, a hardcoded sky
gradient, a constant ambient term, and a tonemapper that crushes the result. Every
sub-phase exists to close that gap.

### 22.1 Why night does not work today

`shaders/ibl_gen.wgsl` builds the environment cubemap from three constants:

```wgsl
let horizon_color = vec3<f32>(0.5, 0.7, 0.9);
let zenith_color  = vec3<f32>(0.05, 0.1, 0.3);
let ground_color  = vec3<f32>(0.05, 0.04, 0.03);
```

Only the sun disc and its glow read `params.sun_color`. So the sky dome's brightness
is completely independent of the sun, the IBL is prefiltered from that dome, and
`evaluate_ibl` delivers full daylight ambient no matter how far the sun's intensity is
turned down. Lowering the sun removes direct light and leaves everything sitting in
bright blue ambient — which reads as "overcast", never as "night".

This is not a tuning problem and no multiplier fixes it. The sky has to become a
*function of the sun* (24C), which is also what makes sunset, golden hour and aerial
perspective possible.

### 22.2 Ordering, and why hardware ray tracing comes before the software fallback

An earlier draft of this plan sequenced Lumen-style mesh-SDF software tracing first and
treated hardware ray tracing as an optional extra. **Studying `example_repo/` reversed
that**, for three reasons:

1. **The stack already qualifies.** Bevy's Solari needs `EXPERIMENTAL_RAY_QUERY`,
   `TEXTURE_BINDING_ARRAY`, `BUFFER_BINDING_ARRAY`, `PARTIALLY_BOUND_BINDING_ARRAY` and
   non-uniform indexing. Somnium's bindless `GlobalResourcePool` already requires every
   one of those except ray query, so the real gap is a single feature flag, not an
   architecture change.
2. **The target hardware supports it.** Development is on an RTX 4080 Laptop, which has
   full hardware RT. Building the portable-but-much-larger path first would mean writing
   the harder implementation to run on a machine that does not need it.
3. **It is dramatically less code for better results.** Lumen reaches dynamic GI through
   five cooperating caches (SDF scene, surface cache, screen probes, radiance cache,
   reflection denoiser). ReSTIR reaches comparable quality with acceleration structures
   plus reservoir resampling. When the ceiling is higher *and* the implementation is
   smaller, sequencing it second is hard to justify.

The software SDF path (24P) and baked probes (24Q) remain in the plan as the portability
tier, and 24J is required to degrade cleanly when ray query is unavailable — but they
are explicitly *fallbacks*, sequenced after the primary path works.

The overall order is then:

1. **24A–24E — foundation.** Units, exposure, tonemapping, sky, sun. Everything
   downstream is judged by eye, and the eye is currently looking through a broken
   curve; fixing the measurement before the thing measured avoids tuning twice. This
   block also contains the night fix. Expect it to change the look of *everything*,
   including water and the 17H/17I foliage tuning.
2. **24F–24I — signal quality, before anything stochastic.** TAA, blue noise, shadows,
   GTAO. Skipping this and going straight to ray tracing produces noise nobody can
   evaluate, and the existing foliage sparkle already proves aliasing is past tolerable.
3. **24J–24O — ray-traced lighting**, strictly in order; each stage feeds the next.
4. **24P–24Q — fallback tiers** for hardware without ray query.
5. **24R–24V — materials and remaining light types.** Independent of the GI work and
   individually shippable, so these are good candidates to interleave when a break from
   the GI arc is wanted. 24S in particular closes out the foliage work.

### 22.3 What was learned from `example_repo/`

Surveyed across four engines rather than one, which is where 24W–24AC came from.
**Spartan** (`SpartanEngine-master`, MIT) is the most directly comparable: a compact
modern renderer with `auto_exposure.hlsl`, `bend_sss.hlsl`, `cloud_shadow.hlsl`,
`restir_pt*.hlsl` and a full post-process chain. Its exposure metering fixed a real
bug — see 24A's note on centre weighting. **O3DE** (`o3de-development`, Apache-2.0)
contributes its colour-grading LUT pipeline and lighting debug views. **Unreal**
(`UnrealEngine-release`, EULA — read only) contributes Lumen's architecture. **Bevy**
remains the primary reference for anything implemented, being the same stack and an
adaptable licence.

Two categories of reference, with very different licensing consequences.

**Bevy (MIT / Apache-2.0) — `example_repo/bevy/bevy-main/`.** The highest-value
reference in the tree, because it is the *same stack*: Rust, wgpu, WGSL. It has working
implementations of most of Phase 24 — `bevy_pbr/src/atmosphere` (Hillaire),
`bevy_solari` (ReSTIR DI/GI + world cache + path tracer), `bevy_anti_alias/src/taa`,
`ssao`, `ssr`, `volumetric_fog`, `light_probe`, `ltc`, `transmission`, `bluenoise`,
`contact_shadows.rs`. Its licence is compatible with this repository, so it may be read,
learned from, and adapted **with attribution** — see ATTRIBUTION §13.27. Bevy having
shipped this on wgpu is also the strongest available evidence that Phase 24 is
achievable on our API rather than only on D3D12/Vulkan directly.

**Unreal Engine 5 (proprietary EULA) — `example_repo/UnrealEngine-release/`, and the
UE 5.6 install.** Lumen is not one algorithm but a pipeline of five cooperating caches:

| Lumen stage | Files | Somnium equivalent |
|---|---|---|
| Scene representation | `LumenScene.usf`, `LumenMeshSDFCulling.usf` | 24P (fallback tier) |
| Surface cache (cards) | `LumenCardCommon.ush`, `LumenSceneLighting.usf` | folded into 24M |
| Screen probes | `LumenScreenProbeGather/Tracing/Filtering.usf` | superseded by 24L |
| World radiance cache | `LumenRadianceCache.usf` | 24M |
| Reflections + denoise | `LumenReflection*.usf` | 24N |

Two structural lessons carry over regardless of which GI path is taken. First, both UE
and Bevy keep **software and hardware tracing as interchangeable backends** behind one
interface rather than committing to either — which is exactly the shape 24J/24P should
have. Second, nearly every stage is split **trace → filter → temporal accumulate** as
separate passes: the denoising is not bolted on at the end, it is half the architecture.

> **Licensing note.** UE source is under the Unreal Engine EULA and is *not* compatible
> with this repository's licence. It is read to understand architecture — pass
> structure, cache topology, order of operations. **No UE code is copied, adapted, or
> translated into Somnium.** Where a technique has a published paper (Hillaire's
> atmosphere, Jimenez's GTAO, Bitterli's ReSTIR), that paper is the implementation
> reference and the citation. This distinction is deliberate and must be preserved:
> Bevy may be adapted with attribution, UE may only be studied.

### 22.4 Definition of done

The phase is finished when, with no code changes between the two:

- Dragging the sun's intensity from noon to zero produces a believable day → dusk →
  night transition, with sky, ambient light and exposure all responding.
- A dark interior lit only by a doorway shows coloured bounce light from the wall the
  sun hits, not a flat ambient fill.
- Distant terrain desaturates into the sky rather than staying fully saturated.
- Backlit foliage glows instead of going flat and dark.
- Foliage stops sparkling when the camera moves.
- The real-time GI visibly converges toward the 24O path-traced reference on a static
  shot, rather than merely looking plausible.
