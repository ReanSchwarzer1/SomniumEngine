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

pub mod bindless;
pub mod capture;
pub mod cluster;
pub mod command;
pub mod culling;
pub mod context;
pub mod geometry;
pub mod indirect;
pub mod instance;
pub mod material;
pub mod meshlet;
pub mod pass;
pub mod profiler;
pub mod renderer;
pub mod shadow;
pub mod terrain;
pub mod texture_pool;
pub mod water_body;

pub use bindless::{GlobalResourcePool, MAX_BINDLESS_TEXTURES};
pub use command::{DrawCommand, SortKey};
pub use context::RenderContext;
pub use pass::gizmo::{GizmoAxis, GizmoMode, gizmo_hit_test};
pub use renderer::{SomniumRenderer, UploadedNode};
