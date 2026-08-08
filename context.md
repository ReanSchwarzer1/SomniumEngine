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
| 22D | ✅ Complete | **Inspector fields were effectively uneditable.** Every part of the chain worked — click focused the field, keys reached it, the committed value mapped to an `InspectorField` and reached the ECS — but focusing pre-filled the edit buffer with the current value and typing **appended** to it. Clicking a field reading `0.000` and typing `7` committed `0.0007`; exposure `1.000` plus `2` committed `1.0002`. Every edit produced a number visually indistinguishable from the original, which is indistinguishable from a field that does nothing. `NumericField` now has select-all-on-focus: the buffer is still pre-filled so a value can be amended, but the first accepted character (digit, `.` or `-`) replaces it, backspace clears the selection, and the selection is drawn as a highlight so the state is visible. Commits are also suppressed when the parsed value is unchanged, so clicking through fields no longer spams the undo stack. Separately, the transform gizmo only re-synced on selection or its own drag, so typing a position left it stranded at the object's old location — it now follows inspector edits. |
| 22E | ✅ Complete | **Drag-to-scrub on inspector fields** (UE-style: drag right increases, left decreases). Press-and-move on any numeric field scrubs it; a 3-pixel threshold keeps an ordinary click a click, and crossing it drops the text-edit state the press handed over so the field never shows a caret mid-drag. Steps are computed against the value and cursor x captured at press rather than accumulated, so nothing drifts if the app writes to the field mid-gesture. Rates are per-field because inspector values span orders of magnitude — 0.05/px for positions and angles, 0.005 for light colour channels and IBL, 0.0002 for chromatic aberration, whose default is 0.004. Undo: a scrub emits `ValueChanging` per step, which the engine applies straight to the component with **no** undo entry, and one closing `ValueChanged` on release, which pushes a single command rewinding to the pre-drag value — otherwise a 200-pixel drag would leave 200 undo entries. Two supporting fixes: `NumericField::is_text_input` now reports the live edit state instead of a constant `true`, so keys reach the game again once a scrub ends the edit session; and `Focus` is re-sent on every press rather than only when the focused node changes, since a scrubbed field stays the focused node while dropping its edit state and could otherwise never be clicked back into typing. |
| 15D | ✅ Complete | **Meshlet/cluster generation at mesh upload** (`somnium_renderer/src/meshlet.rs`). Every static mesh is split into runs of **128 triangles**, matching UE5 Nanite's `NANITE_MAX_CLUSTER_TRIANGLES`, so 15E and 15F can cull and draw below whole-object granularity. Clusters are stored as an offset and count into the mesh's index range rather than a triangle list, which only works if a cluster's triangles are contiguous — so `build_meshlets` returns a **permuted index buffer** and `upload_mesh` uploads that instead. Triangle order within a draw does not affect the image, so the reorder is free; verified on the imported helmet. Clustering is a **Morton sort of triangle centroids cut into fixed-size runs**, not Nanite's METIS graph partition: the space-filling curve keeps spatial neighbours adjacent in the sequence, which is all the bounding volumes need, and it stays O(n log n), allocation-light and deterministic. Each cluster carries a bounding sphere (AABB-centred, conservative rather than minimal) and a **normal cone** — axis plus cosine cutoff — for rejecting a back-facing cluster whole. Clusters whose normals span more than a hemisphere get a cutoff of `-1.0`, which never culls. Pooled (voxel) uploads are deliberately **not** clustered: chunks are remeshed continuously, so the sort would cost more than the culling saves, and a chunk is already small enough to cull as a unit. Malformed input degrades rather than panics — out-of-range indices, trailing partial triangles and NaN positions drop the affected triangles. 14 unit tests, including a permutation check that the reorder preserves every triangle, a containment check on every bounding sphere, and a locality check that two distant point clouds do not end up in one cluster. |
| 15E1 | ✅ Complete | **Hi-Z depth pyramid + occlusion math** (`pass/hiz.rs`, `shaders/hiz.wgsl`, `culling.rs`). An R32Float mip chain the size of the viewport, rebuilt each frame straight after the visibility pass — the moment the depth buffer holds exactly the opaque geometry that can occlude. Every texel holds the **furthest** depth of the region below it: wgpu depth runs 0 near to 1 far, so the reduction is `max`, and taking the max can only make an occluder look nearer than it is, which errs toward drawing. Level 0 is a compute copy rather than a blit because depth textures cannot be bound as storage. **Odd mip sizes** are the trap — halving 5 gives 2 and the trailing row would vanish from the pyramid, letting a real occluder go unrecorded — so the reduction widens to 3 texels on whichever axis is odd. On the CPU side: `project_aabb_to_screen` (returns `None` when the box crosses or sits behind the eye, where the perspective divide is meaningless — treated as visible rather than guessed at), `hiz_mip_level` (picks the level where the footprint spans at most 2×2 texels, which is what keeps the lookup constant-time regardless of on-screen size), and `is_occluded`. Every ambiguous case resolves toward drawing: a cleared pyramid at 1.0 occludes nothing, and equal depths count as visible so an object cannot cull itself on the following frame. 18 unit tests. **Not yet consumed** — the two-phase cull is 15E2. |
| 15E2 | ✅ Complete | **Two-phase occlusion culling.** The frame graph is now cull(1) → visibility(clear) → Hi-Z → cull(2) → visibility(load) → Hi-Z. Phase one tests frustum then occlusion against the *previous* frame's pyramid; phase two re-tests only what phase one rejected **on occlusion** against the pyramid just rebuilt from phase one's depth, which is what catches geometry that became visible this frame. Reprojecting the previous frame alone would drop geometry the moment the camera moves — the second phase is what makes that safe, since anything wrongly rejected gets a look at fresh depth within the same frame. Two details are easy to get wrong and are handled explicitly: frustum rejects are **not** recorded in the phase-two set (they are still off-screen, and resurrecting them would draw outside the view), and phase two **zeroes** the instance count of everything it is not re-testing, or phase one's draws would be submitted twice. `cull.wgsl` gained a transliteration of the `culling.rs` occlusion math, and `CullPass` gained a per-draw flag buffer, a Hi-Z binding, and one params uniform per phase — both dispatches are encoded before either runs, so a single uniform could not carry two `phase` values. Occlusion is held off until a pyramid exists (`hiz_ready`), because wgpu zero-fills a new texture and zero is the near plane, which would read as "everything is occluded"; a resize resets the flag for the same reason. A layout test pins `GpuCullParams` to 192 bytes with per-field offsets, since a Rust/WGSL uniform mismatch does not fail validation — the shader just reads the wrong words and culls the wrong things. |
| 15E-verify | ✅ Complete | **Occlusion culling measured on a real scene.** The demo has one opaque mesh, so it can neither exercise nor benefit from occlusion culling; the imported `car_scene` (47 nodes, heavily self-occluding) can. Three diagnostics made this measurable: `SOMNIUM_CULL_STATS=1` copies the indirect args back after each cull phase and logs how many draws survived (`instance_count` doubles as the verdict, so counting non-zero entries *is* the submitted-draw count); `SOMNIUM_NO_OCCLUSION=1` keeps frustum culling but skips the Hi-Z half, so the two can be told apart; and `SOMNIUM_IMPORT=<path>` imports a model at startup, since the File → Import dialog cannot be scripted. **Result**, camera against the car body: frustum alone drew 24 of 35, frustum + occlusion drew **17 of 35** — 7 more draws removed, **29% fewer** than frustum alone — and the two screenshots are pixel-identical, which is the correctness half: everything occlusion removed was genuinely invisible. `phase2_drawn` was non-zero on some frames during camera motion, confirming the second phase does catch real disocclusions rather than being dead weight. A useful incidental check: `total=35` rather than 48 because 13 of the car's meshes are `alphaMode: BLEND` and route to the transparent queue, which is not indirect-drawn. |
| 23 | ⬜ Planned | **GPU culling for the transparent pass.** Alpha-blended draws bypass culling entirely — they go to the Phase 21 forward queue, which is CPU-submitted per object with no indirect args and so no frustum or occlusion test. On the imported `car_scene` that is 13 of 47 meshes, roughly a quarter of the model. Give the transparent queue its own indirect args and cull dispatch, keeping the back-to-front sort (sort first, then build args, so argument `i` still lines up with instance `i` — the same ordering trap that produced the shard artifact). Occlusion has to stay conservative here: blended geometry can be hidden by opaque occluders, but must never occlude itself. |
| 24A | ✅ Complete | **Physical light units and exposure.** `somnium_core::light_units` — directional lights carry **illuminance in lux**, point/spot **luminous power in lumens** (converted to candela at upload), cameras **EV100** from aperture/shutter/ISO with `exposure = 1/(1.2·2^EV100)`. Presets for both (`lux::DIRECT_SUNLIGHT` = 100 000, `lux::FULL_MOON` = 0.05, `lumens::BULB_60W` = 800). **Auto-exposure** (`pass/auto_exposure.rs` + `auto_exposure.wgsl`): a 256-bin log-luminance histogram built with per-workgroup atomics, reduced to a weighted mean, converted to a target EV and adapted per *second* rather than per frame so the rate does not follow the frame rate; the result stays on the GPU and the post-process pass reads it directly. Three separate copies of the sky gradient turned out to exist — `ibl_gen.wgsl`, the HDR clear colour, and `shading.wgsl`'s background branch — and **all three had to become luminances** scaled by sun illuminance (~0.08 cd/m² per lux), or the background sat five orders of magnitude below the scene and rendered pure black. That scaling is also what finally makes **night work**: verified at 100 000 lux (daylight) and 0.05 lux (full moon), the second producing a dark blue moonlit scene rather than an unchanged bright one. 9 unit tests pin the photometry, including that moonlight metered for daylight is black and metered for night is visible. |
| 24B | ✅ Complete | **AgX tone mapping**, implemented analytically in `postprocess.wgsl` rather than as the 3-D LUT the reference ships — a closed-form curve does not justify shipping and binding a KTX2 asset. Rec.709→AgX inset matrix, log2 encode over [−12.474, +4.026] stops, sixth-order contrast sigmoid, outset matrix, then an inverse-sRGB step so AgX's display encoding does not compound with the sRGB target's own. The tone mapper is selectable (AgX / ACES / Reinhard) through `Tonemapper` on `PostProcessComponent`. ACES was fine while the sun was an arbitrary 3.0 and stops being fine at 100 000 lux: it pushes bright saturated light toward the primaries and clips, where AgX desaturates into the highlight the way film does. |
| 24C | ✅ Complete | **Atmospheric scattering (Hillaire 2020).** `shaders/atmosphere.wgsl` + `pass/atmosphere.rs`. A transmittance LUT (256×128, Bruneton's horizon-concentrating parameterisation) and a multiple-scattering LUT (32×32, second order plus a geometric series for every remaining order) are built once at startup — neither depends on the sun or the camera. The sky is then a real ray-march through Rayleigh, Mie and ozone: 32 steps with analytic per-segment integration rather than a Riemann sum. **The three duplicated sky gradients are now one.** `ibl_gen.wgsl` marches the atmosphere into the environment cubemap, and `shading.wgsl`'s background samples that same cubemap, so background, ambient and reflections cannot disagree. Sharp features — sun disc with limb darkening, moon disc, stars — are drawn analytically over the background at screen resolution instead of being baked in: at 256² per face a texel spans ~0.35°, so a half-degree disc smeared into a blob (observed, then fixed). Keeping the sun disc out of the cubemap also removed a double-count, since the shading pass already computes its specular highlight from the analytic light. |
| 24D | ✅ Complete | **Night sky.** Moon disc (0.53°, ~2 500 cd/m²) with a scattered halo, a procedural star field placed one-per-cell so density stays even, and an airglow floor so a moonless night is dark but never identically zero. Night fades in on the sun's **illuminance, not its elevation** — dimming a light and moving it below the horizon are different things, and intensity is the dial the inspector actually exposes; keying off elevation left a starless sky when the sun was turned down to moonlight (observed, then fixed). The environment cubemap already regenerates whenever the sun changes, so ambient tracks it for free. |
| 24AD | ⬜ Planned | **Velocity buffer for TAA.** 24F reprojects from depth, which is exact for camera motion and wrong for moving objects: geometry that moves while the camera is still ghosts, limited only by the neighbourhood clip. Needs previous-frame per-instance transforms carried through the visibility pass and a velocity target written alongside. Also unlocks motion blur (24Z). |
| 24W | ✅ Complete | **Water in physical units.** Two faults, both left over from before 24A. The sun was treated as a **point source**, which drives GGX toward a singularity on a near-mirror surface — an unbounded spike across a few pixels, which is what made sunlit water blow out; the lobe is now widened by the sun's angular radius so the spike becomes a glitter path, with an energy term keeping total reflected light unchanged. And a leftover `min(…, 40.0)` on the glint, written when the sun was an arbitrary ~5, was crushing it to nothing against a sky that now measures thousands of cd/m² — removed, since the disc widening bounds the peak on physical grounds instead of by an arbitrary ceiling. The diffuse and scatter terms also had hand-tuned 0.25/0.5 coefficients replaced by the actual Lambert normalisation. Output is clamped below `Rgba16Float`'s finite limit: water is the most mirror-like surface in the scene and therefore the likeliest to overshoot, and an Inf here would reach TAA's blend as NaN. |
| 24X | ✅ Complete | **Screen-space contact shadows.** A shadow map cannot resolve contact: its texels cover centimetres at best, and 24H's normal-offset bias deliberately pushes samples off the surface, erasing precisely the darkening where two surfaces meet. A short ray marched through the depth buffer toward the light fills that gap — visible as grass tufts now sitting on the ground rather than floating on flat colour. A **thickness limit** is what makes it usable: without one every thin object casts an infinitely deep shadow volume behind itself, because the march cannot distinguish a leaf from a wall receding from the camera. The start offset is jittered per pixel so the step pattern becomes noise for TAA to resolve rather than visible banding, and the result only ever *darkens* the shadow-map term, which stays authoritative at its own scale. Parameters follow Bend Studio's screen-space shadows; their wavefront scheduling is **not** ported, only the sampling behaviour. |
| 24Y | ✅ Complete | **Colour grading.** White balance on the orange–blue and green–magenta axes, ASC CDL (slope / offset / power — the standard film grades with), contrast pivoting around middle grey rather than black, and saturation. Applied **after** tone mapping, in display space: grading beforehand fights the curve, whose job is to fit scene luminance into a display's range. Exposure and the tone curve decide how bright the image is and how it rolls off; grading decides what it *feels* like, and no amount of the former substitutes. **File-based 3-D LUTs are not included** — that needs a `.cube` loader and an asset path, and is worth its own sub-phase rather than a stub here; the controls are the part that is usable today. |
| 24Z | 🟡 Partial | **Lens realism: depth of field, film grain, dithering.** DoF is driven by the *same* aperture the exposure model already uses, because in a real camera they are one number — opening to f/1.4 both brightens the frame and throws the background out; a renderer that separates them tells a small lie in every shot. Thin-lens circle of confusion against a 36 mm sensor, gathered on a per-pixel-rotated Vogel disk, with a **neighbour test** that only accepts a sample blurred enough to reach this pixel — without it a sharp foreground bleeds over blurred background, the classic tell of a gather-based DoF. Runs **before** bloom so out-of-focus highlights bloom as discs. Grain scales with darkness, because sensor noise lives in shadows and flat grain reads as dirt on the lens. **Dithering is not cosmetic now that exposure is physical**: smooth dark gradients band visibly at 8 bits, and half a bit of noise costs nothing to hide it. **Motion blur is not included** — it needs the velocity buffer from 24AD, which does not exist yet. |
| 24AA | ⬜ Planned | **Cloud shadows.** A scrolling noise mask over the sun's contribution. Cheap, and one of the strongest cues that an outdoor scene is a place rather than a render, because it puts the sky in motion without any volumetric cost. Reference: Spartan's `cloud_shadow.hlsl`. |
| 24AB | ⬜ Planned | **Lighting debug views.** Per-light-type heatmaps, cluster occupancy, exposure histogram readout, a luminance false-colour view. GI is nearly impossible to debug by eye, and every engine surveyed ships these. Reference: O3DE's `LightCullingHeatmap.azsl`, UE's Lumen visualisation modes. |
| 24AC | ⬜ Planned | **FidelityFX SPD and CAS.** Single-pass downsample for the Hi-Z pyramid and bloom chain (one dispatch instead of a pass per mip), and contrast-adaptive sharpening to recover the softness TAA introduces. Reference: Spartan's `spd.hlsl`, `cas.hlsl`. |
| 24E | ✅ Complete | **Sun as a physical disc.** 0.53° angular diameter drives `evaluate_brdf_area`, which widens the specular lobe by the source's angular radius and normalises its energy (Karis' sphere-light approximation). A point source gives a one-pixel highlight on anything smooth, which is among the clearest tells that an image is rendered. The correction is **specular-only** — a first attempt scaled the whole BRDF and would have darkened every lit surface, since diffuse does not care how large a source is. Lights also gained **colour temperature in Kelvin**, one physically meaningful dial replacing three coupled RGB channels; the Planckian fit is sRGB and is decoded to linear before use, which left warm lights far too saturated when skipped. `sun_angular_radius` rides in the light buffer's remaining padding. |
| 24F | ✅ Complete | **Temporal anti-aliasing + specular AA** (absorbs the old Phase 18). Halton-jittered projection; depth-based reprojection; 9-tap Catmull-Rom history sampling (bilinear compounds and goes visibly soft over ~100 frames); Playdead `clip_aabb` neighbourhood clipping with Salvi variance clipping. Blending happens in a **tone-mapped space** — averaging HDR directly lets one bright sample dominate, so a glint flickers rather than resolving, which is the artefact the pass exists to remove. History buffers ping-pong because wgpu forbids binding one texture as both read and write. **Limitation:** reprojection is depth-based, so it handles camera motion exactly but objects that move while the camera is still will ghost until a velocity buffer exists (24AD). Specular AA folds Toksvig normal-map variance back into roughness so mipped detail widens the lobe rather than aliasing. |
| 24G | ✅ Complete | **Sampling infrastructure.** Interleaved gradient noise, Vogel disk (chosen over Poisson tables, which must be shipped and indexed, and over grids, which alias into rings), cosine-weighted hemisphere with Frisvad's branchless basis, R2 and Halton sequences. Shared so the patterns are chosen once — white noise clumps, and clumps survive filtering as blotches. |
| 24H | 🟡 Partial | **Shadow quality: PCSS, normal-offset bias, cascade blending.** Blocker search then Vogel-disk filtering, both rotated per pixel by gradient noise, with the search and filter radii driven by the sun's 24E angular size — so a shadow hardens at its contact point and softens with distance from the caster, which a fixed kernel cannot express. Normal-offset bias replaces constant depth bias: offsetting along the surface normal avoids the acne/peter-panning trade entirely. Cascades blend over the last 10% of each range instead of switching, since an abrupt switch shows as a line where resolution and filter width change together. Reference: Spartan's `shadow_mapping.hlsl`. **Contact shadows landed with 24X.** |
| 24I | ✅ Complete | **GTAO with bent normals.** `pass/gtao.rs` + `gtao.wgsl`. Phase 17I applied only *baked* occlusion, so terrain, procedural meshes and all foliage received sky light unattenuated — the reason contact points stayed flat and shaded bark read sky-blue. GTAO (Jimenez 2016) rather than classic SSAO: it searches each screen-space slice for its **horizon angles** and integrates the visible arc analytically, producing a real visibility fraction rather than a darkening heuristic — which matters because this term will later feed the GI gather, not just tint the image. Normals are reconstructed from depth, taking the *closer* neighbour per axis: a naive central difference straddles silhouettes and yields normals facing nowhere real. Two slices with per-pixel and per-frame rotation, then a depth-weighted denoise; the residual noise is what TAA is for, which is precisely why 24F was its prerequisite. The **bent normal** is the part that changes indirect light's colour rather than only its amount — the irradiance gather uses it, so a surface in a crevice collects light from the opening rather than from the wall beside it. Screen-space AO **multiplies** the baked term rather than replacing it: the two know different things, and taking the minimum would discard whichever is more informative. Reference: Spartan's `ssao.hlsl`. |
| 24J | ✅ Complete | **Ray-tracing scene: BLAS/TLAS via wgpu acceleration structures.** A bottom-level structure per uploaded mesh and a top-level structure rebuilt each frame from the *same draw queue the raster path uses*, so the traced scene and the drawn one cannot drift apart. Positions are the first 12 bytes of the 32-byte vertex, so `BLAS_INPUT` on the existing pools lets the build read geometry in place — no second copy. The plan's claim held: **the feature gap really was just ray query**, since the binding arrays and non-uniform indexing Solari also needs were already mandatory for the bindless pool. Four things beyond the feature bit were needed and none are obvious: an `unsafe` **experimental-features token** (wgpu asks the caller to acknowledge that these APIs may contain soundness bugs), the `max_blas_*`/`max_tlas_instance_count` **limits**, the `max_acceleration_structures_per_shader_stage` **binding limit** — all three default to zero — and `enable wgpu_ray_query` in the shader. Ships with a ray-traced shadow **acceptance test** (`SOMNIUM_RT_DEBUG=1`), because a correctly built acceleration structure and a silently broken one look identical until something traces against it; it showed the helmet self-shadowing, which is what confirmed the build. Degrades cleanly: every entry point checks whether the device granted ray query. |
| 24K | 🟡 Partial | **ReSTIR DI — resampled direct lighting.** The shadow ray from 24J plus the thing that makes rays affordable: *resampling*. Eight unshadowed candidates are drawn across the sun's disc, one is kept in proportion to its contribution by weighted reservoir sampling, and the single expensive ray confirms only that one — the estimator stays unbiased because the kept sample carries the weight of everyone it beat. **Temporal reuse** then combines each pixel's reservoir with its own history, capped at `M_CAP` so a reservoir keeps responding to change rather than fossilising (an uncapped `m` keeps a switched-off light visible for as long as the history has been accumulating). Sampling across the sun's angular disc gives a **real penumbra** rather than PCSS's filtered approximation of one, with no cascades, no depth bias and no peter-panning. Enabled by `SOMNIUM_RESTIR=1`; shading prefers the traced result and falls back to the shadow map when alpha is 0, which is also what an unsupported device produces since wgpu zero-fills the target. **Remaining: spatial reuse** (neighbour reservoirs) and a **multi-light set** — the target function currently evaluates only the sun, where a full implementation would weigh every light's intensity and falloff. **Known limit:** it shadows only visibility-buffer geometry, because terrain and water write depth in their own later passes. |
| 24L | ⬜ Planned | **ReSTIR GI — ray-traced indirect diffuse.** The feature that makes indirect light look like Lumen rather than a constant ambient term: real coloured bounce, contact darkening and light leaking through openings, all fully dynamic with no bake. Reference: `bevy_solari/src/realtime/restir_gi.wgsl`. |
| 24M | ⬜ Planned | **World-space radiance cache for multi-bounce.** A hashed/clipmapped world cache that rays terminate into, so a single traced bounce still resolves to many bounces of energy across frames, and distant geometry costs a lookup instead of a long trace. Reference: `bevy_solari/src/realtime/world_cache_{query,update,compact}.wgsl`; UE's equivalent is `LumenRadianceCache.usf`. |
| 24N | ⬜ Planned | **Ray-traced reflections with a denoiser.** Specular GI proper: screen-space trace first, ray traced where the screen has no answer, radiance cache beyond that, then spatial + temporal denoising — one bounce per pixel is far too noisy raw. Finally gives water something better than a single planar reflection. Reference: `bevy_solari/src/realtime/specular_gi.wgsl`, `bevy_pbr/src/ssr/`. |
| 24O | ⬜ Planned | **Offline path tracer for validation.** A slow, unbiased, accumulate-over-many-frames reference renderer sharing the 24J scene bindings. Not shipped in the frame loop — its whole job is to be *ground truth*, so “does the real-time GI actually converge to the right answer” becomes a comparison rather than an opinion. Bevy ships exactly this alongside Solari and it is the single best idea taken from studying it. Reference: `bevy_solari/src/pathtracer/`. |
| 24P | ⬜ Planned | **Software fallback: mesh SDFs + global distance-field clipmap.** For GPUs without ray query. Bake a signed distance field per mesh at upload and composite into a camera-centred clipmap, then cone-trace it for GI and AO. This is Lumen's software path (`LumenMeshSDFCulling.usf`, `LumenSoftwareRayTracing.ush`) and is the more portable but substantially larger implementation. **Deliberately sequenced after the hardware path**, not before it — see §22.2. |
| 24Q | ⬜ Planned | **Baked light probes: irradiance volumes and reflection probes.** The cheapest fallback tier and still the right answer for static scenes on weak hardware: a grid of SH irradiance probes plus localised reflection cubemaps, blended per object. Reference: `bevy_pbr/src/light_probe/{irradiance_volume,environment_map}.rs`. |
| 24R | ⬜ Planned | **Area lights (LTC).** Rect, disc and tube lights via Linearly Transformed Cosines — analytic, no sampling noise, correct soft shadows and elongated highlights. Softboxes, windows and strip lights are most of what makes an interior read as photographed rather than rendered, and no amount of point-light tuning substitutes. Reference: `bevy_pbr/src/ltc/`, `bevy_light/src/rect_light.rs`. |
| 24S | ✅ Complete | **Transmission and subsurface scattering.** Frostbite's approximation (Barré-Brisebois & Bouchard) rather than a real subsurface solve: light leaving the *far* side of a thin surface, spread by scattering, brightest looking almost straight into the source through the material. **This is what the foliage was missing all along.** Leaves lit only by reflection stay flat and dark regardless of how correct the albedo is — the symptom the grass has shown since Phase 17, and which no amount of albedo or occlusion work could have fixed. Transmitted light is tinted by albedo, which is why backlit foliage reads more saturated than the same leaf lit from the front, and it is deliberately **not** multiplied by the shadow factor: the entire point is light arriving through the surface from the side the shadow map calls dark. Materials take `transmissionFactor` from `KHR_materials_transmission` where present; foliage assets do not set it, so a sidecar cutout mask is taken as evidence of thin geometry and infers 0.5 — the same convention-over-metadata rule the alpha masks and ARM packing already use. `GpuMaterial` grew from 48 to 64 bytes (WGSL rounds the array stride to the 16-byte alignment `base_color` forces); the layout test caught this and was updated rather than deleted. |
| 24T | ✅ Complete | **Emissive materials and physical bloom.** Materials carry `emissiveFactor` and an emissive texture from glTF, added to shading independently of every light in the scene — a screen is as bright in a dark room as a lit one. Bloom is **deliberately not threshold-based**: a threshold asks "which pixels count as bright?", a question with no physical answer whose meaning changes the moment exposure does — a scene metered for night would bloom everything, one metered for noon nothing. Real bloom is light scattering inside the lens, which happens to *all* light in proportion to how much there is. So a progressive 13-tap downsample builds a mip chain and a 9-tap tent upsample sums it back additively (Jimenez, SIGGRAPH 2014); bright regions dominate naturally because they carry more energy. Added **before** exposure and tone mapping, since it is scattering on the way to the sensor rather than a filter over the picture, and built **after** TAA, because a blur of unstable input broadcasts that instability everywhere it reaches. `GpuMaterial` grew 64 → 80 bytes; the layout test was updated again. |
| 24U | ⬜ Planned | **Volumetric fog, aerial perspective and light shafts.** A froxel volume accumulating in-scattering per depth slice, fed by 24C's aerial-perspective LUT so distant hills desaturate correctly and the sun throws real shafts through the canopy. Among the highest perceived-realism-per-line-of-code in the whole phase. Reference: `bevy_pbr/src/volumetric_fog/`. |
| 24V | ✅ Complete | **Local lights in physical units, with source radius.** The photometric half landed with 24A-1 — point and spot lights carry lumens converted to candela, and `smooth_distance_attenuation` already divides by distance squared, so illuminance was correct. What was missing is that they were still **point** sources. Lights now carry a `source_radius` in metres (distinct from `range`, which is reach): a 5 cm bulb a metre away subtends a real angle, and feeding that through `evaluate_brdf_area` is what stops its highlight being a single pixel on anything polished. **IES profiles are not included** — that is an asset-pipeline job and is better as its own sub-phase than half-done here. |
| 15F | ✅ Complete | **Meshlet rendering path.** A draw is now one indirect argument per **cluster**, so frustum, Hi-Z and backface tests all work at 128-triangle granularity instead of per object — 530 cull units where there were 35. `first_vertex` carries the cluster's index offset within its mesh, because the vertex shader adds `instance.index_offset` itself; `first_instance` carries the owning instance, which is also what the cull shader now reads to find the model matrix, since the draw index no longer *is* the instance index. Meshes with no clusters (voxel chunks) stay a single whole-mesh argument, so one pipeline serves both. **The subtle break:** the fragment shader keyed the visibility buffer on `@builtin(primitive_index)`, which restarts at 0 every draw call. Splitting a mesh across many draws would have sent the shading pass to the wrong triangle in every cluster after the first. The triangle id now comes from `vertex_index / 3` in the vertex shader — `vertex_index` includes `first_vertex`, so it is mesh-relative, and all three vertices of a triangle divide to the same value. Cone culling rejects a whole cluster when every triangle in it faces away; it is only sound because the visibility pass culls back faces, and it is skipped for mirroring transforms whose negative determinant would flip the stored axis. **Measured** on the imported car at a fixed viewpoint: whole-mesh draws submitted 21 782 triangles, clusters **16 220** — 25.5% fewer — with opaque geometry pixel-identical (0.00% on the car body, 0.06% on the helmet silhouette; the rest of the frame differs only where the time-animated water is). |
| 15F-fix | ✅ Complete | **Cluster bounds use the box, not the sphere.** The first 15F measurement showed the cluster path submitting **2.1% *more*** geometry than whole-mesh draws. Cause: `push_cluster_args` culled against the bounding *sphere's* AABB, which is up to √3 wider per axis than the cluster's real box and can reach outside the parent mesh's bounds — so boundary clusters survived frustum tests their whole mesh failed, and cluster culling was not the strict refinement it should be. `Meshlet` now stores the local AABB alongside the sphere and culling uses the box. Same viewpoint, same scene: 174 clusters drawn → 127, and a 2.1% regression became a 25.5% improvement. |
| skipped-frame-fix | ✅ Complete | **Double submission after a dropped frame.** Found while reading cull statistics: exactly one frame in 3 914 reported twice the expected draw count. The surface-acquisition failure path returned early without emptying the per-frame queues, so the next frame appended to them and submitted everything twice. Invisible for opaque geometry — same pixels, same depth — which is why it went unnoticed, but it double-blends the transparent pass and wastes a whole frame of work. Queue clearing moved into `clear_frame_queues`, called on every path out of `render`. |
| 17A | ✅ Complete | **Foliage scattering** (`terrain/foliage.rs`). Placement is a **jittered grid**: the terrain is cut into cells sized so one instance per cell hits the requested density, and each cell contributes one candidate placed randomly inside it. That is stratified sampling — even coverage without the clumps and bald patches of independent uniform sampling, at a fraction of the cost of Poisson-disc. Every candidate is derived by hashing its cell coordinate with the seed, so nothing depends on iteration order or carried RNG state: re-scattering gives an identical layout, which matters because the list is rebuilt on every sculpt stroke and foliage that reshuffled mid-edit would be unusable. Candidates are rejected on **slope** and on the **splat layer** beneath them, so grass follows the paint and stops at cliffs. The instance ceiling is enforced by **coarsening the grid**, never by stopping partway through it — truncation would pile everything into the first corner visited and leave the rest bare (there is a test for exactly that). Instances are deliberately not ECS entities, for the same reason voxel chunks are not: thousands of them, regenerated constantly, would flood the outliner and undo stack. They go out as ordinary draw commands, which also means they inherit the whole Phase 15 pipeline — indirect draws, frustum, Hi-Z and per-cluster culling — without foliage knowing any of it exists. `TerrainData` gained an `edit_revision` counter so the scatter is rebuilt only when settings or the terrain actually change. Mesh is a solid tapered-prism tuft (`generate_foliage_tuft`) rather than the usual alpha-tested crossed billboards: the visibility pass culls back faces, so a flat quad would vanish from one side, and `alphaMode: MASK` is imported but not yet cut out in the shader. **F8** toggles foliage on the selected terrain until the layer UI lands. Verified in the editor: 10 876 instances over a 1024x1024 m terrain. Two look fixes came out of that first render — albedo had to drop to ~0.05 because a sun of intensity 5 pushes anything brighter past 1 and tone-maps it to white, and the blade normals had to lean outward instead of nearly straight up, or every blade caught identical light and the tuft read as a flat smudge. 19 unit tests. |
| 17B | ✅ Complete | **Terrain colliders** (`terrain/collider.rs`, `jph_heightfield_shape_create`). Jolt's `HeightFieldShape` needs a square power-of-two sample grid; a Somnium terrain is `chunks * cells + 1` vertices per side (513 by default), so the heightmap is resampled rather than handed over. Resolution rounds **down** to a power of two so the collider is never finer than the mesh it approximates, and caps at 512 — 262 144 samples, which still resolves every 2 m over a kilometre, while 1024 would quadruple the rebuild cost for detail no rigid body can feel. Rebuilds are gated on `TerrainData::edit_revision`, so a sculpt stroke costs one tree build, not one per frame. Two robustness fixes came with it: `PhysicsWorld::create_body` now returns `BodyId::INVALID` when a shape fails to build instead of handing Jolt a null pointer and tripping an assert inside the body interface, and non-finite height samples are replaced before they can poison the tree build. Verified by a stepped simulation, not a mock: a sphere dropped from 8 m rests on a flat field, a sphere on a ramp's high end rests above 5 m (so the height data is genuinely honoured), and malformed fields — non-power-of-two, or a sample buffer shorter than declared — are refused rather than read past the end of. |
| 17C | ✅ Complete | **Terrain layer + foliage inspector.** Two new sections, shown only when the selection has the matching component. **Terrain**: active paint layer plus per-layer texture tiling, which reach into renderer-side `TerrainData` and so bypass the undo stack exactly as sculpting already does. **Foliage**: an Enabled toggle plus density, seed, max slope, layer, and scale range — editing any of them lets the scatter cache notice the component changed and re-scatter on the next frame, with no explicit invalidation. Drag rates are per field again: density lives well under 1 per square metre, and layer indices step in whole numbers. Density is clamped to 4/m² because a square-kilometre terrain at more than that is millions of candidates. F8 stays as a shortcut for the Enabled toggle. |
| 17D | ✅ Complete | **Alpha cutout and double-sided materials** — the two things the engine lacked before real foliage assets could render. **Cutout** happens in the *visibility* pass, not shading: the visibility buffer decides what exists at each pixel, so a leaf's cut-away corners have to be discarded before the depth buffer records a solid quad. That meant giving the pass the albedo textures and a sampler it never had. Derivatives are taken at top level and fed to `textureSampleGrad`, because sampling inside the per-material branch would break WGSL's uniformity rule and dropping to LOD 0 instead would make distant foliage crawl. Only `MASK` cuts out — `OPAQUE` alpha channels are routinely meaningless and clipping `BLEND` would leave hard edges through glass — and a `MASK` material with an unusable cutoff falls back to glTF's 0.5 rather than 0, which everywhere else means "no cutout". **Double-sided** adds a second visibility pipeline with culling off; a leaf card is one flat quad and vanishes entirely from one side otherwise. Draws are partitioned single-sided-first with the boundary recorded, and the pass issues one `multi_draw_indirect` per range. Argument order no longer needs to match the draw queue, because `first_instance` carries the instance explicitly and the cull shader reads it from there. The shading pass flips the geometric normal toward the viewer for flagged materials only — doing it unconditionally would light the inside of closed geometry. **glTF `TANGENT` import turned out to be unnecessary**: the shading pass already derives tangents from triangle edges and UV deltas, so normal maps work on any mesh without vertex tangents. |
| 17E | ⚠️ Partial | **Real foliage assets** — four CC0 Poly Haven models (~101 MB, 1.53 M triangles) in `assets/foliage/`, chosen on measured size and triangle count: `fir_tree_01` is 486 MB / 7 M tris and `pine_tree_01` 937 MB / 17 M, both larger than the entire geometry pool was. Engine work this forced out: the pool grew from 64/32 MB and now **sizes itself to the device's `max_storage_buffer_binding_size`**, since these are storage buffers for vertex pulling and exceeding the limit is a validation error at *bind-group* creation, so it surfaces as a first-frame crash rather than a clean failure; a **capacity guard** refuses an oversized mesh with an error instead of moving the bump pointer past the end and corrupting every later mesh; the **shadow pass gained a fragment stage** purely so alpha-tested geometry can `discard`, as it was depth-only and cast the shadow of every card's whole quad; and foliage `BLEND` materials are **re-tagged to `MASK`** so they take the opaque path — left blended, thousands of instances go through the sorted forward pass with no depth write, no shadows and no GPU culling. Scattering also became a **disc that follows the camera**: blanketing a square kilometre at a believable density is millions of instances, while a 45 m disc gives 18 000. Cell indices stay absolute, so instances do not reshuffle as the camera moves — there is a test for that. **Open issue:** the grass geometry, scatter, placement and shadows are all correct, but it shades grey-blue rather than green. Ruled out: shadow-quad self-shadowing (fixed, no change), the double-sided normal flip (now flips toward the sun, no change), and alpha cutout (these models are modelled blades with JPEG textures and carry no alpha at all). It looks like a material-channel problem — next step is to output albedo, normal and the ARM channels directly from the shading pass rather than guessing again. |
| 17F | ✅ Complete | **Foliage painting** (`terrain/foliage_paint.rs`). 17A filled the whole terrain the moment foliage was switched on, which is the wrong model for authoring — an artist wants grass in the meadow and trees on the ridge, not everywhere at once. A terrain now starts **bare**, and instances are painted. The key design point is **spacing, not per-dab count**: a stroke fires many times a second over the same ground, so "add N per dab" would pile thousands on one spot. Every candidate must instead clear a minimum spacing from what is already there, derived from the density so the two cannot disagree (`1/sqrt(d)` for a packed layout). Painting over covered ground becomes a no-op and a held brush converges instead of growing without bound — there is a test that holds the brush for 40 dabs and asserts the count stays under the area limit. Spacing is **per palette entry**, so grass can be painted under a tree. Candidate radii are `sqrt`-distributed or the brush paints a hot spot in the middle, which is also tested. **Single** mode places one instance at the cursor, which is how trees go down, and repeated clicks do not stack. Erase can target one entry or clear everything. Placement is hash-driven off a stroke counter rather than a live RNG, so a recorded stroke sequence replays identically — what undo and scene reload will need. The palette is four fixed CC0 entries for now, each loaded lazily the first time it is painted, since loading four scanned models up front would add seconds to startup for meshes that may never be placed; a content drawer replaces this later, which is why the brush stores a palette *index* and nothing about the mesh. UI is a Foliage section with Enabled / Paint Mode / Erase / Single toggles and brush size, density, type, scale range and slope limit. 14 unit tests. |
| 17F-fix | ✅ Complete | **Strokes landed a terrain-width away, no brush ring, no type picker.** `TerrainData::raycast` marches in terrain-local space but transforms the result back to **world** before returning; painted instances are stored **local**, because `submit_foliage` composes them with the terrain's transform. Storing the world hit as if it were local applied the terrain's `-512` offset a second time and dropped every stroke off the mesh entirely. The sculpt brush was unaffected because it feeds the same world-space hit to the shader, which also works in world space — so the two conventions had been quietly coexisting. Diagnosed by logging the hit and reading `(-1.2, 3.3)` where a local coordinate has to be in `0..1024`. The **brush ring** now shows for foliage too (amber, `brush.w = 3`): the cursor updater bailed out unless *sculpt* mode was active, so the foliage brush painted blind — which is what let the offset go unnoticed. The **type picker** is a named popup mirroring the Create menu, replacing a numeric index nobody should have to decode; selecting a tree also flips **Single** on, since trees want one-per-click and ground cover wants a spread. The old numeric row is kept for field routing but its whole row is hidden — hiding just the field left a stray "Type" caption, because the label lives in the row. |
| 17G | ✅ Complete | **Foliage performance, distance culling, and a usable type picker.** The slowdown was self-inflicted by Phase 15F: a 6 422-triangle grass tuft expands to **51 indirect arguments**, so 2 000 painted instances meant 102 000 draws and 6.3 MB of arguments and cull bounds uploaded *every frame* — to cull sub-parts of things a few pixels across. Clustering pays for a large mesh drawn once, and is backwards for a small mesh drawn thousands of times. The renderer now counts how often each mesh appears in the frame and drops back to a single whole-mesh argument past 8 copies, cutting a painted field's draw count by ~51x. **Distance culling** rejects instances beyond `cull_distance` (120 m) while the submission list is built, so they never reach the instance buffer or the indirect arguments at all — the GPU cull cannot do this, because a draw has to exist before it can be rejected. The test is horizontal distance, so flying up does not make ground cover vanish from under the camera. The per-frame submission vector is also reused rather than reallocated. **UI:** the Foliage section moved to the top of the inspector, since at the bottom it fell behind the output log and the type button was literally unclickable. The type control became an in-place cycler (`Type: Fir Sapling >`) rather than a popup — this UI has no anchoring, so a floating popup has to be hand-positioned and kept landing somewhere unhelpful; for a handful of entries, cycling cannot be occluded or mispositioned. Picking a tree still switches Single on automatically. |
| 17H | ✅ Complete | **Cutout foliage: alpha masks, alpha-weighted mips, and the island tree.** Three reported faults, three unrelated causes. (1) *Everything looked blue-grey.* Poly Haven ships vegetation as **alpha-cutout cards** — the diffuse atlas carries blade colour only where the mask is opaque (78% of `grass_medium_01` is near-black) and the cutout lives in a **separate `_alpha_` map their glTF never references**. Trusting the glTF meant rendering the black background as if it were the plant, leaving ambient sky as the brightest thing on screen. The loader now folds a sibling `X_alpha_2k.png` into `X_diff_2k.jpg`'s alpha channel by filename convention and promotes the material to `MASK` + double-sided; a missing sidecar is not an error. (2) *Saplings had no trunk.* `ensure_palette_mesh` kept the largest **primitive**, but a glTF node is usually several — the sapling is `branches` + `twigs`, the island tree is trunk + branches + leaves. Primitives are now grouped by node transform, the heaviest **node** wins, and all of its primitives are kept as `FoliagePart`s with a local transform. (3) *The island tree painted nothing.* Not the triangle cap, as assumed: the file lists `KHR_texture_transform` in `extensionsRequired` and the `gltf` crate rejected the import outright. Enabling the feature fixed it; failed imports are now cached so a broken model no longer retries — and stalls — on every brush dab. **Mip generation** was also wrong for cutouts: a plain box filter averages blade colour with the transparent background, so foliage darkened with distance, and averaging a binary mask drops texels under the 0.5 cutoff so coverage erodes until distant grass vanishes. Colour is now averaged **weighted by alpha**, and each level's alpha is rescaled to preserve coverage (Castaño). Both reduce exactly to the old behaviour for opaque textures. |
| 17I | ✅ Complete | **Ambient occlusion reaches indirect light.** The IBL term had a standing note that nothing attenuated sky light, and it showed on foliage: grass albedo is a dark olive, so an unoccluded sky reflection's 4% Fresnel sheen was a large share of each blade's colour — and the sky is blue. `Surface` now carries an `occlusion` term applied to indirect diffuse, and to indirect specular through Lagarde's specular-occlusion fit, never to the sun (which already has shadow maps). Sourcing it needed two attempts: reading AO from the metallic-roughness map's red channel rendered the damaged helmet **pitch black**, because glTF leaves that channel undefined and models with a separate AO texture leave it at zero. Occlusion now comes from the material's own `occlusionTexture` — plus one narrow inference: exporters that pack ARM (AO/Roughness/Metallic) have no way to declare it and simply leave `occlusionTexture` unset, so an `_arm` filename is taken as stating the packing. That is the same convention-over-metadata rule the `_alpha_` sidecars use, and it is scoped to the filename so a plain metallic-roughness map is never misread. `occlusion_map` took over the material struct's padding word, so the GPU layout is unchanged. |
| 16 | ⬜ Planned | Scripting (Rhai or Lua) |
| 25A | ⬜ Planned | **Terrain into the visibility buffer.** Terrain records at `renderer.rs:1516`, *after* the visibility pass (1386/1408), GTAO (1458) and ReSTIR (1443). It therefore misses every one of them, and `terrain.wgsl` carries its own duplicated copies of the shadow and cluster helpers — so each lighting improvement in Phase 24 had to be written twice or silently skipped terrain. This is the same failure as 24C's sky-in-three-places, and the fix is the same: one source. Terrain writes depth and visibility IDs in the pre-pass like everything else, and the duplicated helpers are deleted. **Unblocks GTAO, contact shadows, ReSTIR and correct TAA on terrain in one change, and is what makes 24K verifiable at all.** Reference: O3DE keeps a dedicated `Terrain_DepthPass.azsl` feeding the shared depth buffer rather than a self-contained terrain pass. |
| 25B | ⬜ Planned | **Terrain chunks in the TLAS.** 24J keys a BLAS per mesh by `vertex_offset`; terrain chunks never enter the draw queue it builds from, so terrain neither casts nor receives ray-traced shadows. Add committed chunk geometry as BLAS entries at the current LOD, rebuilt on sculpt. Together with 25A this is the whole of the 24K verification: a hill casting a soft traced shadow onto the valley beside it is a test that cannot pass by accident. |
| 25C | ⬜ Planned | **CDLOD vertex morphing.** `terrain/mesh.rs` builds discrete per-LOD index topology with edge stitching, so an LOD switch swaps geometry in one frame and pops — most visible exactly where it is least wanted, on a ridge line against the sky. CDLOD morphs vertices toward the coarser level's positions across the last part of each range, so the transition is continuous and the switch happens when the two meshes already agree. Reference: `CDLOD-master/source/BasicCDLOD/Shaders/CDLODTerrain.vsh` — `morphVertex`, `g_morphConsts`. |
| 25D | ⬜ Planned | **Macro + detail clipmaps.** One splatmap over the whole terrain sets a hard ceiling on texture detail: enough resolution close up means an impossible texture far away. O3DE's answer is two tiers — a *macro* clipmap covering the entire terrain at low frequency for colour and large-scale variation, and a *detail* clipmap of a few rings centred on the camera carrying full-rate PBR, composited per pixel. Detail cost then scales with screen area rather than world area. Reference: `TerrainMacroClipmapGenerationPass.azsl`, `TerrainDetailClipmapGenerationPass.azsl`, `ClipmapComputeHelpers.azsli`. |
| 25E | ⬜ Planned | **Height-weighted material blending.** The current shader sharpens splat weights, which is halfway there. O3DE's `AppendHeightToWeight` adds each material's own height map into its weight before normalising, so gravel settles *into* the cracks of rock instead of being averaged across it — the difference between two textures cross-faded and two materials meeting. Reference: `TerrainDetailHelpers.azsli`. |
| 25F | ⬜ Planned | **Stochastic hex-tiling.** The strongest remaining tell that terrain is rendered rather than photographed is *repetition*: one tiled albedo at a fixed rate produces a visible grid the eye locks onto immediately, and no amount of lighting work hides it. Heitz–Neyret hex-tiling samples the same texture at three hexagonal-lattice offsets with randomised rotation and blends by barycentric weight, breaking the lattice without a second texture or a visible seam. Reference: `bgfx-master/examples/49-hextile/fs_hextile.sc`; UE's TextureGraph ships the same idea as `AdjustHexaplanar*.usf`. |
| 25G | ⬜ Planned | **Biplanar upgrade for cliffs.** Triplanar projection already runs on steep slopes, but it costs three sample sets per map. Biplanar takes the two dominant axes instead of three, at close to the same quality for two thirds of the taps — which matters once 25D and 25F have multiplied the sample count. Reference: `bevy-plugins/bevy_triplanar_splatting-main/src/shaders/biplanar.wgsl`. |
| 25H | ⬜ Planned | **Parallax occlusion on detail materials.** Terrain is the surface most often viewed at a grazing angle, and a flat normal map reads as a decal on a plane exactly there. POM against the detail height map (already loaded for 25E, so no new texture budget) gives rock and gravel real silhouette displacement. Bound to the detail clipmap's inner rings only, since it is worthless past a few metres. |
| 25I | ⬜ Planned | **Aerial perspective on terrain.** 24C builds the LUT and terrain does not sample it, so distant hills stay saturated while everything else desaturates correctly — which reads as a matte painting behind a rendered scene. Cheap once 25A has terrain in the shared shading path. |
| 25J | ⬜ Planned | **Terrain material UI and colliders** (absorbs the old Phase 17 remainder). Per-layer tiling, tint, roughness and height-blend strength in the inspector, plus a collider built from the committed heightmap so gameplay and physics agree with what is drawn. |

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

### 25.4 Verification plan

Terrain makes the lighting work testable, so each sub-phase states its own check:

- **25A** — the same scene with `SOMNIUM_GTAO=0/1` must differ *on terrain*, which
  it cannot today. Terrain pixels appear in the visibility buffer debug view.
- **25B** — `SOMNIUM_RT_DEBUG=1` shows a terrain hill's shadow cast across terrain.
  This is the 24K acceptance test; 24K stays 🟡 until it passes.
- **25C** — fly a ridge line against the sky and record; no popping frame to frame.
- **25F** — a flat plain from a high camera: the tiling grid must not be findable.
- Every sub-phase keeps `cargo test --workspace` green, currently 198 tests.

---

## 18. Known Issues & Active Bugs

**RESOLVED — `GpuMaterial` layout mismatch (was: primitives cast no shadows,
foliage wrong colours).** WGSL aligns `vec3<f32>` to 16 bytes; Rust's `repr(C)`
aligns `[f32; 3]` to 4. So `emissive: vec3<f32>` in the shader's `Material` sat
at offset 64 with a 96-byte stride, against the CPU struct's offset 52 and
80-byte stride. Material 0 decoded correctly and **every material after it was
read from the wrong bytes** — the error grew with the index, which is why the
glTF helmet looked right while editor primitives and foliage did not.

The visible symptom was not obviously a material bug: garbage `metallic` came
back near 1, and `kD = (1 - F)(1 - metallic)` then zeroes the diffuse lobe, so
the sun contributed almost nothing and the surface was lit by IBL alone. With no
sun term left, multiplying by `shadow_factor` changed nothing — the shadows had
been computed correctly the whole time and had nothing to act on. Measured
before the fix: sun dominated on 0 of 14000 plane samples. After: 8658, garbage
metallic 0, and a cube's shadow reads 3.4 against 110.3 in the open.

Found by measuring rather than reading, via `SOMNIUM_SHADOW_DEBUG` (1 = shadow
factor, 2 = sun only, 3 = ambient only, 4 = shadow-map plumbing, 5 =
blocker_search verdict, 6 = shadow factor in hue, 7 = sun-vs-ambient dominance).
Modes 6 and 7 are what cracked it: 6 proved the shadow was correct, 7 proved the
sun term was missing. Reading the shadow code repeatedly found nothing because
the shadow code was never wrong.

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

**Remaining helmet shimmer is a different phenomenon**, most likely specular
aliasing: it is the one metallic, normal-mapped, high-gloss surface in the
scene, which is exactly what 24F's Toksvig specular AA targets and what TAA can
only partly hide. Worth checking the Toksvig term is actually reaching that
material before assuming more TAA work is needed.

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
