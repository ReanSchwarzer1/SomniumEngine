//! Chunk meshing via `block_mesh::visible_block_faces`, with LOD.
//!
//! Pattern ported from bevy_voxel_world `src/meshing.rs`
//! (ATTRIBUTION.md §13.10): voxel data is always generated at full 34³
//! resolution; for LOD levels > 0 it is downsampled nearest-neighbour to a
//! coarser padded grid before meshing, with the padded border kept aligned so
//! the outermost voxels are never eroded (reduces cracks at LOD boundaries).

use crate::chunk::{CHUNK_SIZE, CHUNK_SIZE_F, PADDED_CHUNK_SIZE};
use crate::voxel::{Voxel, PALETTE_SIZE};
use block_mesh::{visible_block_faces, UnitQuadBuffer, RIGHT_HANDED_Y_UP_CONFIG};
use ndshape::{RuntimeShape, Shape};
use somnium_asset::Vertex;

/// CPU mesh for one chunk, in chunk-local space (origin = chunk min corner).
/// Empty `vertices` means the chunk produced no visible faces.
#[derive(Debug, Default)]
pub struct ChunkMeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

/// Highest supported LOD level (0 = full res, each level halves the grid).
pub const MAX_LOD: u8 = 2;

/// Inner voxel grid size at a given LOD level (32 → 16 → 8).
pub fn lod_inner_size(lod: u8) -> u32 {
    CHUNK_SIZE >> lod.min(MAX_LOD)
}

/// Mesh one chunk from its padded 34³ voxel data.
///
/// `voxels` is laid out by `RuntimeShape::<u32,3>::new([34;3])` linearization.
/// Returns `None` when no face is visible (fully buried or all air).
pub fn mesh_chunk(voxels: &[Voxel], lod: u8) -> Option<ChunkMeshData> {
    debug_assert_eq!(
        voxels.len(),
        (PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE) as usize
    );

    let data_dim = PADDED_CHUNK_SIZE;
    let mesh_dim = lod_inner_size(lod) + 2;
    let mesh_shape = RuntimeShape::<u32, 3>::new([mesh_dim; 3]);

    let resampled;
    let mesh_voxels: &[Voxel] = if mesh_dim == data_dim {
        voxels
    } else {
        resampled = resample_nearest(voxels, data_dim, mesh_dim);
        &resampled
    };

    let faces = RIGHT_HANDED_Y_UP_CONFIG.faces;
    let mut buffer = UnitQuadBuffer::new();
    visible_block_faces(
        mesh_voxels,
        &mesh_shape,
        [0; 3],
        [mesh_dim - 1; 3],
        &faces,
        &mut buffer,
    );

    if buffer.num_quads() == 0 {
        return None;
    }

    let voxel_size = CHUNK_SIZE_F / lod_inner_size(lod) as f32;
    let num_vertices = buffer.num_quads() * 4;
    let mut vertices = Vec::with_capacity(num_vertices);
    let mut indices = Vec::with_capacity(buffer.num_quads() * 6);

    for (group, face) in buffer.groups.into_iter().zip(faces) {
        for unit_quad in group {
            let quad: block_mesh::UnorientedQuad = unit_quad.into();

            indices.extend_from_slice(&face.quad_mesh_indices(vertices.len() as u32));

            let voxel = mesh_voxels[mesh_shape.linearize(quad.minimum) as usize];
            let palette_u = (voxel.palette_index() as f32 + 0.5) / PALETTE_SIZE as f32;
            let normals = face.quad_mesh_normals();
            let corners = face.quad_corners(&quad);

            for (corner, normal) in corners.into_iter().zip(normals) {
                // Padded index → chunk-local metres: strip the 1-voxel border,
                // then scale by the LOD voxel size.
                let c = corner.as_vec3().to_array();
                vertices.push(Vertex {
                    position: [
                        (c[0] - 1.0) * voxel_size,
                        (c[1] - 1.0) * voxel_size,
                        (c[2] - 1.0) * voxel_size,
                    ],
                    normal,
                    uv: [palette_u, 0.5],
                });
            }
        }
    }

    Some(ChunkMeshData { vertices, indices })
}

/// Nearest-neighbour downsample of a padded cubic grid, keeping the padded
/// border mapped to the source border so face culling against neighbouring
/// chunks stays correct at every LOD.
fn resample_nearest(data: &[Voxel], data_dim: u32, mesh_dim: u32) -> Vec<Voxel> {
    let data_shape = RuntimeShape::<u32, 3>::new([data_dim; 3]);
    let mesh_shape = RuntimeShape::<u32, 3>::new([mesh_dim; 3]);

    let map_axis = |i: u32| -> u32 {
        if i == 0 {
            return 0;
        }
        if i >= mesh_dim - 1 {
            return data_dim - 1;
        }
        // Inner cells: proportional mapping between the unpadded grids.
        let mesh_inner = mesh_dim - 2;
        let data_inner = data_dim - 2;
        let mapped = ((i - 1) as f32 + 0.5) * data_inner as f32 / mesh_inner as f32;
        (mapped as u32 + 1).min(data_dim - 2)
    };

    let mut out = Vec::with_capacity(mesh_shape.size() as usize);
    for lin in 0..mesh_shape.size() {
        let [x, y, z] = mesh_shape.delinearize(lin);
        let src = data_shape.linearize([map_axis(x), map_axis(y), map_axis(z)]);
        out.push(data[src as usize]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndshape::{RuntimeShape, Shape};

    /// Padded grid filled below a flat height plane.
    fn flat_ground(height: i32) -> Vec<Voxel> {
        let shape = RuntimeShape::<u32, 3>::new([PADDED_CHUNK_SIZE; 3]);
        (0..shape.size())
            .map(|lin| {
                let [_, y, _] = shape.delinearize(lin);
                // padded y index 1..=32 maps to local voxel y 0..=31
                if (y as i32 - 1) <= height { Voxel::Grass } else { Voxel::Air }
            })
            .collect()
    }

    #[test]
    fn flat_chunk_meshes_top_faces() {
        let mesh = mesh_chunk(&flat_ground(10), 0).expect("non-empty mesh");
        // 32×32 top faces at minimum; sides are culled by solid padding.
        assert!(mesh.vertices.len() >= 32 * 32 * 4);
        assert_eq!(mesh.indices.len() % 6, 0);
        // Top surface must sit at y = 11 (one above the highest solid voxel).
        let max_y = mesh.vertices.iter().map(|v| v.position[1]).fold(f32::MIN, f32::max);
        assert_eq!(max_y, 11.0);
    }

    #[test]
    fn lod_chunk_spans_same_extent() {
        for lod in 0..=MAX_LOD {
            let mesh = mesh_chunk(&flat_ground(15), lod).expect("non-empty mesh");
            let max_x = mesh.vertices.iter().map(|v| v.position[0]).fold(f32::MIN, f32::max);
            assert_eq!(max_x, CHUNK_SIZE_F, "lod {lod} should span the full chunk");
        }
    }

    #[test]
    fn empty_and_buried_chunks_produce_no_mesh() {
        let all_air = vec![Voxel::Air; (PADDED_CHUNK_SIZE.pow(3)) as usize];
        assert!(mesh_chunk(&all_air, 0).is_none());
        let all_solid = vec![Voxel::Stone; (PADDED_CHUNK_SIZE.pow(3)) as usize];
        assert!(mesh_chunk(&all_solid, 0).is_none());
    }
}
