//! The Somnium Renderer.
//!
//! A modern, high-performance rendering backend built on `wgpu`.
//!
//! ## Reference Architecture
//!
//! - **Bindless Resources:** Inspired by O3DE (`Atom/RHI/Bindless.md`).
//! - **Visibility Buffer:** Inspired by The Forge (`IVisibilityBuffer`).
//! - **Stateless Submission:** Inspired by `bgfx` sort keys.
//! - **High Level Material System (HLMS):** Inspired by Ogre-Next.
//! - **Cascaded Shadow Maps (Phase 11):** PSS partitioning + sphere-fit texel snapping.
//! - **CPU frustum early-out (Phase CR):** terrain chunks vs camera AABB; shadow casters vs cascade volumes. GPU 15B stays on F10.
//! - **Ray-traced water reflections (Phase VV Halcyon):** `pass/water_reflection.rs` +
//!   `shaders/rt_hit.wgsl`. Layer 1 is VV+1 refraction (default off). See ATTRIBUTION.md §1.7.

pub mod bindless;
pub mod capability;
pub mod capture;
pub mod cluster;
pub mod command;
pub mod context;
pub mod culling;
pub mod geometry;
pub mod indirect;
pub mod instance;
pub mod jobs;
pub mod material;
pub mod meshlet;
pub mod pass;
pub mod profiler;
pub mod renderer;
pub mod shaders;
pub mod shadow;
pub mod skinning;
pub mod terrain;
pub mod texture_pool;
pub mod timing;
/// MORROWIND-J step 3: one view of the scene, and how a frame's views tile.
pub mod view;
pub mod viewport_resolution;
pub mod water_body;

pub use bindless::{GlobalResourcePool, MAX_BINDLESS_TEXTURES};
pub use command::{DrawCommand, SortKey};
pub use context::RenderContext;
pub use pass::gizmo::{GizmoAxis, GizmoMode, gizmo_hit_test};
pub use renderer::{SceneTarget, SomniumRenderer, UploadedNode};
pub use viewport_resolution::{VIEWPORT_RESOLUTION_LABELS, scene_size_for_preset};
