//! Terrain sculpting and painting brushes (Phase 14D/14E).
//!
//! ## Reference Architecture
//!
//! - `example_repo/fyrox/Fyrox-master/fyrox-impl/src/scene/terrain/brushstroke/mod.rs` —
//!   stroke flow (start → stamp/smear → end) and brush value semantics.
//! - `example_repo/fyrox/Fyrox-master/fyrox-impl/src/scene/terrain/brushstroke/brushraster.rs` —
//!   radial strength `1 - d/r` and the hardness remap
//!   (`s < 1-h ? s/(1-h) : 1`), ported in [`brush_falloff`].
//!
//! Brushes operate directly on [`TerrainData`]'s CPU heightmap / splatmap and
//! mark the touched chunks (or splat rows) dirty; the renderer re-uploads on
//! the next frame.

use super::TerrainData;
use super::textures::TERRAIN_LAYER_COUNT;

/// What the brush does (Phase 14D-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrushMode {
    Raise,
    Lower,
    Smooth,
    Flatten,
    Noise,
    Paint,
}

impl BrushMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Raise => "Raise",
            Self::Lower => "Lower",
            Self::Smooth => "Smooth",
            Self::Flatten => "Flatten",
            Self::Noise => "Noise",
            Self::Paint => "Paint",
        }
    }
}

/// Brush settings shared by all modes (Phase 14D-1).
#[derive(Debug, Clone, Copy)]
pub struct TerrainBrush {
    pub mode: BrushMode,
    /// World-space radius (metres).
    pub radius: f32,
    /// Stroke strength in [0, 1].
    pub strength: f32,
    /// 0 = soft falloff to the rim, 1 = full strength to the rim.
    pub hardness: f32,
    /// Raw-height target used by `Flatten`.
    pub target_height: f32,
    /// Layer index written by `Paint`.
    pub paint_layer: usize,
}

impl Default for TerrainBrush {
    fn default() -> Self {
        Self {
            mode: BrushMode::Raise,
            radius: 8.0,
            strength: 0.5,
            hardness: 0.3,
            target_height: 0.0,
            paint_layer: 0,
        }
    }
}

/// Fyrox radial falloff: linear strength to the rim, then the hardness remap.
fn brush_falloff(dist: f32, radius: f32, hardness: f32) -> f32 {
    if dist >= radius {
        return 0.0;
    }
    let strength = 1.0 - dist / radius;
    let soft = 1.0 - hardness.clamp(0.0, 0.999);
    if strength < soft {
        strength / soft
    } else {
        1.0
    }
}

/// Inclusive vertex region affected by one brush application.
/// Returned so the editor can accumulate an undo snapshot region.
pub type VertexRegion = (u32, u32, u32, u32);

/// Apply one sculpting stamp at a terrain-local XZ position.
///
/// `dt` scales the per-frame contribution so stroke speed is frame-rate
/// independent (Phase 14D-2). Returns the touched vertex region, or `None`
/// if the brush did not overlap the terrain.
pub fn apply_sculpt(
    terrain: &mut TerrainData,
    brush: &TerrainBrush,
    local_x: f32,
    local_z: f32,
    dt: f32,
) -> Option<VertexRegion> {
    let desc = terrain.desc;
    let cell = desc.cell_size;
    let (tx, tz) = (desc.total_vertices_x(), desc.total_vertices_z());

    let x0 = (((local_x - brush.radius) / cell).floor().max(0.0)) as u32;
    let z0 = (((local_z - brush.radius) / cell).floor().max(0.0)) as u32;
    let x1 = (((local_x + brush.radius) / cell).ceil() as i64).clamp(0, tx as i64 - 1) as u32;
    let z1 = (((local_z + brush.radius) / cell).ceil() as i64).clamp(0, tz as i64 - 1) as u32;
    if x0 > x1 || z0 > z1 {
        return None;
    }

    // Raise/Lower speed in raw height units per second at full strength.
    let sculpt_rate = 8.0 / desc.height_scale.max(0.001);

    // Smooth needs a pre-stroke snapshot of the region plus the kernel margin
    // so the kernel reads unmodified heights (snapshotting only the touched
    // region keeps large terrains cheap).
    let snap_x0 = x0.saturating_sub(2);
    let snap_z0 = z0.saturating_sub(2);
    let snap_x1 = (x1 + 2).min(tx - 1);
    let snap_z1 = (z1 + 2).min(tz - 1);
    let snap_w = (snap_x1 - snap_x0 + 1) as usize;
    let snapshot: Option<Vec<f32>> = matches!(brush.mode, BrushMode::Smooth).then(|| {
        let mut s = Vec::with_capacity(snap_w * (snap_z1 - snap_z0 + 1) as usize);
        for zi in snap_z0..=snap_z1 {
            let row = (zi * tx + snap_x0) as usize;
            s.extend_from_slice(&terrain.heightmap[row..row + snap_w]);
        }
        s
    });

    let mut touched = false;
    for zi in z0..=z1 {
        for xi in x0..=x1 {
            let dx = xi as f32 * cell - local_x;
            let dz = zi as f32 * cell - local_z;
            let falloff = brush_falloff((dx * dx + dz * dz).sqrt(), brush.radius, brush.hardness);
            if falloff <= 0.0 {
                continue;
            }
            touched = true;
            let idx = (zi * tx + xi) as usize;
            let amount = brush.strength * falloff * dt;
            let h = terrain.heightmap[idx];
            terrain.heightmap[idx] = match brush.mode {
                BrushMode::Raise => h + amount * sculpt_rate,
                BrushMode::Lower => h - amount * sculpt_rate,
                BrushMode::Smooth => {
                    let snap = snapshot.as_ref().unwrap();
                    let mut sum = 0.0;
                    let mut count = 0.0;
                    for kz in -2i64..=2 {
                        for kx in -2i64..=2 {
                            let sx = (xi as i64 + kx).clamp(snap_x0 as i64, snap_x1 as i64);
                            let sz = (zi as i64 + kz).clamp(snap_z0 as i64, snap_z1 as i64);
                            let local = (sz - snap_z0 as i64) as usize * snap_w
                                + (sx - snap_x0 as i64) as usize;
                            sum += snap[local];
                            count += 1.0;
                        }
                    }
                    h + (sum / count - h) * (amount * 4.0).min(1.0)
                }
                BrushMode::Flatten => h + (brush.target_height - h) * (amount * 4.0).min(1.0),
                BrushMode::Noise => {
                    let n = noise2(xi, zi) * 2.0 - 1.0;
                    h + n * amount * sculpt_rate * 0.5
                }
                BrushMode::Paint => h, // handled by apply_paint
            };
        }
    }
    if !touched {
        return None;
    }
    terrain.mark_region_dirty(x0, z0, x1, z1);
    Some((x0, z0, x1, z1))
}

/// Apply one paint stamp to the splatmap (Phase 14E-1).
///
/// Increases the target layer's channel and renormalizes so all channels sum
/// to 255. Returns the touched texel region.
pub fn apply_paint(
    terrain: &mut TerrainData,
    brush: &TerrainBrush,
    local_x: f32,
    local_z: f32,
    dt: f32,
) -> Option<VertexRegion> {
    let desc = terrain.desc;
    let [wx, wz] = desc.world_size();
    let (sw, sh) = (terrain.splatmap.width, terrain.splatmap.height);
    // Texels per metre.
    let (mx, mz) = (sw as f32 / wx, sh as f32 / wz);
    let layer = brush.paint_layer.min(TERRAIN_LAYER_COUNT as usize - 1);

    let x0 = (((local_x - brush.radius) * mx).floor().max(0.0)) as u32;
    let z0 = (((local_z - brush.radius) * mz).floor().max(0.0)) as u32;
    let x1 = ((((local_x + brush.radius) * mx).ceil()) as i64).clamp(0, sw as i64 - 1) as u32;
    let z1 = ((((local_z + brush.radius) * mz).ceil()) as i64).clamp(0, sh as i64 - 1) as u32;
    if x0 > x1 || z0 > z1 {
        return None;
    }

    let mut touched = false;
    for zi in z0..=z1 {
        for xi in x0..=x1 {
            // Texel center in terrain-local metres.
            let px = (xi as f32 + 0.5) / mx;
            let pz = (zi as f32 + 0.5) / mz;
            let d = ((px - local_x).powi(2) + (pz - local_z).powi(2)).sqrt();
            let falloff = brush_falloff(d, brush.radius, brush.hardness);
            if falloff <= 0.0 {
                continue;
            }
            touched = true;
            let texel = &mut terrain.splatmap.data[(zi * sw + xi) as usize];
            let add = (brush.strength * falloff * dt * 510.0) as i32;
            let mut w: [i32; TERRAIN_LAYER_COUNT as usize] =
                std::array::from_fn(|i| texel[i] as i32);
            // Headroom so a held brush keeps pulling weight toward this layer
            // after normalisation instead of saturating against the others.
            w[layer] = (w[layer] + add).min(255 * TERRAIN_LAYER_COUNT as i32);
            // Renormalize to sum 255 (Phase 14E-1 step 2).
            let sum: i32 = w.iter().sum::<i32>().max(1);
            for (out, wi) in texel.iter_mut().zip(w) {
                *out = ((wi * 255 + sum / 2) / sum).clamp(0, 255) as u8;
            }
            super::splat::enforce_four_nonzero(texel);
        }
    }
    if !touched {
        return None;
    }
    terrain.splatmap.mark_dirty(x0, z0, x1, z1);
    // Painting moves the layer weights foliage is scattered against.
    terrain.edit_revision = terrain.edit_revision.wrapping_add(1);
    Some((x0, z0, x1, z1))
}

/// Procedural initial splat by slope and height (Phase 14E-3):
/// grass on flat ground, rock on steep slopes, snow above `snow_height`,
/// dirt as the slope transition band.
pub fn auto_splat(terrain: &mut TerrainData, snow_height: f32) {
    let desc = terrain.desc;
    let [wx, wz] = desc.world_size();
    let (sw, sh) = (terrain.splatmap.width, terrain.splatmap.height);

    for zi in 0..sh {
        for xi in 0..sw {
            let px = (xi as f32 + 0.5) / sw as f32 * wx;
            let pz = (zi as f32 + 0.5) / sh as f32 * wz;
            let e = desc.cell_size;
            let h = terrain.world_height_at(px, pz);
            let hx = terrain.world_height_at(px + e, pz) - terrain.world_height_at(px - e, pz);
            let hz = terrain.world_height_at(px, pz + e) - terrain.world_height_at(px, pz - e);
            // Surface slope angle from the gradient magnitude.
            let slope_deg = ((hx * hx + hz * hz).sqrt() / (2.0 * e)).atan().to_degrees();

            // Phase 25L: eight materials assigned from altitude, slope and a
            // low-frequency noise. The noise is what keeps the two grasses and
            // the forest floor from laying down as horizontal altitude bands —
            // a real hillside does not change species on a contour line.
            let n = super::heightmap::value_noise(px * 0.01, pz * 0.01, 4242);
            let n2 = super::heightmap::value_noise(px * 0.023, pz * 0.023, 991);

            let rock = smoothstep(32.0, 52.0, slope_deg);
            let gravel = smoothstep(16.0, 32.0, slope_deg) * (1.0 - rock);
            let snow = smoothstep(snow_height - 3.0, snow_height + 3.0, h) * (1.0 - rock);
            // Beaches and wet ground sit at the bottom of the range.
            let low = 1.0 - smoothstep(1.0, 6.0, h);
            let sand = low * (1.0 - gravel) * (1.0 - rock) * smoothstep(0.35, 0.65, n);
            let mud = low * (1.0 - gravel) * (1.0 - rock) * (1.0 - smoothstep(0.35, 0.65, n));

            // Whatever the special cases leave behind is ground cover, split
            // between two grasses and forest floor by the noise.
            let cover = (1.0 - rock - gravel - snow - sand - mud).max(0.0);
            let forest = cover * smoothstep(0.55, 0.8, n2);
            let grass = cover * (1.0 - smoothstep(0.55, 0.8, n2)) * (1.0 - n);
            let meadow = cover * (1.0 - smoothstep(0.55, 0.8, n2)) * n;

            // Order must match LAYER_NAMES / LAYER_MATERIALS. Layers 8–15 stay
            // zero here so Create → Terrain keeps the Phase 25/IV look; XV-G
            // owns the sixteen-layer biome preset.
            let mut weights = [0.0f32; TERRAIN_LAYER_COUNT as usize];
            weights[0] = grass;
            weights[1] = forest;
            weights[2] = rock;
            weights[3] = snow;
            weights[4] = meadow;
            weights[5] = mud;
            weights[6] = sand;
            weights[7] = gravel;
            let sum: f32 = weights.iter().sum::<f32>().max(0.001);
            terrain.splatmap.data[(zi * sw + xi) as usize] =
                std::array::from_fn(|i| (weights[i] / sum * 255.0) as u8);
        }
    }
    terrain.splatmap.mark_dirty(0, 0, sw - 1, sh - 1);
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Deterministic per-vertex hash noise in [0, 1] for the Noise brush.
fn noise2(xi: u32, zi: u32) -> f32 {
    let mut h = xi.wrapping_mul(0x85EB_CA6B) ^ zi.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5BD1_E995);
    h ^= h >> 15;
    (h & 0x00FF_FFFF) as f32 / 16_777_216.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falloff_is_full_at_center_and_zero_at_rim() {
        assert_eq!(brush_falloff(0.0, 5.0, 0.0), 1.0);
        assert_eq!(brush_falloff(5.0, 5.0, 0.0), 0.0);
        assert_eq!(brush_falloff(6.0, 5.0, 0.5), 0.0);
        // Hard brush: full strength until close to the rim.
        assert_eq!(brush_falloff(2.0, 5.0, 0.9), 1.0);
        // Soft brush: linear falloff.
        let mid = brush_falloff(2.5, 5.0, 0.0);
        assert!((mid - 0.5).abs() < 1e-5);
    }
}
