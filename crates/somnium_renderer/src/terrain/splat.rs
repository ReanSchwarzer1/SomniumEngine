//! Packed RGBA splat helpers (Phase XV-C).
//!
//! Sixteen global weights live in four RGBA8 textures. Painting and biome
//! rules keep **at most four non-zero channels** per stored texel; bilinear
//! filtering can still expose more at pixel boundaries, which the shader
//! resolves with strongest-four selection.

use super::textures::{SplatTexel, TERRAIN_LAYER_COUNT};

/// RGBA control maps for thirty-two layers (eight groups of four).
pub const SPLAT_MAP_COUNT: usize = 8;

/// Stored non-zero channels allowed per splat texel.
pub const MAX_STORED_NONZERO: usize = 4;

/// Which RGBA map a layer index lives in.
#[inline]
pub fn splat_group(layer: usize) -> usize {
    layer / 4
}

/// Which channel inside that map.
#[inline]
pub fn splat_channel(layer: usize) -> usize {
    layer % 4
}

/// De-interleave one CPU texel into eight RGBA byte groups.
pub fn deinterleave(texel: &SplatTexel) -> [[u8; 4]; SPLAT_MAP_COUNT] {
    std::array::from_fn(|g| {
        let i = g * 4;
        [texel[i], texel[i + 1], texel[i + 2], texel[i + 3]]
    })
}

/// Quantize so surviving weights sum to 255, dumping remainder on the last
/// non-zero channel (stable, deterministic).
pub fn renormalize_to_255(texel: &mut SplatTexel) {
    let n = TERRAIN_LAYER_COUNT as usize;
    let sum: u32 = texel.iter().map(|&x| x as u32).sum();
    if sum == 0 {
        texel[0] = 255;
        return;
    }
    let mut acc = 0u32;
    let mut last = 0usize;
    for i in 0..n {
        if texel[i] > 0 {
            last = i;
        }
        let v = ((texel[i] as u32 * 255 + sum / 2) / sum).min(255) as u8;
        texel[i] = v;
        acc += u32::from(v);
    }
    if acc != 255 {
        let d = 255i32 - acc as i32;
        texel[last] = (i32::from(texel[last]) + d).clamp(0, 255) as u8;
    }
}

/// Drop the weakest channels until at most four remain, then renormalize.
///
/// Ties decay the **higher** index first so layers 0–7 win over 8–15 when
/// weights are equal. Does not run on sidecar v2 migration — those bytes are
/// copied exactly.
pub fn enforce_four_nonzero(texel: &mut SplatTexel) {
    let n = TERRAIN_LAYER_COUNT as usize;
    let mut live: Vec<(u8, usize)> = (0..n)
        .filter(|&i| texel[i] > 0)
        .map(|i| (texel[i], i))
        .collect();
    if live.len() > MAX_STORED_NONZERO {
        live.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        let drop_n = live.len() - MAX_STORED_NONZERO;
        for item in live.iter().take(drop_n) {
            texel[item.1] = 0;
        }
    }
    renormalize_to_255(texel);
}

/// Expand a sidecar splat payload to thirty-two-channel CPU texels (XV-J).
///
/// v2 copies layers 0–7 and zeros 8–31; v3 copies 0–15 and zeros 16–31; v4 is
/// a straight copy. Four-nonzero is **not** applied — a migrated old scene
/// must keep the look of the layers it already had.
pub fn migrate_sidecar_splat(
    version: u32,
    src: &[u8],
    texel_count: usize,
) -> Result<Vec<SplatTexel>, String> {
    let src_channels: usize = match version {
        2 => 8,
        3 => 16,
        4 => TERRAIN_LAYER_COUNT as usize,
        _ => {
            return Err(format!(
                "terrain sidecar is version {version}; this build reads 2/3 (migrate) or 4"
            ));
        }
    };
    let expected = texel_count * src_channels;
    if src.len() != expected {
        return Err(format!(
            "sidecar splat payload is {} bytes, expected {expected} for v{version}",
            src.len()
        ));
    }
    let mut out = vec![[0u8; TERRAIN_LAYER_COUNT as usize]; texel_count];
    match version {
        4 => {
            for (dst, chunk) in out.iter_mut().zip(src.chunks_exact(src_channels)) {
                dst.copy_from_slice(chunk);
            }
        }
        3 => {
            for (dst, chunk) in out.iter_mut().zip(src.chunks_exact(16)) {
                dst[..16].copy_from_slice(chunk);
            }
        }
        _ => {
            for (dst, chunk) in out.iter_mut().zip(src.chunks_exact(8)) {
                dst[..8].copy_from_slice(chunk);
            }
        }
    }
    Ok(out)
}

/// Strongest-four layer indices, lower index winning ties. Unused slots are
/// `u32::MAX` when fewer than four weights are live.
pub fn strongest_four(weights: &[f32; TERRAIN_LAYER_COUNT as usize]) -> [u32; 4] {
    let mut used = [false; TERRAIN_LAYER_COUNT as usize];
    let mut out = [u32::MAX; 4];
    for slot in 0..4 {
        let mut best = -1.0f32;
        let mut idx = 0usize;
        for i in 0..TERRAIN_LAYER_COUNT as usize {
            if !used[i] && weights[i] > best {
                best = weights[i];
                idx = i;
            }
        }
        if best <= 0.0 {
            break;
        }
        used[idx] = true;
        out[slot] = idx as u32;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fifth_channel_decays_the_weakest() {
        let mut t = [0u8; TERRAIN_LAYER_COUNT as usize];
        t[0] = 80;
        t[1] = 70;
        t[2] = 60;
        t[3] = 50;
        t[8] = 10;
        enforce_four_nonzero(&mut t);
        assert_eq!(t[8], 0);
        assert!(t[0] > 0 && t[1] > 0 && t[2] > 0 && t[3] > 0);
        assert_eq!(t.iter().map(|&x| x as u32).sum::<u32>(), 255);
        assert_eq!(t.iter().filter(|&&x| x > 0).count(), 4);
    }

    #[test]
    fn equal_weights_drop_the_higher_index() {
        let mut t = [0u8; TERRAIN_LAYER_COUNT as usize];
        t[0] = 50;
        t[1] = 50;
        t[2] = 50;
        t[3] = 50;
        t[15] = 50;
        enforce_four_nonzero(&mut t);
        assert_eq!(t[15], 0);
        assert_eq!(t.iter().map(|&x| x as u32).sum::<u32>(), 255);
    }

    #[test]
    fn v2_shaped_eight_channel_texel_is_not_this_helper() {
        // Migration copies bytes; this helper is paint-only.
        let mut t = [0u8; TERRAIN_LAYER_COUNT as usize];
        for i in 0..8 {
            t[i] = 32;
        }
        let before = t;
        // If we *did* enforce, we'd drop four. Confirm the function does that
        // so paint cannot silently keep eight.
        enforce_four_nonzero(&mut t);
        assert_ne!(t, before);
        assert_eq!(t.iter().filter(|&&x| x > 0).count(), 4);
    }

    #[test]
    fn strongest_four_is_deterministic() {
        let mut w = [0.0f32; TERRAIN_LAYER_COUNT as usize];
        w[2] = 0.4;
        w[9] = 0.3;
        w[0] = 0.2;
        w[14] = 0.1;
        w[7] = 0.05;
        assert_eq!(strongest_four(&w), [2, 9, 0, 14]);
    }

    #[test]
    fn v2_sidecar_copies_eight_channels_and_zeros_the_rest() {
        let mut src = [0u8; 8];
        for i in 0..8 {
            src[i] = 20 + i as u8;
        }
        let out = migrate_sidecar_splat(2, &src, 1).unwrap();
        assert_eq!(&out[0][..8], &src);
        assert!(out[0][8..].iter().all(|&x| x == 0));
        assert_eq!(out[0].iter().filter(|&&x| x > 0).count(), 8);
    }

    #[test]
    fn v3_sidecar_copies_sixteen_channels_and_zeros_the_rest() {
        let mut src = [0u8; 16];
        src[0] = 100;
        src[15] = 155;
        let out = migrate_sidecar_splat(3, &src, 1).unwrap();
        assert_eq!(&out[0][..16], &src);
        assert!(out[0][16..].iter().all(|&x| x == 0));
        assert_eq!(out[0][15], 155);
    }

    #[test]
    fn v4_sidecar_is_a_straight_copy() {
        let mut src = [0u8; TERRAIN_LAYER_COUNT as usize];
        src[16] = 80;
        src[31] = 175;
        let out = migrate_sidecar_splat(4, &src, 1).unwrap();
        assert_eq!(out[0], src);
    }

    #[test]
    fn unknown_sidecar_version_is_refused() {
        assert!(migrate_sidecar_splat(1, &[0u8; 8], 1).is_err());
    }

    fn hash_u32(mut x: u32) -> u32 {
        x ^= x >> 16;
        x = x.wrapping_mul(0x7feb_352d);
        x ^= x >> 15;
        x.wrapping_mul(0x846c_a68b)
    }

    fn unit(x: u32) -> f32 {
        (hash_u32(x) as f32) / (u32::MAX as f32)
    }

    fn layer_albedo(i: usize) -> [f32; 3] {
        let h = i as f32 / TERRAIN_LAYER_COUNT as f32;
        let a = (h * std::f32::consts::TAU).sin() * 0.5 + 0.5;
        let b = (h * std::f32::consts::TAU * 1.7).cos() * 0.5 + 0.5;
        [a, 0.35 + 0.4 * b, 1.0 - a * 0.6]
    }

    fn weighted_rgb(weights: &[f32; TERRAIN_LAYER_COUNT as usize]) -> [f32; 3] {
        let mut rgb = [0.0f32; 3];
        let mut wsum = 0.0f32;
        for i in 0..TERRAIN_LAYER_COUNT as usize {
            if weights[i] <= 0.0 {
                continue;
            }
            let c = layer_albedo(i);
            rgb[0] += c[0] * weights[i];
            rgb[1] += c[1] * weights[i];
            rgb[2] += c[2] * weights[i];
            wsum += weights[i];
        }
        if wsum > 0.0 {
            rgb[0] /= wsum;
            rgb[1] /= wsum;
            rgb[2] /= wsum;
        }
        rgb
    }

    fn mask_strongest(
        weights: &[f32; TERRAIN_LAYER_COUNT as usize],
        n: usize,
    ) -> [f32; TERRAIN_LAYER_COUNT as usize] {
        let idx = strongest_four(weights);
        let mut out = [0.0f32; TERRAIN_LAYER_COUNT as usize];
        for slot in idx.iter().take(n) {
            if *slot != u32::MAX {
                out[*slot as usize] = weights[*slot as usize];
            }
        }
        out
    }

    fn srgb_to_lab(rgb: [f32; 3]) -> [f32; 3] {
        let lin = |c: f32| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let r = lin(rgb[0].clamp(0.0, 1.0));
        let g = lin(rgb[1].clamp(0.0, 1.0));
        let b = lin(rgb[2].clamp(0.0, 1.0));
        let x = 0.4124564 * r + 0.3575761 * g + 0.1804375 * b;
        let y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b;
        let z = 0.0193339 * r + 0.1191920 * g + 0.9503041 * b;
        let f = |t: f32| {
            if t > 0.008856 {
                t.cbrt()
            } else {
                7.787 * t + 16.0 / 116.0
            }
        };
        let fx = f(x / 0.95047);
        let fy = f(y);
        let fz = f(z / 1.08883);
        [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
    }

    fn ciede2000(a: [f32; 3], b: [f32; 3]) -> f32 {
        let lab1 = srgb_to_lab(a);
        let lab2 = srgb_to_lab(b);
        let (l1, a1, b1) = (lab1[0] as f64, lab1[1] as f64, lab1[2] as f64);
        let (l2, a2, b2) = (lab2[0] as f64, lab2[1] as f64, lab2[2] as f64);
        let c1 = (a1 * a1 + b1 * b1).sqrt();
        let c2 = (a2 * a2 + b2 * b2).sqrt();
        let c_bar = (c1 + c2) * 0.5;
        let c7 = c_bar.powi(7);
        let g = 0.5 * (1.0 - (c7 / (c7 + 25.0_f64.powi(7))).sqrt());
        let ap1 = a1 * (1.0 + g);
        let ap2 = a2 * (1.0 + g);
        let cp1 = (ap1 * ap1 + b1 * b1).sqrt();
        let cp2 = (ap2 * ap2 + b2 * b2).sqrt();
        let hp = |ap: f64, bb: f64| {
            if ap == 0.0 && bb == 0.0 {
                0.0
            } else {
                bb.atan2(ap).to_degrees().rem_euclid(360.0)
            }
        };
        let h1 = hp(ap1, b1);
        let h2 = hp(ap2, b2);
        let dl = l2 - l1;
        let dc = cp2 - cp1;
        let dh = if cp1 * cp2 == 0.0 {
            0.0
        } else if (h2 - h1).abs() <= 180.0 {
            h2 - h1
        } else if h2 <= h1 {
            h2 - h1 + 360.0
        } else {
            h2 - h1 - 360.0
        };
        let dhp = 2.0 * (cp1 * cp2).sqrt() * (dh.to_radians() * 0.5).sin();
        let l_bar = (l1 + l2) * 0.5;
        let c_bar_p = (cp1 + cp2) * 0.5;
        let h_bar = if cp1 * cp2 == 0.0 {
            h1 + h2
        } else if (h1 - h2).abs() <= 180.0 {
            (h1 + h2) * 0.5
        } else if h1 + h2 < 360.0 {
            (h1 + h2 + 360.0) * 0.5
        } else {
            (h1 + h2 - 360.0) * 0.5
        };
        let t = 1.0 - 0.17 * (h_bar - 30.0).to_radians().cos()
            + 0.24 * (2.0 * h_bar).to_radians().cos()
            + 0.32 * (3.0 * h_bar + 6.0).to_radians().cos()
            - 0.20 * (4.0 * h_bar - 63.0).to_radians().cos();
        let sl = 1.0 + 0.015 * (l_bar - 50.0).powi(2) / (20.0 + (l_bar - 50.0).powi(2)).sqrt();
        let sc = 1.0 + 0.045 * c_bar_p;
        let sh = 1.0 + 0.015 * c_bar_p * t;
        let dth = 30.0 * (-((h_bar - 275.0) / 25.0).powi(2)).exp();
        let rc = 2.0 * (c_bar_p.powi(7) / (c_bar_p.powi(7) + 25.0_f64.powi(7))).sqrt();
        let rt = -rc * (2.0 * dth).to_radians().sin();
        ((dl / sl).powi(2) + (dc / sc).powi(2) + (dhp / sh).powi(2) + rt * (dc / sc) * (dhp / sh))
            .sqrt() as f32
    }

    fn percentile(sorted: &[f32], p: f32) -> f32 {
        if sorted.is_empty() {
            return 0.0;
        }
        let i = ((p * (sorted.len() - 1) as f32).round() as usize).min(sorted.len() - 1);
        sorted[i]
    }

    fn sample_weights(seed: u32, live: usize) -> [f32; TERRAIN_LAYER_COUNT as usize] {
        let mut w = [0.0f32; TERRAIN_LAYER_COUNT as usize];
        for k in 0..live.min(4) {
            let idx = (hash_u32(seed.wrapping_add(k as u32 * 17)) as usize)
                % TERRAIN_LAYER_COUNT as usize;
            w[idx] += 0.35 + unit(seed.wrapping_add(k as u32 * 91)) * 0.65;
        }
        if live >= 5 {
            let idx = (hash_u32(seed.wrapping_add(999)) as usize) % TERRAIN_LAYER_COUNT as usize;
            w[idx] += 0.02 + unit(seed.wrapping_add(1001)) * 0.04;
        }
        w
    }

    #[test]
    fn strongest_four_stays_inside_ciede2000_budget_against_full_blend() {
        let mut de4 = Vec::new();
        let mut de3 = Vec::new();
        let mut discarded = Vec::new();
        for live in [2usize, 3, 4, 5] {
            for n in 0..80u32 {
                let w = sample_weights(live as u32 * 10_000 + n, live);
                let full = weighted_rgb(&w);
                let four = weighted_rgb(&mask_strongest(&w, 4));
                let three = weighted_rgb(&mask_strongest(&w, 3));
                de4.push(ciede2000(full, four));
                de3.push(ciede2000(full, three));
                let sum: f32 = w.iter().sum();
                let kept: f32 = mask_strongest(&w, 4).iter().sum();
                if sum > 0.0 {
                    discarded.push(((sum - kept) / sum).max(0.0));
                }
            }
        }
        de4.sort_by(|a, b| a.partial_cmp(b).unwrap());
        discarded.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let med = percentile(&de4, 0.5);
        let p95 = percentile(&de4, 0.95);
        let disc95 = percentile(&discarded, 0.95);
        assert!(
            med < 1.0 && p95 < 3.0,
            "strongest-four CIEDE2000 median={med:.3} p95={p95:.3} (budget 1.0 / 3.0); strongest-three p95={:.3}",
            percentile(
                &{
                    let mut t = de3;
                    t.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    t
                },
                0.95
            )
        );
        assert!(
            disc95 < 0.05,
            "discarded normalized weight p95={disc95:.3} (budget 0.05)"
        );
    }
}
