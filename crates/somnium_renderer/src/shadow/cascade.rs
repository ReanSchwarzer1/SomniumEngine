//! Cascade shadow map partitioning.
//!
//! Implements the Practical Split Scheme (PSS, Engel 2006) to divide the
//! camera frustum into NUM_CASCADES depth slices, then builds a tight
//! orthographic view-projection matrix for each slice using a bounding
//! sphere for stability and texel-snapping to eliminate shadow-edge shimmer.
//!
//! ## Reference Architecture
//!
//! Cascade partitioning follows the logarithmic-linear blend described in
//! "Cascaded Shadow Maps" (GPU Gems 3, chapter 10) and refined by Microsoft's
//! DirectX SDK shadow mapping sample. The texel-snapping technique is the
//! standard sphere-fitting + grid-alignment approach documented in
//! "Common Techniques to Improve Shadow Depth Maps" (Microsoft DirectX docs).

use super::NUM_CASCADES;
use glam::{Mat4, Vec3, Vec4};

/// Camera near plane (matches hello_engine's perspective_rh near).
pub const CAMERA_NEAR: f32 = 0.1;

/// Maximum shadow distance; geometry beyond this receives no shadow.
pub const SHADOW_DISTANCE: f32 = 100.0;

/// Blend factor for the PSS formula: 0.0 = pure uniform, 1.0 = pure logarithmic.
const LAMBDA: f32 = 0.5;

/// Extra caster depth on both sides of a cascade receiver slab.
///
/// Low-elevation sunlight can project a kilometre-scale terrain caster into a
/// receiver only tens of metres from the camera. Flax uses the same 1 km
/// extension for its CSM culling volume (`ShadowsPass.cpp::cullRangeExtent`).
const CASTER_DEPTH_EXTENSION: f32 = 1000.0;

/// Per-cascade result: view-projection matrix + view-space far depth.
#[derive(Clone, Copy, Debug)]
pub struct CascadeData {
    /// Combined light view × orthographic projection matrix.
    pub view_proj: Mat4,
    /// View-space depth (positive, from camera) at which this cascade ends.
    pub split_depth: f32,
    /// World-space centre of the fitted receiver sphere.
    ///
    /// The shadow cache quantises this by [`Self::texel_size`] to decide when
    /// camera movement crossed a shadow texel.  It is policy input, not a GPU
    /// layout field.
    pub world_center: Vec3,
    /// World metres represented by one texel in this cascade.
    pub texel_size: f32,
}

impl Default for CascadeData {
    fn default() -> Self {
        Self {
            view_proj: Mat4::IDENTITY,
            split_depth: 0.0,
            world_center: Vec3::ZERO,
            texel_size: 1.0,
        }
    }
}

/// Compute the four cascade VP matrices and split depths from the current frame's camera.
///
/// # Arguments
/// * `light_dir` — World-space direction *toward* the light, normalized.
/// * `inv_view_proj` — Inverse of the camera's (proj × view) matrix.
pub fn compute_cascades(light_dir: Vec3, inv_view_proj: Mat4) -> [CascadeData; NUM_CASCADES] {
    let splits = compute_pss_splits(CAMERA_NEAR, SHADOW_DISTANCE);

    let up = if light_dir.dot(Vec3::Y).abs() > 0.99 {
        Vec3::Z
    } else {
        Vec3::Y
    };

    // Full frustum corners in world space (NDC z ∈ [0, 1] for wgpu).
    let full_corners = frustum_corners_world(inv_view_proj);

    let mut cascades = [CascadeData::default(); NUM_CASCADES];

    for i in 0..NUM_CASCADES {
        let near_depth = if i == 0 { CAMERA_NEAR } else { splits[i - 1] };
        let far_depth = splits[i];

        // Lerp t values: fraction of the full [NEAR..SHADOW_DISTANCE] range.
        let near_t = (near_depth - CAMERA_NEAR) / (SHADOW_DISTANCE - CAMERA_NEAR);
        let far_t = (far_depth - CAMERA_NEAR) / (SHADOW_DISTANCE - CAMERA_NEAR);

        let (view_proj, world_center, texel_size) = cascade_vp(
            light_dir,
            up,
            &full_corners,
            near_t,
            far_t,
            super::CASCADE_SIZE as f32,
        );
        cascades[i].view_proj = view_proj;
        cascades[i].split_depth = far_depth;
        cascades[i].world_center = world_center;
        cascades[i].texel_size = texel_size;
    }

    cascades
}

/// Practical Split Scheme: logarithmic-linear blend of cascade far depths.
fn compute_pss_splits(near: f32, far: f32) -> [f32; NUM_CASCADES] {
    let mut splits = [0.0f32; NUM_CASCADES];
    for i in 0..NUM_CASCADES {
        let frac = (i + 1) as f32 / NUM_CASCADES as f32;
        let uniform = near + (far - near) * frac;
        let log = near * (far / near).powf(frac);
        splits[i] = uniform + LAMBDA * (log - uniform);
    }
    splits
}

/// Extract the 8 corners of the full camera frustum in world space.
///
/// Uses NDC corners with z ∈ [0,1] (wgpu depth convention).
fn frustum_corners_world(inv_view_proj: Mat4) -> [Vec3; 8] {
    let ndc: [[f32; 4]; 8] = [
        [-1.0, -1.0, 0.0, 1.0],
        [1.0, -1.0, 0.0, 1.0],
        [1.0, 1.0, 0.0, 1.0],
        [-1.0, 1.0, 0.0, 1.0],
        [-1.0, -1.0, 1.0, 1.0],
        [1.0, -1.0, 1.0, 1.0],
        [1.0, 1.0, 1.0, 1.0],
        [-1.0, 1.0, 1.0, 1.0],
    ];
    ndc.map(|n| {
        let v = inv_view_proj * Vec4::new(n[0], n[1], n[2], n[3]);
        v.truncate() / v.w
    })
}

/// Build the orthographic view-projection matrix for a single cascade sub-frustum.
///
/// Uses a bounding sphere for stability (avoids scale changes on camera rotation)
/// and snaps the sphere center to the texel grid to eliminate shadow shimmer.
fn cascade_vp(
    light_dir: Vec3,
    up: Vec3,
    full_corners: &[Vec3; 8],
    near_t: f32,
    far_t: f32,
    cascade_resolution: f32,
) -> (Mat4, Vec3, f32) {
    // Sub-frustum corners by linear interpolation between near and far full-frustum corners.
    let near_corners: [Vec3; 4] = [
        full_corners[0].lerp(full_corners[4], near_t),
        full_corners[1].lerp(full_corners[5], near_t),
        full_corners[2].lerp(full_corners[6], near_t),
        full_corners[3].lerp(full_corners[7], near_t),
    ];
    let far_corners: [Vec3; 4] = [
        full_corners[0].lerp(full_corners[4], far_t),
        full_corners[1].lerp(full_corners[5], far_t),
        full_corners[2].lerp(full_corners[6], far_t),
        full_corners[3].lerp(full_corners[7], far_t),
    ];
    let all_corners: [Vec3; 8] = [
        near_corners[0],
        near_corners[1],
        near_corners[2],
        near_corners[3],
        far_corners[0],
        far_corners[1],
        far_corners[2],
        far_corners[3],
    ];

    // Bounding sphere center and radius.
    let center = all_corners.iter().copied().fold(Vec3::ZERO, |a, c| a + c) / 8.0;
    let radius = all_corners
        .iter()
        .map(|c| (*c - center).length())
        .fold(0.0f32, f32::max);
    let radius = (radius * 16.0).ceil() / 16.0; // round up slightly for safety

    // Light view: look from "above" the center along the light direction.
    //
    // Phase 25M-2B: at low sun angles the sub-frustum's receiver sphere does
    // not contain casters far behind it along the light direction. Extend the
    // depth slab using Flax's kilometre-scale CSM culling range pattern.
    // Frustum corners contain receivers, not off-frustum casters. Projecting
    // those same corners and clamping with `max(radius * 2)` was a no-op: their
    // axial extent cannot exceed the bounding-sphere radius. Extend the light
    // slab explicitly so low-sun terrain casters remain inside it.
    let back = radius * 2.0 + CASTER_DEPTH_EXTENSION;
    let light_eye = center + light_dir * back;
    let light_view = Mat4::look_at_rh(light_eye, center, up);

    // Texel snapping: round the center's x,y in light view space to the texel grid.
    let texel_size = 2.0 * radius / cascade_resolution;
    let center_ls = light_view.transform_point3(center);
    let snapped_x = (center_ls.x / texel_size).floor() * texel_size;
    let snapped_y = (center_ls.y / texel_size).floor() * texel_size;
    let offset_x = snapped_x - center_ls.x;
    let offset_y = snapped_y - center_ls.y;

    // Orthographic projection in [0,1] depth range (wgpu z convention).
    let left = -radius + offset_x;
    let right = radius + offset_x;
    let bottom = -radius + offset_y;
    let top = radius + offset_y;
    let near = 0.0_f32;
    let far = back + radius * 2.0 + CASTER_DEPTH_EXTENSION;

    let light_proj = ortho_rh_zo(left, right, bottom, top, near, far);
    (light_proj * light_view, center, texel_size)
}

/// Right-handed orthographic projection mapping z to [0, 1] (wgpu depth convention).
fn ortho_rh_zo(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Mat4 {
    let rcp_w = 1.0 / (right - left);
    let rcp_h = 1.0 / (top - bottom);
    let rcp_d = 1.0 / (near - far);
    Mat4::from_cols(
        Vec4::new(2.0 * rcp_w, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 2.0 * rcp_h, 0.0, 0.0),
        Vec4::new(0.0, 0.0, rcp_d, 0.0),
        Vec4::new(
            -(left + right) * rcp_w,
            -(bottom + top) * rcp_h,
            near * rcp_d,
            1.0,
        ),
    )
}
