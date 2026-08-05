//! Phase 15B: frustum culling maths.
//!
//! The actual culling runs on the GPU (`shaders/cull.wgsl`), but the plane
//! extraction and the AABB test are mirrored here in plain Rust so they can be
//! unit-tested without a device — the shader is a direct transliteration.
//!
//! ## Reference Architecture
//!
//! Plane extraction is the Gribb–Hartmann method (*Fast Extraction of Viewing
//! Frustum Planes from the World-View-Projection Matrix*, 2001), with the near
//! plane taken as `row2` rather than `row3 + row2` because wgpu's clip space
//! uses `z ∈ [0, 1]` (Direct3D convention) rather than OpenGL's `[-1, 1]`.
//!
//! The instance-culling shape — a dense per-draw array, with the result written
//! back as each draw's instance count — follows UE5's
//! `InstanceCullingDefinitions.h` (ATTRIBUTION §13.12).

use bytemuck::{Pod, Zeroable};

/// Per-instance local bounds handed to the culling shader.
///
/// **Size**: 32 bytes. `vec4` padding keeps the std430 layout predictable —
/// a bare `vec3` array would be padded differently by the shader.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub struct GpuCullAabb {
    /// Local-space minimum corner (`w` unused).
    pub min: [f32; 4],
    /// Local-space maximum corner (`w` unused).
    pub max: [f32; 4],
}

/// Culling parameters uniform: the six frustum planes plus the draw count.
///
/// **Size**: 112 bytes.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub struct GpuCullParams {
    /// `left, right, bottom, top, near, far`, each `(nx, ny, nz, d)` normalized.
    pub planes: [[f32; 4]; 6],
    /// Number of draws to process.
    pub draw_count: u32,
    /// `0` = cull normally, `1` = force everything visible (debug / fallback).
    pub disabled: u32,
    pub _pad: [u32; 2],
}

/// Extract the six frustum planes from a view-projection matrix.
///
/// Each plane is `(nx, ny, nz, d)` normalized so that
/// `dot(n, p) + d >= 0` means "point `p` is on the inside".
/// Order: left, right, bottom, top, near, far.
pub fn frustum_planes(view_proj: glam::Mat4) -> [[f32; 4]; 6] {
    // glam is column-major; `row(i)` gives the i-th row of the matrix.
    let r0 = view_proj.row(0);
    let r1 = view_proj.row(1);
    let r2 = view_proj.row(2);
    let r3 = view_proj.row(3);

    let raw = [
        r3 + r0, // left
        r3 - r0, // right
        r3 + r1, // bottom
        r3 - r1, // top
        r2,      // near  (z ∈ [0, 1], not r3 + r2)
        r3 - r2, // far
    ];

    let mut planes = [[0.0f32; 4]; 6];
    for (out, p) in planes.iter_mut().zip(raw) {
        // Normalize by the normal's length so `d` is a true distance.
        let len = glam::Vec3::new(p.x, p.y, p.z).length();
        let inv = if len > 1e-6 { 1.0 / len } else { 0.0 };
        *out = [p.x * inv, p.y * inv, p.z * inv, p.w * inv];
    }
    planes
}

/// Transform a local AABB by `model` and return the enclosing world AABB.
///
/// Uses the standard centre/extent form: the transformed extent is the
/// absolute-valued 3×3 basis applied to the local extent, which is far cheaper
/// than transforming all eight corners and gives the same axis-aligned bound.
pub fn transform_aabb(
    model: glam::Mat4,
    min: glam::Vec3,
    max: glam::Vec3,
) -> (glam::Vec3, glam::Vec3) {
    let centre = (min + max) * 0.5;
    let extent = (max - min) * 0.5;

    let world_centre = model.transform_point3(centre);
    let abs_basis = glam::Mat3::from_cols(
        model.x_axis.truncate().abs(),
        model.y_axis.truncate().abs(),
        model.z_axis.truncate().abs(),
    );
    let world_extent = abs_basis * extent;

    (world_centre - world_extent, world_centre + world_extent)
}

/// Conservative AABB-vs-frustum test.
///
/// Returns `false` only when the box is entirely outside at least one plane.
/// Boxes straddling a plane are kept — a false positive costs one wasted draw,
/// while a false negative would pop geometry out of the scene.
pub fn aabb_in_frustum(planes: &[[f32; 4]; 6], min: glam::Vec3, max: glam::Vec3) -> bool {
    // Degenerate/empty bounds (min > max) can never be visible.
    if min.x > max.x || min.y > max.y || min.z > max.z {
        return false;
    }
    for p in planes {
        let n = glam::Vec3::new(p[0], p[1], p[2]);
        // "Positive vertex": the corner furthest along the plane normal. If even
        // that corner is behind the plane, the whole box is outside.
        let pv = glam::Vec3::new(
            if n.x >= 0.0 { max.x } else { min.x },
            if n.y >= 0.0 { max.y } else { min.y },
            if n.z >= 0.0 { max.z } else { min.z },
        );
        if n.dot(pv) + p[3] < 0.0 {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_view_proj() -> glam::Mat4 {
        // Camera at +Z looking at the origin, 45° FOV, 16:9.
        let view = glam::Mat4::look_at_rh(
            glam::Vec3::new(0.0, 0.0, 10.0),
            glam::Vec3::ZERO,
            glam::Vec3::Y,
        );
        let proj = glam::Mat4::perspective_rh(45.0_f32.to_radians(), 16.0 / 9.0, 0.1, 1000.0);
        proj * view
    }

    fn unit_box_at(c: glam::Vec3) -> (glam::Vec3, glam::Vec3) {
        (c - glam::Vec3::splat(0.5), c + glam::Vec3::splat(0.5))
    }

    #[test]
    fn box_at_origin_is_visible() {
        let planes = frustum_planes(test_view_proj());
        let (min, max) = unit_box_at(glam::Vec3::ZERO);
        assert!(aabb_in_frustum(&planes, min, max));
    }

    #[test]
    fn box_behind_the_camera_is_culled() {
        let planes = frustum_planes(test_view_proj());
        // Camera sits at z = +10 looking toward -Z, so z = +50 is behind it.
        let (min, max) = unit_box_at(glam::Vec3::new(0.0, 0.0, 50.0));
        assert!(!aabb_in_frustum(&planes, min, max));
    }

    #[test]
    fn box_far_off_to_the_side_is_culled() {
        let planes = frustum_planes(test_view_proj());
        let (min, max) = unit_box_at(glam::Vec3::new(500.0, 0.0, 0.0));
        assert!(!aabb_in_frustum(&planes, min, max));
    }

    #[test]
    fn box_beyond_the_far_plane_is_culled() {
        let planes = frustum_planes(test_view_proj());
        let (min, max) = unit_box_at(glam::Vec3::new(0.0, 0.0, -2000.0));
        assert!(!aabb_in_frustum(&planes, min, max));
    }

    #[test]
    fn a_box_straddling_the_edge_is_kept() {
        // Conservative: anything partially inside must survive, or geometry
        // would visibly pop out at the screen edge.
        let planes = frustum_planes(test_view_proj());
        let huge = (glam::Vec3::splat(-1000.0), glam::Vec3::splat(1000.0));
        assert!(aabb_in_frustum(&planes, huge.0, huge.1));
    }

    #[test]
    fn empty_bounds_are_never_visible() {
        let planes = frustum_planes(test_view_proj());
        let min = glam::Vec3::splat(f32::INFINITY);
        let max = glam::Vec3::splat(f32::NEG_INFINITY);
        assert!(!aabb_in_frustum(&planes, min, max));
    }

    #[test]
    fn planes_are_normalized() {
        for p in frustum_planes(test_view_proj()) {
            let len = glam::Vec3::new(p[0], p[1], p[2]).length();
            assert!((len - 1.0).abs() < 1e-4, "plane normal not unit length: {len}");
        }
    }

    #[test]
    fn translation_moves_the_world_box() {
        let model = glam::Mat4::from_translation(glam::Vec3::new(10.0, 0.0, 0.0));
        let (min, max) = transform_aabb(model, glam::Vec3::splat(-1.0), glam::Vec3::splat(1.0));
        assert!((min.x - 9.0).abs() < 1e-5, "min.x = {}", min.x);
        assert!((max.x - 11.0).abs() < 1e-5, "max.x = {}", max.x);
    }

    #[test]
    fn rotation_grows_the_axis_aligned_bound() {
        // A 45° yaw on a unit cube must widen its axis-aligned extent to √2.
        let model = glam::Mat4::from_rotation_y(std::f32::consts::FRAC_PI_4);
        let (min, max) = transform_aabb(model, glam::Vec3::splat(-1.0), glam::Vec3::splat(1.0));
        let expected = 2.0_f32.sqrt();
        assert!((max.x - expected).abs() < 1e-4, "max.x = {}", max.x);
        assert!((min.x + expected).abs() < 1e-4, "min.x = {}", min.x);
    }

    #[test]
    fn scaling_scales_the_bound() {
        let model = glam::Mat4::from_scale(glam::Vec3::new(3.0, 1.0, 1.0));
        let (min, max) = transform_aabb(model, glam::Vec3::splat(-1.0), glam::Vec3::splat(1.0));
        assert!((max.x - 3.0).abs() < 1e-5);
        assert!((min.x + 3.0).abs() < 1e-5);
    }

    #[test]
    fn a_distant_object_is_culled_but_becomes_visible_when_moved_into_view() {
        // End-to-end: same mesh, two transforms — the cull result must differ.
        let planes = frustum_planes(test_view_proj());
        let (lmin, lmax) = (glam::Vec3::splat(-1.0), glam::Vec3::splat(1.0));

        let off_screen = glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.0, 400.0));
        let (wmin, wmax) = transform_aabb(off_screen, lmin, lmax);
        assert!(!aabb_in_frustum(&planes, wmin, wmax), "should be culled behind camera");

        let in_view = glam::Mat4::from_translation(glam::Vec3::ZERO);
        let (wmin, wmax) = transform_aabb(in_view, lmin, lmax);
        assert!(aabb_in_frustum(&planes, wmin, wmax), "should be visible at origin");
    }
}
