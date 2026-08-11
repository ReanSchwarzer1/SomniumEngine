//! Terrain chunk mesh generation (Phase 14B).
//!
//! ## Reference Architecture
//!
//! - `example_repo/fyrox/Fyrox-master/fyrox-impl/src/scene/terrain/geometry.rs` —
//!   grid mesh construction, quad → 2-triangle emission order.
//! - `example_repo/CDLOD-master` (Filip Strugar) — power-of-two LOD subdivision
//!   so vertices of a coarse level are always a subset of the fine level.
//!
//! The T-junction stitching here is a CPU-side scheme derived from the Fyrox
//! approach (pre-built index topology per LOD, no GPU morphing): every chunk
//! is triangulated in 2×2-cell blocks as a triangle fan around the block
//! center. A block's boundary ring normally has 8 vertices (4 corners + 4 edge
//! midpoints). When a block edge lies on a chunk border whose neighbor renders
//! at a coarser LOD, the midpoint of that edge is omitted from the ring, so
//! the border vertices exactly match the neighbor's coarser vertex spacing —
//! watertight as long as adjacent chunks differ by at most one LOD level
//! (enforced by [`super::TerrainData::select_lods`]).

use somnium_asset::Vertex;

/// Neighbor-is-coarser flags for the four chunk edges.
pub const EDGE_WEST: u8 = 1 << 0; // -X
pub const EDGE_EAST: u8 = 1 << 1; // +X
pub const EDGE_NORTH: u8 = 1 << 2; // -Z
pub const EDGE_SOUTH: u8 = 1 << 3; // +Z

/// Number of LOD levels (0 = full detail … `MAX_LOD` = coarsest).
pub const MAX_TERRAIN_LOD: u8 = 4;

/// Generate the full-resolution vertex grid for one chunk.
///
/// Positions are in terrain-local space (terrain origin = vertex (0, 0) of
/// chunk (0, 0)). Normals are central differences over the global heightmap so
/// chunk borders shade identically on both sides. UV is the terrain-global
/// [0, 1] coordinate used for splatmap lookup.
#[allow(clippy::too_many_arguments)]
pub fn build_chunk_vertices(
    heightmap: &[f32],
    total_x: u32,
    total_z: u32,
    chunk_cells: u32,
    grid_pos: [u32; 2],
    cell_size: f32,
    height_scale: f32,
) -> Vec<Vertex> {
    let base_x = grid_pos[0] * chunk_cells;
    let base_z = grid_pos[1] * chunk_cells;
    let verts_per_edge = chunk_cells + 1;

    let height_at = |xi: i64, zi: i64| -> f32 {
        let xi = xi.clamp(0, total_x as i64 - 1) as usize;
        let zi = zi.clamp(0, total_z as i64 - 1) as usize;
        heightmap[zi * total_x as usize + xi]
    };

    let mut vertices = Vec::with_capacity((verts_per_edge * verts_per_edge) as usize);
    for lz in 0..verts_per_edge {
        let zi = (base_z + lz) as i64;
        for lx in 0..verts_per_edge {
            let xi = (base_x + lx) as i64;
            let h = height_at(xi, zi) * height_scale;

            // Finite differences from neighboring heights (Phase 14B-1 formula).
            let dx =
                (height_at(xi + 1, zi) - height_at(xi - 1, zi)) * height_scale / (2.0 * cell_size);
            let dz =
                (height_at(xi, zi + 1) - height_at(xi, zi - 1)) * height_scale / (2.0 * cell_size);
            let normal = glam::Vec3::new(-dx, 1.0, -dz).normalize();

            vertices.push(Vertex {
                position: [xi as f32 * cell_size, h, zi as f32 * cell_size],
                normal: normal.to_array(),
                uv: [
                    xi as f32 / (total_x - 1) as f32,
                    zi as f32 / (total_z - 1) as f32,
                ],
            });
        }
    }
    vertices
}

/// Build the index list for one (LOD, edge-mask) combination.
///
/// Indices reference the full-resolution `(chunk_cells+1)²` vertex grid of a
/// chunk; coarser LODs skip vertices with stride `1 << lod`. All chunks share
/// the same topology, so the resulting buffers are cached terrain-wide.
pub fn build_lod_indices(chunk_cells: u32, lod: u8, edge_mask: u8) -> Vec<u32> {
    let step = 1u32 << lod;
    let cells = chunk_cells >> lod;
    debug_assert!(
        cells >= 2 && cells % 2 == 0,
        "chunk_cells must keep an even cell count at every LOD"
    );
    let verts_per_row = chunk_cells + 1;
    // Map LOD-cell coordinates → full-resolution vertex index.
    let vi = |xc: u32, zc: u32| -> u32 { (zc * step) * verts_per_row + xc * step };

    let on_stitched_edge = |p: (u32, u32)| -> bool {
        (p.0 == 0 && edge_mask & EDGE_WEST != 0)
            || (p.0 == cells && edge_mask & EDGE_EAST != 0)
            || (p.1 == 0 && edge_mask & EDGE_NORTH != 0)
            || (p.1 == cells && edge_mask & EDGE_SOUTH != 0)
    };

    let mut indices = Vec::with_capacity((cells * cells * 6) as usize);
    let mut ring: Vec<(u32, u32)> = Vec::with_capacity(8);

    for bz in (0..cells).step_by(2) {
        for bx in (0..cells).step_by(2) {
            // Boundary ring of the 2×2-cell block: corners and edge midpoints.
            let ring8 = [
                (bx, bz),
                (bx + 1, bz), // north midpoint
                (bx + 2, bz),
                (bx + 2, bz + 1), // east midpoint
                (bx + 2, bz + 2),
                (bx + 1, bz + 2), // south midpoint
                (bx, bz + 2),
                (bx, bz + 1), // west midpoint
            ];
            ring.clear();
            for (i, p) in ring8.iter().enumerate() {
                let is_midpoint = i % 2 == 1;
                if is_midpoint && on_stitched_edge(*p) {
                    continue; // snap border to the coarser neighbor's spacing
                }
                ring.push(*p);
            }

            // Wound counter-clockwise seen from above (+Y).
            //
            // The ring is built +X then +Z, which traces *clockwise* in the XZ
            // plane viewed from above, so emitting `[center, a, b]` made every
            // terrain triangle a back face. The old terrain pass set
            // `cull_mode: None` — with a comment saying the winding was
            // "uniform but unverified" — which hid it completely. Phase 25A-2
            // put terrain in the visibility pass, which back-face culls, and
            // the whole surface disappeared: a flat terrain rendered nothing at
            // all, and a sculpted one showed only the slopes whose underside
            // faced the camera.
            let center = vi(bx + 1, bz + 1);
            for i in 0..ring.len() {
                let a = ring[i];
                let b = ring[(i + 1) % ring.len()];
                indices.extend_from_slice(&[center, vi(b.0, b.1), vi(a.0, a.1)]);
            }
        }
    }
    indices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_detail_triangle_count_matches_regular_grid() {
        for lod in 0..=MAX_TERRAIN_LOD {
            let cells = 64u32 >> lod;
            let indices = build_lod_indices(64, lod, 0);
            // 8 fan triangles per 2×2 block == 2 triangles per cell.
            assert_eq!(indices.len() as u32, cells * cells * 2 * 3, "lod {lod}");
            let max = *indices.iter().max().unwrap();
            assert!(max < 65 * 65, "index out of range at lod {lod}");
        }
    }

    #[test]
    fn every_triangle_faces_up() {
        // The failure this guards against does not look like a winding bug: the
        // terrain simply is not there. The old terrain pass drew with culling
        // off, so a back-facing surface still rendered, and the fault only
        // surfaced when Phase 25A-2 moved terrain into the back-face-culling
        // visibility pass and a flat terrain rendered zero pixels.
        let heightmap = vec![0.0f32; 129 * 129];
        let verts = build_chunk_vertices(&heightmap, 129, 129, 64, [0, 0], 1.0, 1.0);
        for lod in 0..=MAX_TERRAIN_LOD {
            for mask in [0u8, EDGE_WEST, EDGE_EAST | EDGE_SOUTH, 0b1111] {
                let indices = build_lod_indices(64, lod, mask);
                for tri in indices.chunks_exact(3) {
                    let p = |i: u32| glam::Vec3::from(verts[i as usize].position);
                    let (a, b, c) = (p(tri[0]), p(tri[1]), p(tri[2]));
                    // Counter-clockwise seen from +Y gives a +Y face normal,
                    // which is what `FrontFace::Ccw` expects of ground.
                    let normal = (b - a).cross(c - a);
                    assert!(
                        normal.y > 0.0,
                        "lod {lod} mask {mask:04b}: triangle {tri:?} faces down (n.y = {})",
                        normal.y,
                    );
                }
            }
        }
    }

    #[test]
    fn stitched_edge_omits_fine_border_vertices() {
        // LOD 0 with a coarser WEST neighbor: no referenced vertex may sit on
        // the west edge (x == 0) at an odd z — those rows only exist at the
        // fine resolution and would crack against the neighbor.
        let indices = build_lod_indices(64, 0, EDGE_WEST);
        for &i in &indices {
            let x = i % 65;
            let z = i / 65;
            if x == 0 {
                assert_eq!(
                    z % 2,
                    0,
                    "fine-only vertex (0, {z}) referenced on stitched edge"
                );
            }
        }
        // Unstitched east edge keeps full resolution: odd-z verts referenced.
        let east_odd = indices.iter().any(|&i| i % 65 == 64 && (i / 65) % 2 == 1);
        assert!(east_odd, "unstitched edge lost resolution");
    }

    #[test]
    fn vertices_cover_chunk_and_normals_are_up_on_flat_ground() {
        let heightmap = vec![0.0f32; 129 * 129];
        let verts = build_chunk_vertices(&heightmap, 129, 129, 64, [1, 1], 1.0, 1.0);
        assert_eq!(verts.len(), 65 * 65);
        assert_eq!(verts[0].position, [64.0, 0.0, 64.0]);
        for v in &verts {
            assert_eq!(v.normal, [0.0, 1.0, 0.0]);
        }
    }

    #[test]
    fn border_vertices_match_between_adjacent_chunks() {
        // Chunk (0,0) east edge and chunk (1,0) west edge must produce
        // identical positions/normals — they sample the same global heightmap.
        let mut heightmap = vec![0.0f32; 129 * 129];
        for (i, h) in heightmap.iter_mut().enumerate() {
            *h = ((i * 31) % 97) as f32 * 0.13;
        }
        let a = build_chunk_vertices(&heightmap, 129, 129, 64, [0, 0], 1.0, 2.0);
        let b = build_chunk_vertices(&heightmap, 129, 129, 64, [1, 0], 1.0, 2.0);
        for z in 0..65u32 {
            let ea = a[(z * 65 + 64) as usize];
            let wb = b[(z * 65) as usize];
            assert_eq!(ea.position, wb.position);
            assert_eq!(ea.normal, wb.normal);
        }
    }

    #[test]
    fn sinusoidal_hill_has_continuous_finite_normals_across_chunks() {
        let n = 129u32;
        let mut heightmap = vec![0.0f32; (n * n) as usize];
        for z in 0..n {
            for x in 0..n {
                let xf = x as f32 / (n - 1) as f32 * std::f32::consts::TAU;
                let zf = z as f32 / (n - 1) as f32 * std::f32::consts::TAU;
                heightmap[(z * n + x) as usize] = (xf.sin() * zf.sin() + 1.0) * 0.5;
            }
        }
        let a = build_chunk_vertices(&heightmap, n, n, 64, [0, 0], 1.0, 20.0);
        let b = build_chunk_vertices(&heightmap, n, n, 64, [1, 0], 1.0, 20.0);
        for z in 0..65u32 {
            let left = a[(z * 65 + 64) as usize];
            let right = b[(z * 65) as usize];
            assert_eq!(left.position, right.position);
            assert_eq!(left.normal, right.normal);
            let normal = glam::Vec3::from(left.normal);
            assert!(normal.is_finite());
            assert!((normal.length() - 1.0).abs() < 1e-5);
        }
    }
}
