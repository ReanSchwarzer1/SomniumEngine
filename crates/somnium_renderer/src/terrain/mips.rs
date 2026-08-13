//! Channel-aware mip generation for packed terrain arrays (Phase XV-B).
//!
//! Independently re-expresses Toksvig / Godot-style roughness compensation:
//! unresolved normal variance at coarser mips is folded into roughness so
//! distant specular does not sparkle. Albedo is filtered in linear space.
//! Normals are averaged as vectors and renormalized. Height and AO stay linear.
//!
//! Named validation reference: Godot 4.7.1 `Image::generate_mipmap_roughness`
//! (MIT). No Godot source is copied.

/// Packed albedo (sRGB RGB + linear height A) or surface (normal XY, roughness, AO).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackedKind {
    AlbedoHeight,
    Surface,
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn unpack_nxy(r: u8, g: u8) -> [f32; 3] {
    let x = f32::from(r) / 255.0 * 2.0 - 1.0;
    let y = f32::from(g) / 255.0 * 2.0 - 1.0;
    let z = (1.0 - x * x - y * y).max(0.0).sqrt();
    [x, y, z]
}

/// Godot-reference roughness limiter from mean-normal length `r` in (0, 1].
///
/// `kappa = (3r - r³) / (1 - r²)`, `variance = 0.25 / kappa`, then
/// `sqrt(rough² + min(3·var, 0.4²))`.
pub fn toksvig_roughness(source_rough: f32, mean_normal_length: f32) -> f32 {
    let r = mean_normal_length.clamp(0.0, 1.0);
    if r >= 0.999 || r <= 0.0 {
        return source_rough.clamp(0.04, 1.0);
    }
    let kappa = (3.0 * r - r * r * r) / (1.0 - r * r);
    let variance = 0.25 / kappa.max(1e-4);
    let extra = (3.0 * variance).min(0.4 * 0.4);
    (source_rough * source_rough + extra)
        .sqrt()
        .clamp(0.04, 1.0)
}

fn texel(data: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * width + x) * 4) as usize;
    [data[i], data[i + 1], data[i + 2], data[i + 3]]
}

/// Box-filter one mip with channel semantics. `src` is `width × height` RGBA8.
pub fn downsample(src: &[u8], width: u32, height: u32, kind: PackedKind) -> (u32, u32, Vec<u8>) {
    let dw = (width / 2).max(1);
    let dh = (height / 2).max(1);
    let mut dst = vec![0u8; (dw * dh * 4) as usize];
    for y in 0..dh {
        for x in 0..dw {
            let x0 = (x * 2).min(width - 1);
            let y0 = (y * 2).min(height - 1);
            let x1 = (x0 + 1).min(width - 1);
            let y1 = (y0 + 1).min(height - 1);
            let samples = [
                texel(src, width, x0, y0),
                texel(src, width, x1, y0),
                texel(src, width, x0, y1),
                texel(src, width, x1, y1),
            ];
            let n = samples.len() as f32;
            let out = match kind {
                PackedKind::AlbedoHeight => {
                    let mut lin = [0.0f32; 3];
                    let mut height = 0.0f32;
                    for s in samples {
                        lin[0] += srgb_to_linear(f32::from(s[0]) / 255.0);
                        lin[1] += srgb_to_linear(f32::from(s[1]) / 255.0);
                        lin[2] += srgb_to_linear(f32::from(s[2]) / 255.0);
                        height += f32::from(s[3]) / 255.0;
                    }
                    [
                        (linear_to_srgb(lin[0] / n) * 255.0).round() as u8,
                        (linear_to_srgb(lin[1] / n) * 255.0).round() as u8,
                        (linear_to_srgb(lin[2] / n) * 255.0).round() as u8,
                        ((height / n) * 255.0).round() as u8,
                    ]
                }
                PackedKind::Surface => {
                    let mut nsum = [0.0f32; 3];
                    let mut rough = 0.0f32;
                    let mut ao = 0.0f32;
                    for s in samples {
                        let v = unpack_nxy(s[0], s[1]);
                        nsum[0] += v[0];
                        nsum[1] += v[1];
                        nsum[2] += v[2];
                        rough += f32::from(s[2]) / 255.0;
                        ao += f32::from(s[3]) / 255.0;
                    }
                    let mean = [nsum[0] / n, nsum[1] / n, nsum[2] / n];
                    let len = (mean[0] * mean[0] + mean[1] * mean[1] + mean[2] * mean[2])
                        .sqrt()
                        .max(1e-6);
                    let nn = [mean[0] / len, mean[1] / len, mean[2] / len];
                    let r = toksvig_roughness(rough / n, len.min(1.0));
                    [
                        ((nn[0] * 0.5 + 0.5) * 255.0).round() as u8,
                        ((nn[1] * 0.5 + 0.5) * 255.0).round() as u8,
                        (r * 255.0).round() as u8,
                        ((ao / n) * 255.0).round() as u8,
                    ]
                }
            };
            let i = ((y * dw + x) * 4) as usize;
            dst[i..i + 4].copy_from_slice(&out);
        }
    }
    (dw, dh, dst)
}

/// Full mip chain including level 0.
pub fn build_mip_chain(
    data: &[u8],
    width: u32,
    height: u32,
    kind: PackedKind,
) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels = vec![(width, height, data.to_vec())];
    let (mut w, mut h) = (width, height);
    while w > 1 || h > 1 {
        let prev = &levels.last().unwrap().2;
        let (nw, nh, next) = downsample(prev, w, h, kind);
        w = nw;
        h = nh;
        levels.push((w, h, next));
    }
    levels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toksvig_raises_roughness_when_normals_cancel() {
        let flat = toksvig_roughness(0.2, 1.0);
        let cancelled = toksvig_roughness(0.2, 0.4);
        assert!(cancelled > flat, "{cancelled} vs {flat}");
    }

    #[test]
    fn albedo_mip_is_not_a_byte_average_of_srgb() {
        // Two texels: black and white. Linear mean is 0.5, sRGB encode ~186,
        // byte average of 0 and 255 is 127.
        let src = vec![
            0u8, 0, 0, 128, 255, 255, 255, 128, 0, 0, 0, 128, 255, 255, 255, 128,
        ];
        let (_, _, mip) = downsample(&src, 2, 2, PackedKind::AlbedoHeight);
        assert!(mip[0] > 160, "expected linear-aware grey, got {}", mip[0]);
    }

    #[test]
    fn toksvig_matches_godot_generate_mipmap_roughness_limiter() {
        // Named validation reference: Godot 4.7.1 `Image::generate_mipmap_roughness`
        // (MIT). Independently re-expressed — this pins the published limiter,
        // not Godot source.
        //
        //   kappa = (3r − r³) / (1 − r²)
        //   variance = 0.25 / kappa
        //   roughness = sqrt(source² + min(3·variance, 0.4²))
        let r = 0.7f32;
        let source = 0.3f32;
        let kappa = (3.0 * r - r * r * r) / (1.0 - r * r);
        let variance = 0.25 / kappa;
        let expected = (source * source + (3.0 * variance).min(0.16)).sqrt();
        let got = toksvig_roughness(source, r);
        assert!((got - expected).abs() < 1e-5, "{got} vs {expected}");
    }
}
