//! Phase 15D: meshlet (cluster) generation at mesh upload.
//!
//! A meshlet is a small, spatially coherent run of triangles — 128 of them
//! here, matching UE5 Nanite's `NANITE_MAX_CLUSTER_TRIANGLES`. Splitting a mesh
//! into clusters is what lets later phases cull and draw at a finer granularity
//! than a whole object: 15E rejects clusters against a Hi-Z pyramid, 15F issues
//! per-cluster indirect draws.
//!
//! ## Why the indices get reordered
//!
//! Each meshlet is stored as an *offset and count into the mesh's index range*,
//! not as a list of triangle indices. That only works if a meshlet's triangles
//! are contiguous, so [`build_meshlets`] returns a permuted index buffer and the
//! caller uploads that instead of the original. Triangle order within a draw has
//! no effect on the rendered image, so this is free.
//!
//! ## Clustering
//!
//! Nanite partitions a triangle adjacency graph with METIS. That is a lot of
//! machinery for the quality gained here, so this sorts triangles by the Morton
//! code of their centroid and cuts the resulting curve into fixed-size runs.
//! Morton order keeps points that are close in space close in the sequence, so
//! the runs come out compact — which is all the bounding volumes need to be
//! useful. It is also O(n log n), allocation-light, and deterministic.
//!
//! ## What each cluster carries
//!
//! - a bounding sphere, for frustum and occlusion rejection
//! - a **normal cone** (axis + cosine cutoff), for backface rejection of the
//!   whole cluster at once. A cluster whose normals all point within some angle
//!   of an axis can be skipped entirely when the camera is outside that cone.
//!   Clusters that fail to be cone-like get a cutoff of -1, which never culls.

use somnium_asset::Vertex;

/// Triangles per cluster. Matches UE5 Nanite's `NANITE_MAX_CLUSTER_TRIANGLES`.
pub const MAX_MESHLET_TRIANGLES: usize = 128;

/// One cluster of triangles within a mesh.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Meshlet {
    /// First triangle, counted from the start of the mesh's index range.
    pub triangle_offset: u32,
    /// Number of triangles, at most [`MAX_MESHLET_TRIANGLES`].
    pub triangle_count: u32,
    /// Bounding sphere centre, in mesh-local space.
    pub center: [f32; 3],
    /// Bounding sphere radius.
    pub radius: f32,
    /// Local-space AABB. Kept alongside the sphere because culling wants the
    /// tighter of the two: the sphere's own AABB is up to sqrt(3) larger per
    /// axis, enough that boundary clusters survive a frustum test their parent
    /// mesh fails — which measured as the cluster path submitting *more*
    /// geometry than whole-mesh draws at some viewpoints.
    pub aabb_min: [f32; 3],
    pub aabb_max: [f32; 3],
    /// Average triangle normal, normalized. Zero when the cluster has no
    /// consistent facing.
    pub cone_axis: [f32; 3],
    /// Cosine of the cone's half-angle. `-1.0` disables cone culling for this
    /// cluster, which is the safe answer when the normals disagree too much.
    pub cone_cutoff: f32,
}

impl Meshlet {
    /// Index offset of this cluster's first index, relative to the mesh.
    pub fn index_offset(&self) -> u32 {
        self.triangle_offset * 3
    }

    /// Number of indices this cluster covers.
    pub fn index_count(&self) -> u32 {
        self.triangle_count * 3
    }

    /// Threshold for rejecting the whole cluster as backfacing (Phase 15F).
    ///
    /// `cone_cutoff` stores the tightest cosine between any triangle normal and
    /// the cone axis — it describes the cone, not the test. A cluster is
    /// invisible only when the view direction faces away from *every* normal in
    /// it, and the worst case is the normal tilted furthest toward the camera,
    /// which works out to `dot(view_dir, axis) >= sin(half_angle)`.
    ///
    /// So a perfectly flat cluster (`cone_cutoff` 1) yields 0: it is hidden the
    /// moment you are behind it. A cluster spanning a full hemisphere yields 1
    /// and is essentially never rejected. Using `cone_cutoff` directly here
    /// would invert that and cull flat clusters almost never while culling
    /// wide ones aggressively — the exact opposite of what is wanted.
    ///
    /// Returns `2.0` when the cluster spans more than a hemisphere: a dot
    /// product can never reach it, so the test simply never fires and no branch
    /// is needed in the shader.
    pub fn backface_cutoff(&self) -> f32 {
        if self.cone_cutoff <= 0.0 {
            return 2.0;
        }
        (1.0 - self.cone_cutoff * self.cone_cutoff).max(0.0).sqrt()
    }
}

/// Result of clustering one mesh.
pub struct MeshletBuild {
    pub meshlets: Vec<Meshlet>,
    /// The mesh's indices, permuted so every meshlet is a contiguous run.
    /// A permutation of whole triangles — the same triangles, reordered.
    pub indices: Vec<u32>,
}

/// Split a mesh into clusters and return the reordered index buffer.
///
/// Indices whose count is not a multiple of three have the trailing partial
/// triangle dropped, and triangles referencing a vertex outside `vertices` are
/// skipped: a malformed mesh should lose a triangle, not panic the uploader.
pub fn build_meshlets(vertices: &[Vertex], indices: &[u32]) -> MeshletBuild {
    let triangle_count = indices.len() / 3;
    if triangle_count == 0 || vertices.is_empty() {
        return MeshletBuild {
            meshlets: Vec::new(),
            indices: Vec::new(),
        };
    }

    // Centroid per triangle, plus the scene bounds needed to normalize before
    // quantizing to Morton codes.
    let mut centroids: Vec<[f32; 3]> = Vec::with_capacity(triangle_count);
    let mut valid: Vec<usize> = Vec::with_capacity(triangle_count);
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];

    for tri in 0..triangle_count {
        let i0 = indices[tri * 3] as usize;
        let i1 = indices[tri * 3 + 1] as usize;
        let i2 = indices[tri * 3 + 2] as usize;
        if i0 >= vertices.len() || i1 >= vertices.len() || i2 >= vertices.len() {
            continue;
        }
        let (p0, p1, p2) = (
            vertices[i0].position,
            vertices[i1].position,
            vertices[i2].position,
        );
        let c = [
            (p0[0] + p1[0] + p2[0]) / 3.0,
            (p0[1] + p1[1] + p2[1]) / 3.0,
            (p0[2] + p1[2] + p2[2]) / 3.0,
        ];
        if !c.iter().all(|v| v.is_finite()) {
            continue;
        }
        for axis in 0..3 {
            min[axis] = min[axis].min(c[axis]);
            max[axis] = max[axis].max(c[axis]);
        }
        centroids.push(c);
        valid.push(tri);
    }

    if valid.is_empty() {
        return MeshletBuild {
            meshlets: Vec::new(),
            indices: Vec::new(),
        };
    }

    // Sort along the Morton curve. A degenerate axis (a flat mesh) has zero
    // extent, so the reciprocal is guarded — every centroid then maps to the
    // same coordinate on that axis, which is correct.
    let extent = [
        (max[0] - min[0]).max(1e-6),
        (max[1] - min[1]).max(1e-6),
        (max[2] - min[2]).max(1e-6),
    ];
    let mut order: Vec<(u32, usize)> = valid
        .iter()
        .enumerate()
        .map(|(slot, &tri)| {
            let c = centroids[slot];
            let q =
                |v: f32, lo: f32, ext: f32| (((v - lo) / ext) * 1023.0).clamp(0.0, 1023.0) as u32;
            let code = morton3(
                q(c[0], min[0], extent[0]),
                q(c[1], min[1], extent[1]),
                q(c[2], min[2], extent[2]),
            );
            (code, tri)
        })
        .collect();
    // Tie-break on the original triangle index so the output is deterministic
    // for meshes with coincident centroids.
    order.sort_unstable_by_key(|&(code, tri)| (code, tri));

    // Emit the permuted index buffer and cut it into fixed-size runs.
    let mut out_indices: Vec<u32> = Vec::with_capacity(order.len() * 3);
    for &(_, tri) in &order {
        out_indices.extend_from_slice(&indices[tri * 3..tri * 3 + 3]);
    }

    let mut meshlets = Vec::with_capacity(order.len().div_ceil(MAX_MESHLET_TRIANGLES));
    let mut start = 0usize;
    while start < order.len() {
        let count = MAX_MESHLET_TRIANGLES.min(order.len() - start);
        meshlets.push(build_one(
            vertices,
            &out_indices[start * 3..(start + count) * 3],
            start as u32,
            count as u32,
        ));
        start += count;
    }

    MeshletBuild {
        meshlets,
        indices: out_indices,
    }
}

/// Bounding sphere and normal cone for one contiguous run of triangles.
fn build_one(
    vertices: &[Vertex],
    tri_indices: &[u32],
    triangle_offset: u32,
    triangle_count: u32,
) -> Meshlet {
    // Bounding sphere from the AABB centre. Not the minimal sphere, but it is
    // conservative, which is the only hard requirement for a culling volume.
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for &i in tri_indices {
        let p = vertices[i as usize].position;
        for axis in 0..3 {
            min[axis] = min[axis].min(p[axis]);
            max[axis] = max[axis].max(p[axis]);
        }
    }
    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let mut radius = 0.0f32;
    for &i in tri_indices {
        let p = vertices[i as usize].position;
        let d = [p[0] - center[0], p[1] - center[1], p[2] - center[2]];
        radius = radius.max((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt());
    }

    // Cone axis: the average geometric normal.
    let mut axis = [0.0f32; 3];
    for t in 0..tri_indices.len() / 3 {
        let n = triangle_normal(
            vertices[tri_indices[t * 3] as usize].position,
            vertices[tri_indices[t * 3 + 1] as usize].position,
            vertices[tri_indices[t * 3 + 2] as usize].position,
        );
        for a in 0..3 {
            axis[a] += n[a];
        }
    }
    let axis_len = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();

    // A cluster whose normals cancel out has no meaningful facing, so it must
    // never be cone-culled.
    //
    // The sentinel for that is 2.0, not -1.0. `cull.wgsl` skips the cone test
    // entirely when `cone.w > 1.0` — unreachable by a dot product — but -1.0
    // *passes* that guard and enters the test with a zero axis, where
    // `normalize(vec3(0))` is NaN and the cluster is rejected.
    //
    // A cube hits this every time: its six face normals cancel exactly, so all
    // twelve triangles land in one meshlet with a zero-sum axis and the whole
    // mesh disappears. Planes survived because their normals all point one way,
    // which is why primitives seemed to work until a cube was involved.
    if axis_len < 1e-6 {
        return Meshlet {
            triangle_offset,
            triangle_count,
            center,
            radius,
            aabb_min: min,
            aabb_max: max,
            cone_axis: [0.0, 1.0, 0.0],
            cone_cutoff: 2.0,
        };
    }
    let axis = [axis[0] / axis_len, axis[1] / axis_len, axis[2] / axis_len];

    // Widest deviation from the axis sets the half-angle.
    let mut min_dot = 1.0f32;
    for t in 0..tri_indices.len() / 3 {
        let n = triangle_normal(
            vertices[tri_indices[t * 3] as usize].position,
            vertices[tri_indices[t * 3 + 1] as usize].position,
            vertices[tri_indices[t * 3 + 2] as usize].position,
        );
        // Degenerate triangles have no normal and should not widen the cone.
        if n == [0.0, 0.0, 0.0] {
            continue;
        }
        min_dot = min_dot.min(axis[0] * n[0] + axis[1] * n[1] + axis[2] * n[2]);
    }

    Meshlet {
        triangle_offset,
        triangle_count,
        center,
        radius,
        aabb_min: min,
        aabb_max: max,
        cone_axis: axis,
        // Spread beyond a hemisphere cannot be culled by a cone at all.
        cone_cutoff: if min_dot <= 0.0 { -1.0 } else { min_dot },
    }
}

/// Unnormalized-safe triangle normal. Returns zero for degenerate triangles.
fn triangle_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        e1[1] * e2[2] - e1[2] * e2[1],
        e1[2] * e2[0] - e1[0] * e2[2],
        e1[0] * e2[1] - e1[1] * e2[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len < 1e-12 || !len.is_finite() {
        [0.0, 0.0, 0.0]
    } else {
        [n[0] / len, n[1] / len, n[2] / len]
    }
}

/// Interleave the low 10 bits of three coordinates into a 30-bit Morton code.
fn morton3(x: u32, y: u32, z: u32) -> u32 {
    (spread_bits(x) << 2) | (spread_bits(y) << 1) | spread_bits(z)
}

/// Spread 10 bits so each occupies every third position: `abc` → `00a00b00c`.
fn spread_bits(mut v: u32) -> u32 {
    v &= 0x0000_03ff;
    v = (v | (v << 16)) & 0x030000ff;
    v = (v | (v << 8)) & 0x0300f00f;
    v = (v | (v << 4)) & 0x030c30c3;
    v = (v | (v << 2)) & 0x09249249;
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(p: [f32; 3]) -> Vertex {
        Vertex {
            position: p,
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
        }
    }

    /// A strip of `n` axis-aligned triangles in the XZ plane, all facing +Y.
    fn grid(n: usize) -> (Vec<Vertex>, Vec<u32>) {
        let mut verts = Vec::new();
        let mut idx = Vec::new();
        for i in 0..n {
            let x = i as f32;
            let base = verts.len() as u32;
            verts.push(v([x, 0.0, 0.0]));
            verts.push(v([x, 0.0, 1.0]));
            verts.push(v([x + 1.0, 0.0, 0.0]));
            // Wound so the normal is +Y.
            idx.extend_from_slice(&[base, base + 1, base + 2]);
        }
        (verts, idx)
    }

    /// Triangles as unordered vertex triples, for permutation comparisons.
    fn tri_set(indices: &[u32]) -> Vec<[u32; 3]> {
        let mut t: Vec<[u32; 3]> = indices
            .chunks_exact(3)
            .map(|c| {
                let mut a = [c[0], c[1], c[2]];
                a.sort_unstable();
                a
            })
            .collect();
        t.sort_unstable();
        t
    }

    #[test]
    fn an_empty_mesh_produces_no_meshlets() {
        let b = build_meshlets(&[], &[]);
        assert!(b.meshlets.is_empty());
        assert!(b.indices.is_empty());
    }

    #[test]
    fn a_small_mesh_is_one_meshlet() {
        let (verts, idx) = grid(10);
        let b = build_meshlets(&verts, &idx);
        assert_eq!(b.meshlets.len(), 1);
        assert_eq!(b.meshlets[0].triangle_offset, 0);
        assert_eq!(b.meshlets[0].triangle_count, 10);
    }

    #[test]
    fn a_mesh_at_the_cap_is_still_one_meshlet() {
        let (verts, idx) = grid(MAX_MESHLET_TRIANGLES);
        let b = build_meshlets(&verts, &idx);
        assert_eq!(b.meshlets.len(), 1);
        assert_eq!(b.meshlets[0].triangle_count, MAX_MESHLET_TRIANGLES as u32);
    }

    #[test]
    fn a_large_mesh_splits_into_contiguous_runs() {
        let (verts, idx) = grid(300);
        let b = build_meshlets(&verts, &idx);
        assert_eq!(b.meshlets.len(), 3);
        assert_eq!(
            b.meshlets
                .iter()
                .map(|m| m.triangle_count)
                .collect::<Vec<_>>(),
            vec![128, 128, 44],
        );
        // Offsets must tile the range with no gap and no overlap: 15F draws
        // straight from them, so a gap would silently drop triangles.
        let mut expected = 0;
        for m in &b.meshlets {
            assert_eq!(m.triangle_offset, expected);
            expected += m.triangle_count;
        }
        assert_eq!(expected, 300);
    }

    #[test]
    fn reordering_preserves_every_triangle() {
        let (verts, idx) = grid(300);
        let b = build_meshlets(&verts, &idx);
        assert_eq!(b.indices.len(), idx.len());
        assert_eq!(tri_set(&b.indices), tri_set(&idx));
    }

    #[test]
    fn index_offset_and_count_are_in_indices_not_triangles() {
        let (verts, idx) = grid(200);
        let b = build_meshlets(&verts, &idx);
        let last = b.meshlets.last().unwrap();
        assert_eq!(last.index_offset(), last.triangle_offset * 3);
        assert_eq!(last.index_count(), last.triangle_count * 3);
        assert_eq!(
            (last.index_offset() + last.index_count()) as usize,
            b.indices.len(),
        );
    }

    #[test]
    fn the_bounding_sphere_contains_every_vertex_it_covers() {
        let (verts, idx) = grid(300);
        let b = build_meshlets(&verts, &idx);
        for m in &b.meshlets {
            let lo = m.index_offset() as usize;
            let hi = lo + m.index_count() as usize;
            for &i in &b.indices[lo..hi] {
                let p = verts[i as usize].position;
                let d = [p[0] - m.center[0], p[1] - m.center[1], p[2] - m.center[2]];
                let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                assert!(
                    dist <= m.radius + 1e-4,
                    "vertex {dist} outside radius {}",
                    m.radius,
                );
            }
        }
    }

    #[test]
    fn a_flat_cluster_gets_a_tight_cone() {
        let (verts, idx) = grid(20);
        let b = build_meshlets(&verts, &idx);
        let m = b.meshlets[0];
        // All triangles face +Y, so the cone collapses onto that axis.
        assert!(
            (m.cone_axis[1] - 1.0).abs() < 1e-4,
            "axis {:?}",
            m.cone_axis
        );
        assert!(m.cone_cutoff > 0.999, "cutoff {}", m.cone_cutoff);
    }

    #[test]
    fn opposing_normals_disable_cone_culling() {
        // Two triangles facing exactly opposite ways: the average normal
        // cancels, and no cone can describe them.
        let verts = vec![
            v([0.0, 0.0, 0.0]),
            v([0.0, 0.0, 1.0]),
            v([1.0, 0.0, 0.0]),
            v([0.0, 1.0, 0.0]),
            v([1.0, 1.0, 0.0]),
            v([0.0, 1.0, 1.0]),
        ];
        let idx = vec![0, 1, 2, 3, 4, 5];
        let b = build_meshlets(&verts, &idx);
        assert_eq!(b.meshlets.len(), 1);
        // 2.0, not -1.0. `cull.wgsl` skips the cone test only when
        // `cone.w > 1.0`; -1.0 passes that guard and enters the test with a
        // zero axis, where `normalize(vec3(0))` is NaN and the cluster is
        // dropped. This test asserted the sentinel that caused the bug, so it
        // passed while every cube in the engine vanished — a cube's six face
        // normals cancel exactly, which is precisely this case.
        assert_eq!(b.meshlets[0].cone_cutoff, 2.0);
        assert!(
            b.meshlets[0].cone_cutoff > 1.0,
            "the sentinel must exceed the guard in cull.wgsl, or the test is              not checking the thing that matters",
        );
    }

    #[test]
    fn out_of_range_indices_are_dropped_not_panicked_on() {
        let (mut verts, mut idx) = grid(4);
        verts.truncate(6); // now only triangles 0 and 1 are addressable
        idx.extend_from_slice(&[9999, 10000, 10001]);
        let b = build_meshlets(&verts, &idx);
        // Two valid triangles survive; the rest reference missing vertices.
        assert_eq!(b.meshlets.len(), 1);
        assert_eq!(b.meshlets[0].triangle_count, 2);
        assert_eq!(b.indices.len(), 6);
    }

    #[test]
    fn a_trailing_partial_triangle_is_ignored() {
        let (verts, mut idx) = grid(3);
        idx.push(0); // 10 indices: 3 triangles and one stray
        let b = build_meshlets(&verts, &idx);
        assert_eq!(b.meshlets[0].triangle_count, 3);
        assert_eq!(b.indices.len(), 9);
    }

    #[test]
    fn morton_order_keeps_neighbours_together() {
        // Two clusters of points far apart on X. Morton sorting must not
        // interleave them, or the bounding spheres would both span the gap.
        let mut verts = Vec::new();
        let mut idx = Vec::new();
        for group in 0..2 {
            for i in 0..MAX_MESHLET_TRIANGLES {
                let x = group as f32 * 1000.0 + i as f32 * 0.01;
                let base = verts.len() as u32;
                verts.push(v([x, 0.0, 0.0]));
                verts.push(v([x, 0.0, 1.0]));
                verts.push(v([x + 0.01, 0.0, 0.0]));
                idx.extend_from_slice(&[base, base + 1, base + 2]);
            }
        }
        let b = build_meshlets(&verts, &idx);
        assert_eq!(b.meshlets.len(), 2);
        // Each cluster stays local; a mixed cluster would need radius ~500.
        for m in &b.meshlets {
            assert!(m.radius < 10.0, "cluster spans {}", m.radius);
        }
    }

    #[test]
    fn nan_positions_do_not_produce_meshlets() {
        let verts = vec![
            v([f32::NAN, 0.0, 0.0]),
            v([0.0, 0.0, 1.0]),
            v([1.0, 0.0, 0.0]),
        ];
        let b = build_meshlets(&verts, &[0, 1, 2]);
        assert!(b.meshlets.is_empty());
    }

    #[test]
    fn the_aabb_is_tighter_than_the_bounding_sphere() {
        // The sphere is derived from the AABB, so its own AABB is always at
        // least as large. Culling uses the box for exactly this reason.
        let (verts, idx) = grid(60);
        for m in build_meshlets(&verts, &idx).meshlets {
            for a in 0..3 {
                assert!(m.aabb_min[a] >= m.center[a] - m.radius - 1e-4);
                assert!(m.aabb_max[a] <= m.center[a] + m.radius + 1e-4);
            }
        }
    }

    #[test]
    fn the_aabb_contains_every_vertex_it_covers() {
        let (verts, idx) = grid(300);
        let b = build_meshlets(&verts, &idx);
        for m in &b.meshlets {
            let lo = m.index_offset() as usize;
            let hi = lo + m.index_count() as usize;
            for &i in &b.indices[lo..hi] {
                let p = verts[i as usize].position;
                for a in 0..3 {
                    assert!(p[a] >= m.aabb_min[a] - 1e-4 && p[a] <= m.aabb_max[a] + 1e-4);
                }
            }
        }
    }

    #[test]
    fn a_flat_cluster_is_hidden_as_soon_as_you_are_behind_it() {
        let (verts, idx) = grid(20);
        let m = build_meshlets(&verts, &idx).meshlets[0];
        // cone_cutoff ~1 (all normals identical) -> cutoff ~0, so any view
        // direction with a positive dot against the axis rejects it.
        assert!(m.backface_cutoff() < 1e-3, "cutoff {}", m.backface_cutoff());
    }

    #[test]
    fn a_cluster_spanning_a_hemisphere_is_never_rejected() {
        let m = Meshlet {
            triangle_offset: 0,
            triangle_count: 1,
            center: [0.0; 3],
            radius: 1.0,
            aabb_min: [-1.0; 3],
            aabb_max: [1.0; 3],
            cone_axis: [0.0, 1.0, 0.0],
            cone_cutoff: -1.0,
        };
        // 2.0 is unreachable by a dot product, so the test never fires.
        assert_eq!(m.backface_cutoff(), 2.0);
    }

    #[test]
    fn a_wider_cone_is_harder_to_reject() {
        let mk = |c: f32| Meshlet {
            triangle_offset: 0,
            triangle_count: 1,
            center: [0.0; 3],
            radius: 1.0,
            aabb_min: [-1.0; 3],
            aabb_max: [1.0; 3],
            cone_axis: [0.0, 1.0, 0.0],
            cone_cutoff: c,
        };
        // Tighter cone (cutoff nearer 1) -> smaller threshold -> culls sooner.
        assert!(mk(0.99).backface_cutoff() < mk(0.5).backface_cutoff());
        assert!(mk(0.5).backface_cutoff() < mk(0.1).backface_cutoff());
        // sin(60 degrees) for a cone half-angle of 60 degrees.
        assert!((mk(0.5).backface_cutoff() - 0.8660254).abs() < 1e-5);
    }

    #[test]
    fn morton_interleaves_bits_in_xyz_order() {
        assert_eq!(morton3(0, 0, 0), 0);
        assert_eq!(morton3(1, 0, 0), 0b100);
        assert_eq!(morton3(0, 1, 0), 0b010);
        assert_eq!(morton3(0, 0, 1), 0b001);
        assert_eq!(morton3(1, 1, 1), 0b111);
        // Only the low 10 bits of each axis participate.
        assert_eq!(morton3(1 << 10, 0, 0), 0);
    }
}
