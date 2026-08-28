//! Stable logical addresses shared by terrain assets and renderer residency.

/// One page in a terrain material layer's existing BC7 mip chain.
///
/// Albedo/height and normal/roughness/occlusion use the same address and are
/// always admitted as a pair. Keeping the address in `somnium_asset` avoids a
/// renderer-only file-format type without inventing a second archive format:
/// the shipped BC7 packs are already page-seekable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VirtualPageId {
    /// Material layer index.
    pub layer: u8,
    /// Mip level, with zero being the full-resolution source.
    pub mip: u8,
    /// Page coordinate along U.
    pub x: u16,
    /// Page coordinate along V.
    pub y: u16,
}

impl VirtualPageId {
    /// Construct a logical page address.
    #[must_use]
    pub const fn new(layer: u8, mip: u8, x: u16, y: u16) -> Self {
        Self { layer, mip, x, y }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_addresses_have_deterministic_layer_mip_order() {
        let mut pages = [
            VirtualPageId::new(1, 0, 0, 0),
            VirtualPageId::new(0, 2, 0, 0),
            VirtualPageId::new(0, 1, 1, 0),
        ];
        pages.sort();
        assert_eq!(pages[0], VirtualPageId::new(0, 1, 1, 0));
        assert_eq!(pages[2], VirtualPageId::new(1, 0, 0, 0));
    }
}
