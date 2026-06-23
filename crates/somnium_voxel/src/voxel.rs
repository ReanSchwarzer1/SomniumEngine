//! Voxel cell type and its `block_mesh` trait implementations.

use block_mesh::{MergeVoxel, Voxel as BlockMeshVoxel, VoxelVisibility};

/// One voxel cell. `Air` is empty; every other variant is an opaque cube.
///
/// The discriminant doubles as the palette texel index used by the demo's
/// voxel material (see `Voxel::palette_index`), so the order here must match
/// the palette texture built by the integration layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum Voxel {
    #[default]
    Air = 0,
    Grass = 1,
    Dirt = 2,
    Stone = 3,
    Sand = 4,
    Snow = 5,
}

/// Number of entries in the color palette (texels in the palette texture).
pub const PALETTE_SIZE: u32 = 6;

impl Voxel {
    /// All variants in palette order (index = `palette_index`). The
    /// integration layer iterates this to build the palette texture.
    pub const ALL: [Voxel; PALETTE_SIZE as usize] = [
        Voxel::Air,
        Voxel::Grass,
        Voxel::Dirt,
        Voxel::Stone,
        Voxel::Sand,
        Voxel::Snow,
    ];

    /// `true` for every variant except `Air`.
    pub fn is_solid(self) -> bool {
        self != Voxel::Air
    }

    /// Texel index into the 1-D palette texture (`Air` never reaches the mesher).
    pub fn palette_index(self) -> u32 {
        self as u32
    }

    /// Linear-RGBA palette color for this voxel type. The integration layer
    /// bakes these into the palette texture sampled by the shading pass.
    pub fn palette_color(self) -> [u8; 4] {
        match self {
            Voxel::Air   => [0, 0, 0, 255],
            Voxel::Grass => [96, 156, 58, 255],
            Voxel::Dirt  => [124, 92, 64, 255],
            Voxel::Stone => [128, 128, 132, 255],
            Voxel::Sand  => [212, 196, 144, 255],
            Voxel::Snow  => [235, 240, 245, 255],
        }
    }
}

impl BlockMeshVoxel for Voxel {
    fn get_visibility(&self) -> VoxelVisibility {
        if self.is_solid() {
            VoxelVisibility::Opaque
        } else {
            VoxelVisibility::Empty
        }
    }
}

impl MergeVoxel for Voxel {
    type MergeValue = u8;

    fn merge_value(&self) -> u8 {
        *self as u8
    }
}
