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
    /// Phase 15F normal cone: local-space axis in `xyz`, backface threshold in
    /// `w`. `w = 2.0` disables the test, which is what whole-mesh draws use.
    pub cone: [f32; 4],
}

impl GpuCullAabb {
    /// Bounds that never cull: an infinite box with the cone test disabled.
    /// Used for meshes with no recorded AABB, where guessing at the extent
    /// risks deleting geometry.
    pub fn never_culled() -> Self {
        Self {
            min: [f32::MIN, f32::MIN, f32::MIN, 0.0],
            max: [f32::MAX, f32::MAX, f32::MAX, 0.0],
            cone: [0.0, 0.0, 0.0, 2.0],
        }
    }

    /// Bounds from a local-space AABB, with cone culling disabled.
    pub fn from_aabb(min: [f32; 3], max: [f32; 3]) -> Self {
        Self {
            min: [min[0], min[1], min[2], 0.0],
            max: [max[0], max[1], max[2], 0.0],
            cone: [0.0, 0.0, 0.0, 2.0],
        }
    }
}

/// Culling parameters uniform: the six frustum planes plus the draw count.
///
/// **Size**: 224 bytes.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub struct GpuCullParams {
    /// `left, right, bottom, top, near, far`, each `(nx, ny, nz, d)` normalized.
    pub planes: [[f32; 4]; 6],
    /// Number of draws to process.
    pub draw_count: u32,
    /// `0` = cull normally, `1` = force everything visible (debug / fallback).
    pub disabled: u32,
    /// Phase 15E2. `0` = phase one, which tests frustum then occlusion against
    /// the *previous* frame's pyramid. `1` = phase two, which re-tests only the
    /// draws phase one rejected on occlusion, against the pyramid just rebuilt
    /// from phase one's depth.
    pub phase: u32,
    /// `0` leaves frustum culling in place but skips the occlusion half. Held
    /// off on the first frame, before the pyramid has any real content.
    pub occlusion_enabled: u32,
    /// View-projection used to project bounds to screen for the Hi-Z lookup.
    pub view_proj: [[f32; 4]; 4],
    /// Hi-Z level 0 dimensions, in texels.
    pub hiz_size: [f32; 2],
    /// Levels in the pyramid, so the shader can clamp its mip choice.
    pub hiz_mip_count: u32,
    /// Dense argument boundary between single- and double-sided pipelines.
    pub single_sided_args: u32,
    /// DOOM-G: `1` appends survivors to GPU-counted compact streams.
    pub counted_draws: u32,
    pub _pad: [u32; 3],
    /// World-space camera position, for the Phase 15F normal-cone test
    /// (`w` unused).
    pub camera_pos: [f32; 4],
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

/// Conservative test against any of several frusta.
///
/// Used for cascade shadow casters (Phase CR-E): a box that misses the camera
/// can still cast into view, so the test is against cascade volumes, never
/// the camera frustum alone. Kept if it straddles any one cascade.
pub fn aabb_in_any_frustum(frusta: &[[[f32; 4]; 6]], min: glam::Vec3, max: glam::Vec3) -> bool {
    frusta
        .iter()
        .any(|planes| aabb_in_frustum(planes, min, max))
}

/// World AABB of a local box, then a conservative camera-frustum test.
///
/// This is the CPU early-out used for terrain chunks (Phase CR-B) before they
/// reach `draw_queue`. Same maths as the GPU 15B shader; the point of doing it
/// here is that a rejected chunk never becomes a draw.
pub fn chunk_in_frustum(
    planes: &[[f32; 4]; 6],
    model: glam::Mat4,
    min: glam::Vec3,
    max: glam::Vec3,
) -> bool {
    let (wmin, wmax) = transform_aabb(model, min, max);
    aabb_in_frustum(planes, wmin, wmax)
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
    fn box_exactly_touching_a_plane_is_kept() {
        // A zero signed distance is inside. Using <= here (in either the CPU
        // mirror or cull.wgsl) would turn camera motion at a chunk boundary
        // into single-frame terrain holes.
        let planes = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 10.0],
            [0.0, -1.0, 0.0, 10.0],
            [0.0, 0.0, 1.0, 10.0],
            [0.0, 0.0, -1.0, 10.0],
            [1.0, 0.0, 0.0, 100.0],
        ];
        assert!(aabb_in_frustum(
            &planes,
            glam::Vec3::new(-2.0, -1.0, -1.0),
            glam::Vec3::new(0.0, 1.0, 1.0),
        ));
    }

    #[test]
    fn every_plane_is_conservative_at_large_world_coordinates() {
        // Exercise both signs of every axis at a magnitude where careless
        // epsilon/normal handling tends to show up. Repeating the plane six
        // times isolates the exact half-space under test.
        let plane_origin = glam::Vec3::new(1_000_000.0, -750_000.0, 500_000.0);
        let half_extent = 8.0;
        for normal in [
            glam::Vec3::X,
            glam::Vec3::NEG_X,
            glam::Vec3::Y,
            glam::Vec3::NEG_Y,
            glam::Vec3::Z,
            glam::Vec3::NEG_Z,
        ] {
            let plane = [normal.x, normal.y, normal.z, -normal.dot(plane_origin)];
            let planes = [plane; 6];
            for (offset, expected) in [(0.0, true), (-1.0, true), (1.0, false)] {
                let centre = plane_origin - normal * (half_extent + offset);
                let min = centre - glam::Vec3::splat(half_extent);
                let max = centre + glam::Vec3::splat(half_extent);
                assert_eq!(
                    aabb_in_frustum(&planes, min, max),
                    expected,
                    "normal={normal:?} offset={offset}"
                );
            }
        }
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
            assert!(
                (len - 1.0).abs() < 1e-4,
                "plane normal not unit length: {len}"
            );
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
        assert!(
            !aabb_in_frustum(&planes, wmin, wmax),
            "should be culled behind camera"
        );

        let in_view = glam::Mat4::from_translation(glam::Vec3::ZERO);
        let (wmin, wmax) = transform_aabb(in_view, lmin, lmax);
        assert!(
            aabb_in_frustum(&planes, wmin, wmax),
            "should be visible at origin"
        );
    }

    #[test]
    fn a_terrain_chunk_behind_the_camera_is_cpu_culled() {
        let planes = frustum_planes(test_view_proj());
        let model = glam::Mat4::IDENTITY;
        // Default chunk is 64 m on XZ. Camera at z=+10 looking toward -Z.
        let min = glam::Vec3::new(-32.0, 0.0, 40.0);
        let max = glam::Vec3::new(32.0, 16.0, 104.0);
        assert!(
            !chunk_in_frustum(&planes, model, min, max),
            "chunk behind the camera must not reach draw_queue"
        );
    }

    #[test]
    fn a_terrain_chunk_straddling_the_near_plane_is_kept() {
        let planes = frustum_planes(test_view_proj());
        let model = glam::Mat4::IDENTITY;
        // Camera at z=+10; this box covers both in front and behind.
        let min = glam::Vec3::new(-8.0, 0.0, -20.0);
        let max = glam::Vec3::new(8.0, 16.0, 20.0);
        assert!(
            chunk_in_frustum(&planes, model, min, max),
            "a straddling chunk must stay visible"
        );
    }

    #[test]
    fn cascade_any_keeps_a_box_inside_one_frustum() {
        let cam = frustum_planes(test_view_proj());
        let empty = [[0.0, 1.0, 0.0, -1.0e9]; 6];
        let frusta = [empty, cam];
        let (min, max) = unit_box_at(glam::Vec3::ZERO);
        assert!(aabb_in_any_frustum(&frusta, min, max));
        let (behind_min, behind_max) = unit_box_at(glam::Vec3::new(0.0, 0.0, 50.0));
        assert!(!aabb_in_any_frustum(&frusta, behind_min, behind_max));
    }

    /// Same FPS camera as `hello_engine::EditorCamera`.
    fn fps_view_proj(pos: glam::Vec3, yaw_deg: f32, pitch_deg: f32, aspect: f32) -> glam::Mat4 {
        let yaw = yaw_deg.to_radians();
        let pitch = pitch_deg.to_radians();
        let forward = glam::Vec3::new(
            yaw.cos() * pitch.cos(),
            pitch.sin(),
            yaw.sin() * pitch.cos(),
        )
        .normalize();
        let view = glam::Mat4::look_at_rh(pos, pos + forward, glam::Vec3::Y);
        let proj = glam::Mat4::perspective_rh(45.0_f32.to_radians(), aspect, 0.1, 1000.0);
        proj * view
    }

    /// Default 16×16×64 m landscape tile, translated so the origin is the centre.
    fn count_landscape_chunks(view_proj: glam::Mat4) -> (u32, u32) {
        let planes = frustum_planes(view_proj);
        let model = glam::Mat4::from_translation(glam::Vec3::new(-512.0, 0.0, -512.0));
        let chunk = 64.0;
        let mut vis = 0u32;
        let mut culled = 0u32;
        for cz in 0..16u32 {
            for cx in 0..16u32 {
                let min = glam::Vec3::new(cx as f32 * chunk, 0.0, cz as f32 * chunk);
                let max = min + glam::Vec3::new(chunk, 120.0, chunk);
                if chunk_in_frustum(&planes, model, min, max) {
                    vis += 1;
                } else {
                    culled += 1;
                }
            }
        }
        (vis, culled)
    }

    #[test]
    fn looking_away_from_the_default_landscape_cpu_culls_chunks() {
        // DefaultLandscapePreset camera: (0, relief*1.15+30, depth*0.45).
        let cam = glam::Vec3::new(0.0, 150.75, 460.8);
        // Yaw 0 looks +X. Half the 1 km tile sits at x < 0, behind the camera.
        let (vis, culled) = count_landscape_chunks(fps_view_proj(cam, 0.0, -22.0, 16.0 / 9.0));
        assert!(
            culled > 0 && vis > 0,
            "turning 90° must drop vis and raise cpu-cull (vis={vis} culled={culled})"
        );
        // Yaw +90 looks +Z, away from the coast. Most of the tile is behind.
        let (vis_back, culled_back) =
            count_landscape_chunks(fps_view_proj(cam, 90.0, 0.0, 16.0 / 9.0));
        assert!(
            culled_back > vis_back,
            "looking away from the tile must cull the majority (vis={vis_back} culled={culled_back})"
        );
    }
}

// ── Phase 15E: Hi-Z occlusion test ──────────────────────────────────────────
//
// The test projects a candidate's world-space AABB to screen, looks up the
// furthest recorded depth over that footprint in the Hi-Z pyramid, and rejects
// the candidate when its own nearest point is behind that. Everything here errs
// toward drawing: a false "visible" costs one wasted draw, a false "occluded"
// deletes geometry from the image.

/// A candidate's screen footprint plus how near it gets to the camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenBounds {
    /// Screen-space rect in UV units, `[min_x, min_y, max_x, max_y]`, clamped
    /// to `0..1`.
    pub rect: [f32; 4],
    /// Nearest NDC depth of the box, `0` at the near plane.
    pub min_depth: f32,
}

/// Project a world-space AABB through `view_proj` to a screen rect.
///
/// Returns `None` when the box crosses or sits behind the camera plane, where
/// the perspective divide is meaningless — such a box is treated as visible
/// rather than guessed at.
pub fn project_aabb_to_screen(
    min: [f32; 3],
    max: [f32; 3],
    view_proj: glam::Mat4,
) -> Option<ScreenBounds> {
    let mut lo = [f32::INFINITY; 2];
    let mut hi = [f32::NEG_INFINITY; 2];
    let mut min_depth = f32::INFINITY;

    for i in 0..8 {
        let corner = glam::Vec3::new(
            if i & 1 == 0 { min[0] } else { max[0] },
            if i & 2 == 0 { min[1] } else { max[1] },
            if i & 4 == 0 { min[2] } else { max[2] },
        );
        let clip = view_proj * corner.extend(1.0);
        // w <= 0 means the corner is at or behind the eye.
        if clip.w <= 1e-6 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        // NDC x/y are -1..1; screen V runs downward, hence the flip.
        let u = ndc.x * 0.5 + 0.5;
        let v = 1.0 - (ndc.y * 0.5 + 0.5);
        lo[0] = lo[0].min(u);
        lo[1] = lo[1].min(v);
        hi[0] = hi[0].max(u);
        hi[1] = hi[1].max(v);
        min_depth = min_depth.min(ndc.z);
    }

    if !min_depth.is_finite() {
        return None;
    }

    Some(ScreenBounds {
        rect: [
            lo[0].clamp(0.0, 1.0),
            lo[1].clamp(0.0, 1.0),
            hi[0].clamp(0.0, 1.0),
            hi[1].clamp(0.0, 1.0),
        ],
        min_depth: min_depth.max(0.0),
    })
}

/// Pick the pyramid level whose texels are large enough that the footprint
/// spans at most 2x2 of them.
///
/// That bound is what keeps the lookup constant-time: four samples cover any
/// candidate, however large on screen. Choosing a level too low would need more
/// samples and could miss an occluder between them.
pub fn hiz_mip_level(rect: [f32; 4], width: u32, height: u32, mip_count: u32) -> u32 {
    let w_px = (rect[2] - rect[0]) * width as f32;
    let h_px = (rect[3] - rect[1]) * height as f32;
    let extent = w_px.max(h_px).max(1.0);
    // ceil(log2(extent)) - 1: at 2 texels the 2x2 block already covers it.
    let level = extent.log2().ceil() as i32 - 1;
    level.clamp(0, mip_count as i32 - 1) as u32
}

/// Decide whether a candidate is hidden.
///
/// `furthest_depth` is the maximum of the (up to) four Hi-Z texels covering the
/// footprint. Occluded means the candidate's nearest point is strictly behind
/// everything recorded there.
pub fn is_occluded(bounds: &ScreenBounds, furthest_depth: f32) -> bool {
    // A cleared pyramid holds 1.0, the far plane, which occludes nothing.
    if furthest_depth >= 1.0 {
        return false;
    }
    // Matches cull.wgsl: a candidate that covers a quarter of the screen is
    // too close for four coarse Hi-Z samples to be a safe rejection.
    let area = (bounds.rect[2] - bounds.rect[0]) * (bounds.rect[3] - bounds.rect[1]);
    if area > 0.25 {
        return false;
    }
    bounds.min_depth > furthest_depth
}

#[cfg(test)]
mod hiz_tests {
    use super::*;

    fn persp() -> glam::Mat4 {
        let proj = glam::Mat4::perspective_rh(60f32.to_radians(), 16.0 / 9.0, 0.1, 1000.0);
        let view = glam::Mat4::look_at_rh(
            glam::Vec3::new(0.0, 0.0, 10.0),
            glam::Vec3::ZERO,
            glam::Vec3::Y,
        );
        proj * view
    }

    #[test]
    fn a_box_in_front_of_the_camera_projects_to_a_rect() {
        let b = project_aabb_to_screen([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0], persp()).unwrap();
        assert!(b.rect[0] < 0.5 && b.rect[2] > 0.5, "rect {:?}", b.rect);
        assert!(b.rect[1] < 0.5 && b.rect[3] > 0.5, "rect {:?}", b.rect);
        assert!(
            b.min_depth > 0.0 && b.min_depth < 1.0,
            "depth {}",
            b.min_depth
        );
    }

    #[test]
    fn a_box_behind_the_camera_is_not_projected() {
        // Behind the eye the divide flips signs and would produce a bogus rect.
        let b = project_aabb_to_screen([-1.0, -1.0, 20.0], [1.0, 1.0, 22.0], persp());
        assert!(b.is_none());
    }

    #[test]
    fn a_box_straddling_the_camera_plane_is_not_projected() {
        let b = project_aabb_to_screen([-1.0, -1.0, -1.0], [1.0, 1.0, 30.0], persp());
        assert!(b.is_none());
    }

    #[test]
    fn a_nearer_box_reports_a_smaller_depth() {
        let near = project_aabb_to_screen([-1.0, -1.0, 4.0], [1.0, 1.0, 5.0], persp()).unwrap();
        let far = project_aabb_to_screen([-1.0, -1.0, -50.0], [1.0, 1.0, -49.0], persp()).unwrap();
        assert!(near.min_depth < far.min_depth);
    }

    #[test]
    fn the_rect_is_clamped_to_the_screen() {
        let b =
            project_aabb_to_screen([-500.0, -500.0, -1.0], [500.0, 500.0, 1.0], persp()).unwrap();
        assert_eq!(b.rect, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn a_bigger_footprint_selects_a_coarser_level() {
        let small = hiz_mip_level([0.5, 0.5, 0.5039, 0.5039], 1024, 1024, 11); // ~4 px
        let large = hiz_mip_level([0.0, 0.0, 1.0, 1.0], 1024, 1024, 11); // 1024 px
        assert!(large > small, "small {small} large {large}");
    }

    #[test]
    fn a_footprint_of_two_texels_uses_the_base_level() {
        // 2 px across is already covered by one 2x2 block at level 0.
        assert_eq!(
            hiz_mip_level([0.0, 0.0, 2.0 / 1024.0, 2.0 / 1024.0], 1024, 1024, 11),
            0
        );
    }

    #[test]
    fn the_level_never_leaves_the_pyramid() {
        // A full-screen footprint must not index past the top level.
        let level = hiz_mip_level([0.0, 0.0, 1.0, 1.0], 4096, 4096, 5);
        assert!(level < 5, "level {level} outside a 5-level pyramid");
        // A degenerate rect must not produce a negative level.
        assert_eq!(hiz_mip_level([0.5, 0.5, 0.5, 0.5], 1024, 1024, 11), 0);
    }

    #[test]
    fn a_candidate_behind_the_recorded_depth_is_occluded() {
        let b = ScreenBounds {
            rect: [0.0, 0.0, 0.1, 0.1],
            min_depth: 0.8,
        };
        assert!(is_occluded(&b, 0.5));
    }

    #[test]
    fn a_candidate_in_front_of_the_recorded_depth_is_visible() {
        let b = ScreenBounds {
            rect: [0.0, 0.0, 0.1, 0.1],
            min_depth: 0.3,
        };
        assert!(!is_occluded(&b, 0.5));
    }

    #[test]
    fn an_empty_region_never_occludes() {
        // Cleared depth is 1.0. Even a candidate at the far plane must survive,
        // or the first frame would cull the entire scene.
        let b = ScreenBounds {
            rect: [0.0, 0.0, 1.0, 1.0],
            min_depth: 1.0,
        };
        assert!(!is_occluded(&b, 1.0));
    }

    #[test]
    fn equal_depths_count_as_visible() {
        // A candidate exactly coplanar with the occluder is the object itself on
        // the next frame; rejecting it would make geometry flicker out.
        let b = ScreenBounds {
            rect: [0.0, 0.0, 0.1, 0.1],
            min_depth: 0.5,
        };
        assert!(!is_occluded(&b, 0.5));
    }

    #[test]
    fn a_large_screen_footprint_is_never_occluded() {
        // Standing next to a scaled tree fills the view. The four Hi-Z samples
        // would otherwise hit neighbouring geometry and delete the tree.
        let b = ScreenBounds {
            rect: [0.0, 0.0, 0.6, 0.6],
            min_depth: 0.8,
        };
        assert!(!is_occluded(&b, 0.2));
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn cull_params_matches_the_wgsl_struct() {
        // `CullParams` in cull.wgsl, under std140-ish uniform rules:
        //   planes            array<vec4<f32>, 6>   offset   0, size 96
        //   draw_count        u32                   offset  96
        //   disabled          u32                   offset 100
        //   phase             u32                   offset 104
        //   occlusion_enabled u32                   offset 108
        //   view_proj         mat4x4<f32>           offset 112 (align 16), size 64
        //   hiz_size          vec2<f32>             offset 176
        //   hiz_mip_count     u32                   offset 184
        //   single_sided_args u32                   offset 188
        //   counted_draws     u32                   offset 192
        //   implicit padding                        offset 196, size 12
        //   camera_pos        vec4<f32>             offset 208, size 16
        //                                           total  224
        //
        // A mismatch here does not fail to compile or validate — the shader
        // simply reads the wrong words and culls the wrong things, which shows
        // up as geometry flickering out rather than as an error.
        assert_eq!(std::mem::size_of::<GpuCullParams>(), 224);
        assert_eq!(std::mem::align_of::<GpuCullParams>(), 4);

        let p = GpuCullParams {
            planes: [[0.0; 4]; 6],
            draw_count: 0,
            disabled: 0,
            phase: 0,
            occlusion_enabled: 0,
            view_proj: [[0.0; 4]; 4],
            hiz_size: [0.0; 2],
            hiz_mip_count: 0,
            single_sided_args: 0,
            counted_draws: 0,
            _pad: [0; 3],
            camera_pos: [0.0; 4],
        };
        let base = &p as *const _ as usize;
        let off = |field: *const _| field as usize - base;
        assert_eq!(off(&p.draw_count as *const _ as *const u8), 96);
        assert_eq!(off(&p.phase as *const _ as *const u8), 104);
        assert_eq!(off(&p.view_proj as *const _ as *const u8), 112);
        assert_eq!(off(&p.hiz_size as *const _ as *const u8), 176);
        assert_eq!(off(&p.hiz_mip_count as *const _ as *const u8), 184);
        assert_eq!(off(&p.single_sided_args as *const _ as *const u8), 188);
        assert_eq!(off(&p.counted_draws as *const _ as *const u8), 192);
        assert_eq!(off(&p.camera_pos as *const _ as *const u8), 208);
    }
}
