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
}
