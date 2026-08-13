//! Deterministic sixteen-layer biome splat (Phase XV-G).
//!
//! Shared by startup and Create → Terrain. Weights are elevation / slope /
//! curvature / water / exposure / noise, then strongest-four quantized. Paint
//! locks survive a rebuild when `preserve_overrides` is set.

use super::splat::enforce_four_nonzero;
use super::textures::{SplatTexel, TERRAIN_LAYER_COUNT};
use super::{DEFAULT_WATER_LEVEL_METRES, TerrainData};

/// Versioned Appalachia landscape kit. Bump when default rules change.
pub const BIOME_PRESET_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug)]
pub struct BiomePreset {
    pub version: u32,
    pub water_level: f32,
    pub snow_height: f32,
    pub seed_a: u32,
    pub seed_b: u32,
}

impl BiomePreset {
    pub fn appalachia(snow_height: f32) -> Self {
        Self {
            version: BIOME_PRESET_VERSION,
            water_level: DEFAULT_WATER_LEVEL_METRES,
            snow_height,
            seed_a: 4242,
            seed_b: 991,
        }
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Unnormalized layer weights at one sample. Indices match `LAYER_MATERIALS`.
pub fn weights_at(
    height: f32,
    slope_deg: f32,
    curvature: f32,
    n: f32,
    n2: f32,
    preset: &BiomePreset,
) -> [f32; TERRAIN_LAYER_COUNT as usize] {
    let water = preset.water_level;
    let above = height - water;
    let steep = smoothstep(38.0, 58.0, slope_deg);
    let cliff = smoothstep(48.0, 68.0, slope_deg);
    let talus_band = smoothstep(28.0, 42.0, slope_deg) * (1.0 - cliff);
    let gravel = smoothstep(16.0, 30.0, slope_deg) * (1.0 - steep);
    let snow = smoothstep(preset.snow_height - 4.0, preset.snow_height + 5.0, height)
        * (1.0 - cliff * 0.85);

    let damp_band = (1.0 - smoothstep(0.15, 1.4, above)) * (1.0 - steep);
    let dry_beach =
        smoothstep(0.2, 1.2, above) * (1.0 - smoothstep(3.5, 7.0, above)) * (1.0 - steep);
    let pebble_shore = dry_beach * smoothstep(0.45, 0.75, n) * smoothstep(8.0, 18.0, slope_deg);
    let mud = (1.0 - smoothstep(0.4, 4.0, above))
        * (1.0 - steep)
        * smoothstep(0.02, 0.12, curvature.max(0.0))
        * (1.0 - n);
    let red_clay = smoothstep(2.0, 8.0, above)
        * (1.0 - smoothstep(18.0, 32.0, above))
        * smoothstep(10.0, 22.0, slope_deg)
        * (1.0 - cliff)
        * smoothstep(0.55, 0.8, n2);
    let dry_earth = smoothstep(1.5, 6.0, above)
        * (1.0 - smoothstep(14.0, 22.0, above))
        * (1.0 - steep)
        * (1.0 - n)
        * 0.65;
    let sparse = smoothstep(3.0, 10.0, above)
        * (1.0 - smoothstep(28.0, 40.0, above))
        * (1.0 - steep)
        * n
        * 0.55;
    let mossy = steep * (1.0 - cliff) * (1.0 - snow) * smoothstep(0.4, 0.75, n2) * 0.7;
    let rock = steep * (1.0 - cliff) * (1.0 - snow) * (1.0 - mossy);

    let cover = (1.0
        - cliff
        - talus_band
        - gravel
        - snow
        - damp_band
        - dry_beach
        - pebble_shore
        - mud
        - red_clay
        - dry_earth
        - sparse
        - mossy
        - rock)
        .max(0.0);
    let forest = cover * smoothstep(0.55, 0.82, n2);
    let grass = cover * (1.0 - smoothstep(0.55, 0.82, n2)) * (1.0 - n);
    let meadow = cover * (1.0 - smoothstep(0.55, 0.82, n2)) * n;

    let mut w = [0.0f32; TERRAIN_LAYER_COUNT as usize];
    w[0] = grass;
    w[1] = forest;
    w[2] = rock;
    w[3] = snow;
    w[4] = meadow;
    w[5] = mud;
    w[6] = pebble_shore;
    w[7] = gravel;
    w[8] = dry_beach * (1.0 - pebble_shore);
    w[9] = damp_band;
    w[10] = dry_earth;
    w[11] = red_clay;
    w[12] = sparse;
    w[13] = mossy;
    w[14] = cliff;
    w[15] = talus_band;
    w
}

/// Bake biome weights into the splatmap. Locked texels are left alone when
/// `preserve_overrides` is true.
pub fn apply_biome(terrain: &mut TerrainData, preset: &BiomePreset, preserve_overrides: bool) {
    let desc = terrain.desc;
    let [wx, wz] = desc.world_size();
    let (sw, sh) = (terrain.splatmap.width, terrain.splatmap.height);
    let e = desc.cell_size.max(0.5);

    for zi in 0..sh {
        for xi in 0..sw {
            let idx = (zi * sw + xi) as usize;
            if preserve_overrides && terrain.splat_lock.get(idx).copied().unwrap_or(0) != 0 {
                continue;
            }
            if !preserve_overrides {
                if let Some(lock) = terrain.splat_lock.get_mut(idx) {
                    *lock = 0;
                }
            }
            let px = (xi as f32 + 0.5) / sw as f32 * wx;
            let pz = (zi as f32 + 0.5) / sh as f32 * wz;
            let h = terrain.world_height_at(px, pz);
            let hx = terrain.world_height_at(px + e, pz) - terrain.world_height_at(px - e, pz);
            let hz = terrain.world_height_at(px, pz + e) - terrain.world_height_at(px, pz - e);
            let slope_deg = ((hx * hx + hz * hz).sqrt() / (2.0 * e)).atan().to_degrees();
            let hxx =
                terrain.world_height_at(px + e, pz) + terrain.world_height_at(px - e, pz) - 2.0 * h;
            let hzz =
                terrain.world_height_at(px, pz + e) + terrain.world_height_at(px, pz - e) - 2.0 * h;
            let curvature = (hxx + hzz) / e;
            let n = super::heightmap::value_noise(px * 0.01, pz * 0.01, preset.seed_a);
            let n2 = super::heightmap::value_noise(px * 0.023, pz * 0.023, preset.seed_b);
            let weights = weights_at(h, slope_deg, curvature, n, n2, preset);
            let sum: f32 = weights.iter().sum::<f32>().max(0.001);
            let mut texel: SplatTexel =
                std::array::from_fn(|i| (weights[i] / sum * 255.0).round() as u8);
            enforce_four_nonzero(&mut texel);
            terrain.splatmap.data[idx] = texel;
        }
    }
    terrain.splatmap.mark_dirty(0, 0, sw - 1, sh - 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waterline_prefers_damp_sand() {
        let p = BiomePreset::appalachia(65.0);
        let w = weights_at(p.water_level, 2.0, 0.0, 0.5, 0.5, &p);
        assert!(w[9] > w[0], "damp sand should beat grass at the datum");
        assert!(w[9] > w[14], "waterline is not a cliff");
    }

    #[test]
    fn steep_faces_select_the_cliff_layer() {
        let p = BiomePreset::appalachia(65.0);
        let w = weights_at(p.water_level + 40.0, 70.0, 0.0, 0.5, 0.5, &p);
        let cliff = w[14];
        assert!(cliff > w.iter().copied().sum::<f32>() * 0.4);
    }

    #[test]
    fn high_flat_ground_selects_snow() {
        let p = BiomePreset::appalachia(65.0);
        let w = weights_at(80.0, 4.0, 0.0, 0.4, 0.4, &p);
        assert!(w[3] > w[0] && w[3] > w[14]);
    }

    #[test]
    fn the_same_inputs_are_bit_identical() {
        let p = BiomePreset::appalachia(65.1);
        let a = weights_at(20.0, 12.0, 0.05, 0.3, 0.7, &p);
        let b = weights_at(20.0, 12.0, 0.05, 0.3, 0.7, &p);
        assert_eq!(a, b);
    }
}
