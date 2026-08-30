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

/// The mask a dab is multiplied by, on top of its radial falloff.
///
/// A brush whose only shape is `1 - d/r` lays down a perfectly circular,
/// perfectly even disc, and a surface painted with it looks airbrushed: every
/// edge is a clean arc and every overlap is a visible lens. Unreal's answer is
/// an alpha texture with a randomised rotation per stamp; this is the same
/// idea with the texture generated rather than imported, which keeps it
/// working before a project has any brush assets and keeps the whole thing
/// testable without a GPU.
///
/// The rotation matters as much as the pattern. Without it, every dab stamps
/// the identical mask and a drag turns into a visible repeat; with it, the
/// same three patterns cover a hillside without reading as a texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrushAlpha {
    /// The plain radial falloff. Predictable, and what sculpting usually
    /// wants — a raised hill should not be full of holes.
    Smooth,
    /// Hard-edged stipple. Breaks the rim into grains, which is what makes
    /// a sand or gravel layer stop looking like a decal.
    Speckle,
    /// Soft cloud noise. An uneven blend rather than an even wash.
    Clouds,
    /// Ridged noise: streaks that read as erosion or grain.
    Ridged,
}

impl BrushAlpha {
    /// Every pattern, in cycling order.
    pub const ALL: [Self; 4] = [Self::Smooth, Self::Speckle, Self::Clouds, Self::Ridged];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Smooth => "Smooth",
            Self::Speckle => "Speckle",
            Self::Clouds => "Clouds",
            Self::Ridged => "Ridged",
        }
    }

    /// The next pattern in [`Self::ALL`], for a shortcut that cycles.
    #[must_use]
    pub fn next(self) -> Self {
        let index = Self::ALL.iter().position(|a| *a == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    /// The multiplier at a texel `(dx, dz)` metres from the dab centre.
    ///
    /// Always in `[0, 1]`, and always exactly `1.0` for [`Self::Smooth`], so
    /// a caller can apply this unconditionally without a branch and without
    /// changing the behaviour anyone already relies on.
    #[must_use]
    pub fn mask(self, dx: f32, dz: f32, radius: f32, rotation: f32, scale: f32) -> f32 {
        if self == Self::Smooth {
            return 1.0;
        }
        let radius = radius.max(1e-4);
        let (sin, cos) = rotation.sin_cos();
        // Dab-local coordinates, rotated. Normalising by the radius is what
        // keeps the pattern the same relative size whether the brush is two
        // metres across or fifty — a mask that stayed a fixed world size would
        // vanish into a big brush and swallow a small one.
        let u = (dx * cos - dz * sin) / radius * scale;
        let v = (dx * sin + dz * cos) / radius * scale;
        let n = fbm(u, v);
        match self {
            Self::Smooth => 1.0,
            // A threshold, softened just enough to antialias against the
            // splat resolution rather than to alias into single texels.
            Self::Speckle => smoothstep(0.36, 0.62, n),
            Self::Clouds => 0.25 + 0.75 * n,
            Self::Ridged => {
                let ridge = 1.0 - (n * 2.0 - 1.0).abs();
                0.15 + 0.85 * ridge
            }
        }
    }
}

/// A rotation for the dab centred at `(x, z)`, in radians.
///
/// Derived from the position rather than from a counter on purpose. A counter
/// would re-roll every frame the brush is held down, and a mask that changes
/// sixty times a second is a shimmer, not a texture. Quantising to the brush
/// radius means the pattern is *stable while the pointer is still* and rolls
/// over as it travels, which is the behaviour a stamp-per-dab tool gets for
/// free and a continuous one has to arrange.
#[must_use]
pub fn dab_rotation(x: f32, z: f32, radius: f32) -> f32 {
    let cell = radius.max(0.25);
    #[allow(clippy::cast_possible_truncation)]
    let (cx, cz) = ((x / cell).floor() as i32, (z / cell).floor() as i32);
    hash2(cx, cz) * std::f32::consts::TAU
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A hash of two integers into `[0, 1)`. Not cryptographic and not trying to
/// be; it needs to be cheap, deterministic across runs, and free of visible
/// axis-aligned structure, which the two odd multipliers and the xorshift
/// give it.
fn hash2(x: i32, z: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x27d4_eb2d) ^ (z as u32).wrapping_mul(0x1656_67b1);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_f491);
    h ^= h >> 13;
    (h >> 8) as f32 / 16_777_216.0
}

/// Bilinear value noise with a smoothstep interpolant.
fn value_noise(x: f32, z: f32) -> f32 {
    #[allow(clippy::cast_possible_truncation)]
    let (xi, zi) = (x.floor() as i32, z.floor() as i32);
    let (fx, fz) = (x - x.floor(), z - z.floor());
    let (ux, uz) = (
        fx * fx * (3.0 - 2.0 * fx),
        fz * fz * (3.0 - 2.0 * fz),
    );
    let a = hash2(xi, zi);
    let b = hash2(xi + 1, zi);
    let c = hash2(xi, zi + 1);
    let d = hash2(xi + 1, zi + 1);
    let top = a + (b - a) * ux;
    let bottom = c + (d - c) * ux;
    top + (bottom - top) * uz
}

/// Three octaves, normalised to `[0, 1]`. Three because two reads as a blur
/// and four costs more than the eye can tell apart at splat resolution.
fn fbm(x: f32, z: f32) -> f32 {
    let mut sum = 0.0;
    let mut amplitude = 0.5;
    let mut frequency = 1.0;
    let mut total = 0.0;
    for _ in 0..3 {
        sum += value_noise(x * frequency, z * frequency) * amplitude;
        total += amplitude;
        amplitude *= 0.5;
        frequency *= 2.1;
    }
    (sum / total).clamp(0.0, 1.0)
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
    /// The mask multiplied onto the radial falloff. See [`BrushAlpha`].
    pub alpha: BrushAlpha,
    /// Pattern repeats across the brush diameter. Larger is finer.
    pub alpha_scale: f32,
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
            // Off by default: sculpting with a holey brush is worse than
            // sculpting with a clean one, and painting is where this earns
            // its keep. The author turns it on for the layer that needs it.
            alpha: BrushAlpha::Smooth,
            alpha_scale: 3.5,
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

    let rotation = dab_rotation(local_x, local_z, brush.radius);

    let mut touched = false;
    for zi in z0..=z1 {
        for xi in x0..=x1 {
            let dx = xi as f32 * cell - local_x;
            let dz = zi as f32 * cell - local_z;
            let falloff = brush_falloff((dx * dx + dz * dz).sqrt(), brush.radius, brush.hardness)
                * brush
                    .alpha
                    .mask(dx, dz, brush.radius, rotation, brush.alpha_scale);
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

    // One rotation for the whole dab: the mask has to be a coherent shape,
    // not per-texel noise.
    let rotation = dab_rotation(local_x, local_z, brush.radius);

    let mut touched = false;
    for zi in z0..=z1 {
        for xi in x0..=x1 {
            // Texel center in terrain-local metres.
            let px = (xi as f32 + 0.5) / mx;
            let pz = (zi as f32 + 0.5) / mz;
            let d = ((px - local_x).powi(2) + (pz - local_z).powi(2)).sqrt();
            let falloff = brush_falloff(d, brush.radius, brush.hardness)
                * brush.alpha.mask(
                    px - local_x,
                    pz - local_z,
                    brush.radius,
                    rotation,
                    brush.alpha_scale,
                );
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
            if let Some(lock) = terrain.splat_lock.get_mut((zi * sw + xi) as usize) {
                *lock = 1;
            }
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

/// Procedural initial splat. Delegates to the XV-G biome preset.
pub fn auto_splat(terrain: &mut TerrainData, snow_height: f32) {
    super::biome::apply_biome(
        terrain,
        &super::biome::BiomePreset::appalachia(snow_height),
        false,
    );
}

/// Island map: hero bank only (layers 0–15). GPU format stays 32 slots.
pub fn auto_splat_island(terrain: &mut TerrainData, snow_height: f32) {
    super::biome::apply_biome(
        terrain,
        &super::biome::BiomePreset::island(snow_height),
        false,
    );
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

#[cfg(test)]
mod alpha_tests {
    use super::{BrushAlpha, dab_rotation};

    /// `Smooth` must be exactly the old behaviour, not approximately it:
    /// every existing sculpt test and every authored heightmap depends on the
    /// radial falloff being the whole story when no pattern is chosen.
    #[test]
    fn the_smooth_pattern_is_an_exact_identity() {
        for (dx, dz) in [(0.0, 0.0), (1.0, -2.0), (7.5, 7.5), (-3.0, 0.25)] {
            assert_eq!(
                BrushAlpha::Smooth.mask(dx, dz, 8.0, 1.1, 3.5),
                1.0,
                "Smooth must not perturb the falloff at all"
            );
        }
    }

    /// A mask outside `[0, 1]` would either erase the falloff's rim or push
    /// weight past saturation, and both look like a bug in the brush rather
    /// than in the mask.
    #[test]
    fn every_pattern_stays_within_the_unit_range() {
        for alpha in BrushAlpha::ALL {
            for i in -40..40 {
                for j in -40..40 {
                    let (dx, dz) = (i as f32 * 0.2, j as f32 * 0.2);
                    let m = alpha.mask(dx, dz, 8.0, 0.7, 3.5);
                    assert!(
                        (0.0..=1.0).contains(&m),
                        "{} produced {m} at ({dx}, {dz})",
                        alpha.label()
                    );
                }
            }
        }
    }

    /// The point of the whole thing: a patterned dab is *uneven*. A mask that
    /// came back constant would be a smooth brush wearing a different name.
    #[test]
    fn a_patterned_dab_is_not_uniform() {
        for alpha in [BrushAlpha::Speckle, BrushAlpha::Clouds, BrushAlpha::Ridged] {
            let mut min = f32::MAX;
            let mut max = f32::MIN;
            for i in -20..20 {
                for j in -20..20 {
                    let m = alpha.mask(i as f32 * 0.3, j as f32 * 0.3, 8.0, 0.0, 3.5);
                    min = min.min(m);
                    max = max.max(m);
                }
            }
            assert!(
                max - min > 0.25,
                "{} varied by only {:.3} across a dab",
                alpha.label(),
                max - min
            );
        }
    }

    /// Rotation is what stops a drag from stamping one repeating tile. It
    /// must be stable while the pointer is still and different once it has
    /// travelled a brush width — a rotation that changed every frame would be
    /// a shimmer instead of a texture.
    #[test]
    fn the_rotation_holds_still_and_then_rolls_over() {
        let radius = 8.0;
        let here = dab_rotation(100.0, 100.0, radius);
        assert_eq!(
            here,
            dab_rotation(100.5, 101.0, radius),
            "a hand resting on the mouse must not re-roll the pattern"
        );
        let far = dab_rotation(100.0 + radius * 3.0, 100.0, radius);
        assert!(
            (here - far).abs() > 1e-6,
            "three brush widths away must not be the same stamp"
        );
    }

    /// The pattern is normalised by the radius, so a big brush and a small
    /// one show the same number of features rather than the big one showing
    /// a wash and the small one a single blob.
    #[test]
    fn the_pattern_scales_with_the_brush() {
        let small = BrushAlpha::Clouds.mask(2.0, 1.0, 4.0, 0.0, 3.5);
        let large = BrushAlpha::Clouds.mask(4.0, 2.0, 8.0, 0.0, 3.5);
        assert!((small - large).abs() < 1e-5);
    }

    #[test]
    fn cycling_visits_every_pattern_and_returns() {
        let mut seen = Vec::new();
        let mut alpha = BrushAlpha::Smooth;
        for _ in 0..BrushAlpha::ALL.len() {
            seen.push(alpha);
            alpha = alpha.next();
        }
        assert_eq!(seen, BrushAlpha::ALL.to_vec());
        assert_eq!(alpha, BrushAlpha::Smooth, "and wraps");
    }
}
