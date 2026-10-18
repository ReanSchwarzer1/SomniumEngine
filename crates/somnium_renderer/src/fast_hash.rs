//! A small multiplicative hasher for the renderer's per-draw lookups (mesh offsets, a few `u32`s).
//!
//! The default SipHash is built to resist hash flooding, which nothing in a frame's draw list needs, and it costs tens
//! of nanoseconds a lookup: across the half-dozen lookups each draw makes in a frame that was a large share of 69 ms at
//! 100,000 draws (`examples/scale_probe.rs`). The maps keyed here are the renderer's own and none is iterated for its
//! order, so the hasher is the only thing that changes.

/// Builds [`FastHasher`]s.
#[derive(Default, Clone, Copy, Debug)]
pub(crate) struct FastHash;

impl std::hash::BuildHasher for FastHash {
    type Hasher = FastHasher;
    fn build_hasher(&self) -> FastHasher {
        FastHasher(0)
    }
}

/// The hasher (an FxHash-style multiply and rotate).
pub(crate) struct FastHasher(u64);

const K: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl std::hash::Hasher for FastHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0.rotate_left(5) ^ u64::from(*b)).wrapping_mul(K);
        }
    }
    fn write_u32(&mut self, n: u32) {
        self.0 = (self.0.rotate_left(5) ^ u64::from(n)).wrapping_mul(K);
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(K);
    }
    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }
}

/// A `HashMap` with [`FastHash`].
pub(crate) type FastMap<K, V> = std::collections::HashMap<K, V, FastHash>;
/// A `HashSet` with [`FastHash`].
pub(crate) type FastSet<T> = std::collections::HashSet<T, FastHash>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_offsets_spread_and_tuples_differ_by_order() {
        use std::hash::BuildHasher;
        let h = |v: u32| FastHash.hash_one(v);
        // hashbrown takes its control byte from the top bits: neighbouring offsets must not share them all.
        let tops: std::collections::BTreeSet<u64> =
            (0..256u32).map(|i| h(i * 1024) >> 57).collect();
        assert!(tops.len() > 64, "{}", tops.len());
        assert_ne!(
            FastHash.hash_one((1u32, 2u32)),
            FastHash.hash_one((2u32, 1u32))
        );
        let mut map: FastMap<u32, u32> = FastMap::default();
        for i in 0..10_000 {
            map.insert(i * 4096, i);
        }
        assert!((0..10_000).all(|i| map[&(i * 4096)] == i));
    }
}
