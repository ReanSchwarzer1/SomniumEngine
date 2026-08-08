# Attribution & Reference Architecture

> **Somnium Engine** is an original work written from scratch in Rust.  
> **No source code has been copied** from any third-party project.  
> The codebases listed here are studied for architectural patterns only.  
> All reference repositories reside in `example_repo/` and are excluded from the Cargo workspace.

---

## Table of Contents

1. [Unreal Engine 5](#1-unreal-engine-5)
2. [The Forge Framework](#2-the-forge-framework)
3. [bgfx](#3-bgfx)
4. [Open 3D Engine (O3DE)](#4-open-3d-engine-o3de)
5. [Ogre-Next (HLMS)](#5-ogre-next-hlms)
6. [DirectX Shader Compiler (DXC)](#6-directx-shader-compiler-dxc)
7. [SwiftShader](#7-swiftshader)
8. [VoxelHex](#8-voxelhex)
9. [Unity uGUI](#9-unity-ugui)
10. [Unity ML-Agents](#10-unity-ml-agents)
11. [SpartanEngine (BRDF)](#11-spartanengine-brdf)
12. [Cascaded Shadow Maps (Phase 11)](#12-cascaded-shadow-maps-phase-11)
13. [Phase 11.5 Editor Systems](#13-phase-115-editor-systems)
14. [Pattern Index](#14-pattern-index)
15. [Citation Rules](#15-citation-rules)

  - 13.1 Transform Gizmos
  - 13.2 Editor Grid
  - 13.3 HDR & Tone Mapping
  - 13.4 Undo/Redo Architecture
  - 13.5 GPU Particle System
  - 13.6 Selection Outline
  - 13.7 Toolbar Dropdown Z-Order Fix
  - 13.8 Output Log Capture
  - 13.9 Water Shader Architecture (Phase 12+)
  - 13.10 Voxel World — Chunk Meshing & LOD (Phase 13+)
  - 13.11 Cel-Shading Architecture (Phase 12+)
  - 13.12 UE5 Shader Definitions Headers
  - 13.13 Phase 12 Native UI — Fyrox generational pool (12A-1)
  - 13.14 Phase 12 Native UI — Fyrox widget/message/draw architecture (12A-2)
  - 13.15 Phase 12 Native UI — Fyrox widget library: Canvas/StackPanel/Border/Button/Grid (12A-3/12A-5)
  - 13.16 Phase 12 Native UI — fontdue font atlas + glyph rendering (12A-4)
  - 13.17 Phase 12 Native UI — UiPass wgpu render pass (12B-1)
  - 13.18 Phase 12 Native UI — ScrollViewer, TextBox, NumericField (12D-full)
  - 13.19 Ocean PBR Textures (Phase 13)
  - 13.20 Heightmap Terrain — Fyrox terrain + CDLOD + triplanar splatting (Phase 14 SSS)
  - 13.21 Light Gizmos — Bevy light gizmo shapes (Phase 13E)

---

## 1. Unreal Engine 5

**Copyright:** © Epic Games, Inc. All rights reserved.  
**License:** Unreal Engine End User License Agreement  
**Source:** `example_repo/UnrealEngine-release/`  
**Relevance:** Lifecycle design, platform abstraction, ECS (MassEntity), editor UX

### 1.1 Engine Lifecycle — `FEngineLoop`

| UE5 Source | Pattern Studied | Somnium Implementation |
|---|---|---|
| `Engine/Source/Runtime/Launch/Public/LaunchEngineLoop.h` | Phased lifecycle: `PreInit → Init → Tick → Exit` | `somnium_core::app::Engine<G>` state machine: `Uninitialized → Running → Suspended → ShuttingDown` |
| `FEngineLoop::PreInitPreStartupScreen()` | Subsystem init ordering (RHI before modules) | `Engine::resumed()`: window → `RenderContext` → `SomniumRenderer` → `UiManager` → `game.on_init()` |
| `FEngineLoop::Tick()` | `PreTick`, physics step, `Tick`, `PostTick`, render | `about_to_wait()`: `time.tick()` → `physics.step()` → `on_update()` → `on_render()` → `render()` |
| `FEngineLoop::Exit()` | Ordered teardown with `on_shutdown` callback | `initiate_shutdown()` → `game.on_shutdown()` → `event_loop.exit()` |

**Specific pattern:** UE5's `FEngineLoop` separates engine initialization from game initialization. Somnium mirrors this: the engine brings up the GPU and windowing subsystems before calling `game.on_init()`, giving game code a fully initialized `EngineContext`.

### 1.2 Platform Event Abstraction

| UE5 Source | Pattern Studied | Somnium Implementation |
|---|---|---|
| `Engine/Source/Runtime/ApplicationCore/Public/GenericPlatform/GenericApplication.h` | Platform-agnostic window/input layer | `somnium_core::event` module — game code never imports `winit` |
| `GenericApplicationMessageHandler.h` | Typed virtual callbacks per event kind | `EngineEvent` enum variants (one per event type) |
| `FSlateApplication::ProcessWindowActivatedMessage()` | Focus routing between OS windows | `window.focus_window()` on RMB press to reclaim focus from wry WebViews |

**Specific pattern:** UE5's `GenericApplicationMessageHandler` defines a virtual method per input event (e.g., `OnKeyDown`, `OnMouseButtonDown`). Somnium collapses these into a single `EngineEvent` enum dispatched to `GameApp::on_event()`, trading a small runtime match overhead for ergonomic pattern matching.

### 1.3 ECS — MassEntity

| UE5 Source | Pattern Studied | Somnium Implementation |
|---|---|---|
| `Engine/Source/Runtime/MassEntity/Public/MassArchetypeTypes.h` | `FMassArchetypeData`: entities grouped by component signature | `somnium_ecs::archetype::Archetype` with `ComponentSet` bitmask key |
| `MassArchetypeTypes.h` :: `FMassArchetypeChunkIterator` | Chunk-based iteration over SoA columns | `Archetype::column(col_idx)` → raw pointer column, iterated by `row` index |
| `MassEntityManager.h` :: `FMassEntityManager` | Global world owning archetypes + entity → archetype map | `somnium_ecs::world::World` |
| `MassCommandBuffer.h` :: `FMassCommandBuffer` | Deferred structural mutations to avoid invalidating iterators | Reserved for Phase 10+ (structural changes during iteration) |
| `MassEntityQuery.h` :: `FMassEntityQuery` | Cached archetype matching with `Requirements` | `World::query_archetypes()` with `required: ComponentSet` + `excluded: ComponentSet` |

**Specific pattern:** UE5 MassEntity groups entities by "archetype" — their exact set of component types — and stores each component type in a contiguous array within that archetype. Somnium's `Archetype` struct does exactly this: each column is a type-erased `Vec<u8>` indexed by `ComponentId`. Iterating a `(Transform, MeshComponent)` query walks two contiguous arrays with no pointer indirection per entity.

### 1.4 Editor UX Conventions

| UE5 Feature | Somnium Equivalent | Notes |
|---|---|---|
| Viewport right-click fly-cam (W/A/S/D/Q/E + Shift) | `EditorCamera` in `hello_engine/src/main.rs` | Identical bindings and speed-boost via Shift |
| Outliner panel (actor hierarchy) | `#right_panel` → `#outliner-list` | Entities synced every 60 frames via IPC |
| Details panel (per-actor properties) | `#right_panel` → `#selection-details` | Shows Transform TRS for selected entity |
| Content Browser (asset grid) | `#bottom_browser` | Static layout, asset loading in Phase 10 |
| Toolbar (Play/Pause/Stop) | `#toolbar` | Buttons wired in HTML; engine integration pending |
| FPS counter (top-right toolbar) | `#fps-counter` | Updated via `update_fps` IPC message each frame |

---

## 2. The Forge Framework

**Copyright:** © 2017-2025 The Forge Interactive Inc.  
**License:** Apache License 2.0  
**Source:** `example_repo/The-Forge-master/`  
**Relevance:** Visibility Buffer architecture, RHI design, multi-backend GPU abstraction

### 2.1 Visibility Buffer — `IVisibilityBuffer`

| Forge Source | Pattern Studied | Somnium Implementation |
|---|---|---|
| `Common_3/Renderer/Interfaces/IVisibilityBuffer.h` | Triangle filtering: write (InstanceID, PrimitiveID) per pixel | `VisibilityBufferPass`: R32Uint texture, packs `(inst+1) << 22 | prim_id` |
| `Common_3/Renderer/VisibilityBuffer/VisibilityBuffer.cpp` | Two-pass render loop: visibility → shading | `renderer.rs::render()`: Pass 1 clears to 0, draws all geometry; Pass 2 fullscreen shade |
| Forge programmable vertex pulling | No vertex buffer binding in VS; reads from storage arrays | `visibility.wgsl`: `vertices[instances[inst_idx].vertex_offset + indices[...]]` |
| Forge barycentric reconstruction | Perspective-correct barycentrics from NDC triangle | `shading.wgsl`: `det`, `w0`, `w1`, `w2` from NDC coords of clip-space triangle |

**Specific pattern:** The Forge's visibility buffer uses the GPU's `SV_PrimitiveID` to store the triangle index. Somnium uses WGSL's `@builtin(primitive_index)` — the direct WGSL equivalent. The packing `(instance_id + 1) << 22 | primitive_id` matches Forge's bitfield layout philosophy, with the `+1` offset added by Somnium to reserve 0 as the sky sentinel (The Forge uses a separate depth clear strategy).

### 2.2 RHI Design — `IGraphics`

| Forge Source | Pattern Studied | Somnium Implementation |
|---|---|---|
| `Common_3/Graphics/Interfaces/IGraphics.h` | `ResourceState` enum for texture/buffer transitions | Expressed via `wgpu::TextureUsages` and pass load/store ops |
| `IGraphics.h` :: `DescriptorType` | Uniform, Storage, Sampler, Texture, RWTexture types | Mapped to `wgpu::BindingType` variants |
| `IGraphics.h` :: `ShaderStage` | Vertex/Fragment/Compute bitflags | `wgpu::ShaderStages::VERTEX | FRAGMENT` |
| `IGraphics.h` :: `PipelineType` | Graphics vs Compute | Separate `create_render_pipeline` / `create_compute_pipeline` paths |
| Multi-backend architecture (DX12/Vulkan/Metal) | Shared interface, backend-specific impl | Delegated to `wgpu` which handles all backends transparently |

### 2.3 Application Lifecycle — `IApp`

| Forge Source | Pattern Studied | Somnium Implementation |
|---|---|---|
| `Common_3/Application/Interfaces/IApp.h` | `Init / Load / Update / Draw / Unload / Exit` | `GameApp::on_init / on_update / on_render / on_shutdown` |
| `IApp::mSettings.mReloadType` | `ReloadDesc` for hot GPU resource swap without full restart | Reserved: `somnium_renderer::SomniumRenderer::resize()` recreates pass resources on window resize |

---

## 3. bgfx

**Copyright:** © 2011-2026 Branimir Karadzic.  
**License:** BSD 2-Clause  
**Source:** `example_repo/bgfx-master/`  
**Relevance:** Stateless draw command submission, sort key design, pipeline state caching

### 3.1 Stateless Draw Submission

| bgfx Source | Pattern Studied | Somnium Implementation |
|---|---|---|
| `include/bgfx/bgfx.h` | Stateless submit: no mutable pipeline state between calls | `DrawCommand` struct: pure data, no side effects on submit |
| `bgfx.h` :: `bgfx::submit(view, program, ...)` | Commands tagged with view ID for sorting | `DrawCommand::sort_key` encodes `pass_id \| material_id \| mesh_id` |
| `src/renderer.h` :: `SortKey` | 64-bit key: translucency + material + depth | `SortKey(u64)` bit layout: `pass_id[63:56] | material_id[55:32] | mesh_id[31:0]` |
| `src/renderer.h` :: `StateCacheLru` | LRU pipeline state dedup to minimize API calls | Planned: `MaterialSystem::get_or_create_pipeline()` in Phase 10 |

**Specific pattern:** bgfx never mutates GPU state directly during scene traversal. Game code calls `bgfx::submit()` with a view ID and a sort key; bgfx sorts and deduplicates before issuing draw calls. Somnium's `renderer.submit(DrawCommand)` + `draw_queue.sort_by_key(|cmd| cmd.sort_key)` implements this exactly, though the sort currently happens in Rust-side before the render pass begins.

### 3.2 Vertex Layout

| bgfx Source | Pattern Studied | Somnium Implementation |
|---|---|---|
| `src/vertexlayout.cpp` | Hash-based vertex attribute layout caching | `somnium_asset::Vertex` is a fixed 32-byte layout (`position + normal + uv`) |
| `VertexLayout::begin() / add() / end()` | Builder pattern for vertex declarations | Reserved for Phase 10 when multiple vertex formats are needed |

---

## 4. Open 3D Engine (O3DE)

**Copyright:** © Contributors to the Open 3D Engine Project.  
**License:** Apache License 2.0 / MIT  
**Source:** `example_repo/o3de-development/`  
**Relevance:** Bindless resource management (Atom RHI)

### 4.1 Atom RHI Bindless

| O3DE Source | Pattern Studied | Somnium Implementation |
|---|---|---|
| `Gems/Atom/RHI/Bindless.md` | Global descriptor pool: one `BindGroup` for all resources | `GlobalResourcePool`: single `@group(0)` bind group shared by both passes |
| O3DE bindless texture array | `binding_array<texture_2d<f32>>` for all scene textures | `@group(0) @binding(4) var textures: binding_array<texture_2d<f32>>` (1024 slots) |
| Descriptor index in instance data | Material/mesh IDs passed per-instance, not per-draw | `GpuInstanceData::material_id` → `materials[instance.material_id]` in fragment shader |
| Partially bound arrays | Not all 1024 texture slots need valid descriptors | `wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY` required at device creation |

**Specific pattern:** O3DE's Atom renderer avoids per-material bind group rebinding by allocating all descriptors into a single global array and indexing dynamically in the shader. Somnium's `GlobalResourcePool` implements exactly this: `binding_array<texture_2d<f32>>(1024)` filled with a dummy 1×1 white texture, replaced with real textures as assets load.

**Feature requirements set at device creation:**

```rust
wgpu::Features::TEXTURE_BINDING_ARRAY
| SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
| PARTIALLY_BOUND_BINDING_ARRAY
| PRIMITIVE_INDEX
```

---

## 5. Ogre-Next (HLMS)

**Copyright:** © The OGRE Team.  
**License:** MIT License  
**Source:** `example_repo/ogre-next-master/`  
**Relevance:** Data-driven material system (High Level Material System)

### 5.1 HLMS — Material System

| Ogre-Next Source | Pattern Studied | Somnium Implementation |
|---|---|---|
| `OgreMain/include/OgreHlms.h` | HLMS: generates concrete shader permutations from property sets | `somnium_renderer::material::hlms::MaterialSystem` — cache of pipeline permutations |
| `OgreHlmsDatablock` | Per-material data block (roughness, metallic, textures) | `GpuMaterial { base_color, roughness, metallic, albedo_map, normal_map, ... }` |
| HLMS property system | Compile-time shader macros for feature flags | Reserved: Phase 10 will add `#define HAS_NORMAL_MAP` permutations |

**Current state:** `MaterialSystem` is a placeholder struct. The actual material properties live in `MaterialPool` (GPU storage buffer). Full HLMS-style permutation generation is Phase 10 work.

---

## 6. DirectX Shader Compiler (DXC)

**Copyright:** © Microsoft Corporation. All rights reserved.  
**License:** University of Illinois/NCSA Open Source License  
**Source:** `example_repo/DirectXShaderCompiler-main/`  
**Relevance:** Shader compilation model, HLSL semantics (mapped to WGSL)

### 6.1 Compiler-as-Library Model

| DXC Source | Pattern Studied | Somnium Implementation |
|---|---|---|
| `include/dxc/dxcapi.h` | `IDxcCompiler3`: compile from in-memory source, return diagnostics | Planned: hot-reload via `device.create_shader_module()` with file watcher in Phase 10 |
| HLSL semantic annotations (`POSITION`, `TEXCOORD`, `SV_InstanceID`) | Data flow documentation discipline | WGSL builtins: `@builtin(position)`, `@builtin(instance_index)`, `@builtin(primitive_index)` |
| DXIL intermediate representation | Separating "compile" from "link" stages | Current: `include_str!()` compile-time embedding; Phase 10: runtime WGSL reload |

### 6.2 HLSL → WGSL Semantic Mapping

| HLSL Semantic | WGSL Equivalent | Used In |
|---|---|---|
| `SV_VertexID` | `@builtin(vertex_index)` | `visibility.wgsl::vs_main`, `shading.wgsl::vs_main` |
| `SV_InstanceID` | `@builtin(instance_index)` | `visibility.wgsl::vs_main` |
| `SV_PrimitiveID` | `@builtin(primitive_index)` | `visibility.wgsl::fs_main` |
| `SV_Position` | `@builtin(position)` | Both VS outputs |
| `nointerpolation` | `@interpolate(flat)` | `instance_id` pass-through in visibility shader |

---

## 7. SwiftShader

**Copyright:** © Google LLC.  
**License:** Apache License 2.0  
**Source:** `example_repo/swiftshader-master/`  
**Relevance:** Deep understanding of Vulkan API semantics and validation behavior

### 7.1 Vulkan Semantics Study

| SwiftShader Source | Knowledge Gained | Applied In |
|---|---|---|
| `src/Vulkan/VkCommandBuffer.cpp` | `vkCmdClearAttachments` uses `VkClearValue` union — float vs uint32 path | `renderer.rs`: R32Uint vis_buffer cleared with `Color { r: 0.0, ... }` not `u32::MAX as f64` to avoid DX12 bit-cast issue |
| `src/Vulkan/VkPipeline.cpp` | Pipeline state compilation, descriptor set layout validation | Informed `GlobalResourcePool` layout matching shader bindings exactly |
| `src/Pipeline/` | JIT compilation pipeline, shader stage linking | Understanding of why `create_shader_module` can fail at runtime |

---

## 8. VoxelHex

**Copyright:** © VoxelHex Contributors.  
**License:** Apache License 2.0 / MIT  
**Source:** `example_repo/VoxelHex-main/`  
**Relevance:** Compute-shader raytracing patterns in wgpu

### 8.1 GPU Ray Traversal

| VoxelHex Source | Pattern Studied | Somnium Future Use |
|---|---|---|
| `src/raytracing/` | Compute-shader driven ray-AABB traversal over sparse voxel structures | Phase 14+: hybrid ray tracing for reflections / AO using compute shaders |
| Wry integration in `whisp/` | wry WebView usage patterns in Rust | Informed `somnium_ui` WebView builder setup and transparency configuration |

---

## 9. Unity uGUI

**Copyright:** © Unity Technologies.  
**License:** Unity Companion License  
**Source:** `example_repo/uGUI-main/`  
**Relevance:** Component-based UI hierarchy, event routing

### 9.1 UI Component Model

| uGUI Source | Pattern Studied | Somnium Application |
|---|---|---|
| `com.unity.ugui/Runtime/UI/Core/Canvas.cs` | `Canvas` as the root UI render target | `UiManager` owns all panels; one root per logical section |
| `com.unity.ugui/Runtime/UI/Core/Graphic.cs` | `Graphic` base class with dirty-flag rebuilds | Reserved: `somnium_ui` panels rebuild DOM when IPC data changes |
| `com.unity.ugui/Runtime/EventSystem/` | Raycasting-based input routing to UI elements | Current: wry handles its own hit-testing; JS `onclick` dispatches IPC |
| `EventSystem/InputModules/StandaloneInputModule.cs` | Focus management between UI elements | Informed the `window.focus_window()` RMB fix for viewport camera focus |

---

## 10. Unity ML-Agents

**Copyright:** © Unity Technologies.  
**License:** Unity Companion License  
**Source:** `example_repo/ml-agents-develop/`  
**Relevance:** Agent/environment communication model for future AI components

### 10.1 Agent ↔ Environment Interface

| ML-Agents Source | Pattern Studied | Somnium Future Use |
|---|---|---|
| `com.unity.ml-agents/Runtime/Agent.cs` | Agent as an ECS component with observation/action buffers | Phase 15+: `AiAgent` component with `observe()` → neural net → `act()` cycle |
| `Runtime/Sensors/SensorComponent.cs` | Modular sensor architecture (camera, raycasts, physics overlap) | Phase 15+: `SensorComponent` ECS component with pluggable sensor implementations |
| `Runtime/Communicator/` | Python ↔ Unity socket protocol | Phase 15+: Rust-side inference via `tract` or ONNX Runtime FFI |

---

## 11. SpartanEngine (BRDF)

**Copyright:** © Panos Karabelas (SpartanEngine contributors).  
**License:** MIT  
**Source:** Not in `example_repo/`; studied from public repository  
**Relevance:** Cook-Torrance BRDF implementation ported to WGSL

### 11.1 PBR BRDF Functions

| SpartanEngine Function | Somnium WGSL Equivalent | File |
|---|---|---|
| `F_Schlick(f0, v_dot_h)` | `F_Schlick(f0, v_dot_h)` | `brdf.wgsl` |
| `D_GGX(n_dot_h, roughness)` | `D_GGX(n_dot_h, roughness)` | `brdf.wgsl` |
| Smith Joint GGX visibility term | `V_SmithGGX(n_dot_v, n_dot_l, roughness)` | `brdf.wgsl` |
| Burley (Disney) diffuse | `Diffuse_Burley(albedo, roughness, n_dot_v, n_dot_l, v_dot_h)` | `brdf.wgsl` |
| `evaluate_brdf(surface, l)` | `evaluate_brdf(surface, l)` | `brdf.wgsl` |

**Adaptation:** The original C++ used `float3` HLSL types. Translated to WGSL `vec3<f32>` with WGSL-idiomatic `saturate()`, `max()`, and no implicit conversions. The `Surface` and `AngularInfo` structs mirror SpartanEngine's `Material` and `LightComputeData` aggregates.

---

## 12. Cascaded Shadow Maps (Phase 11)

**Primary References:**

| Source | Author / Publisher | Topic |
|---|---|---|
| "Cascaded Shadow Maps" — GPU Gems 3, Chapter 10 | NVIDIA Developer Zone | PSS (Practical Split Scheme) logarithmic-linear blend for cascade partition |
| "Common Techniques to Improve Shadow Depth Maps" — Microsoft DirectX Graphics Samples | Microsoft Corporation | Texel-snapping for stable shadow edges, depth-bias tuning |
| "Sample Distribution Shadow Maps" — EGSR 2010 | Rouslan Dimitrov (NVIDIA) | Bounding-sphere fit for stable ortho projection |

**No source code was copied.** The mathematical formulations (PSS split formula, bounding sphere, texel snap) are documented algorithms re-implemented in Rust and WGSL from the published papers and articles above.

### 12.1 PSS Cascade Partitioning

| Reference Algorithm | Somnium Implementation |
|---|---|
| GPU Gems 3 §10.3 — Practical Split Scheme: `C_i = λ·C_log_i + (1-λ)·C_uni_i` | `shadow/cascade.rs::compute_cascades`: `ratio=far/near`, `C_log=near*ratio^(i/N)`, `C_uni=near+range*(i/N)`, blend with `LAMBDA=0.5` |
| GPU Gems 3 §10.4 — Fitting to a tight bounding sphere to avoid shadow aliasing on rotation | `shadow/cascade.rs::cascade_vp`: frustum corners extracted via `inv_view_proj`, bounding sphere center + radius computed, `look_at_rh` at `center + light_dir * radius * 2` |
| Microsoft DirectX Samples — texel snapping | `shadow/cascade.rs::cascade_vp`: `center_ls.x/y` snapped to `world_units_per_texel = (2*radius) / CASCADE_SIZE as f32` grid |
| wgpu NDC depth [0, 1] (not OpenGL [-1, 1]) | `shadow/cascade.rs::ortho_rh_zo`: custom orthographic matrix mapping Z to [0,1]; equivalent to `glm::orthoZO` from GLM |

### 12.2 PCF Shadow Sampling

| Reference | Somnium Implementation |
|---|---|
| Standard 3×3 PCF with `textureSampleCompare` | `shading.wgsl::sample_shadow`: 9-tap loop over `(-1..=1, -1..=1)` offsets in UV space, average of `textureSampleCompare(shadow_atlas, shadow_sampler, uv, ref_depth)` |
| Cascade index from view-space Z | `shading.wgsl::get_cascade_index(view_depth)`: compares `abs(view_depth)` against `light.cascade_splits[0..3]`, returns first cascade whose split exceeds the depth |
| Atlas UV remapping | `shading.wgsl::atlas_uv(cascade, uv)`: maps cascade UV into one of 4 quadrants in the 4096×4096 atlas (`cascade%2` → x, `cascade/2` → y) |

---

## 13. Phase 11.5 Editor Systems

### 13.1 Transform Gizmos (Phase 11.5B)

**Primary References:**

| Source | Topic |
|---|---|
| Unreal Engine 5 — `Editor/UnrealEd/Private/EditorViewportClient.cpp` | Translate/Rotate/Scale axis handle UX conventions |
| Blender source — `source/blender/editors/transform/` | Ray-axis-line closest-point formula for drag projection |

**Mathematical formulations (no code copied):**

| Algorithm | Somnium Implementation |
|---|---|
| Ray vs axis-line closest approach (two-ray distance minimization) | `app.rs::ray_axis_param()` — standard parametric derivation, two unknowns (t on ray, s on axis line), solved via 2×2 linear system |
| Ray-plane intersection for rotation | `app.rs::ring_angle()` — dot-product form: `t = (center-origin)·normal / dir·normal` |
| Screen-space → world-space unprojection | `app.rs::ndc_to_world()` — `inv_view_proj * (ndc, 0.5, 1)` then perspective divide |
| AABB slab method for picking | `somnium_renderer::pass::gizmo::ray_aabb()` — standard slab parametric intersection |

### 13.2 Editor Grid (Phase 11.5H)

**Primary References:**

| Source | Topic |
|---|---|
| Evan Wallace, "Antialiased Grid Shader" (2016) | `fwidth()`-based derivative anti-aliasing for grid lines |
| Acerola / bgfx grid.glsl | Infinite XZ-plane grid via ray reconstruction, distance fade, axis highlighting |

**Adaptation:** Re-implemented in WGSL (`grid.wgsl`); distance fade uses smoothstep over 50–100 m range; separate minor (1 m, gray 0.35) / major (10 m, gray 0.65) / axis (red/blue 0.90) line weights.

### 13.3 HDR & Tone Mapping (Phase 11.5K)

**Primary References:**

| Source | Topic |
|---|---|
| Stephen Hill / Narkowicz — ACES filmic tone mapping curve | `(x*(2.51x+0.03))/(x*(2.43x+0.59)+0.14)` analytic fit |
| John Hable, "Filmic Tonemapping Operators" (2010) | Shoulder/toe curve motivation; ACES chosen over Hable for wider coverage |

**Somnium implementation:** `postprocess.wgsl::aces_film()` — component-wise vec3 application, clamped to [0,1]. No LUT.

### 13.4 Undo/Redo Architecture (Phase 11.5E)

**Reference:** Command Pattern (GoF) — `EditorCommand` trait with `execute()` / `undo()` / `description()`.  
`UndoStack` is a bounded deque (128-command capacity); `redo_stack` cleared on any new push.  
`push_silent()` added to record effects that have already been applied (gizmo drag final state).

### 13.5 GPU Particle System (Phase 11.5J)

**Primary Reference:** `example_repo/bevy-plugins/bevy_enoki-master/crates/enoki2d/src/`

Key patterns studied:
- `update.rs`: `ParticleStore` (Vec of `Particle`), `ParticleSpawnerState` (max_particles, active, spawn timer), `ParticleEffectInstance`; Bevy task pool parallel simulation
- `lib.rs`: Two separate wgsl shader handles — `particle_vertex.wgsl` + `particle_color_frag.wgsl`
- `Particle` struct: transform, duration, duration_fraction, velocity, color, frame, linear_acceleration, linear_damp, angular_acceleration, angular_damp, gravity_speed, gravity_direction
- GPU billboard: 6-vertex quad per instance, expand corners in view space using camera right/up

**Somnium adaptation:**
- `ParticleEmitter` is an ECS component with max_particles, spawn_rate, lifetime, initial_speed, spread_angle, size_start/end, color_start/end, gravity.
- CPU simulation in `simulate_particles()` uses an LCG (no rayon dependency) and `while`-loop particle advance with `swap_remove` for dead particles.
- GPU instance: 32 bytes (`position: [f32;3], size: f32, color: [f32;4]`) vs bevy_enoki's 80 bytes. Simplified because no sprite sheet or angular velocity.
- `ParticlePass::record()` uploads to storage buffer and calls `draw(0..6, 0..count)`.
- Fragment shader: smooth radial alpha (`smoothstep(0.5, 0.2, d)`) for soft billboard appearance.

**Files:** `somnium_core/src/lib.rs` (ParticleEmitter, simulate_particles), `somnium_renderer/src/pass/particle.rs`, `somnium_renderer/src/shaders/particle.wgsl`

### 13.6 Selection Outline (Phase 11.5I)

**Primary Reference:** `example_repo/bevy-plugins/bevy_mod_outline-master/src/`

Key patterns studied:
- Three-pass stencil architecture: `node.rs` — stencil write pass, then extruded volume pass with front-face cull
- Clip-space vertex extrusion: `outline.wgsl` — projects normals to clip space, normalizes XY, scales by `clip.w` for perspective-correct constant screen width; aspect correction applied
- `OutlineInstanceUniform`: `world_from_local mat3x4`, `volume_colour`, `volume_offset`

**Somnium adaptation:** Reduced to two sub-passes (not three) because the Somnium pipeline doesn't need a separate depth-prime pass. Storage-buffer vertex pulling avoids a separate vertex buffer for the extruded mesh. The uniform buffer carries `vertex_offset`/`index_offset` as `u32` fields so the same GeometryPool buffers used by the visibility pass can be reused. Outline color is hardcoded orange (#FA9412) rather than a per-entity component.

**Files:** `somnium_renderer/src/pass/outline.rs`, `somnium_renderer/src/shaders/outline.wgsl`

### 13.7 Toolbar Dropdown Z-Order Fix (BUG-005)

**Reference:** `example_repo/fyrox/Fyrox-master/fyrox-ui/src/popup.rs` — Fyrox creates popups at root canvas level (not as children of the menu button) and sends `WidgetMessage::Topmost` on open to escape parent clip bounds.


**Somnium adaptation:** Fyrox's approach is not directly applicable to wry WebViews because each panel is a native HWND with a fixed clip rectangle. The analogous fix is to dynamically resize the toolbar HWND when a dropdown opens:
- JS sends `menu_opened { height }` IPC when a dropdown opens (`toggleMenu()`).
- `UiManager::expand_toolbar(height)` calls `WebView::set_bounds()` to grow the toolbar HWND.
- JS sends `menu_closed {}` IPC when menus close (`closeAllMenus()`).
- `UiManager::collapse_toolbar()` restores the 40 px height.
- A `document.addEventListener('click', ...)` global handler ensures menus close on outside clicks.

**Files changed:** `somnium_ui/src/editor.html`, `somnium_ui/src/lib.rs`, `somnium_core/src/app.rs`.

### 13.8 Output Log Capture (Phase 11.5M)

**Reference:** `example_repo/fyrox/Fyrox-master/editor/src/log.rs` — Fyrox's `LogSettings` ring-buffer concept: a background thread receives `Log::message` calls and appends entries to a bounded `VecDeque`; the editor panel polls it each frame and renders the tail.

**Somnium adaptation:** Fyrox uses its own `Log` trait (not `tracing`). Somnium implements the same producer/consumer split using `tracing_subscriber`:

- `LogCaptureLayer` implements `tracing_subscriber::Layer<S>`, capturing `INFO`/`WARN`/`ERROR` events via `on_event` and sending `LogEntry` through an `mpsc::channel`.
- `make_log_capture()` returns `(LogCaptureLayer, Receiver<LogEntry>)`.
- In `Engine::run()`, both the `fmt::layer()` and `capture_layer` are installed via `tracing_subscriber::registry().with(...).try_init()`.
- `Engine.log_rx` holds the receiver; `about_to_wait()` drains it every 5 frames and forwards each entry to the HTML output log via `UiManager::send_message("append_log", ...)`.
- The ring-buffer truncation (500 lines max) is handled on the JS side in `appendLog()`.

**Files:** `somnium_core/src/log_capture.rs`, `somnium_core/src/app.rs`.

### 13.9 Water Shader Architecture (Phase 12+)

**Reference:** `example_repo/bevy-plugins/bevy_water-main/` — `src/wave.rs`, `src/water.rs`, `src/water/material.rs`.

**Copyright:** bevy_water authors (MIT/Apache 2.0 dual-licensed).

**Key patterns studied:**

| Pattern | Reference location | Planned Somnium use |
|---|---|---|
| FBM (Fractional Brownian Motion) wave noise | `wave.rs::fbm()` — 4-octave value noise with rotating `M2` matrix to avoid directional bias | `water.wgsl`: 4-octave FBM for displaced vertex height and normal perturbation |
| Multi-layer directional wave function | `wave.rs::sample_directional_wave()` — 2–4 additive layers with opposing time offsets | Phase 12 water vertex shader; quality levels (1–4 layers) |
| Dual-direction crossfade blending | `water.rs::WaveDirection` — `dir_a`/`dir_b` blend with asymmetric `smoothstep(0.0, 0.85, blend)` | Wind direction changes in real-time without discontinuous wave jump |
| Tile-offset desync | `WaveDirection.tile_offset` — per-entity offset so water tiles don't animate in lockstep | Phase 12 tiled water planes |
| CPU wave height query | `wave.rs::get_wave_height()` / `get_wave_point()` — evaluates same math on CPU for buoyancy | Phase 12 floating rigid body support |

**Architecture decision for Somnium:** bevy_water uses Bevy's material extension system (`ExtendedMaterial`). Somnium will adapt the math (FBM noise + directional wave layers) into a dedicated `WaterPass` that runs after the visibility shading pass, writing to the HDR target with depth testing. The CPU-side `get_wave_height()` math will live in `somnium_core` for physics buoyancy.

### 13.10 Voxel World — Chunk Meshing & LOD (Phase 13+)

**Reference:** `example_repo/bevy-plugins/bevy_voxel_world-main/` — `src/chunk.rs`, `src/meshing.rs`, `src/voxel_world.rs`.

**Copyright:** bevy_voxel_world authors (MIT/Apache 2.0 dual-licensed).

**Key patterns studied:**

| Pattern | Reference location | Planned Somnium use |
|---|---|---|
| Fixed-size padded chunk | `chunk.rs`: `CHUNK_SIZE_U = 32`, `PADDED_CHUNK_SIZE = 34` (1-voxel border for face culling across chunk boundaries) | Phase 13 chunk size; padding prevents seam cracks |
| `block_mesh::visible_block_faces()` culling | `meshing.rs::generate_chunk_mesh()` — calls `visible_block_faces()` from the `block_mesh` crate with `RIGHT_HANDED_Y_UP_CONFIG` | Somnium will use the same `block_mesh` crate for greedy face merging |
| LOD via voxel downsampling | `meshing.rs::generate_chunk_mesh_for_shape()` — when `data_padded_shape != mesh_padded_shape`, calls `resample_voxels_nearest()` to halve the voxel grid before meshing | LOD levels 1/2/4× via nearest-neighbour downsampling; higher LODs mesh coarser chunks |
| Async chunk task pattern | `chunk.rs::ChunkThread<C, I>` — wraps `bevy::tasks::Task<ChunkTask>` in a `SparseSet` component; polled each frame, mesh applied when ready | Phase 13 non-blocking chunk generation on a rayon thread pool |
| `NeedsRemesh` / `NeedsDespawn` marker components | `chunk.rs` — ECS-driven chunk lifecycle via marker components rather than direct function calls | Clean separation: mark → system picks up → generate → apply |

**Architecture decision for Somnium:** The chunk pipeline will be: `World::set_voxel()` → mark chunk `NeedsRemesh` → background thread generates mesh with `block_mesh` → upload to `GeometryPool` → render via existing visibility buffer. LOD blending will use alpha fade at transition distances.

**Implementation (Phase 14, complete):** `crates/somnium_voxel/` — see context.md §19. Adaptations from the reference:

| bevy_voxel_world pattern | Somnium adaptation |
|---|---|
| `ChunkThread` wrapping `bevy::tasks::Task`, polled as ECS component | `rayon::spawn` workers + `std::sync::mpsc` channel drained in `VoxelWorld::update()` |
| `NeedsRemesh` / `NeedsDespawn` marker components | Per-chunk `dirty` flag + `version` counter; stale in-flight results discarded by version comparison |
| Chunk entities with Bevy `Mesh` assets | No ECS entities: one `DrawCommand` per chunk through the visibility-buffer pipeline; GPU memory recycled via a new `GeometryPool` free-list (`upload_mesh_pooled` / `free_mesh`) |
| `voxel_lookup_delegate` closure per chunk | Deterministic `TerrainConfig::voxel(pos)` (original FBM value noise) + sparse `set_voxel` edit overlay snapshot passed to workers |
| `resample_voxels_nearest` + border-aligned `map_nearest_1d` | Same algorithm re-implemented in `mesh.rs::resample_nearest` (LOD 0/1/2 = 32³/16³/8³) |
| `ATTRIBUTE_TEX_INDEX` custom vertex attribute + texture array material | 6×1 palette texture; voxel type encoded as constant per-face UV at the texel center (Somnium `Vertex` has no spare attribute) |
| Vertex-color ambient occlusion (`face_aos`) | Not ported — `Vertex` has no color channel (future work) |

The FBM value-noise terrain generator (`terrain.rs`) is original code, not derived from bevy_voxel_world (whose examples use the external `noise` crate).

### 13.11 Cel-Shading Architecture (Phase 12+)

**Reference:** `example_repo/bevy-plugins/bevy_wind_waker_shader-main/` — `src/components.rs`, `src/assets/toon_shader.wgsl`.

**Copyright:** bevy_wind_waker_shader authors (MIT/Apache 2.0 dual-licensed).

**Key patterns studied:**

| Pattern | Reference location | Planned Somnium use |
|---|---|---|
| 1D gradient texture quantization | `toon_shader.wgsl`: `uv = vec2(out.color.r, 0.0); textureSample(mask, ...)` — intensity drives U coordinate, texture maps intensity → quantized band | Phase 12 cel pass: sample `toon_ramp.png` at `N·L` to get discrete shading bands |
| ZAtoon mask texture | `src/assets/ZAtoon.png` — greyscale ramp with sharp step at 0.5 for two-tone shading | Same approach; Somnium will ship a default 256×1 ramp texture |
| Rim highlight | `toon_shader.wgsl`: `rim = 1 - abs(dot(eye, world_normal)); rim_factor = rim^4` — edge pixels boosted toward `rim_color` | Phase 12 cel pass: add rim in the shading pass after main lighting |
| `highlight_color` / `shadow_color` uniforms | `WindWakerShader` struct (`components.rs`): per-material color pair for lit and unlit regions | Somnium `CelMaterial` ECS component with `highlight: [f32;4]`, `shadow: [f32;4]`, `rim: [f32;4]` |
| Time-of-day presets | `WindWakerShaderBuilder`: 12 color palettes (time × weather) hardcoded | Phase 12: `CelPreset` enum in Somnium with the same 12 combinations |

**Architecture decision for Somnium:** The cel pass will be a post-shading fullscreen pass that reads the existing shading buffer and applies the ramp lookup + rim highlight. This avoids modifying the visibility buffer pipeline and allows per-material cel vs. PBR mixing via a stencil bit.

### 13.12 UE5 Shader Definitions Headers

**Reference:** `example_repo/UnrealEngine-release/UnrealEngine-release/Engine/Shaders/Shared/`

**Copyright:** © Epic Games, Inc. All rights reserved. Unreal Engine EULA.

**Files studied:** `NaniteDefinitions.h`, `LightDefinitions.h`, `InstanceCullingDefinitions.h`, `FroxelDefinitions.h`, `IndirectVirtualTextureDefinitions.h`.

**Key constants and structs documented for future reference:**

| File | Key constant / struct | Value | Somnium relevance |
|---|---|---|---|
| `NaniteDefinitions.h` | `NANITE_MAX_CLUSTER_TRIANGLES` | 128 (7-bit) | Upper bound for Somnium cluster size in Phase 14 GPU-driven rendering |
| `NaniteDefinitions.h` | `NANITE_MAX_CLUSTER_VERTICES` | 256 (8-bit) | Phase 14 meshlet vertex budget |
| `NaniteDefinitions.h` | `NANITE_STREAMING_PAGE_GPU_SIZE` | 128 KiB (17-bit) | Streaming page budget reference for Phase 14 |
| `NaniteDefinitions.h` | `NANITE_MAX_CLUSTER_HIERARCHY_DEPTH` | 32 | DAG depth cap for LOD hierarchy |
| `LightDefinitions.h` | `LIGHT_TYPE_*` | 0–3 (Directional/Point/Spot/Rect) | Already matches Somnium's `LightType` enum ordering |
| `LightDefinitions.h` | `LIGHT_EXTRA_DATA` bit layout | type at bits 11–12, shadow at 13 | Pattern for packing light flags into a single `u32` in Phase 12 clustered lighting |
| `InstanceCullingDefinitions.h` | `INSTANCE_CULLING_PRESERVE_INSTANCE_ORDER_BIT_MASK` | `1U` | Phase 13 GPU culling: instance order preservation flag |
| `InstanceCullingDefinitions.h` | `INSTANCE_CULLING_DYNAMIC_INSTANCE_DATA_OFFSET_BIT_MASK` | `2U` | Dynamic vs. static instance data offset disambiguation |
| `FroxelDefinitions.h` | `FROXEL_TILE_SIZE` | 8 (8×8 screen tiles) | Phase 12 froxel-based clustered lighting tile granularity |
| `FroxelDefinitions.h` | `FPackedFroxel` | `{uint XY; int Z}` | Froxel coordinate packing: XY in screen tiles, Z as signed slice index |
| `FroxelDefinitions.h` | `FROXEL_INVALID_SLICE` | `1 << 28` | Sentinel for "no froxel" — reserved top 4 bits |
| `IndirectVirtualTextureDefinitions.h` | `FIndirectVirtualTextureUniform` | 48 bytes (3×`uint4` + `uint4×2` + `uint4`) | IVT uniform block layout reference for Phase 15 virtual texturing |
| `IndirectVirtualTextureDefinitions.h` | `FIndirectVirtualTextureEntry` | `uint2 PackedCoordinateAndSize` | Page table entry encoding: coordinates and size packed into 64 bits |

**Phase 12 froxel design note:** The 8×8 tile size and `FPackedFroxel` layout from UE5 will guide Somnium's clustered light list. A compute shader will bin point lights into froxels using depth-sliced frustum subdivision (Z exponential), exactly mirroring UE5's approach but implemented from scratch in WGSL.

---

### 13.13 Phase 12 Native UI — Fyrox generational pool

**Reference:** `example_repo/fyrox/Fyrox-master/fyrox-core/src/pool/handle.rs` and `mod.rs`

**Copyright:** Dmitry Stepanov and Fyrox Engine contributors. MIT License.

**Pattern (12A-1):** Generational arena allocator — `Handle<T>` (index: u32, generation: u32, PhantomData); `Pool<T>` (records: Vec, free_stack: Vec<u32>). `INVALID_GENERATION = 0`; `Handle::NONE` sentinel. `spawn_with`: reuses free slot (increments generation) or appends new record. `try_free`: validates generation then pushes to free_stack.

**Somnium port:** `crates/somnium_ui/src/pool.rs` — identical semantics, stripped of Fyrox's reflection/visitor/PayloadContainer machinery. Added `Handle::transmute<U>()` for bridging opaque `NodeHandle = Handle<UiNodeTag>` to `Pool<UiNode>` internal handles.

---

### 13.14 Phase 12 Native UI — Fyrox widget/message/draw architecture

**References:**
- `example_repo/fyrox/Fyrox-master/fyrox-ui/src/widget.rs` — `Widget` struct (layout fields, alignment, margin, desired_size, actual_size, clip_bounds, children, parent)
- `example_repo/fyrox/Fyrox-master/fyrox-ui/src/message.rs` — `UiMessage` (destination handle, direction, Box<dyn Any> data), `MessageDirection`
- `example_repo/fyrox/Fyrox-master/fyrox-ui/src/draw.rs` — `Vertex` (pos: Vec2, tex_coord: Vec2, color: Color), `Command` (clip_bounds, triangles range), `DrawingContext`
- `example_repo/fyrox/Fyrox-master/fyrox-ui/src/control.rs` — `Control` trait: `measure_override`, `arrange_override`, `draw`, `handle_routed_message`
- `example_repo/fyrox/Fyrox-master/fyrox-ui/src/lib.rs` — `measure_node()` / `arrange_node()` two-pass layout algorithm (margin subtraction, min/max clamp, alignment offset, `ceil()` rounding)

**Pattern (12A-2):** Widget = layout base (no behavior). UiNode = Widget + Box<dyn Control>. Two-pass layout: measure pass (bottom-up, desired size) → arrange pass (top-down, final bounds). DrawingContext accumulates vertex/index lists with per-command clip rects.

**Somnium ports:**
- `crates/somnium_ui/src/types.rs` — `Rect`, `Thickness`, `HorizontalAlignment`, `VerticalAlignment`
- `crates/somnium_ui/src/draw.rs` — `Vertex` (pos, uv, color: [u8;4]), `DrawCommand` (clip_rect, texture_id, index range), `DrawingContext` (push_rect_filled, push_rect_border, push_textured_rect, clip stack)
- `crates/somnium_ui/src/message.rs` — `UiMessage`, `MessageDirection`, `WidgetMessage`
- `crates/somnium_ui/src/widget.rs` — `Widget`, `WidgetBuilder`
- `crates/somnium_ui/src/node.rs` — `Control` trait, `UiNode`
- `crates/somnium_ui/src/ui.rs` — `UserInterface` (perform_layout, hit_test, send/poll message, draw)

---

### 13.15 Phase 12 Native UI — Fyrox widget library

**References:**
- `example_repo/fyrox/Fyrox-master/fyrox-ui/src/canvas.rs` — Canvas: infinite-space measure, absolute arrange at desired_local_position
- `example_repo/fyrox/Fyrox-master/fyrox-ui/src/stack_panel.rs` — StackPanel: Vertical/Horizontal orientation, sequential measure+arrange
- `example_repo/fyrox/Fyrox-master/fyrox-ui/src/border.rs` — Border: stroke-shrunk inner rect measure, per-side stroke draw
- `example_repo/fyrox/Fyrox-master/fyrox-ui/src/button.rs` — Button: MouseDown capture → MouseUp (in-bounds) → ButtonMessage::Click via `emit` Vec
- `example_repo/fyrox/Fyrox-master/fyrox-ui/src/grid.rs` — Grid: SizeMode::Strict/Auto/Stretch, 4-group measurement algorithm, arrange_dims

**Pattern (12A-3/12A-5):** Each widget is a plain struct implementing `Control`. The LayoutCtx's `row()`/`column()` accessors allow Grid to read child placement without holding pool borrows. Button emits click messages via a `&mut Vec<UiMessage>` parameter added to `handle_routed_message` (adapted from Fyrox's `ui.post()`). Grid's mutable measurement state (cells, groups, unmeasured counts) uses `RefCell` to satisfy `&self` Control constraints. `theme.rs` provides the UE5 dark palette as `[u8;4]` RGBA constants.

**Somnium ports:**
- `crates/somnium_ui/src/theme.rs` — UE5 dark editor palette constants (BG_DARK, ACCENT_BLUE, TEXT_PRIMARY, etc.)
- `crates/somnium_ui/src/widgets/canvas.rs` — Canvas + CanvasBuilder
- `crates/somnium_ui/src/widgets/stack_panel.rs` — StackPanel + StackPanelBuilder + Orientation enum
- `crates/somnium_ui/src/widgets/border.rs` — Border + BorderBuilder
- `crates/somnium_ui/src/widgets/button.rs` — Button + ButtonBuilder + ButtonMessage::Click
- `crates/somnium_ui/src/widgets/text.rs` — Text (placeholder until Phase 12A-4 font atlas)
- `crates/somnium_ui/src/widgets/grid.rs` — Grid + GridBuilder + GridDimension + SizeMode + Row/Column/Cell

---

### 13.16 Phase 12 Native UI — fontdue font atlas

**Reference:** `fontdue` crate v0.7 by Sven Niederberger (MIT License), pure-Rust TrueType rasterizer.  
`fontdue::Font::rasterize(char, px)` → `(Metrics, Vec<u8>)` — grayscale coverage bitmap.  
`fontdue::Font::metrics(char, px)` → `Metrics` — advance/bearing data without rasterizing.  
`fontdue::Font::horizontal_line_metrics(px)` → `LineMetrics` { ascent, descent } for baseline placement.

**Pattern (12A-4):** `FontAtlas` (512×512 RGBA8) uses shelf packing. Glyphs cached by `GlyphKey { codepoint, px_bits, font_id }`. Atlas RGB=255, A=coverage, so vertex color tints text without extra shader modes. `DrawingContext::push_text` places glyphs using freetype y-up → screen y-down: `glyph_top = baseline_y - (ymin + px_h)`. `LayoutCtx::measure_text` uses `Font::metrics` (no rasterization, layout-only path). `texture_id = Some(0)` convention reserved for the atlas; UiPass will bind it at slot 0.

**Somnium port:**
- `crates/somnium_ui/src/font.rs` — `FontAtlas`, `GlyphKey`, `GlyphInfo`, `FONT_ATLAS_TEXTURE_ID = 0`
- `crates/somnium_ui/src/draw.rs` — `DrawingContext::push_text`, `font_atlas: FontAtlas` field (persists across clear)
- `crates/somnium_ui/src/node.rs` — `LayoutCtx::measure_text` (atlas metrics path)
- `crates/somnium_ui/src/ui.rs` — `UserInterface::add_font(bytes) -> u8`
- `crates/somnium_ui/src/widgets/text.rs` — real `Text` widget (measure via atlas, draw via push_text)

### 13.17 Phase 12 Native UI — UiPass wgpu render pass

**Pattern (12B-1):** Original design — no direct reference codebase.  
Architecture follows the same wgpu 29 pass conventions established in `GridPass` and `GizmoPass`:
- `bind_group_layouts: &[Some(&bgl)]` + `immediate_size: 0` in `PipelineLayoutDescriptor`
- `multiview_mask: None`, `depth_slice: None`, `compilation_options: Default::default()`
- Vertex format: `Float32x2` pos @ 0, `Float32x2` uv @ 8, `Unorm8x4` color @ 16 (20 B stride)
- BG0 (VERTEX): 64-byte ortho uniform — `Mat4::orthographic_rh(0, W, H, 0, 0, 1)` (y-down)
- BG1 (FRAGMENT): two pre-built bind groups switched lazily per DrawCommand — white 1×1 for `texture_id=None`, font atlas for `texture_id=Some(0)`
- Alpha blend: `SrcAlpha / OneMinusSrcAlpha` for color, `One / OneMinusSrcAlpha` for alpha
- Per-command scissor rect clamped to surface bounds; zero-area commands skipped
- Buffer resize strategy: double capacity, create new buffer, old dropped automatically

**Somnium port:**
- `crates/somnium_ui/src/pass.rs` — `UiPass` struct, `new(device, queue, format)`, `prepare()`, `render()`
- `crates/somnium_ui/src/ui_pass.wgsl` — `vs_main` (ortho transform), `fs_main` (vertex_color × textureSample)

### 13.18 Phase 12 Native UI — ScrollViewer, TextBox, NumericField (12D-full)

**References:**
- `example_repo/fyrox/Fyrox-master/fyrox-ui/src/scroll_viewer.rs` — ScrollViewer: clipped viewport, vertical content offset, scrollbar interaction
- `example_repo/fyrox/Fyrox-master/fyrox-ui/src/text_box.rs` — TextBox: `KeyboardInput` routing, internal string buffer, caret, backspace/delete, `TextBoxMessage::Text`
- `example_repo/fyrox/Fyrox-master/fyrox-ui/src/numeric_field.rs` — NumericField: wraps TextBox with f32 value, increment/decrement, `NumericFieldMessage::Value(f32)`

**Pattern (12D-full):** All three widgets follow the `Control` trait port pattern established in 12A-3/12A-5. ScrollViewer clips its content to the widget bounds and translates the child viewport by a vertical scroll offset. TextBox routes `KeyboardInput` messages from the `UserInterface` focus system to maintain an editable string buffer, dispatching `TextBoxMessage::Text` on each change. NumericField adds a typed f32 layer on top of TextBox, parsing the string and emitting `NumericFieldMessage::Value(f32)`.

**Somnium ports:**
- `crates/somnium_ui/src/widgets/scroll_viewer.rs` — `ScrollViewer` + `ScrollViewerBuilder`
- `crates/somnium_ui/src/widgets/text_box.rs` — `TextBox` + `TextBoxBuilder` + `TextBoxMessage`
- `crates/somnium_ui/src/widgets/numeric_field.rs` — `NumericField` + `NumericFieldBuilder` + `NumericFieldMessage`

---

### 13.19 Ocean PBR Textures (Phase 13)

**Reference:** User-provided assets (`assets/ocean_pbr/`).
**Summary:** Standard Unreal Engine layout (BaseColor, Normal_DX, ORM: AO/Roughness/Metallic). Used for physically based rendering of the water surface. Mipmaps manually generated on the CPU with custom edge wrapping to prevent bleeding.

### 13.20 Heightmap Terrain — Fyrox terrain + CDLOD + triplanar splatting (Phase 14 SSS)

**References:**

| Source | License | Files studied |
|---|---|---|
| Fyrox — `example_repo/fyrox/Fyrox-master/fyrox-impl/src/scene/terrain/` | MIT | `mod.rs` (Terrain/Chunk structs, height accessors, `raycast`, brush stroke flow, `collect_render_data` LOD ranges), `geometry.rs` (grid mesh + quadrant ranges), `quadtree.rs` (distance-based LOD selection), `brushstroke/mod.rs` + `brushstroke/brushraster.rs` (stamp/smear flow, radial strength + hardness remap) |
| CDLOD — `example_repo/CDLOD-master` (Filip Strugar) | (paper + source) | Power-of-two LOD subdivision; log2 distance → LOD mapping |
| bevy_triplanar_splatting — `example_repo/bevy-plugins/bevy_triplanar_splatting-main/` | MIT/Apache-2.0 | Array-texture splat material layout, triplanar weight blending (`pow(abs(n), k)` normalization) |
| bevy_terrain — `example_repo/bevy-plugins/bevy_terrain-main/`; terra — `example_repo/terra-main/` | MIT/Apache-2.0 | Surveyed for architecture comparison (chunked clipmap / planetary CDLOD); not ported |

**Key adaptations (no code copied):**

| Reference pattern | Somnium adaptation |
|---|---|
| Fyrox: per-chunk R32F heightmap **texture**, GPU vertex displacement of a shared grid mesh, quadtree instance selection | CPU-side vertex generation per chunk (heights baked into `Vertex` buffers); flat per-chunk LOD (log2 distance, CDLOD-style) with a ≤1-level neighbor constraint instead of per-chunk quadtrees — fits the engine's explicit draw-call model |
| Fyrox: pre-built index topology per LOD; quadrant `ElementRange`s | Shared index buffers cached per `(lod, edge_mask)`; **block-fan stitching** (original scheme): 2×2-cell fans whose border midpoints are omitted toward coarser neighbors — watertight T-junction-free borders |
| Fyrox: per-layer material × per-chunk mask textures, one draw per layer per node | Single terrain-global RGBA splatmap (4 layers, one channel each) + `texture_2d_array` PBR layers — one draw per chunk total |
| Fyrox `brushraster.rs`: strength `1 − d/r`, hardness remap `s < 1-h ? s/(1-h) : 1` | Ported directly in `terrain/brush.rs::brush_falloff` |
| Fyrox `Terrain::raycast`: per-cell 2D march + triangle test | Ray march at half-cell steps against bilinear `world_height_at`, then 16-step bisection refine |
| Fyrox editor: brush stroke start/stamp/end + undo via texture snapshot | Stroke snapshot at LMB-down, region accumulation, `TerrainEditCmd` with a restore-op queue (commands lack renderer access) |
| bevy_triplanar_splatting WGSL | Triplanar cliff projection on steep slopes in `terrain.wgsl` (rock layer, `smoothstep(0.45, 0.7, 1 − |n.y|)` blend) |
| Frostbite-style height blending (procedural-shader-splatting talk) | `height_blend()` in `terrain.wgsl`: albedo alpha carries procedural height; materials "poke through" at transitions |

The four procedural layer textures (value-noise grass/dirt/rock/snow) and the in-shader brush cursor ring are original code.

### 13.21 Light Gizmos — Bevy light gizmo shapes (Phase 13E)

**Reference:** `example_repo/bevy/bevy-main/crates/bevy_light/src/gizmos.rs`
(plus `examples/gizmos/light_gizmos.rs`).

**Copyright:** Bevy contributors (MIT/Apache-2.0 dual-licensed).

**Patterns studied:**

| Bevy pattern | Reference detail | Somnium adaptation |
|---|---|---|
| Point light gizmo | `point_light_gizmo()` — sphere at `range`, plus a small sphere at the light `radius` | Wire sphere at `range` built from 3 great circles. Somnium's `LightComponent` has no `radius` field, so the inner sphere is replaced by a small axis cross marking the origin |
| Spot light gizmo | `spot_light_gizmo()` — a cone per angle with `height = range * cos(angle)`, `radius = range * sin(angle)`, offset from the light along its direction, plus two 3-D arcs across the cap | Same cone sizing (so the rim lies exactly on the range sphere — asserted by a unit test). Cap arcs replaced by 4 apex→rim spokes: cheaper and reads the same at editor scale. Inner cone dimmed to distinguish it from the outer |
| Directional light gizmo | `directional_light_gizmo()` — a single arrow along `rotation * Vec3::NEG_Z` | Same direction convention and arrow, plus 4 parallel offset rays so the light reads as directional at a glance |
| `LightGizmoColor::MatchLightColor` (default) | Gizmo takes the color of the light it represents | Same, with the light color normalized to full brightness so dim/HDR colors stay legible |
| `draw_all` config + `ShowLightGizmo` marker component | Opt-in per light, or all lights via config | All lights draw; the **selected** light is full brightness and the rest are dimmed to 45%, which suits a single-viewport editor. `L` toggles the whole overlay |

**Architecture note:** Bevy has a retained immediate-mode gizmo system with generic
3-D primitives (`bevy_gizmos::primitives::dim3`). Somnium has no such layer, so
`LightGizmoPass` emits world-space line segments on the CPU into one growable
vertex buffer and issues a single `LineList` draw for every light — no per-light
draw call and no model matrix (unlike the transform `GizmoPass`, whose geometry
is static and placed by a uniform).

**Files:** `somnium_renderer/src/pass/light_gizmo.rs`,
`somnium_renderer/src/shaders/light_gizmo.wgsl`,
`somnium_core/src/app.rs` (`submit_light_gizmos`).

### 13.22 FXAA — Timothy Lottes / NVIDIA (Phase 15A2)

**Reference:** Timothy Lottes, *FXAA 3.11* (NVIDIA, 2011) — the widely published
`Fxaa3_11.h` console-quality preset. Studied from the published algorithm; no
source was copied.

**Algorithm used:** luma (Rec. 601) at the centre pixel plus the four diagonal
neighbours → local min/max contrast test to skip flat areas → edge direction
from the diagonal luma differences → a 2-tap inner average and a 4-tap wider
average along that direction, falling back to the inner average when the wider
blur pushes luma outside the neighbourhood range.

**Somnium adaptations:**

| Reference behaviour | Somnium |
|---|---|
| `FxaaTex` sampling with implicit derivatives | `textureSampleLevel(..., 0.0)`. The contrast early-out is a data-dependent branch, and WGSL forbids implicit-derivative sampling in non-uniform control flow. The source has no mips, so explicit LOD 0 is equivalent. |
| Tuning via preprocessor `FXAA_QUALITY__*` presets | `EDGE_THRESHOLD` / `EDGE_THRESHOLD_MIN` constants at the reference defaults (0.125 / 0.0312), with `inv_size` supplied per-frame in a uniform. |
| Typically the last pass before present | Runs before the gizmo / outline / particle / UI passes, so editor overlays and text are not smeared. |
| Operates on gamma-space luma | Runs on an sRGB target, so sampling returns linear values and luma is computed in linear space. Slightly less perceptually tuned, but edge detection is unaffected in practice. |

**Files:** `somnium_renderer/src/pass/fxaa.rs`, `somnium_renderer/src/shaders/fxaa.wgsl`

### 13.25 Bundled foliage assets — Poly Haven (Phase 17E)

**These are third-party art assets shipped in the repository**, unlike every
other entry here, which records a technique the code was informed by.

| Asset | Path | Authors |
|---|---|---|
| Grass Medium 01 | `assets/foliage/grass_medium_01/` | Rob Tuytel (photography), Rico Cilliers (modeling) |
| Grass Bermuda 01 | `assets/foliage/grass_bermuda_01/` | Rico Cilliers |
| Fir Sapling | `assets/foliage/fir_sapling/` | Rob Tuytel (photography), Rico Cilliers (modeling) |
| Island Tree 02 | `assets/foliage/island_tree_02/` | Rob Tuytel (scanning), Rico Cilliers (cleanup) |

**License: CC0 1.0** (public domain dedication), from <https://polyhaven.com>.
Poly Haven's license page states the assets may be redistributed, used
commercially, and included in products that are sold, and that **attribution is
not required**. It is given here anyway, because the work deserves it.

**Why these four:** the 2k texture variants, chosen against the alternatives on
measured size and triangle count rather than by eye. `fir_tree_01` is 486 MB and
7 million triangles and `pine_tree_01` is 937 MB and 17 million — unreasonable
in a git repository, and larger than the engine's entire geometry pool was
before Phase 17E raised it. The four shipped here total about 101 MB and
1.53 million triangles, with no single file above 39 MB.

**Re-fetching:** every file comes from the Poly Haven file API
(`https://api.polyhaven.com/files/<asset>`), which lists each format and
resolution with an `include` map of relative-path to URL. The textures do not
sit beside the glTF and the `.bin` is shared from the 8k tree, so that map has
to be followed rather than the directory layout guessed at.

The `*_alpha_*.png` cutout masks alongside each model's textures come from the same
Poly Haven asset pages and are covered by the same CC0 dedication. They are fetched
separately because Poly Haven's glTF exports do not reference them.

### 13.24 Water surface — Fresnel reflection + Beer-Lambert absorption (Phase 22)

**References:**

| Source | Topic |
|---|---|
| Christophe Schlick, *An Inexpensive BRDF Model for Physically-based Rendering* (1994) | Fresnel approximation, used twice: half-vector for the sun lobe, view-vector for the environment reflection |
| Bruce Walter et al., *Microfacet Models for Refraction through Rough Surfaces* (EGSR 2007) | GGX normal distribution and the Smith geometry term |
| Beer-Lambert law | Per-channel absorption through the water column |
| Tessendorf, *Simulating Ocean Water* (SIGGRAPH course notes) | Deep water reads as reflected sky plus subsurface scattering, not as a surface albedo |

**Somnium adaptations:**

| Reference | Somnium |
|---|---|
| Planar-reflection or SSR pass for the reflected term | Reuses the Phase 19 prefiltered environment cubemap — no extra pass, and roughness maps straight onto the mip chain |
| Absorption applied as a single scalar | Per-channel `exp(-d * clarity * ABSORPTION)` with red absorbed ~10x faster than blue, which is what produces the blue-green cast rather than a painted-on blue |
| Refraction of an offscreen "under-water" render | Samples a copy of the HDR target taken immediately before the water pass, offset by the surface normal's horizontal component |
| Infinite ocean assumed | The depth buffer clears to 1.0, so a far-plane reading is treated explicitly as "no backdrop": zero transmission, all scattering. Without that test the sky leaks through the blue channel and open ocean reads as a swimming pool |

**Files:** `somnium_renderer/src/shaders/water.wgsl`, `somnium_renderer/src/pass/water.rs`

### 13.23 Image-Based Lighting — Karis split-sum (Phase 19)

**References:**

| Source | Topic |
|---|---|
| Brian Karis, *Real Shading in Unreal Engine 4* (SIGGRAPH 2013) | Split-sum approximation: prefiltered environment map (mip = roughness) × a BRDF integration term |
| Dimitar Lazarov, *Physically Based Lighting in Call of Duty: Black Ops* (SIGGRAPH 2011) | Analytic fit to the environment BRDF term, used in place of a 2-D LUT |
| Hammersley / Van der Corput sequence | Low-discrepancy sampling for the GGX prefilter |

**Somnium adaptations:**

| Reference | Somnium |
|---|---|
| Environment captured from an HDRI asset | Captured from the engine's **own procedural sky**, so reflections always match the drawn background, need no asset, and stay correct when the sun moves |
| BRDF integration stored in a 2-D LUT texture | Lazarov's analytic approximation in `env_brdf_approx` — no LUT to generate, store, or bind |
| Separate cosine-convolved irradiance cubemap for diffuse | The roughest prefiltered mip stands in for irradiance. Not a true cosine convolution; a visually close shortcut that avoids a second prefilter chain. Documented as a candidate for improvement |
| Prefilter typically as a compute pass | Render pass per (face, mip). One submission per face because `queue.write_buffer` lands once per submission — 36 tiny submissions, and it only runs when the sun changes |

**Files:** `somnium_renderer/src/pass/ibl.rs`, `somnium_renderer/src/shaders/ibl_gen.wgsl`,
`somnium_renderer/src/shaders/shading.wgsl` (`evaluate_ibl`)

## 14. Pattern Index

Cross-reference: which Somnium file implements which reference pattern.

| Somnium File | Reference Patterns |
|---|---|
| `somnium_core/src/app.rs` | UE5 `FEngineLoop` lifecycle, UE5 `GenericApplication` focus routing |
| `somnium_core/src/event.rs` | UE5 `GenericApplicationMessageHandler` typed event dispatch |
| `somnium_core/src/context.rs` | UE5 `FWorldContext` zero-copy context bundle |
| `somnium_core/src/time.rs` | Original; hybrid sleep/spin framerate limiting |
| `somnium_ecs/src/archetype.rs` | UE5 MassEntity `FMassArchetypeData`, `FMassArchetypeChunkIterator` |
| `somnium_ecs/src/world.rs` | UE5 MassEntity `FMassEntityManager`, bgfx `StateCacheLru` concept |
| `somnium_renderer/src/renderer.rs` | Forge `IApp::Draw`, bgfx stateless submit + sort |
| `somnium_renderer/src/bindless.rs` | O3DE Atom RHI bindless descriptor pool |
| `somnium_renderer/src/command.rs` | bgfx `SortKey`, stateless draw submission |
| `somnium_renderer/src/pass/visibility.rs` | Forge `IVisibilityBuffer`, DXC `SV_PrimitiveID` |
| `somnium_renderer/src/pass/shading.rs` | Forge visibility shading pass architecture |
| `somnium_renderer/src/material/hlms.rs` | Ogre-Next HLMS permutation cache |
| `somnium_renderer/src/shaders/visibility.wgsl` | Forge programmable vertex pulling, DXC semantic mapping |
| `somnium_renderer/src/shaders/shading.wgsl` | Forge barycentric reconstruction, procedural sky original |
| `somnium_renderer/src/shaders/brdf.wgsl` | SpartanEngine Cook-Torrance BRDF (ported to WGSL) |
| `somnium_renderer/src/pass/particle.rs` | bevy_enoki billboard instancing, 6-vertex draw per instance, storage buffer vertex pull |
| `somnium_renderer/src/shaders/particle.wgsl` | bevy_enoki camera-right/up billboard expansion, smooth radial alpha |
| `somnium_renderer/src/pass/outline.rs` | bevy_mod_outline two-subpass stencil; clip-space normal extrusion; storage-buffer vertex pulling |
| `somnium_renderer/src/shaders/outline.wgsl` | bevy_mod_outline clip-space extrusion math (perspective-correct `normalize(xy) * w`) |
| `somnium_ui/src/lib.rs` | Unity uGUI Canvas concept; native `UiManager` wraps `UserInterface` + `UiPass` (Phase 12C — wry removed) |
| `examples/hello_engine/src/main.rs` | UE5 editor camera (fly-cam bindings) |
| `somnium_renderer/src/shadow/mod.rs` | `GpuDirectionalLight` struct layout, CSM atlas design (Phase 11) |
| `somnium_renderer/src/shadow/cascade.rs` | GPU Gems 3 PSS splits, bounding sphere VP, texel snapping, `ortho_rh_zo` (Phase 11) |
| `somnium_renderer/src/pass/shadow.rs` | Depth-only cascade shadow pass, 4 viewport iterations (Phase 11) |
| `somnium_renderer/src/shaders/shadow.wgsl` | Cascade index uniform, programmable vertex pulling for depth-only (Phase 11) |
| `somnium_core/src/lib.rs` | `LightComponent` + `LightType` ECS components (Phase 11); `WorldTransform`, `Parent`, `Children` (Phase 11.5A) |
| `somnium_renderer/src/pass/gizmo.rs` | Transform gizmo geometry (arrows/rings/cubes), AABB picking, GizmoPass render pipeline |
| `somnium_renderer/src/shaders/gizmo.wgsl` | Per-vertex-color unlit shader, no depth test, screen-size-constant model matrix |
| `somnium_renderer/src/pass/grid.rs` | Grid overlay pass, alpha-blended into HDR target |
| `somnium_renderer/src/shaders/grid.wgsl` | Ray-XZ-plane intersection, fwidth() AA, distance fade, axis highlights |
| `somnium_renderer/src/pass/postprocess.rs` | HDR render target management, `HDR_FORMAT` constant, resize, ACES + vignette pipeline |
| `somnium_renderer/src/shaders/postprocess.wgsl` | ACES filmic tone mapping, radial vignette, full-screen triangle UV |
| `somnium_core/src/app.rs` | Gizmo drag state machine, ray picking math (`ndc_to_world`, `ray_axis_param`, `ring_angle`) |
| `somnium_core/src/editor_commands.rs` | `SetTransformCmd`, `SetNameCmd`, `SetLightCmd`, `ReparentCmd`, `DeleteEntityCmd`, `CreateEntityCmd`, `UndoStack` |
| `somnium_core/src/log_capture.rs` | Fyrox `LogSettings` ring-buffer concept — `tracing_subscriber::Layer` capture, `mpsc::channel` IPC forwarding (Phase 11.5M) |
| `somnium_ui/src/pool.rs` | Fyrox `fyrox-core/src/pool/{handle.rs,mod.rs}` — generational arena, Handle<T> transmute bridging (Phase 12A-1) |
| `somnium_ui/src/types.rs` | Fyrox `fyrox-ui/src/{alignment.rs,thickness.rs}` — Rect, Thickness, HorizontalAlignment, VerticalAlignment (Phase 12A-2) |
| `somnium_ui/src/draw.rs` | Fyrox `fyrox-ui/src/draw.rs` — Vertex, DrawCommand, DrawingContext with clip rect stack (Phase 12A-2) |
| `somnium_ui/src/message.rs` | Fyrox `fyrox-ui/src/message.rs` — UiMessage, MessageDirection, WidgetMessage (Phase 12A-2) |
| `somnium_ui/src/widget.rs` | Fyrox `fyrox-ui/src/widget.rs` — Widget layout fields, WidgetBuilder (Phase 12A-2) |
| `somnium_ui/src/node.rs` | Fyrox `fyrox-ui/src/control.rs` — Control trait (measure/arrange/draw/handle_message), UiNode (Phase 12A-2) |
| `somnium_ui/src/ui.rs` | Fyrox `fyrox-ui/src/lib.rs` — UserInterface: two-pass layout, hit-test, message queue, draw (Phase 12A-2) |
| `somnium_ui/src/theme.rs` | UE5 editor color palette §1.4 — dark theme constants for Phase 12D native editor (Phase 12A-3/12A-5) |
| `somnium_ui/src/widgets/canvas.rs` | Fyrox `fyrox-ui/src/canvas.rs` — Canvas absolute positioning (Phase 12A-3/12A-5) |
| `somnium_ui/src/widgets/stack_panel.rs` | Fyrox `fyrox-ui/src/stack_panel.rs` — StackPanel linear layout (Phase 12A-3/12A-5) |
| `somnium_ui/src/widgets/border.rs` | Fyrox `fyrox-ui/src/border.rs` — Border stroke+background (Phase 12A-3/12A-5) |
| `somnium_ui/src/widgets/button.rs` | Fyrox `fyrox-ui/src/button.rs` — Button click emission (Phase 12A-3/12A-5) |
| `somnium_ui/src/font.rs` | fontdue 0.7 glyph atlas — shelf packing, GlyphKey hash, freetype baseline placement (Phase 12A-4) |
| `somnium_ui/src/widgets/text.rs` | fontdue font metrics (measure) + atlas glyph quads (draw) (Phase 12A-4) |
| `somnium_ui/src/widgets/grid.rs` | Fyrox `fyrox-ui/src/grid.rs` — Grid 4-group SizeMode layout (Phase 12A-3/12A-5) |
| `somnium_ui/src/pass.rs` | Original wgpu 29 UI render pass — ortho uniform, dual BG1 variants, lazy bind group switch, doubling buffer resize (Phase 12B-1) |
| `somnium_ui/src/ui_pass.wgsl` | Original WGSL — ortho VS, vertex_color × textureSample FS (Phase 12B-1) |
| `somnium_ui/src/widgets/scroll_viewer.rs` | Fyrox `fyrox-ui/src/scroll_viewer.rs` — clipped scroll viewport, vertical content offset (Phase 12D-full) |
| `somnium_ui/src/widgets/text_box.rs` | Fyrox `fyrox-ui/src/text_box.rs` — keyboard text input, caret, TextBoxMessage (Phase 12D-full) |
| `somnium_ui/src/widgets/numeric_field.rs` | Fyrox `fyrox-ui/src/numeric_field.rs` — f32 numeric input, NumericFieldMessage::Value (Phase 12D-full) |
| `somnium_voxel/src/chunk.rs` | bevy_voxel_world `chunk.rs` — 32³ chunks padded to 34³ for cross-chunk face culling (Phase 14) |
| `somnium_voxel/src/mesh.rs` | bevy_voxel_world `meshing.rs` — `visible_block_faces` + `RIGHT_HANDED_Y_UP_CONFIG`, border-aligned nearest-neighbour LOD resample (Phase 14) |
| `somnium_voxel/src/world.rs` | bevy_voxel_world `chunk.rs::ChunkThread` async task pattern + `NeedsRemesh`/`NeedsDespawn` markers → rayon + mpsc + version-guarded dirty flags (Phase 14) |
| `somnium_voxel/src/terrain.rs` | Original — hash-based FBM value noise heightmap (Phase 14) |
| `somnium_renderer/src/geometry.rs` | Original — bump allocator (Phase 7) + first-fit free-list for dynamic chunk meshes (Phase 14) |
| `somnium_renderer/src/terrain/mod.rs` | Fyrox `terrain/mod.rs` — chunked heightmap, height accessors, raycast; CDLOD log2 LOD selection (Phase 14 SSS) |
| `somnium_renderer/src/terrain/mesh.rs` | Fyrox `terrain/geometry.rs` grid emission + original block-fan T-junction stitching (Phase 14 SSS) |
| `somnium_renderer/src/terrain/brush.rs` | Fyrox `brushstroke/brushraster.rs` falloff + hardness remap; stamp flow from `brushstroke/mod.rs` (Phase 14 SSS) |
| `somnium_renderer/src/terrain/textures.rs` | Original procedural PBR layers; array-texture layout from bevy_triplanar_splatting (Phase 14 SSS) |
| `somnium_renderer/src/pass/terrain.rs` | WaterPass integration pattern (HDR + vis-depth); own pipeline per Phase 14 SSS plan |
| `somnium_renderer/src/shaders/terrain.wgsl` | bevy_triplanar_splatting triplanar blend; shadow/cluster helpers mirror `shading.wgsl`; height blend + brush ring original (Phase 14 SSS) |
| `somnium_core/src/editor_commands.rs` (TerrainEditCmd) | Fyrox editor brush-stroke undo concept → region snapshot + restore-op queue (Phase 14 SSS) |
| `somnium_core/src/app.rs` (terrain editing) | Fyrox `editor/src/interaction/terrain.rs` interaction model — mode toggle, cursor raycast, stroke lifecycle (Phase 14 SSS) |
| `somnium_renderer/src/pass/light_gizmo.rs` | Bevy `bevy_light/src/gizmos.rs` — per-light-type gizmo shapes and cone sizing; batched LineList emission is original (Phase 13E) |
| `somnium_renderer/src/pass/fxaa.rs` | FXAA 3.11 (Lottes/NVIDIA) — LDR intermediate target + resolve pass (Phase 15A2) |
| `somnium_renderer/src/shaders/fxaa.wgsl` | FXAA 3.11 edge detect + directional blur, adapted to `textureSampleLevel` for WGSL uniformity (Phase 15A2) |
| `somnium_renderer/src/pass/ibl.rs`, `shaders/ibl_gen.wgsl` | Karis split-sum prefiltered environment map; Hammersley/GGX importance sampling (Phase 19) |
| `somnium_renderer/src/shaders/shading.wgsl` (`evaluate_ibl`) | Karis split-sum IBL + Lazarov analytic env-BRDF fit (Phase 19) |
| `somnium_renderer/src/culling.rs` | Gribb–Hartmann frustum-plane extraction (near = `row2` for wgpu `z ∈ [0,1]`); UE5 `InstanceCullingDefinitions.h` flag-in-place shape (Phase 15B) |
| `somnium_renderer/src/pass/cull.rs`, `shaders/cull.wgsl` | UE5 instance-culling pass — verdict written as each draw's `instance_count` (Phase 15B) |
| `somnium_renderer/src/indirect.rs` | UE5 `InstanceCullingDefinitions.h` — GPU-resident draw args, `instance_count` as the cull flag (Phase 15A) |
| `somnium_renderer/src/shaders/light_gizmo.wgsl` | Original — world-space unlit line shader (no model matrix), mirrors `gizmo.wgsl` (Phase 13E) |

---

## 15. Citation Rules

1. **All reference code lives in `example_repo/`**, which is excluded from the Cargo workspace (`exclude = ["example_repo"]` in root `Cargo.toml`). It is never compiled.

2. **Every Somnium crate** that adopts a reference pattern documents it in a `## Reference Architecture` block in its `lib.rs` `//!` doc comment, naming the specific file path within `example_repo/`.

3. **Shader code** is original WGSL. Where a mathematical formulation is derived from a reference (e.g., the BRDF functions), the source is cited in a `// Ported/Inspired by` comment at the top of the shader file.

4. **No binaries, shader bytecode, or compiled artifacts** from any reference project are included in any Somnium build output.

5. **The `example_repo/` directory must never be added to the Cargo workspace** — not even as a `[patch]` or local dependency. Its presence is documentation only.

---

### 13.26 Advanced lighting references (Phase 24, planned)

Phase 24 is planned against published papers rather than any engine's source. Listed
here up front so the implementation cites the technique it actually follows:

| Technique | Reference |
|---|---|
| Sky / atmosphere | Hillaire, *A Scalable and Production Ready Sky and Atmosphere Rendering Technique* (EGSR 2020) |
| Tonemapping | Troy Sobotka, AgX; Narkowicz / Hill, ACES filmic approximations |
| Physical camera & exposure | Lagarde & de Rousiers, *Moving Frostbite to PBR* (SIGGRAPH 2014) |
| Ambient occlusion | Jimenez et al., *Practical Real-Time Strategies for Accurate Indirect Occlusion* (GTAO, SIGGRAPH 2016) |
| Specular occlusion | Lagarde & de Rousiers, as above — already used in Phase 17I |
| Soft shadows | Fernando, *Percentage-Closer Soft Shadows* (NVIDIA, 2005) |
| Specular anti-aliasing | Toksvig normal-variance; Kaplanyan et al., *Filtering Distributions of Normals* |
| Temporal AA | Karis, *High Quality Temporal Supersampling* (SIGGRAPH 2014) |
| Distance-field tracing | Wright et al., *Dynamic Occlusion with Signed Distance Fields* (SIGGRAPH 2015) |
| Volumetric fog | Hillaire, *Physically Based and Unified Volumetric Rendering in Frostbite* (SIGGRAPH 2015) |
| Split-sum IBL | Karis, *Real Shading in Unreal Engine 4* — already used in Phase 19 |

#### On studying the Unreal Engine 5 source

Lumen's architecture was studied by reading the shader sources in a local UE 5.6
install (`Engine/Shaders/Private/Lumen`, 50 `.usf` files) to understand how the stages
fit together: scene representation → surface cache → screen probes → world radiance
cache → reflections, and the trace/filter/temporal split repeated at each stage. That
structural understanding informs the ordering of Phase 24 sub-phases in `context.md` §22.

**No Unreal Engine code is copied, adapted, or translated into Somnium.** The UE source
is licensed under the Unreal Engine EULA, which is incompatible with this repository's
licence, and it is not vendored here — it lives only in the Epic Games install outside
the repo. Where UE files are named in `context.md`, they are named as *reading
references for the reader*, in the same spirit as the `example_repo/` rules below. Every
technique Somnium actually implements is written from the published paper in the table
above.

---

### 13.27 Bevy — the primary Phase 24 reference (MIT / Apache-2.0)

`example_repo/bevy/bevy-main/` is the most directly useful reference in the tree,
because it is the same stack Somnium is built on: Rust, wgpu, WGSL. Bevy is dual
licensed **MIT / Apache-2.0**, which is compatible with this repository — so unlike the
Unreal sources it may be read, learned from, and **adapted with attribution**.

Modules that Phase 24 draws on:

| Somnium sub-phase | Bevy source |
|---|---|
| 24A physical light units | `bevy_light/src/{directional_light,point_light,rect_light}.rs` |
| 24C Hillaire atmosphere | `bevy_pbr/src/atmosphere/` (`bruneton_functions.wgsl`, `sky_view_lut.wgsl`, `aerial_view_lut.wgsl`) |
| 24F temporal AA | `bevy_anti_alias/src/taa/` |
| 24G blue noise | `bevy_pbr/src/bluenoise/` |
| 24H contact shadows | `bevy_pbr/src/contact_shadows.rs` |
| 24I GTAO | `bevy_pbr/src/ssao/` |
| 24J acceleration structures | `bevy_solari/src/scene/{blas.rs,binder.rs}` |
| 24K ReSTIR DI | `bevy_solari/src/realtime/{restir_di.wgsl,presample_light_tiles.wgsl}` |
| 24L ReSTIR GI | `bevy_solari/src/realtime/restir_gi.wgsl` |
| 24M world radiance cache | `bevy_solari/src/realtime/world_cache_*.wgsl` |
| 24N specular GI / SSR | `bevy_solari/src/realtime/specular_gi.wgsl`, `bevy_pbr/src/ssr/` |
| 24O reference path tracer | `bevy_solari/src/pathtracer/` |
| 24Q light probes | `bevy_pbr/src/light_probe/` |
| 24R area lights (LTC) | `bevy_pbr/src/ltc/` |
| 24S transmission / SSS | `bevy_pbr/src/transmission/`, `medium.rs` |
| 24U volumetric fog | `bevy_pbr/src/volumetric_fog/` |

Rules for using it, consistent with the `example_repo/` policy at the end of this file:

1. Any WGSL or Rust **derived** from Bevy carries a comment naming the source file and
   Bevy's licence at the point of use — not merely a line in this table.
2. Where a published paper exists (Hillaire's atmosphere, Jimenez's GTAO, Bitterli's
   ReSTIR, Heitz's LTC), that paper is the primary reference and Bevy is read as a
   worked example of applying it on wgpu. The citation goes to the paper.
3. Bevy is **not** added to the Cargo workspace, and no Bevy crate becomes a dependency.
   Somnium stays a from-scratch engine; this is a reading reference.

Bevy having shipped ReSTIR GI and a Hillaire atmosphere on wgpu is also the strongest
available evidence that Phase 24 is achievable on this API rather than requiring raw
Vulkan or D3D12 — which is why the plan targets hardware ray tracing first rather than
treating it as a stretch goal.

---

### 13.28 Physical light units, exposure and AgX (Phase 24A / 24B)

| Piece | Reference |
|---|---|
| Photometric light units, EV100, the 1.2 exposure constant | Lagarde & de Rousiers, *Moving Frostbite to PBR* (SIGGRAPH 2014); Filament's documented camera model |
| Lux / lumen preset tables | Standard photometric references, cross-checked against `bevy_light`'s `light_consts` (MIT / Apache-2.0) |
| Histogram auto-exposure | Standard log-luminance histogram + weighted reduction; adaptation rate expressed per second |
| AgX | Troy Sobotka, [AgX](https://github.com/sobotka/AgX). The analytic inset/outset matrices and the sixth-order contrast fit are the widely reproduced minimal formulation of it. |
| ACES fit (retained as an option) | Narkowicz 2015 — already used since Phase 11.5K |

`light_units.rs` is written from the papers above, not transcribed from any engine.
Bevy was read as a worked example of applying the same model on wgpu, per the rules in
§13.27; its preset *values* are physical constants rather than authored content.

---

### 13.29 Atmospheric scattering (Phase 24C / 24D)

| Piece | Reference |
|---|---|
| Sky model, multiple-scattering approximation, analytic segment integration | Hillaire, *A Scalable and Production Ready Sky and Atmosphere Rendering Technique* (EGSR 2020) |
| Transmittance LUT parameterisation, ray-sphere helpers | Bruneton & Neyret, *Precomputed Atmospheric Scattering* (EGSR 2008), and Bruneton's 2017 revision |
| Rayleigh / Mie / ozone coefficients | The values published with Hillaire's paper, which are the standard Earth fit |
| LUT resolutions | Chosen to match `bevy_pbr::atmosphere` (MIT / Apache-2.0), which is also a wgpu implementation |

Written from the papers. Bevy's `bruneton_functions.wgsl` was read to confirm the exact
form of the transmittance mapping — the easiest part of the model to get subtly wrong —
and its LUT sizes were adopted; both are covered by §13.27's terms.
