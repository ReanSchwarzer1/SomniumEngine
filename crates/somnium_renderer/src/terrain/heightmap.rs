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
            let v = if tz > 1 {
                z as f32 / (tz - 1) as f32
            } else {
                0.0
            };
            for x in 0..tx {
                let u = if tx > 1 {
                    x as f32 / (tx - 1) as f32
                } else {
                    0.0
                };
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
                    if n > 0 {
                        sum / n as f32
                    } else {
                        self.sample(u, v)
                    }
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

/// Bytes of `.tbmp` header before the pixel data. `c_TotalHeaderSize`.
const TBMP_HEADER: usize = 256;
/// `.tbmp` stores 16-bit heights.
const TBMP_BPP: usize = 2;

/// CDLOD's `.tbmp` — a **tiled** bitmap, not a linear raster.
///
/// The name is the warning: `TiledBitmap.cpp`. The header is
/// `[pixelFormat, width, height, version, blockDim]` as `i32`, then the image
/// stored as `blockDim × blockDim` tiles, row-major within a tile and row-major
/// across tiles (`TiledBitmap::GetBlockStartPos`).
///
/// This was first written by reading the header bytes and guessing: word 4 is
/// 256, and `256 + 4096 × 2048 × 2` happens to equal the file's length exactly,
/// so "word 4 is the header size" fitted the evidence perfectly and was wrong —
/// it is `blockDim`, and the header is *always* 256 bytes. Decoded as linear
/// rows the terrain came out as regular horizontal terraces separated by black
/// walls, which is what reading a tiled image row-wise looks like once it
/// becomes geometry. A size check is not a format check.
fn load_tbmp(path: &str) -> Result<HeightImage, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    if bytes.len() < TBMP_HEADER {
        return Err(format!("{path}: too short to be a .tbmp"));
    }
    let word = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let (width, height, version) = (word(4), word(8), word(12));
    // `TiledBitmap::Open`: the block dimension is only present from version 1.
    let block_dim = if version > 0 { word(16) } else { 128 };
    if width == 0 || height == 0 || block_dim == 0 {
        return Err(format!(
            "{path}: degenerate header {width}x{height} block {block_dim}"
        ));
    }

    let expected = TBMP_HEADER + width as usize * height as usize * TBMP_BPP;
    if bytes.len() < expected {
        return Err(format!(
            "{path}: {width}x{height} needs {expected} bytes but the file has {}",
            bytes.len(),
        ));
    }

    let (bw, bh) = (block_dim as usize, block_dim as usize);
    let blocks_x = width.div_ceil(block_dim) as usize;
    let blocks_y = height.div_ceil(block_dim) as usize;
    // Trailing blocks are narrower/shorter when the image is not a whole number
    // of blocks, and the stride maths depends on it.
    let edge_w = width as usize - (blocks_x - 1) * bw;
    let edge_h = height as usize - (blocks_y - 1) * bh;

    let mut samples = vec![0.0f32; width as usize * height as usize];
    let mut pos = TBMP_HEADER;
    for by in 0..blocks_y {
        let this_h = if by == blocks_y - 1 { edge_h } else { bh };
        for bx in 0..blocks_x {
            let this_w = if bx == blocks_x - 1 { edge_w } else { bw };
            for y in 0..this_h {
                let dst_row = (by * bh + y) * width as usize + bx * bw;
                for x in 0..this_w {
                    let src = pos + (y * this_w + x) * TBMP_BPP;
                    let v = u16::from_le_bytes([bytes[src], bytes[src + 1]]);
                    samples[dst_row + x] = v as f32 / u16::MAX as f32;
                }
            }
            pos += this_w * this_h * TBMP_BPP;
        }
    }

    Ok(HeightImage {
        width,
        height,
        samples,
    })
}

/// Any image the `image` crate decodes. Integer sources retain their native
/// 16-bit precision and FLOAT32 EXR sources remain FLOAT32 all the way into the
/// terrain resampler. Only genuinely 8-bit inputs take the 8-bit path.
///
/// The distinction matters more than it looks: an 8-bit heightmap over a
/// kilometre of relief quantises to ~4 m steps, which reads as visible terracing
/// on every slope. 16-bit sources are the norm for exactly that reason.
fn load_image(path: &str) -> Result<HeightImage, String> {
    if path.to_ascii_lowercase().ends_with(".exr") {
        return load_exr(path);
    }
    let img = image::open(path).map_err(|e| format!("{path}: {e}"))?;
    let (width, height) = (img.width(), img.height());
    let samples = match img {
        image::DynamicImage::ImageLuma16(buf) => buf
            .pixels()
            .map(|p| p.0[0] as f32 / u16::MAX as f32)
            .collect(),
        image::DynamicImage::ImageLumaA16(buf) => buf
            .pixels()
            .map(|p| p.0[0] as f32 / u16::MAX as f32)
            .collect(),
        image::DynamicImage::ImageRgb16(buf) => buf
            .pixels()
            .map(|p| {
                let [r, g, b] = p.0;
                (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / u16::MAX as f32
            })
            .collect(),
        image::DynamicImage::ImageRgba16(buf) => buf
            .pixels()
            .map(|p| {
                let [r, g, b, _] = p.0;
                (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / u16::MAX as f32
            })
            .collect(),
        image::DynamicImage::ImageRgb32F(buf) => buf
            .pixels()
            .map(|p| {
                let [r, g, b] = p.0;
                0.2126 * r + 0.7152 * g + 0.0722 * b
            })
            .collect(),
        image::DynamicImage::ImageRgba32F(buf) => buf
            .pixels()
            .map(|p| {
                let [r, g, b, _] = p.0;
                0.2126 * r + 0.7152 * g + 0.0722 * b
            })
            .collect(),
        other => other
            .to_luma8()
            .pixels()
            .map(|p| p.0[0] as f32 / u8::MAX as f32)
            .collect(),
    };
    Ok(HeightImage {
        width,
        height,
        samples,
    })
}

/// FBM in [0, 1] used by both procedural hills and the island.
fn relief_fbm(u: f32, v: f32, seed: u32) -> f32 {
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
    (sum / norm.max(0.0001)).clamp(0.0, 1.0)
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
            let h = relief_fbm(u, v, seed);
            out[(z * tx + x) as usize] = h * h * amplitude;
        }
    }
    out
}

fn island_smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Low rolling islet: beach, coastal plain, gentle FBM hills. Rim below water.
///
/// `peak_metres` is clamped — a 40 m recipe on a ~130 m footprint is a sea stack.
pub fn island_relief(tx: u32, tz: u32, seed: u32, peak_metres: f32, water_level: f32) -> Vec<f32> {
    let rise = (peak_metres - water_level).clamp(7.0, 13.0);
    let half = 256.0;
    let mut out = vec![0.0f32; (tx * tz) as usize];
    for z in 0..tz {
        for x in 0..tx {
            let u = if tx > 1 {
                x as f32 / (tx - 1) as f32
            } else {
                0.5
            };
            let v = if tz > 1 {
                z as f32 / (tz - 1) as f32
            } else {
                0.5
            };
            let nx = u * 2.0 - 1.0;
            let nz = v * 2.0 - 1.0;
            let r = (nx * nx + nz * nz).sqrt();
            let theta = nz.atan2(nx);
            let (ca, sa) = (theta.cos(), theta.sin());
            let n_coast = relief_fbm(0.5 + ca * 0.32, 0.5 + sa * 0.32, seed.wrapping_add(3));
            let n_bay = relief_fbm(0.5 + ca * 0.7, 0.5 + sa * 0.7, seed.wrapping_add(11));
            // ~130 m across; mild bays, not a sawtooth coast.
            let coast_r = (0.26 + (n_coast - 0.5) * 0.04 + (n_bay - 0.5) * 0.025).clamp(0.21, 0.33);
            let inland_m = (coast_r - r) * half;
            let offshore_m = (r - coast_r) * half;
            // Wide beach so ~13 m of rise cannot become a cliff.
            let land = island_smoothstep(-14.0, 42.0, inland_m);
            let t = (r / coast_r.max(0.2)).clamp(0.0, 1.0);
            // Inner plateau of grass; outer ring is the beach.
            let plateau = 1.0 - island_smoothstep(0.30, 0.88, t);
            let ocean_h = water_level - 0.9 - island_smoothstep(3.0, 80.0, offshore_m) * 6.8;

            let wu = u + (relief_fbm(u * 1.8, v * 1.8, seed.wrapping_add(5)) - 0.5) * 0.05;
            let wv = v + (relief_fbm(u * 1.8, v * 1.8, seed.wrapping_add(7)) - 0.5) * 0.05;
            let rolling = relief_fbm(wu * 2.6, wv * 2.6, seed);
            let bumps = relief_fbm(wu * 6.5, wv * 6.5, seed.wrapping_add(19));
            let inland_h = water_level
                + 0.4
                + (1.0 - plateau) * 1.6
                + plateau * (5.5 + rolling * rise * 0.55 + (bumps - 0.5) * 1.6);
            out[(z * tx + x) as usize] = ocean_h + land * (inland_h - ocean_h);
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
        HeightImage {
            width,
            height,
            samples,
        }
    }

    #[test]
    fn resampling_spans_the_whole_source() {
        // The corners of the destination must land on the corners of the
        // source, or a resample silently crops the landform.
        let out = ramp(4, 4).resample(8, 8, 10.0);
        assert!(
            (out[0] - 0.0).abs() < 1e-5,
            "top-left should be the source minimum"
        );
        assert!(
            (out[out.len() - 1] - 10.0).abs() < 1e-5,
            "bottom-right should be the maximum"
        );
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
        let out = HeightImage {
            width: w,
            height: h,
            samples,
        }
        .resample(8, 8, 1.0);
        for h in &out {
            assert!(
                (h - 0.5).abs() < 0.2,
                "downsample kept an aliased spike: {h}"
            );
        }
    }

    #[test]
    fn a_non_square_source_stretches_rather_than_cropping() {
        // CDLOD's dataset is 4096x2048 against a square terrain, so this is the
        // normal case rather than an edge one.
        let out = ramp(8, 2).resample(4, 4, 1.0);
        assert!((out[out.len() - 1] - 1.0).abs() < 1e-5);
    }

    /// Build a `.tbmp` whose value at (x, y) is `x + y * width`, tiled.
    fn synthetic_tbmp(width: u32, height: u32, block_dim: u32) -> Vec<u8> {
        let mut out = vec![0u8; TBMP_HEADER];
        out[4..8].copy_from_slice(&width.to_le_bytes());
        out[8..12].copy_from_slice(&height.to_le_bytes());
        out[12..16].copy_from_slice(&1u32.to_le_bytes()); // version
        out[16..20].copy_from_slice(&block_dim.to_le_bytes());

        let bx_count = width.div_ceil(block_dim);
        let by_count = height.div_ceil(block_dim);
        for by in 0..by_count {
            for bx in 0..bx_count {
                let w = (width - bx * block_dim).min(block_dim);
                let h = (height - by * block_dim).min(block_dim);
                for y in 0..h {
                    for x in 0..w {
                        let gx = bx * block_dim + x;
                        let gy = by * block_dim + y;
                        let v = (gx + gy * width) as u16;
                        out.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
        }
        out
    }

    #[test]
    fn a_tiled_tbmp_is_de_tiled_rather_than_read_as_rows() {
        // The bug this pins cost a full diagnosis pass: `.tbmp` is a *tiled*
        // bitmap, and reading it linearly produced regular horizontal terraces
        // once the heights became geometry. Non-square, and not a whole number
        // of blocks, so the edge-block stride is exercised too.
        let (w, h, block) = (10u32, 6u32, 4u32);
        let dir = std::env::temp_dir().join("somnium_tbmp_tiled");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tiled.tbmp");
        std::fs::write(&path, synthetic_tbmp(w, h, block)).unwrap();

        let img = load(path.to_str().unwrap()).expect("decode");
        assert_eq!((img.width, img.height), (w, h));
        for y in 0..h {
            for x in 0..w {
                let expected = (x + y * w) as f32 / u16::MAX as f32;
                let got = img.samples[(y * w + x) as usize];
                assert!(
                    (got - expected).abs() < 1e-6,
                    "({x}, {y}) decoded to {got}, expected {expected}",
                );
            }
        }
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
        assert!(
            max - min > 5.0,
            "relief is too flat to judge anything against"
        );
    }

    #[test]
    fn island_relief_peaks_above_water_and_rim_is_submerged() {
        let water = 16.1;
        let h = island_relief(65, 65, 9, 55.0, water);
        assert!(
            h.iter().any(|&x| x > water),
            "inland must break the surface"
        );
        let above = h.iter().filter(|&&x| x > water).count();
        assert!(
            above < h.len() / 4,
            "land should be a compact island, not the whole tile ({above}/{})",
            h.len()
        );
        let tx = 65u32;
        let tz = 65u32;
        for z in 0..tz {
            for x in 0..tx {
                if x == 0 || z == 0 || x + 1 == tx || z + 1 == tz {
                    let v = h[(z * tx + x) as usize];
                    assert!(
                        v < water,
                        "edge ({x},{z}) should sit below the datum ({v} >= {water})"
                    );
                }
            }
        }
        let land: Vec<f32> = h.iter().copied().filter(|&x| x > water).collect();
        let min_land = land.iter().copied().fold(f32::MAX, f32::min);
        let max_land = land.iter().copied().fold(f32::MIN, f32::max);
        assert!(
            max_land - min_land > 4.0,
            "inland should roll, not sit flat ({min_land}..{max_land})"
        );
        assert!(
            max_land < water + 22.0,
            "island should be a low hill, not a sea stack ({max_land})"
        );
        assert!(
            min_land < water + 4.0,
            "some shore should sit close to the datum ({min_land})"
        );
    }

    #[test]
    fn float_exr_keeps_more_than_eight_bit_precision() {
        // A 1024-step ramp collapses to at most 256 values through `to_luma8`.
        // Writing and reading the real codec pins the exact route used by the
        // Great Lakes source rather than merely testing an in-memory helper.
        let dir = std::env::temp_dir().join("somnium_float_height_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ramp.exr");
        let image = image::ImageBuffer::from_fn(1024, 1, |x, _| {
            let v = x as f32 / 1023.0;
            image::Rgb([v, v, v])
        });
        image.save(&path).expect("encode EXR");

        let decoded = load(path.to_str().unwrap()).expect("decode EXR");
        let mut distinct = decoded.samples.clone();
        distinct.sort_by(f32::total_cmp);
        distinct.dedup_by(|a, b| a.to_bits() == b.to_bits());
        assert!(
            distinct.len() > 256,
            "FLOAT32 height collapsed to {} distinct levels",
            distinct.len(),
        );
    }

    #[test]
    fn baked_great_lakes_height_is_smooth_and_high_precision() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(crate::terrain::DEFAULT_HEIGHTMAP);
        let image = load(path.to_str().unwrap()).expect("Great Lakes runtime height");
        assert_eq!((image.width, image.height), (1025, 1025));
        assert!(image.samples.iter().all(|height| height.is_finite()));
        let mut distinct = image.samples.clone();
        distinct.sort_by(f32::total_cmp);
        distinct.dedup_by(|a, b| a.to_bits() == b.to_bits());
        assert!(distinct.len() > 256, "runtime terrain lost precision");
        let mut largest_neighbor_step = 0.0f32;
        for z in 0..image.height {
            for x in 0..image.width - 1 {
                let a = image.samples[(z * image.width + x) as usize];
                let b = image.samples[(z * image.width + x + 1) as usize];
                largest_neighbor_step = largest_neighbor_step.max((a - b).abs());
            }
        }
        assert!(
            largest_neighbor_step < 0.08,
            "bake introduced a discontinuity of {largest_neighbor_step}"
        );
    }
}

/// Read the first flat EXR layer without assuming it contains RGB channels.
/// Height sources commonly contain a single `Y` FLOAT channel (the Great
/// Lakes source does), which `image::open` rejects even though it is valid EXR.
fn load_exr(path: &str) -> Result<HeightImage, String> {
    use exr::prelude::*;
    let image = read_first_flat_layer_from_file(path).map_err(|e| format!("{path}: {e}"))?;
    let layer = image.layer_data;
    let channel = layer
        .channel_data
        .list
        .iter()
        .find(|channel| channel.name.eq_case_insensitive("Y"))
        .or_else(|| layer.channel_data.list.first())
        .ok_or_else(|| format!("{path}: EXR contains no flat channel"))?;
    let samples: Vec<f32> = channel.sample_data.values_as_f32().collect();
    if samples.iter().any(|value| !value.is_finite()) {
        return Err(format!(
            "{path}: EXR height channel contains non-finite values"
        ));
    }
    Ok(HeightImage {
        width: layer.size.0 as u32,
        height: layer.size.1 as u32,
        samples,
    })
}
