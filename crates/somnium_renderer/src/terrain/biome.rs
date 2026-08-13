//! Deterministic thirty-two-layer biome splat (Phase XV-G / XV-Zeta).
//!
//! Shared by startup and Create → Terrain. Weights are elevation / slope /
//! curvature / water / exposure / noise, then strongest-four quantized. Paint
//! locks survive a rebuild when `preserve_overrides` is set.

use super::splat::enforce_four_nonzero;
use super::textures::{SplatTexel, TERRAIN_LAYER_COUNT};
use super::{DEFAULT_WATER_LEVEL_METRES, TerrainData};

/// Versioned Appalachia landscape kit. Bump when default rules change.
pub const BIOME_PRESET_VERSION: u32 = 3;

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

fn biome_fbm(x: f32, z: f32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut freq = 1.0;
    let mut norm = 0.0;
    for octave in 0..4u32 {
        sum += super::heightmap::value_noise(x * freq, z * freq, seed.wrapping_add(octave * 7919))
            * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.07;
    }
    sum / norm.max(0.0001)
}

/// Unnormalized layer weights at one sample. Indices match `LAYER_MATERIALS`.
///
/// `n` / `n2` / `n3` are [0, 1] field samples. Callers should pass warped FBM,
/// not a single octave of value noise: bilinear cells make isolines that read
/// as a ruler across a hillside.
pub fn weights_at(
    height: f32,
    slope_deg: f32,
    curvature: f32,
    n: f32,
    n2: f32,
    n3: f32,
    preset: &BiomePreset,
) -> [f32; TERRAIN_LAYER_COUNT as usize] {
    let water = preset.water_level;
    let above = height - water;
    let steep = smoothstep(38.0, 58.0, slope_deg);
    let cliff = smoothstep(48.0, 68.0, slope_deg);
    let talus_band = smoothstep(28.0, 42.0, slope_deg) * (1.0 - cliff);
    let gravel = smoothstep(16.0, 30.0, slope_deg) * (1.0 - steep);

    // Wide cap plus mid-elevation patches. Fantasy placement is intentional:
    // snow that only lived in a 9 m band at `relief * 0.62` was invisible from
    // the preset camera.
    let snow_cap = smoothstep(preset.snow_height - 16.0, preset.snow_height + 8.0, height)
        * (1.0 - cliff * 0.4);
    let snow_patch = smoothstep(water + 8.0, preset.snow_height * 0.9, height)
        * (1.0 - cliff)
        * (1.0 - steep * 0.35)
        * smoothstep(0.58, 0.84, n3);
    let snow = (snow_cap + snow_patch * 0.85).min(1.0);

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
    let mossy = steep * (1.0 - cliff) * (1.0 - snow) * smoothstep(0.4, 0.75, n2) * 0.35;
    let gray_rock = steep * (1.0 - cliff) * (1.0 - snow) * (1.0 - n2) * 0.9;
    let rock = steep * (1.0 - cliff) * (1.0 - snow) * 0.15;
    let moss_carpet = steep * (1.0 - cliff) * (1.0 - snow) * smoothstep(0.55, 0.85, n) * 0.55;
    let lichen = steep * (1.0 - cliff) * smoothstep(0.2, 0.5, n2) * 0.35;
    let granite = talus_band * smoothstep(0.35, 0.75, n2);
    let wetland = (1.0 - smoothstep(0.3, 3.5, above))
        * (1.0 - steep)
        * smoothstep(0.04, 0.18, curvature.max(0.0))
        * 0.85;
    let limestone = (1.0 - steep)
        * (1.0 - snow)
        * smoothstep(6.0, 16.0, above)
        * (1.0 - smoothstep(36.0, 52.0, above))
        * smoothstep(0.68, 0.90, n2)
        * 0.55;
    let hard_snow = snow * smoothstep(0.50, 0.82, n);

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
        - gray_rock
        - rock
        - moss_carpet
        - lichen
        - granite
        - wetland
        - limestone
        - hard_snow)
        .max(0.0);

    // Overlap forest and meadow instead of a 0.28-wide n2 gate. That gate made
    // 100 m value-noise cells into a straight grass|rock contour.
    let forest_share = 0.20 + 0.55 * smoothstep(0.18, 0.82, n2);
    let meadow_share = 1.0 - forest_share;
    let forest = cover * forest_share;
    let meadow_cover = cover * meadow_share;
    let duff = forest * (0.35 + 0.45 * smoothstep(0.30, 0.80, n));
    let pine = forest * (0.25 + 0.40 * (1.0 - n));
    let autumn = forest * (0.12 + 0.28 * smoothstep(0.10, 0.45, n3));
    let lawn = meadow_cover * (0.40 + 0.35 * (1.0 - n));
    let wild = meadow_cover * (0.25 + 0.40 * n);
    let meadow = meadow_cover * 0.22;
    let scatter_moss = cover * (1.0 - steep) * smoothstep(0.55, 0.85, n) * 0.22;
    let scatter_path = cover * (1.0 - steep) * smoothstep(0.78, 0.94, n3) * (1.0 - n2) * 0.28;
    let scatter_rock = cover * (1.0 - steep) * smoothstep(0.74, 0.93, n2) * n3 * 0.22;

    let mut w = [0.0f32; TERRAIN_LAYER_COUNT as usize];
    w[0] = lawn * 0.18;
    w[1] = forest * 0.40;
    w[2] = rock + scatter_rock * 0.45;
    w[3] = snow * (1.0 - hard_snow);
    w[4] = meadow;
    w[5] = mud * 0.55;
    w[6] = pebble_shore;
    w[7] = gravel;
    w[8] = dry_beach * (1.0 - pebble_shore) * 0.35;
    w[9] = damp_band;
    w[10] = dry_earth * 0.35;
    w[11] = red_clay * 0.4;
    w[12] = sparse * 0.55 + meadow_cover * 0.12;
    w[13] = mossy;
    w[14] = cliff * 0.55;
    w[15] = talus_band * 0.45;
    w[16] = lawn;
    w[17] = duff;
    w[18] = gray_rock;
    w[19] = cliff * 0.45;
    w[20] = moss_carpet + scatter_moss;
    w[21] = limestone + lichen * 0.35;
    w[22] = mud * 0.45;
    w[23] = pine;
    w[24] = wild;
    w[25] = wetland;
    w[26] = granite + talus_band * 0.4 + scatter_rock * 0.55;
    w[27] = dry_beach * (1.0 - pebble_shore) * 0.65;
    w[28] = lichen;
    w[29] = autumn;
    w[30] = scatter_path;
    w[31] = hard_snow;
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
            let warp_x = super::heightmap::value_noise(
                px * 0.004,
                pz * 0.004,
                preset.seed_a.wrapping_add(17),
            );
            let warp_z = super::heightmap::value_noise(
                px * 0.004,
                pz * 0.004,
                preset.seed_b.wrapping_add(31),
            );
            let wx = px + (warp_x - 0.5) * 90.0;
            let wz = pz + (warp_z - 0.5) * 90.0;
            let n = biome_fbm(wx * 0.012, wz * 0.012, preset.seed_a);
            let n2 = biome_fbm(wx * 0.027, wz * 0.027, preset.seed_b);
            let n3 = biome_fbm(px * 0.055, pz * 0.055, preset.seed_a.wrapping_add(1109));
            let weights = weights_at(h, slope_deg, curvature, n, n2, n3, preset);
            let sum: f32 = weights.iter().sum::<f32>().max(0.001);
            let mut texel: SplatTexel =
                std::array::from_fn(|i| (weights[i] / sum * 255.0).round() as u8);
            enforce_four_nonzero(&mut texel);
            terrain.splatmap.data[idx] = texel;
        }
    }
    terrain.splatmap.mark_dirty(0, 0, sw - 1, sh - 1);
    terrain.invalidate_unique_colour();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waterline_prefers_damp_sand() {
        let p = BiomePreset::appalachia(65.0);
        let w = weights_at(p.water_level, 2.0, 0.0, 0.5, 0.5, 0.5, &p);
        assert!(w[9] > w[0], "damp sand should beat grass at the datum");
        assert!(w[9] > w[14], "waterline is not a cliff");
    }

    #[test]
    fn steep_faces_select_the_cliff_layer() {
        let p = BiomePreset::appalachia(65.0);
        let w = weights_at(p.water_level + 40.0, 70.0, 0.0, 0.5, 0.5, 0.5, &p);
        let cliff = w[14] + w[19];
        assert!(cliff > w.iter().copied().sum::<f32>() * 0.4);
    }

    #[test]
    fn high_flat_ground_selects_snow() {
        let p = BiomePreset::appalachia(65.0);
        let w = weights_at(80.0, 4.0, 0.0, 0.4, 0.4, 0.5, &p);
        assert!(w[3] + w[31] > w[0] && w[3] + w[31] > w[14]);
    }

    #[test]
    fn mid_slopes_can_grow_snow_patches() {
        let p = BiomePreset::appalachia(50.0);
        let w = weights_at(p.water_level + 22.0, 8.0, 0.0, 0.6, 0.4, 0.85, &p);
        assert!(
            w[3] + w[31] > 0.04,
            "snow patches should appear below the cap (snow {})",
            w[3] + w[31]
        );
    }

    #[test]
    fn inland_cover_prefers_lush_green() {
        let p = BiomePreset::appalachia(65.0);
        let w = weights_at(p.water_level + 12.0, 6.0, 0.0, 0.2, 0.3, 0.4, &p);
        let green = w[16] + w[24] + w[1] + w[17];
        let soil = w[5] + w[10] + w[11];
        assert!(
            green > soil,
            "inland should read green/forest, not mud/earth (green {green} soil {soil})"
        );
        assert!(
            w[16] + w[24] > w[0],
            "lush/wildgrass should beat ochre grass"
        );
    }

    #[test]
    fn inland_cover_keeps_several_layers_alive() {
        let p = BiomePreset::appalachia(50.0);
        let w = weights_at(p.water_level + 12.0, 6.0, 0.0, 0.35, 0.45, 0.4, &p);
        let significant = w.iter().filter(|&&x| x > 0.05).count();
        assert!(
            significant >= 3,
            "inland should not be a single material ({significant} layers, {w:?})"
        );
    }

    #[test]
    fn the_same_inputs_are_bit_identical() {
        let p = BiomePreset::appalachia(65.1);
        let a = weights_at(20.0, 12.0, 0.05, 0.3, 0.7, 0.4, &p);
        let b = weights_at(20.0, 12.0, 0.05, 0.3, 0.7, 0.4, &p);
        assert_eq!(a, b);
    }
}
