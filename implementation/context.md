# Somnium Engine: Project Context & Implementation Summary

This document provides a comprehensive overview of the Somnium Engine project as of **Phase 7**, detailing the architecture, implemented features, and technical foundations.

## Project Vision
A high-performance, data-oriented Rust game engine leveraging modern rendering techniques (**Visibility Buffers**, **Bindless**, **Mega-Buffers**) and industry-standard native physics (**JoltPhysics**).

---

## 1. Architectural Overview

The project is structured as a Cargo workspace with a focus on modularity and high-performance interop, drawing inspiration from *Unreal Engine 5*, *The Forge*, and *O3DE*.

### Core Crates:
- **`somnium_core`**: The application lifecycle manager. Handles windowing (`winit`), frame timing, orchestrates subsystems, and defines core components like `Transform`, `MeshComponent`, and `MaterialComponent`.
- **`somnium_ecs`**: An archetype-based Entity Component System. Supports cache-friendly SoA (Struct-of-Arrays) storage, type-safe queries, and mass-entity manipulation.
- **`somnium_renderer`**: A `wgpu`-based rendering backend. Implements a **Visibility Buffer** pipeline with:
    - **GeometryPool**: A "Mega-Buffer" allocator for global vertex/index storage.
    - **InstancePool**: Storage-buffer based instance data (transforms, offsets).
    - **MaterialPool**: Global bindless material storage for PBR properties.
    - **Programmable Vertex Pulling**: WGSL shaders that fetch geometry directly from storage buffers, bypassing fixed-function input assembly.
- **`somnium_asset`**: Asset management pipeline. Handles GLTF loading, mesh processing, and asynchronous resource management.
- **`somnium_physics_sys`**: Low-level FFI bindings to the C++ **JoltPhysics** engine. Compiles ~150 Jolt source files via `cc` and a custom C-bridge (`jolt_bridge.cpp`).
- **`somnium_physics`**: Safe Rust wrapper over the Jolt FFI. Provides `PhysicsWorld`, `RigidBody`, and automated ECS synchronization.
- **`somnium_audio`**: High-level audio management using **Kira**, supporting spatial audio and dynamic mixing.

---

## 2. Completed Milestones

### Phase 1-3: Core & ECS
- Built the archetype ECS with efficient SoA storage and query support.
- Established the `GameApp` trait and `Engine` application loop.
- Implemented robust frame-timing, logging, and `egui` integration.

### Phase 4-6: Rendering, Physics & Audio
- **Visibility Buffer Pipeline**: Implemented a two-pass renderer (Visibility + Shading) to minimize overdraw and state changes.
- **Jolt Physics**: Integrated the full Jolt engine with safe Rust abstractions and automatic transform synchronization.
- **Spatial Audio**: Integrated Kira for high-performance sound playback.
- **UI Integration**: Integrated `egui` for the engine dashboard and debug overlays.

- [x] Phase 7: Mesh & Material System (Visibility Buffer, Bindless)
- [x] Phase 8: Advanced Shading & Asset Pipeline (PBR, GLTF, Shadows)
- [/] Phase 9: UE5-style UI/UX Integration (Content Browser, Details, Outliner)

### Phase 7: Mesh & Material System (Completed)
- **Mega-Buffer Architecture**: Implemented global vertex/index pools to support bindless rendering of thousands of meshes in a single draw path.
- **Programmable Vertex Pulling**: Finalized the move to storage-buffer based geometry access. Shaders now fetch vertex data manually based on `instance_id`.
- **Bindless Material System**: Materials are stored in a global buffer, allowing the fragment shader to lookup PBR properties dynamically.
- **3D ECS Integration**: Added `Transform`, `MeshComponent`, and `MaterialComponent` with full support in the engine context and render loop.
- **Rendering Visibility Resolved**: Fixed camera view-projection updates and vertex alignment mismatches (Phase 7 final debug).

---

## 3. Current Project State

### Examples
- **`hello_engine`**: Located in `examples/hello_engine`.
    - Spawns a 3D physics-enabled environment.
    - Demonstrates **3D Mesh Rendering** (Cube) via the Visibility Buffer.
    - Real-time **Physics Synchronization** (falling dynamic cubes colliding with a static floor).
    - **Interactive Dashboard**: Shows real-time FPS, entity counts, and rendering stats via `egui`.

### Build Status
- **OS**: Windows (MSVC) tested.
- **Dependencies**: 
    - `wgpu` (29.0)
    - `glam` (0.29)
    - `kira` (0.9)
    - `jolt_physics` (FFI)
    - `bytemuck` (for GPU data packing)
    - `gltf` (for asset loading)

---

- **Cook-Torrance PBR**: Implemented full GGX/Schlick BRDF with Burley diffuse.
- **Bindless Texture System**: Support for 1024 textures via `binding_array`.
- **Barycentric Interpolation**: Perspective-correct UV/Normal interpolation in the shading pass.
- **GLTF Texture Support**: `AssetManager` now extracts albedo, normal, and metallic-roughness maps.

---

## 4. Next Steps (Phase 9: UE5-style UI/UX)

- **Outliner**: Hierarchy view of all entities in the scene.
- **Details Panel**: Real-time inspection and editing of components (Transform, Material, Physics).
- **Content Browser**: Thumbnail-based view of loaded assets (meshes, textures).
- **Viewport Interaction**: Mouse picking and gizmos (future).

---

## 5. Technical Reference

- **Geometry Format**: Standard interleaved `Vertex` struct (Position, Normal, UV).
- **Visibility Encoding**: `instance_id` (22 bits) and `primitive_id` (10 bits) packed into a `u32` R32Uint texture.
- **Instance Data**: Packed into `GpuInstanceData` (Mat4 + mesh offsets + material ID).
- **FFI Boundary**: `somnium_physics_sys/src/jolt_bridge.cpp` remains the bridge for Jolt extensions.
