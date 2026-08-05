# Somnium Engine — Project Context

> **Last updated:** 2026-06-12  
> **Current phase:** 14 SSS (heightmap terrain — COMPLETE; chunked CDLOD-style heightmap, splatmap PBR painting, sculpting brushes, editor terrain mode. Voxel world also complete, §19)  
> **Toolchain:** Rust 1.85, wgpu 29, winit 0.30

---

## Table of Contents

1. [Project Summary](#1-project-summary)
2. [Repository Layout](#2-repository-layout)
3. [High-Level Architecture](#3-high-level-architecture)
4. [Crate Dependency Graph](#4-crate-dependency-graph)
5. [somnium_core — Lifecycle & Events](#5-somnium_core--lifecycle--events)
6. [somnium_renderer — Visibility Buffer Pipeline](#6-somnium_renderer--visibility-buffer-pipeline)
7. [somnium_ecs — Entity Component System](#7-somnium_ecs--entity-component-system)
8. [somnium_ui — Native Editor UI](#8-somnium_ui--native-editor-ui-phase-12-complete)
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
│   ├── somnium_ui/             wry WebView manager, HTML editor, IPC bridge
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

## 8. somnium_ui — Native Editor UI (Phase 12 Complete)

### 8.1 Architecture

The editor UI is rendered entirely by the wgpu backend — no OS WebView dependency. `UiPass` composites the widget tree over the 3D viewport each frame using an alpha-blending screen-space render pass.

```
OS Window (HWND)  ← wgpu renders 3D scene, then UI overlay
  │
  └── UiPass (wgpu, alpha blend, LoadOp::Load)
        │
        └── UserInterface widget tree
              ┌──────────────────────────────┐  Row 0  28 px  (menu bar)
              │ Somnium Engine │ File │ Edit │
              ├────────┬───────────────┬─────┤
              │Toolbar │  3D Viewport  │Right│  Row 1  *  (main area)
              │ 40 px  │ (transparent) │280px│
              ├────────┴───────────────┴─────┤
              │      Output Log  192 px      │  Row 2
              └──────────────────────────────┘
```

### 8.2 Key types

| Type | File | Role |
|---|---|---|
| `UserInterface` | `ui.rs` | Widget tree, two-pass layout (measure/arrange), hit-test, message queue, draw dispatch |
| `UiPass` | `pass.rs` | wgpu render pass: ortho proj, vertex/index buffers, font atlas, scissor, alpha blend |
| `UiManager` | `lib.rs` | Entry point: `new()`, `end_frame()`, `build_editor_layout()`, outliner/inspector rebuilds |
| `FontAtlas` | `font.rs` | fontdue 0.7, 512×512 Rgba8, shelf packing, `measure_text`, `ascent` |
| `DrawingContext` | `draw.rs` | Command list: `push_rect`, `push_text`, clip stack |

### 8.3 Widget Library

All widgets port the Fyrox UI architecture (see ATTRIBUTION §13.13–13.17):

| Widget | File | Description |
|---|---|---|
| `Canvas` | `widgets/canvas.rs` | Absolute positioning container |
| `StackPanel` | `widgets/stack_panel.rs` | Linear layout (Horizontal / Vertical) |
| `Border` | `widgets/border.rs` | Background fill + per-side stroke |
| `Button` | `widgets/button.rs` | Click emission via `ButtonMessage::Click` |
| `Text` | `widgets/text.rs` | fontdue-rendered text label |
| `Grid` | `widgets/grid.rs` | WPF-style rows/columns: Strict / Auto / Stretch size modes |
| `ScrollViewer` | `widgets/scroll_viewer.rs` | Clipped vertical scroll container |
| `TextBox` | `widgets/text_box.rs` | Single-line keyboard text input |
| `NumericField` | `widgets/numeric_field.rs` | f32 numeric input with `NumericFieldMessage::Value` |

### 8.4 Editor Layout (Phase 12D-full)

`UiManager::build_editor_layout()` constructs the full editor tree on init:

```
outer_grid (3 rows: 28px | * | 192px)
├── menu_bar_h  (row 0) — Grid(stretch col | auto col)
│     ├── menu_stack (col 0) — StackPanel(Horizontal)
│     │     └── Buttons: [Somnium Engine, File, Edit, View, Create]
│     └── fps_text  (col 1) — Text "FPS: --" (right-aligned)
│
├── main_grid (row 1) — Grid(40px | * | 280px cols)
│     ├── toolbar_h  (col 0) — StackPanel(Vertical): tool mode buttons
│     ├── viewport_h (col 1) — Border(transparent): 3D render target area
│     └── right_panel_h (col 2)
│           ├── outliner_scroll — ScrollViewer
│           │     └── outliner_stack — StackPanel(Vertical): entity Buttons (rebuilt per-frame)
│           └── inspector_h — Grid(rows per TRS field)
│                 └── NumericFields ×9: tx/ty/tz, rx/ry/rz, sx/sy/sz
│
└── bottom_h (row 2) — Grid(22px | * rows)
      ├── log_header_border (row 0) — Border + Text "Output Log"
      └── log_scroll (row 1) — ScrollViewer
            └── log_stack — StackPanel(Vertical): log Text lines (rebuilt as logs arrive)
```

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
| 16 | ⬜ Planned | Scripting (Rhai or Lua) |
| 17 | ⬜ Planned | Terrain improvements: foliage scattering, terrain colliders, layer UI |

---

## 18. Known Issues & Active Bugs

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
