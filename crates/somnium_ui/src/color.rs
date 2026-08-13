//! Linear RGB(A) ↔ sRGB ↔ HSV ↔ hex helpers for the Iris colour picker (26-F).
//!
//! Storage is linear. Swatches and the spectrum use an approximate sRGB encode
//! (`pow(x, 1/2.2)`). Hex edits sRGB bytes.

/// Approximate linear → sRGB (display).
pub fn linear_to_srgb(x: f32) -> f32 {
    x.max(0.0).powf(1.0 / 2.2)
}

/// Approximate sRGB → linear (storage).
pub fn srgb_to_linear(x: f32) -> f32 {
    x.max(0.0).powf(2.2)
}

pub fn linear_rgba_to_srgb_u8(linear: [f32; 4]) -> [u8; 4] {
    [
        (linear_to_srgb(linear[0]) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8,
        (linear_to_srgb(linear[1]) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8,
        (linear_to_srgb(linear[2]) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8,
        (linear[3].clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

pub fn srgb_u8_to_linear_rgba(bytes: [u8; 4]) -> [f32; 4] {
    [
        srgb_to_linear(bytes[0] as f32 / 255.0),
        srgb_to_linear(bytes[1] as f32 / 255.0),
        srgb_to_linear(bytes[2] as f32 / 255.0),
        bytes[3] as f32 / 255.0,
    ]
}

/// HSV in 0..360 / 0..1 / 0..1 from sRGB 0..1 channels.
pub fn srgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let h = if delta < 1e-6 {
        0.0
    } else if (max - r).abs() < 1e-6 {
        60.0 * (((g - b) / delta) % 6.0)
    } else if (max - g).abs() < 1e-6 {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };
    let h = if h < 0.0 { h + 360.0 } else { h };
    let s = if max < 1e-6 { 0.0 } else { delta / max };
    (h, s, max)
}

pub fn hsv_to_srgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let c = v * s;
    let h_ = (h % 360.0) / 60.0;
    let x = c * (1.0 - ((h_ % 2.0) - 1.0).abs());
    let (r1, g1, b1) = match h_.floor() as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (r1 + m, g1 + m, b1 + m)
}

pub fn linear_to_hsv(linear: [f32; 3]) -> (f32, f32, f32) {
    srgb_to_hsv(
        linear_to_srgb(linear[0]),
        linear_to_srgb(linear[1]),
        linear_to_srgb(linear[2]),
    )
}

pub fn hsv_to_linear(h: f32, s: f32, v: f32) -> [f32; 3] {
    let (r, g, b) = hsv_to_srgb(h, s, v);
    [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b)]
}

pub fn linear_to_hex(linear: [f32; 3]) -> String {
    let u = linear_rgba_to_srgb_u8([linear[0], linear[1], linear[2], 1.0]);
    format!("#{:02X}{:02X}{:02X}", u[0], u[1], u[2])
}

pub fn hex_to_linear(text: &str) -> Option<[f32; 3]> {
    let s = text.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    let r = ((n >> 16) & 0xFF) as u8;
    let g = ((n >> 8) & 0xFF) as u8;
    let b = (n & 0xFF) as u8;
    let rgba = srgb_u8_to_linear_rgba([r, g, b, 255]);
    Some([rgba[0], rgba[1], rgba[2]])
}

/// Split a coefficient triple into a unit tint and a magnitude so absorption
/// `(0.22, 0.07, 0.03)` reads as a colour rather than near-black.
pub fn split_magnitude(rgb: [f32; 3]) -> ([f32; 3], f32) {
    let mag = rgb[0].max(rgb[1]).max(rgb[2]).max(1e-6);
    ([rgb[0] / mag, rgb[1] / mag, rgb[2] / mag], mag)
}

pub fn join_magnitude(tint: [f32; 3], mag: f32) -> [f32; 3] {
    [tint[0] * mag, tint[1] * mag, tint[2] * mag]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrips_white_and_a_mid_grey() {
        let white = hex_to_linear("#FFFFFF").unwrap();
        assert!((white[0] - 1.0).abs() < 1e-4);
        assert_eq!(linear_to_hex([1.0, 1.0, 1.0]), "#FFFFFF");
        let mid = hex_to_linear("#808080").unwrap();
        assert_eq!(linear_to_hex(mid), "#808080");
    }

    #[test]
    fn hsv_red_is_hue_zero() {
        let (h, s, v) = srgb_to_hsv(1.0, 0.0, 0.0);
        assert!((h - 0.0).abs() < 1e-3);
        assert!((s - 1.0).abs() < 1e-3);
        assert!((v - 1.0).abs() < 1e-3);
        let (r, g, b) = hsv_to_srgb(0.0, 1.0, 1.0);
        assert!((r - 1.0).abs() < 1e-3 && g.abs() < 1e-3 && b.abs() < 1e-3);
    }

    #[test]
    fn absorption_split_keeps_tint_and_magnitude() {
        let rgb = [0.22, 0.07, 0.03];
        let (tint, mag) = split_magnitude(rgb);
        assert!((mag - 0.22).abs() < 1e-5);
        assert!((tint[0] - 1.0).abs() < 1e-5);
        let back = join_magnitude(tint, mag);
        for i in 0..3 {
            assert!((back[i] - rgb[i]).abs() < 1e-5);
        }
    }

    #[test]
    fn linear_srgb_encode_is_monotonic() {
        assert!(linear_to_srgb(0.0) < linear_to_srgb(0.2));
        assert!(linear_to_srgb(0.2) < linear_to_srgb(1.0));
    }
}
