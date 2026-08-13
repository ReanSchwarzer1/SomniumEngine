//! Packed RGBA splat helpers (Phase XV-C).
//!
//! Sixteen global weights live in four RGBA8 textures. Painting and biome
//! rules keep **at most four non-zero channels** per stored texel; bilinear
//! filtering can still expose more at pixel boundaries, which the shader
//! resolves with strongest-four selection.

use super::textures::{SplatTexel, TERRAIN_LAYER_COUNT};

/// RGBA control maps for sixteen layers (0–3, 4–7, 8–11, 12–15).
pub const SPLAT_MAP_COUNT: usize = 4;

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

/// De-interleave one CPU texel into four RGBA bytes groups.
pub fn deinterleave(texel: &SplatTexel) -> [[u8; 4]; SPLAT_MAP_COUNT] {
    [
        [texel[0], texel[1], texel[2], texel[3]],
        [texel[4], texel[5], texel[6], texel[7]],
        [texel[8], texel[9], texel[10], texel[11]],
        [texel[12], texel[13], texel[14], texel[15]],
    ]
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
}
