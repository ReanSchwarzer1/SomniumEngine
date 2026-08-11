//! Procedural heightmap terrain.
//!
//! Deterministic FBM value noise — every worker thread can sample any world
//! position without shared state, so chunk generation needs no locking.
//! The noise itself is original code (hash-based value noise with smoothstep
//! interpolation, 4-octave FBM), not ported from a reference.

use crate::voxel::Voxel;
use glam::IVec3;

/// Terrain shape parameters. All fields are plain data so a `TerrainConfig`
/// can be cloned into worker threads.
#[derive(Debug, Clone)]
pub struct TerrainConfig {
    /// Noise seed.
    pub seed: u32,
    /// Base ground level (metres) in the flat basin around the world origin.
    pub base_height: f32,
    /// Maximum hill amplitude (metres) once outside the basin.
    pub amplitude: f32,
    /// Noise frequency (cycles per metre).
    pub frequency: f32,
    /// Distance (metres) from the origin where the flat basin ends…
    pub basin_radius: f32,
    /// …and where the hills reach full amplitude.
    pub hills_radius: f32,
    /// Surface height above which grass turns to snow.
    pub snow_height: f32,
    /// Surface height below which grass turns to sand (around the water line).
    pub sand_height: f32,
}

impl Default for TerrainConfig {
    fn default() -> Self {
        Self {
            seed: 1337,
            base_height: -3.0,
            amplitude: 14.0,
            frequency: 0.015,
            basin_radius: 12.0,
            hills_radius: 56.0,
            snow_height: 9.0,
            sand_height: -1.5,
        }
    }
}

impl TerrainConfig {
    /// Terrain surface height (metres) at the given world XZ position.
    ///
    /// Clamped to stay inside the vertical chunk range used by the demo
    /// (chunks y = -1..=0 → world y in [-32, 32)).
    pub fn height(&self, x: f32, z: f32) -> f32 {
        let dist = (x * x + z * z).sqrt();
        let ramp = smoothstep(self.basin_radius, self.hills_radius, dist);
        let n = fbm(x * self.frequency, z * self.frequency, self.seed); // [-1, 1]
        (self.base_height + ramp * (2.0 + n * self.amplitude)).clamp(-28.0, 27.0)
    }

    /// Voxel at the given world-space voxel coordinate.
    pub fn voxel(&self, pos: IVec3) -> Voxel {
        let h = self.height(pos.x as f32 + 0.5, pos.z as f32 + 0.5);
        let surface = h.floor() as i32;

        if pos.y > surface {
            return Voxel::Air;
        }
        let depth = surface - pos.y;
        if depth == 0 {
            if h >= self.snow_height {
                Voxel::Snow
            } else if h <= self.sand_height {
                Voxel::Sand
            } else {
                Voxel::Grass
            }
        } else if depth <= 3 {
            if h <= self.sand_height {
                Voxel::Sand
            } else {
                Voxel::Dirt
            }
        } else {
            Voxel::Stone
        }
    }
}

// ── Value noise ──────────────────────────────────────────────────────────────

/// Integer lattice hash → [0, 1). Wang-style integer mix.
fn hash2(ix: i32, iz: i32, seed: u32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(0x85EB_CA6B)
        ^ (iz as u32).wrapping_mul(0xC2B2_AE35)
        ^ seed.wrapping_mul(0x27D4_EB2F);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h & 0x00FF_FFFF) as f32 / 16_777_216.0
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Bilinear value noise with smoothstep fade, output in [-1, 1].
fn value_noise(x: f32, z: f32, seed: u32) -> f32 {
    let ix = x.floor();
    let iz = z.floor();
    let fx = x - ix;
    let fz = z - iz;
    let (ix, iz) = (ix as i32, iz as i32);

    let ux = fx * fx * (3.0 - 2.0 * fx);
    let uz = fz * fz * (3.0 - 2.0 * fz);

    let a = hash2(ix, iz, seed);
    let b = hash2(ix + 1, iz, seed);
    let c = hash2(ix, iz + 1, seed);
    let d = hash2(ix + 1, iz + 1, seed);

    let v = a + (b - a) * ux + (c - a) * uz + (a - b - c + d) * ux * uz;
    v * 2.0 - 1.0
}

/// 4-octave FBM, output roughly in [-1, 1]. Each octave is rotated 45° to
/// avoid axis-aligned ridges.
fn fbm(x: f32, z: f32, seed: u32) -> f32 {
    const COS_R: f32 = std::f32::consts::FRAC_1_SQRT_2;
    const SIN_R: f32 = std::f32::consts::FRAC_1_SQRT_2;

    let (mut x, mut z) = (x, z);
    let mut amplitude = 0.5;
    let mut sum = 0.0;
    let mut norm = 0.0;
    for octave in 0..4u32 {
        sum += value_noise(x, z, seed.wrapping_add(octave * 7919)) * amplitude;
        norm += amplitude;
        amplitude *= 0.5;
        let (nx, nz) = (x * COS_R - z * SIN_R, x * SIN_R + z * COS_R);
        x = nx * 2.0;
        z = nz * 2.0;
    }
    sum / norm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let cfg = TerrainConfig::default();
        assert_eq!(cfg.height(10.0, -42.0), cfg.height(10.0, -42.0));
        assert_eq!(
            cfg.voxel(IVec3::new(3, -5, 7)),
            cfg.voxel(IVec3::new(3, -5, 7))
        );
    }

    #[test]
    fn solid_below_air_above() {
        let cfg = TerrainConfig::default();
        for &(x, z) in &[(0, 0), (40, -40), (-100, 65)] {
            let h = cfg.height(x as f32 + 0.5, z as f32 + 0.5).floor() as i32;
            assert!(cfg.voxel(IVec3::new(x, h, z)).is_solid());
            assert_eq!(cfg.voxel(IVec3::new(x, h + 1, z)), Voxel::Air);
            assert!(cfg.voxel(IVec3::new(x, h - 10, z)).is_solid());
        }
    }

    #[test]
    fn height_stays_in_vertical_chunk_range() {
        let cfg = TerrainConfig::default();
        for x in (-200..200).step_by(7) {
            for z in (-200..200).step_by(7) {
                let h = cfg.height(x as f32, z as f32);
                assert!((-32.0..32.0).contains(&h), "height {h} out of range");
            }
        }
    }
}
