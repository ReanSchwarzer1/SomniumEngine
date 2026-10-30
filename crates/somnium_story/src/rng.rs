//! The story's one source of chance: skill rolls and bark picks.
//!
//! SplitMix64, saved with the story, so a reload replays the same rolls and a test can pin them. Chance never comes
//! from the clock or the platform, which is what keeps a branching story reproducible from a save and a seed.

use serde::{Deserialize, Serialize};

/// A small, saveable, deterministic generator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Rng(u64);

impl Rng {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (Lemire's multiply-shift; `n == 0` gives 0).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        ((u128::from(self.next_u64()) * u128::from(n)) >> 64) as u64
    }

    /// A die: uniform in `1..=sides`.
    pub fn roll(&mut self, sides: u32) -> i64 {
        i64::try_from(self.below(u64::from(sides.max(1)))).unwrap_or(0) + 1
    }

    /// The generator's state, for a save.
    #[must_use]
    pub fn state(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_rolls_the_same_dice() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        let rolls: Vec<i64> = (0..64).map(|_| a.roll(20)).collect();
        assert_eq!(rolls, (0..64).map(|_| b.roll(20)).collect::<Vec<_>>());
        assert!(rolls.iter().all(|r| (1..=20).contains(r)));
        let mut counts = [0u32; 6];
        let mut rng = Rng::new(1);
        for _ in 0..60_000 {
            counts[(rng.roll(6) - 1) as usize] += 1;
        }
        assert!(
            counts.iter().all(|c| (9_000..11_000).contains(c)),
            "{counts:?}"
        );
    }
}
