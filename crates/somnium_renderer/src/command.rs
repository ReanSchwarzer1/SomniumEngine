//! Abstract stateless draw command submission.
//!
//! ## Reference Architecture
//!
//! Inspired by `bgfx` stateless submission and sort keys.
//! Instead of binding state directly via immediate mode API calls,
//! ECS systems emit `DrawCommand`s tagged with a 64-bit `SortKey`.
//! The renderer sorts these commands to minimize state changes
//! (e.g., pipeline swaps, bind group swaps) before executing them.

use glam::Mat4;

/// A 64-bit key used to sort draw commands before submission.
///
/// Typical packing (from most significant to least significant bits):
/// - View/Pass ID (e.g., Opaque vs Transparent, Shadow vs Main)
/// - Translucency depth (for back-to-front sorting)
/// - Material ID (to minimize pipeline changes)
/// - Mesh ID (to minimize vertex buffer bindings)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct SortKey(pub u64);

impl SortKey {
    /// Create a new sort key.
    ///
    /// # Parameters
    /// - `pass_id`: The render pass (0 = earliest).
    /// - `material_id`: The material pipeline handle.
    /// - `mesh_id`: The geometry handle.
    #[must_use]
    pub fn new(pass_id: u8, material_id: u16, mesh_id: u32) -> Self {
        let pass = u64::from(pass_id) << 56;
        let mat = u64::from(material_id) << 32;
        let mesh = u64::from(mesh_id);
        Self(pass | mat | mesh)
    }
}

/// An abstract command to draw a mesh with a material.
#[derive(Debug, Clone)]
pub struct DrawCommand {
    /// The key used to sort this command against others.
    pub sort_key: SortKey,
    /// Offset into the global vertex buffer.
    pub vertex_offset: u32,
    /// Offset into the global index buffer.
    pub index_offset: u32,
    /// Number of indices to draw.
    pub index_count: u32,
    /// ID into the material pool.
    pub material_id: u32,
    /// The world transformation matrix.
    pub transform: Mat4,
    /// Whether this draw reaches the shadow atlas at all (Phase 24AE).
    ///
    /// Separate from the screen-radius test in `Renderer::shadow_casters`,
    /// which is automatic and size-based. This one is *authored*: foliage sets
    /// it from its own shadow distance, because a field of grass is the case
    /// where the artist wants the cut nearer than "too small to see" would put
    /// it. Everything else sets `true` and is judged on size alone.
    pub casts_shadow: bool,
}

// ─── Visibility-buffer packing limits (Phase 15C) ────────────────────────────

/// Maximum number of draws in one frame.
///
/// No longer set by the visibility buffer, which now writes instance id and
/// primitive id into separate channels of an `Rg32Uint` and caps neither. This
/// is simply the instance buffer's own budget.
pub const MAX_DRAWS_PER_FRAME: u32 = 65_535;

/// Maximum triangles in a single draw.
///
/// **No longer a hardware-imposed cap.** The visibility buffer used to pack
/// instance and primitive ids into one 32-bit channel, and this was the
/// primitive half: a mesh past it wrapped and shaded from an unrelated
/// triangle, which is what shattered the island tree's 714 000-triangle leaf
/// mesh. Both ids now occupy their own channel of an `Rg32Uint`, so neither
/// wraps. Kept as a sanity bound only.
pub const MAX_TRIANGLES_PER_DRAW: u32 = 4_000_000;
