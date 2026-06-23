//! Chunk dimensions and coordinate math.
//!
//! A chunk is a 32³ block of voxels. Voxel data is stored **padded** to 34³ —
//! a 1-voxel border sampled from neighbouring terrain — so `block_mesh` can
//! cull faces across chunk boundaries without seam cracks
//! (bevy_voxel_world `chunk.rs` pattern, ATTRIBUTION.md §13.10).

use glam::{IVec3, Vec3};

/// Voxels along one edge of a chunk (inner, unpadded).
pub const CHUNK_SIZE: u32 = 32;
/// `CHUNK_SIZE` as f32 — world-space edge length in metres (1 voxel = 1 m).
pub const CHUNK_SIZE_F: f32 = CHUNK_SIZE as f32;
/// Inner size plus the 1-voxel border on each side.
pub const PADDED_CHUNK_SIZE: u32 = CHUNK_SIZE + 2;

/// Integer chunk coordinate. World-space origin of a chunk is `coord * 32`.
pub type ChunkCoord = IVec3;

/// World-space position (metres) of a chunk's minimum corner.
pub fn chunk_origin(coord: ChunkCoord) -> Vec3 {
    (coord * CHUNK_SIZE as i32).as_vec3()
}

/// Chunk coordinate containing the given world-space voxel coordinate.
pub fn chunk_of_voxel(voxel: IVec3) -> ChunkCoord {
    IVec3::new(
        voxel.x.div_euclid(CHUNK_SIZE as i32),
        voxel.y.div_euclid(CHUNK_SIZE as i32),
        voxel.z.div_euclid(CHUNK_SIZE as i32),
    )
}

/// All chunks whose **padded** voxel volume contains the given world voxel.
/// A voxel on a chunk border also lives in the padding of up to 7 neighbours,
/// which therefore need remeshing when it changes.
pub fn chunks_touching_voxel(voxel: IVec3) -> Vec<ChunkCoord> {
    let home = chunk_of_voxel(voxel);
    let local = voxel - home * CHUNK_SIZE as i32;
    let max = CHUNK_SIZE as i32 - 1;

    let mut coords = vec![home];
    let mut push_offset = |off: IVec3| {
        let c = home + off;
        if !coords.contains(&c) {
            coords.push(c);
        }
    };

    let xs: &[i32] = if local.x == 0 { &[-1] } else if local.x == max { &[1] } else { &[] };
    let ys: &[i32] = if local.y == 0 { &[-1] } else if local.y == max { &[1] } else { &[] };
    let zs: &[i32] = if local.z == 0 { &[-1] } else if local.z == max { &[1] } else { &[] };

    for &dx in xs.iter().chain(std::iter::once(&0)) {
        for &dy in ys.iter().chain(std::iter::once(&0)) {
            for &dz in zs.iter().chain(std::iter::once(&0)) {
                if (dx, dy, dz) != (0, 0, 0) {
                    push_offset(IVec3::new(dx, dy, dz));
                }
            }
        }
    }
    coords
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_of_negative_voxel() {
        assert_eq!(chunk_of_voxel(IVec3::new(-1, 0, 31)), IVec3::new(-1, 0, 0));
        assert_eq!(chunk_of_voxel(IVec3::new(-32, -33, 32)), IVec3::new(-1, -2, 1));
    }

    #[test]
    fn border_voxel_touches_neighbours() {
        // Interior voxel: only the home chunk.
        assert_eq!(chunks_touching_voxel(IVec3::new(5, 5, 5)).len(), 1);
        // Face voxel: home + 1 neighbour.
        assert_eq!(chunks_touching_voxel(IVec3::new(0, 5, 5)).len(), 2);
        // Corner voxel: home + 7 neighbours.
        assert_eq!(chunks_touching_voxel(IVec3::new(0, 0, 0)).len(), 8);
    }
}
