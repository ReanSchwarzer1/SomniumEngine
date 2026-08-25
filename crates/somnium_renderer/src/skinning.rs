//! MORROWIND-U — GPU skinning, and the decision behind it.
//!
//! # The problem, stated precisely
//!
//! Somnium's visibility-buffer pipeline assumes geometry is **static**:
//!
//! - [`GeometryPool`](crate::geometry::GeometryPool) hands out permanent vertex
//!   ranges;
//! - `meshlet.rs` precomputes per-cluster bounds at upload time;
//! - `cull.wgsl` tests those bounds, and Hi-Z assumes last frame's depth
//!   predicts this frame's;
//! - **ray tracing reads positions straight out of the shared pool**
//!   (`geometry.rs:122` says so in as many words).
//!
//! Skinned geometry moves every frame, which invalidates the second, third and
//! fourth of those.
//!
//! # The decision: skin-to-buffer
//!
//! A compute pass writes posed vertices into a transient slice of the *same*
//! pool, and everything downstream keeps working unchanged because it is
//! reading the buffer it always read.
//!
//! The alternative — **skin-in-shader**, applying the palette during the
//! visibility pass's vertex stage — costs no extra memory, and needs:
//!
//! 1. conservative meshlet bounds, because `cull.wgsl` would be testing bounds
//!    computed from an unposed mesh;
//! 2. a BLAS rebuild anyway, because ray tracing does not go through the vertex
//!    stage at all and would trace against the rest pose — a character casting
//!    a ray-traced shadow of its T-pose;
//! 3. teaching every current and future consumer of the pool that positions may
//!    be a function of a palette. Measured on the tree at MORROWIND-U:
//!    `grep -rl "vertices\[" src/shaders` returns **eight modules besides this
//!    one** — `visibility`, `shading`, `shadow`, `transparent`, `outline`,
//!    `rt_hit`, `restir_gi` and `lighting_extra` — and the count only goes up.
//!
//! (2) and (3) are most of skin-to-buffer's cost without its property that
//! nothing downstream changes. §A.5 predicted skin-to-buffer and set the rule
//! *"if the measurement is ambiguous, take the simple one"*. It is not
//! ambiguous: skin-in-shader is strictly more work here **and** leaves ray
//! tracing wrong until the BLAS rebuild it was supposed to avoid is written.
//!
//! **The measured cost of the choice is [`SkinningPalettes::posed_bytes`]** —
//! 32 bytes per posed vertex per skinned instance, resident, plus one
//! read-modify-write of that per frame. For a thousand characters at 8,000
//! vertices each that is 256 MB, which is the number a future sub-phase
//! comparing designs has to beat, and it is why [`SkinBudget`] exists rather
//! than the pass allocating until it stops.
//!
//! # What is here and what is not
//!
//! Here: the palette buffer, the skin-vertex packing, the transient posed-span
//! bookkeeping, the conservative bounds a moving mesh needs, and the dispatch
//! shape. **The renderer-side frame integration is MORROWIND-U's second half**
//! and is named in the phase record rather than half-written here.

use glam::{Mat4, Vec3};
use somnium_anim::{MAX_JOINTS_PER_SKELETON, Skeleton, Skin};

/// A skinned instance's registration with the pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkinnedHandle(pub u32);

/// What the compute pass needs to know about one skinned instance.
///
/// Mirrors `SkinInstance` in `skinning.wgsl`. Sixteen bytes, `repr(C)`, and
/// checked against the shader by a test — the mismatch MORROWIND-D found
/// between a `vec4<f32>` and a `[f32; 4]` is the reason that test exists at
/// all.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SkinInstance {
    /// Where the rest vertices start, in the shared pool.
    pub rest_offset: u32,
    /// Where the posed vertices go, in the same pool.
    pub posed_offset: u32,
    pub vertex_count: u32,
    /// Where this instance's joints start, in the palette buffer.
    pub palette_base: u32,
}

/// One vertex's skin binding, packed for the GPU.
///
/// Sixteen bytes: four `u16` joints in two `u32`, four `f16` weights in two
/// more. **Halving this was the reason `JointIndex` is `u16`** — it is a
/// per-vertex array, so eight bytes saved here is eight bytes on every vertex
/// of every character in the scene.
///
/// `f16` weights are not a compromise worth arguing about: a weight is in
/// `0..=1` and the visible quantisation of an 11-bit mantissa on a blend is
/// nothing. Positions would be a different matter and are still `f32`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SkinVertex {
    pub joints_01: u32,
    pub joints_23: u32,
    pub weights_01: u32,
    pub weights_23: u32,
}

impl SkinVertex {
    /// Pack a binding.
    #[must_use]
    pub fn pack(binding: somnium_anim::SkinBinding) -> Self {
        let j = binding.joints;
        let w = binding.weights;
        Self {
            joints_01: (j[0] as u32) | ((j[1] as u32) << 16),
            joints_23: (j[2] as u32) | ((j[3] as u32) << 16),
            weights_01: pack_f16x2(w[0], w[1]),
            weights_23: pack_f16x2(w[2], w[3]),
        }
    }

    /// Unpack, for tests and for a debug view.
    #[must_use]
    pub fn unpack(self) -> somnium_anim::SkinBinding {
        let (w0, w1) = unpack_f16x2(self.weights_01);
        let (w2, w3) = unpack_f16x2(self.weights_23);
        somnium_anim::SkinBinding {
            joints: [
                (self.joints_01 & 0xffff) as u16,
                (self.joints_01 >> 16) as u16,
                (self.joints_23 & 0xffff) as u16,
                (self.joints_23 >> 16) as u16,
            ],
            weights: [w0, w1, w2, w3],
        }
    }
}

/// WGSL's `pack2x16float`, on the CPU.
fn pack_f16x2(a: f32, b: f32) -> u32 {
    (f32_to_f16(a) as u32) | ((f32_to_f16(b) as u32) << 16)
}

fn unpack_f16x2(packed: u32) -> (f32, f32) {
    (
        f16_to_f32((packed & 0xffff) as u16),
        f16_to_f32((packed >> 16) as u16),
    )
}

/// IEEE 754 binary16, round-to-nearest-even.
///
/// Written out rather than pulled in: `half` is a dependency for twenty lines,
/// and this has to agree bit for bit with WGSL's `pack2x16float` — which is a
/// thing to test against rather than to trust a crate about.
///
/// Weights are `0..=1`, so the sub-normal and overflow paths are unreachable in
/// practice and are still handled, because "unreachable in practice" is how a
/// NaN gets into a vertex buffer.
fn f32_to_f16(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;

    if exponent == 0xff {
        // Inf or NaN. A NaN must stay a NaN rather than becoming Inf, or a bad
        // weight silently becomes a very large one.
        return sign | 0x7c00 | if mantissa != 0 { 0x0200 } else { 0 };
    }

    let unbiased = exponent - 127 + 15;
    if unbiased >= 0x1f {
        return sign | 0x7c00; // overflow to infinity
    }
    if unbiased <= 0 {
        if unbiased < -10 {
            return sign; // underflows to zero
        }
        // Sub-normal: shift the implicit leading one back in.
        let mantissa = mantissa | 0x0080_0000;
        let shift = (14 - unbiased) as u32;
        let rounded = (mantissa + (1 << (shift - 1))) >> shift;
        return sign | rounded as u16;
    }

    let rounded_mantissa = (mantissa + 0x0000_0fff + ((mantissa >> 13) & 1)) >> 13;
    // Rounding can carry into the exponent, which is correct and is why the
    // exponent is added rather than or-ed.
    sign | (((unbiased as u32) << 10) + rounded_mantissa) as u16
}

fn f16_to_f32(value: u16) -> f32 {
    let sign = ((value as u32) & 0x8000) << 16;
    let exponent = ((value as u32) >> 10) & 0x1f;
    let mantissa = (value as u32) & 0x3ff;

    let bits = if exponent == 0 {
        if mantissa == 0 {
            sign
        } else {
            // Sub-normal: renormalise.
            let shift = mantissa.leading_zeros() - 21;
            let exponent = 127 - 15 - shift;
            sign | (exponent << 23) | ((mantissa << (shift + 1)) & 0x007f_ffff)
        }
    } else if exponent == 0x1f {
        sign | 0x7f80_0000 | (mantissa << 13)
    } else {
        sign | ((exponent + 127 - 15) << 23) | (mantissa << 13)
    };
    f32::from_bits(bits)
}

/// A ceiling on posed-vertex memory.
///
/// Exists because the honest cost of skin-to-buffer is memory proportional to
/// posed vertices, and a design whose cost is proportional to something needs a
/// number rather than a hope. A registration that would exceed the budget is
/// **refused**, with the numbers, rather than allocating until the device
/// stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkinBudget {
    pub max_posed_vertices: u32,
    pub max_instances: u32,
}

impl Default for SkinBudget {
    fn default() -> Self {
        // Two million posed vertices is 64 MB at 32 bytes each — about 250
        // characters at 8,000 vertices, which is a crowd rather than a party
        // and is a tenth of what a mid-range card has. KENSHI's crowd phase is
        // where this number gets argued with; MORROWIND-U's job is that there
        // *is* one.
        Self {
            max_posed_vertices: 2_000_000,
            max_instances: 4_096,
        }
    }
}

/// Why a registration was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkinError {
    /// The skin does not match the skeleton — a joint index past the end, or a
    /// different skeleton id. Would read past the palette on the GPU.
    SkinDoesNotFitSkeleton,
    /// The skeleton has more joints than the palette layout allows.
    TooManyJoints { joints: usize, limit: usize },
    /// Vertex count and binding count disagree.
    BindingCountMismatch { vertices: usize, bindings: usize },
    /// The posed-vertex budget is spent.
    PosedVertexBudget { wanted: u32, remaining: u32 },
    /// The instance budget is spent.
    InstanceBudget { limit: u32 },
}

/// CPU-side bookkeeping for the skinning pass.
///
/// Holds no GPU objects: the buffers belong to the renderer, and this is the
/// part that can be tested without a device — which for a system whose failure
/// mode is an out-of-bounds palette read is the part worth testing hardest.
#[derive(Debug, Default)]
pub struct SkinningPalettes {
    budget: SkinBudget,
    instances: Vec<SkinInstance>,
    /// Per instance: how many joints, and the rest-pose bounds to expand.
    joints: Vec<u32>,
    rest_bounds: Vec<([f32; 3], [f32; 3])>,
    palette: Vec<Mat4>,
    posed_vertices_used: u32,
}

impl SkinningPalettes {
    #[must_use]
    pub fn new(budget: SkinBudget) -> Self {
        Self {
            budget,
            ..Self::default()
        }
    }

    /// Register a skinned instance.
    ///
    /// `rest_offset` is where the mesh's rest vertices already live in the
    /// pool; `posed_offset` is a span the caller reserved through
    /// [`GeometryPool::reserve_vertices`](crate::geometry::GeometryPool::reserve_vertices).
    /// `rest_bounds` is the mesh's local-space AABB at bind.
    pub fn register(
        &mut self,
        skeleton: &Skeleton,
        skin: &Skin,
        rest_offset: u32,
        posed_offset: u32,
        vertex_count: u32,
        rest_bounds: ([f32; 3], [f32; 3]),
    ) -> Result<SkinnedHandle, SkinError> {
        if skeleton.len() > MAX_JOINTS_PER_SKELETON {
            return Err(SkinError::TooManyJoints {
                joints: skeleton.len(),
                limit: MAX_JOINTS_PER_SKELETON,
            });
        }
        if skin.bindings.len() != vertex_count as usize {
            return Err(SkinError::BindingCountMismatch {
                vertices: vertex_count as usize,
                bindings: skin.bindings.len(),
            });
        }
        if !skin.fits(skeleton) {
            return Err(SkinError::SkinDoesNotFitSkeleton);
        }
        if self.instances.len() as u32 >= self.budget.max_instances {
            return Err(SkinError::InstanceBudget {
                limit: self.budget.max_instances,
            });
        }
        let remaining = self
            .budget
            .max_posed_vertices
            .saturating_sub(self.posed_vertices_used);
        if vertex_count > remaining {
            return Err(SkinError::PosedVertexBudget {
                wanted: vertex_count,
                remaining,
            });
        }

        let handle = SkinnedHandle(self.instances.len() as u32);
        let palette_base = self.palette.len() as u32;
        self.instances.push(SkinInstance {
            rest_offset,
            posed_offset,
            vertex_count,
            palette_base,
        });
        self.joints.push(skeleton.len() as u32);
        self.rest_bounds.push(rest_bounds);
        self.palette
            .extend(std::iter::repeat_n(Mat4::IDENTITY, skeleton.len()));
        self.posed_vertices_used += vertex_count;
        Ok(handle)
    }

    /// Write one instance's palette.
    ///
    /// The matrices come from [`somnium_anim::Pose::to_palette`], and this is
    /// the only thing that crosses the seam: the renderer never sees a `Pose`.
    ///
    /// Returns `false` for an unknown handle or a wrong-length palette rather
    /// than writing part of it — a half-written palette is a character with
    /// some joints from this frame and some from the last.
    pub fn set_palette(&mut self, handle: SkinnedHandle, matrices: &[Mat4]) -> bool {
        let Some(instance) = self.instances.get(handle.0 as usize) else {
            return false;
        };
        let count = self.joints[handle.0 as usize] as usize;
        if matrices.len() != count {
            return false;
        }
        let base = instance.palette_base as usize;
        self.palette[base..base + count].copy_from_slice(matrices);
        true
    }

    /// The bounds a posed instance actually occupies this frame.
    ///
    /// **The piece that makes skin-to-buffer correct rather than merely
    /// working.** The pool's stored AABB for the posed span was computed from
    /// the rest pose and is wrong the moment the character moves, so `cull.wgsl`
    /// would test a box the geometry has walked out of and the character would
    /// vanish at the edge of the screen.
    ///
    /// The conservative answer, at `O(joints)` rather than `O(vertices)`: take
    /// the rest AABB's eight corners, transform them by **every** palette
    /// matrix, and union. That over-estimates — a joint that moves only the
    /// left hand still expands the box as if it moved the whole mesh — and
    /// over-estimating is the safe direction for a cull test.
    #[must_use]
    pub fn posed_bounds(&self, handle: SkinnedHandle) -> Option<([f32; 3], [f32; 3])> {
        let instance = self.instances.get(handle.0 as usize)?;
        let count = self.joints[handle.0 as usize] as usize;
        let (min, max) = self.rest_bounds[handle.0 as usize];
        let base = instance.palette_base as usize;

        let corners = [
            Vec3::new(min[0], min[1], min[2]),
            Vec3::new(max[0], min[1], min[2]),
            Vec3::new(min[0], max[1], min[2]),
            Vec3::new(max[0], max[1], min[2]),
            Vec3::new(min[0], min[1], max[2]),
            Vec3::new(max[0], min[1], max[2]),
            Vec3::new(min[0], max[1], max[2]),
            Vec3::new(max[0], max[1], max[2]),
        ];

        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for matrix in &self.palette[base..base + count] {
            for corner in corners {
                let moved = matrix.transform_point3(corner);
                lo = lo.min(moved);
                hi = hi.max(moved);
            }
        }
        if !lo.is_finite() || !hi.is_finite() {
            return None;
        }
        Some((lo.to_array(), hi.to_array()))
    }

    /// The compute dispatch for this frame.
    ///
    /// `(x, y, z)` workgroups: `x` covers the widest instance's vertices at 64
    /// per group, `y` is one per instance. A ragged dispatch — the shader
    /// returns early for a thread past its instance's vertex count — because
    /// one dispatch per instance would be one bind and one call per character,
    /// and a thousand characters is a thousand of each.
    #[must_use]
    pub fn dispatch(&self) -> (u32, u32, u32) {
        if self.instances.is_empty() {
            return (0, 0, 0);
        }
        let widest = self
            .instances
            .iter()
            .map(|i| i.vertex_count)
            .max()
            .unwrap_or(0);
        (
            widest.div_ceil(WORKGROUP_SIZE),
            self.instances.len() as u32,
            1,
        )
    }

    /// How many threads the ragged dispatch wastes, as a fraction.
    ///
    /// Kept because it is the honest cost of the ragged shape, and because a
    /// scene of one 60,000-vertex hero and nine hundred 900-vertex crowd
    /// members wastes most of the dispatch — which is a real finding for KENSHI
    /// rather than a hypothetical, and this is how it gets noticed.
    #[must_use]
    pub fn dispatch_waste(&self) -> f32 {
        let (x, y, _) = self.dispatch();
        let launched = (x * WORKGROUP_SIZE) as u64 * y as u64;
        if launched == 0 {
            return 0.0;
        }
        let useful: u64 = self.instances.iter().map(|i| i.vertex_count as u64).sum();
        1.0 - (useful as f64 / launched as f64) as f32
    }

    /// Resident posed-vertex memory, in bytes.
    ///
    /// **The measured cost of choosing skin-to-buffer.** 32 bytes per posed
    /// vertex — one `Vertex`, since the posed span is in the same pool and has
    /// the same layout.
    #[must_use]
    pub fn posed_bytes(&self) -> u64 {
        self.posed_vertices_used as u64 * std::mem::size_of::<somnium_asset::Vertex>() as u64
    }

    #[must_use]
    pub fn instances(&self) -> &[SkinInstance] {
        &self.instances
    }

    #[must_use]
    pub fn palette(&self) -> &[Mat4] {
        &self.palette
    }

    #[must_use]
    pub fn budget(&self) -> SkinBudget {
        self.budget
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }
}

/// Must match `@workgroup_size(64)` in `skinning.wgsl`. Asserted by a test that
/// reads the shader, because the two drifting apart is a dispatch that skins
/// three quarters of a character.
pub const WORKGROUP_SIZE: u32 = 64;

#[cfg(test)]
mod tests;
