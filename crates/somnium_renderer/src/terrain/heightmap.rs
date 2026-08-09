//! Heightmap sources for the terrain (Phase 25L).
//!
//! ## Reference Architecture
//!
//! - `example_repo/CDLOD-master/source/BasicCDLOD/TestData/` — the `.tbmp`
//!   raster format its demo ships terrain in, and the dataset this loader was
//!   written against. MIT, © 2010 Filip Strugar; the same reference the chunked
//!   LOD scheme came from in Phase 14.
//!
//! Terrain has had a heightmap field since Phase 14 and no way to fill it except
//! the sculpt brush, so every test scene has been either flat or a hand-raised
//! bump. That is fine for exercising the brush and useless for judging
//! materials: layer assignment by altitude and slope has nothing to assign
//! against, and a flat plain hides every lighting feature that depends on
//! relief.

/// A heightmap decoded to normalised `0..=1` samples, row-major.
pub struct HeightImage {
    pub width: u32,
    pub height: u32,
    pub samples: Vec<f32>,
}

impl HeightImage {
    /// Bilinear sample at normalised coordinates, clamped at the edges.
    fn sample(&self, u: f32, v: f32) -> f32 {
        if self.width == 0 || self.height == 0 {
            return 0.0;
        }
        let fx = (u.clamp(0.0, 1.0)) * (self.width - 1) as f32;
        let fz = (v.clamp(0.0, 1.0)) * (self.height - 1) as f32;
        let (x0, z0) = (fx.floor() as u32, fz.floor() as u32);
        let (x1, z1) = ((x0 + 1).min(self.width - 1), (z0 + 1).min(self.height - 1));
        let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
        let at = |x: u32, z: u32| self.samples[(z * self.width + x) as usize];
        let top = at(x0, z0) * (1.0 - tx) + at(x1, z0) * tx;
        let bot = at(x0, z1) * (1.0 - tx) + at(x1, z1) * tx;
        top * (1.0 - tz) + bot * tz
    }

    /// Resample into a terrain grid of `tx × tz` vertices, scaled to `amplitude`
    /// metres of relief.
    ///
    /// The terrain grid is almost never the same size as the source — CDLOD's
    /// dataset is 4096×2048 against a default terrain of 1025², and it is not
    /// even the same aspect — so this stretches to fit rather than cropping.
    /// Stretching keeps the whole landform, which is what a test scene wants.
    /// **Area-averaged when downsampling**, bilinear when not.
    ///
    /// Point-sampling a source finer than the destination is the same mistake as
    /// a texture without mips, and it is far more destructive on a heightmap:
    /// CDLOD's dataset is 4096×2048 against a 1025² grid, so bilinear taps land
    /// four source texels apart and the aliasing becomes *geometry*. Rendered
    /// that way the terrain came out as hard horizontal terraces separated by
    /// near-vertical black walls — spikes, not a landscape. Averaging the source
    /// footprint each destination vertex covers is what makes a downsampled
    /// heightmap smooth.
    pub fn resample(&self, tx: u32, tz: u32, amplitude: f32) -> Vec<f32> {
        let mut out = vec![0.0f32; (tx * tz) as usize];
        // Source texels covered per destination vertex, per axis.
        let step_x = self.width as f32 / tx as f32;
        let step_z = self.height as f32 / tz as f32;
        let box_filter = step_x > 1.0 || step_z > 1.0;

        for z in 0..tz {
            let v = if tz > 1 { z as f32 / (tz - 1) as f32 } else { 0.0 };
            for x in 0..tx {
                let u = if tx > 1 { x as f32 / (tx - 1) as f32 } else { 0.0 };
                let h = if box_filter {
                    // Centred on the vertex, spanning the footprint it owns.
                    let cx = u * (self.width - 1) as f32;
                    let cz = v * (self.height - 1) as f32;
                    let x0 = (cx - step_x * 0.5).round().max(0.0) as u32;
                    let z0 = (cz - step_z * 0.5).round().max(0.0) as u32;
                    let x1 = ((cx + step_x * 0.5).round() as u32).min(self.width - 1);
                    let z1 = ((cz + step_z * 0.5).round() as u32).min(self.height - 1);
                    let mut sum = 0.0f32;
                    let mut n = 0u32;
                    for sz in z0..=z1 {
                        for sx in x0..=x1 {
                            sum += self.samples[(sz * self.width + sx) as usize];
                            n += 1;
                        }
                    }
                    if n > 0 { sum / n as f32 } else { self.sample(u, v) }
                } else {
                    self.sample(u, v)
                };
                out[(z * tx + x) as usize] = h * amplitude;
            }
        }
        out
    }
}

/// Load a heightmap from `.tbmp`, or from any image format the engine can
/// decode.
pub fn load(path: &str) -> Result<HeightImage, String> {
    if path.to_ascii_lowercase().ends_with(".tbmp") {
        load_tbmp(path)
    } else {
        load_image(path)
    }
}

/// CDLOD's `.tbmp`: a 256-byte header followed by 16-bit little-endian samples.
///
/// The header is not documented anywhere in the reference; it was read off the
/// file. Words 1 and 2 are width and height, and word 4 is the header size —
/// `4096 × 2048 × 2 + 256` is exactly the file's length, which is what confirms
/// the reading rather than the layout being guessed at.
fn load_tbmp(path: &str) -> Result<HeightImage, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    if bytes.len() < 32 {
        return Err(format!("{path}: too short to be a .tbmp"));
    }
    let word = |i: usize| {
        u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]])
    };
    let (width, height, header) = (word(4), word(8), word(16) as usize);
    let expected = header + (width as usize * height as usize * 2);
    if width == 0 || height == 0 || bytes.len() < expected {
        return Err(format!(
            "{path}: header says {width}x{height} with a {header}-byte header, \
             which needs {expected} bytes but the file has {}",
            bytes.len(),
        ));
    }
    let samples = bytes[header..expected]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]) as f32 / u16::MAX as f32)
        .collect();
    Ok(HeightImage { width, height, samples })
}

/// Any image the `image` crate decodes. 16-bit greyscale is preserved at full
/// precision; anything else is taken from the luminance of the 8-bit form.
///
/// The distinction matters more than it looks: an 8-bit heightmap over a
/// kilometre of relief quantises to ~4 m steps, which reads as visible terracing
/// on every slope. 16-bit sources are the norm for exactly that reason.
fn load_image(path: &str) -> Result<HeightImage, String> {
    let img = image::open(path).map_err(|e| format!("{path}: {e}"))?;
    let (width, height) = (img.width(), img.height());
    let samples = match img {
        image::DynamicImage::ImageLuma16(buf) => {
            buf.pixels().map(|p| p.0[0] as f32 / u16::MAX as f32).collect()
        }
        other => other
            .to_luma8()
            .pixels()
            .map(|p| p.0[0] as f32 / u8::MAX as f32)
            .collect(),
    };
    Ok(HeightImage { width, height, samples })
}

/// Value-noise FBM relief, for when no heightmap file is supplied.
///
/// Not a substitute for real terrain data, but it gives ridges, valleys and a
/// range of altitudes and slopes — which is all the material assignment and the
/// lighting need in order to be judged at all.
pub fn fbm_relief(tx: u32, tz: u32, seed: u32, amplitude: f32) -> Vec<f32> {
    let mut out = vec![0.0f32; (tx * tz) as usize];
    for z in 0..tz {
        for x in 0..tx {
            let (u, v) = (x as f32 / tx as f32, z as f32 / tz as f32);
            let mut sum = 0.0;
            let mut amp = 0.5;
            let mut freq = 3.0;
            let mut norm = 0.0;
            for octave in 0..6u32 {
                sum += value_noise(u * freq, v * freq, seed.wrapping_add(octave * 7919)) * amp;
                norm += amp;
                amp *= 0.5;
                freq *= 2.0;
            }
            // Squared, so valleys are broad and flat and peaks are sharp —
            // linear FBM gives a uniformly lumpy field that reads as noise
            // rather than as landscape.
            let h = (sum / norm).clamp(0.0, 1.0);
            out[(z * tx + x) as usize] = h * h * amplitude;
        }
    }
    out
}

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

pub(crate) fn value_noise(x: f32, z: f32, seed: u32) -> f32 {
    let (ix, iz) = (x.floor() as i32, z.floor() as i32);
    let (fx, fz) = (x - x.floor(), z - z.floor());
    let (ux, uz) = (fx * fx * (3.0 - 2.0 * fx), fz * fz * (3.0 - 2.0 * fz));
    let a = hash2(ix, iz, seed);
    let b = hash2(ix + 1, iz, seed);
    let c = hash2(ix, iz + 1, seed);
    let d = hash2(ix + 1, iz + 1, seed);
    a + (b - a) * ux + (c - a) * uz + (a - b - c + d) * ux * uz
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(width: u32, height: u32) -> HeightImage {
        // Height rises linearly with z, so every check below has an exact
        // expected value.
        let mut samples = Vec::new();
        for z in 0..height {
            for _ in 0..width {
                samples.push(z as f32 / (height - 1) as f32);
            }
        }
        HeightImage { width, height, samples }
    }

    #[test]
    fn resampling_spans_the_whole_source() {
        // The corners of the destination must land on the corners of the
        // source, or a resample silently crops the landform.
        let out = ramp(4, 4).resample(8, 8, 10.0);
        assert!((out[0] - 0.0).abs() < 1e-5, "top-left should be the source minimum");
        assert!((out[out.len() - 1] - 10.0).abs() < 1e-5, "bottom-right should be the maximum");
    }

    #[test]
    fn resampling_is_monotonic_along_a_ramp() {
        let (tx, tz) = (16u32, 16u32);
        let out = ramp(4, 8).resample(tx, tz, 1.0);
        for z in 1..tz {
            let prev = out[((z - 1) * tx) as usize];
            let cur = out[(z * tx) as usize];
            assert!(cur >= prev, "row {z} dipped: {prev} -> {cur}");
        }
    }

    #[test]
    fn downsampling_averages_rather_than_point_sampling() {
        // A source alternating 0 and 1 every texel averages to ~0.5 when
        // downsampled. Point-sampling would return 0 or 1 and turn the
        // alternation into geometry — which is exactly what CDLOD's 4096-wide
        // dataset did to a 1025-wide grid: hard terraces with vertical walls.
        let (w, h) = (64u32, 64u32);
        let mut samples = Vec::with_capacity((w * h) as usize);
        for z in 0..h {
            for x in 0..w {
                samples.push(if (x + z) % 2 == 0 { 0.0 } else { 1.0 });
            }
        }
        let out = HeightImage { width: w, height: h, samples }.resample(8, 8, 1.0);
        for h in &out {
            assert!((h - 0.5).abs() < 0.2, "downsample kept an aliased spike: {h}");
        }
    }

    #[test]
    fn a_non_square_source_stretches_rather_than_cropping() {
        // CDLOD's dataset is 4096x2048 against a square terrain, so this is the
        // normal case rather than an edge one.
        let out = ramp(8, 2).resample(4, 4, 1.0);
        assert!((out[out.len() - 1] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_truncated_tbmp_is_refused_rather_than_read_past() {
        let dir = std::env::temp_dir().join("somnium_tbmp_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("truncated.tbmp");
        let mut bytes = vec![0u8; 256];
        bytes[4..8].copy_from_slice(&4096u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&2048u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&256u32.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();
        assert!(load(path.to_str().unwrap()).is_err());
    }

    #[test]
    fn fbm_relief_stays_within_its_amplitude_and_is_deterministic() {
        let a = fbm_relief(32, 32, 7, 50.0);
        let b = fbm_relief(32, 32, 7, 50.0);
        assert_eq!(a, b, "same seed must give the same landscape");
        assert!(a.iter().all(|h| (0.0..=50.0).contains(h)));
        // And it must actually vary — a constant field would pass the bounds
        // check while giving the material assignment nothing to work with.
        let min = a.iter().cloned().fold(f32::MAX, f32::min);
        let max = a.iter().cloned().fold(f32::MIN, f32::max);
        assert!(max - min > 5.0, "relief is too flat to judge anything against");
    }
}
