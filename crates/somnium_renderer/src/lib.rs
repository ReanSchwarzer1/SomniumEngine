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

pub mod animated_geometry;
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
pub mod quality;
pub mod renderer;
pub mod shaders;
pub mod shadow;
pub mod skinning;
pub mod terrain;
pub mod texture_pool;
pub mod timing;
/// MORROWIND-J step 3: one view of the scene, and how a frame's views tile.
pub mod vertex_paint;
pub mod view;
pub mod viewport_resolution;
pub mod water_body;
pub mod wind;

pub use bindless::{GlobalResourcePool, MAX_BINDLESS_TEXTURES};
pub use command::{DrawCommand, SortKey};
pub use context::RenderContext;
pub use pass::gizmo::{GizmoAxis, GizmoMode, gizmo_hit_test};
pub use quality::{GraphicsBudget, GraphicsPreset};
pub use renderer::{SceneTarget, SomniumRenderer, UploadedNode};
pub use viewport_resolution::{VIEWPORT_RESOLUTION_LABELS, scene_size_for_preset};

/// The eye drawn in the sky by the shading pass (from `somnium.SkyEye`).
/// `intensity` 0 draws none.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyEyeParams {
    /// World direction toward the eye's centre.
    pub direction: glam::Vec3,
    /// tan(half the angular width).
    pub tan_half_width: f32,
    pub color: glam::Vec3,
    /// cd/m² at the iris's hottest ring.
    pub intensity: f32,
    pub openness: f32,
    pub pupil: f32,
    pub pulse_hz: f32,
    pub glow: f32,
}

impl Default for SkyEyeParams {
    fn default() -> Self {
        Self {
            direction: glam::Vec3::NEG_Z,
            tan_half_width: 0.4,
            color: glam::Vec3::ZERO,
            intensity: 0.0,
            openness: 0.42,
            pupil: 0.14,
            pulse_hz: 0.0,
            glow: 0.0,
        }
    }
}
