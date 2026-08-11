//! The macro tier: whole-terrain, low-frequency colour (Phase 25D).
//!
//! Eight photographed materials give a texel of ground that holds up under a
//! boot. They cannot give a *landscape*, because every patch of grass in the
//! terrain is the same patch of grass — the splatmap chooses among eight
//! materials and has nothing else to say. Real ground varies continuously at
//! scales no tiling texture reaches: a slope that gets more sun is paler along
//! its whole length, a hollow that holds water is darker and greener, a ridge
//! is scoured. At distance the detail layers converge to their own mean and the
//! terrain goes flat and uniform, which is the "matte painting" read.
//!
//! O3DE solves this with a **macro material** — an authored colour and normal
//! map covering the terrain at low frequency, with the detail materials
//! composited over it (`TerrainMacroHelpers.azsli`, and `GetDetailColor`'s
//! blend modes in `TerrainDetailHelpers.azsli`). Somnium has no authored
//! satellite map and no way to paint one yet, so the macro layer is **derived
//! from the landform** instead: altitude, macro-scale slope, how much of a
//! hollow a point sits in, and two octaves of large-scale noise. That is data
//! the heightfield already has, and deriving it means the variation
//! *correlates* with the terrain rather than floating over it.
//!
//! # Value convention
//!
//! Texels are **display-referred**, centred on 0.5, and the shader composites
//! them in the same approximately-perceptual space `terrain_material.wgsl`
//! already blends albedo in (Phase 25E's `sqrt`). 0.5 is the neutral value for
//! the overlay blend, so a macro map of uniform 0.5 leaves the detail exactly
//! as it was — which is what makes `macro_strength = 0` and "no macro map"
//! agree, and what the `a_flat_terrain_stays_neutral` test pins.
//!
//! The texture is therefore **not** sRGB-tagged: these are blend operands, not
//! colours to be linearised.

use super::heightmap::value_noise;

/// Texels per edge of the macro map.
///
/// Deliberately coarse. The macro tier's entire job is the frequencies the
/// detail layers cannot reach; at 512 over a 1 km terrain a texel is ~2 m, and
/// anything finer is work the detail path is already doing better. It is also
/// 1 MB, so it costs nothing next to the layer arrays.
pub const MACRO_SIZE: u32 = 512;

/// Blend of the detail albedo against the macro colour.
///
/// Mirrors O3DE's `TextureBlendMode` (`BlendUtility.azsli`) and must stay in
/// step with `terrain_macro_blend` in `terrain_material.wgsl`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum MacroBlendMode {
    /// `factor * detail * macro`. Right when the macro map is a tint.
    Multiply = 0,
    /// Straight cross-fade to the macro colour. Right when the macro map is
    /// authored imagery and should win outright at distance.
    Lerp = 1,
    /// `detail + 2*macro - 1`. A contrastier push than multiply.
    LinearLight = 2,
    /// Keeps the detail's own light and dark structure and pushes its colour
    /// toward the macro. The default, because the detail texture is the thing
    /// worth preserving and the macro is here for hue and level.
    Overlay = 3,
}

/// How hard the macro tier asserts itself, as the `factor` in O3DE's
/// `ApplyTextureBlend`.
///
/// Low on purpose. Everything above roughly 0.6 starts reading as a coloured
/// gel over the ground rather than as the ground varying.
pub const DEFAULT_MACRO_STRENGTH: f32 = 0.45;

/// A generated macro map, ready to upload.
#[derive(Clone)]
pub struct MacroMap {
    /// `MACRO_SIZE² * 4` bytes, RGBA8, row-major.
    pub texels: Vec<u8>,
    pub size: u32,
}

/// Bilinear height lookup in a row-major heightfield, clamped at the edges.
fn height_at(heights: &[f32], total_x: u32, total_z: u32, x: f32, z: f32) -> f32 {
    let cx = x.clamp(0.0, (total_x - 1) as f32);
    let cz = z.clamp(0.0, (total_z - 1) as f32);
    let x0 = cx.floor() as u32;
    let z0 = cz.floor() as u32;
    let x1 = (x0 + 1).min(total_x - 1);
    let z1 = (z0 + 1).min(total_z - 1);
    let fx = cx - x0 as f32;
    let fz = cz - z0 as f32;
    let idx = |xi: u32, zi: u32| heights[(zi * total_x + xi) as usize];
    let a = idx(x0, z0) * (1.0 - fx) + idx(x1, z0) * fx;
    let b = idx(x0, z1) * (1.0 - fx) + idx(x1, z1) * fx;
    a * (1.0 - fz) + b * fz
}

/// Two octaves of value noise at the wavelengths the detail layers cannot
/// express — hundreds of metres and tens of metres.
fn large_scale(u: f32, v: f32, seed: u32) -> f32 {
    value_noise(u * 3.0, v * 3.0, seed) * 0.65
        + value_noise(u * 11.0, v * 11.0, seed ^ 0x9E37) * 0.35
}

/// Build the macro map for a heightfield.
///
/// `heights` is row-major `total_x * total_z` in raw units; `height_scale` and
/// `cell_size` convert to metres, which is what makes the slope and hollow
/// terms independent of the terrain's resolution.
#[must_use]
pub fn generate(
    heights: &[f32],
    total_x: u32,
    total_z: u32,
    cell_size: f32,
    height_scale: f32,
    seed: u32,
) -> MacroMap {
    let size = MACRO_SIZE;
    let mut texels = Vec::with_capacity((size * size * 4) as usize);

    // Altitude is only meaningful relative to this terrain's own range — a
    // 40 m dune and a 900 m massif should both read as "high near the top".
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for h in heights {
        lo = lo.min(*h);
        hi = hi.max(*h);
    }
    let span = (hi - lo).max(1e-3);

    // The hollow term compares a point against a neighbourhood *tens of metres*
    // wide, not against its neighbours — a boulder is not a valley.
    let hollow_radius_m = 45.0;
    let hollow_step = (hollow_radius_m / cell_size).max(1.0);

    for zi in 0..size {
        for xi in 0..size {
            // Macro texel centre in heightfield coordinates.
            let u = (xi as f32 + 0.5) / size as f32;
            let v = (zi as f32 + 0.5) / size as f32;
            let hx = u * (total_x - 1) as f32;
            let hz = v * (total_z - 1) as f32;

            let h = height_at(heights, total_x, total_z, hx, hz);
            let altitude = ((h - lo) / span).clamp(0.0, 1.0);

            // Macro-scale slope: metres of rise over the sample distance,
            // which is why both terms are converted before the ratio.
            let s = hollow_step;
            let dx = height_at(heights, total_x, total_z, hx + s, hz)
                - height_at(heights, total_x, total_z, hx - s, hz);
            let dz = height_at(heights, total_x, total_z, hx, hz + s)
                - height_at(heights, total_x, total_z, hx, hz - s);
            let run = 2.0 * s * cell_size;
            let grade = ((dx * dx + dz * dz).sqrt() * height_scale / run).min(2.0);

            // Positive where the point sits below its surroundings: a hollow
            // that collects water and stays green, versus a scoured ridge.
            let mean = 0.25
                * (height_at(heights, total_x, total_z, hx + s, hz)
                    + height_at(heights, total_x, total_z, hx - s, hz)
                    + height_at(heights, total_x, total_z, hx, hz + s)
                    + height_at(heights, total_x, total_z, hx, hz - s));
            let hollow = (((mean - h) * height_scale) / hollow_radius_m).clamp(-1.0, 1.0);

            // Two decorrelated noise fields so the patchiness drifts in hue and
            // not only in brightness — one field would only ever dim and
            // brighten, which reads as dirt on a lens.
            let n_value = large_scale(u, v, seed) * 2.0 - 1.0;
            let n_hue = large_scale(u + 3.7, v + 1.9, seed ^ 0x5151_2A3B) * 2.0 - 1.0;

            // 0.5 is neutral for the overlay blend, so every term below is a
            // signed push away from "leave the detail alone".
            let mut r = 0.5;
            let mut g = 0.5;
            let mut b = 0.5;

            // High and steep: sun-bleached, drier, slightly warm.
            let exposed = (altitude * 0.6 + grade * 0.35).min(1.0);
            r += exposed * 0.075;
            g += exposed * 0.060;
            b += exposed * 0.035;

            // Hollows: darker, greener, cooler.
            r -= hollow.max(0.0) * 0.085;
            g -= hollow.max(0.0) * 0.030;
            b -= hollow.max(0.0) * 0.075;

            // Large-scale patchiness.
            r += n_value * 0.055 + n_hue * 0.030;
            g += n_value * 0.055 - n_hue * 0.010;
            b += n_value * 0.045 - n_hue * 0.035;

            // Alpha is a per-texel strength. Steep ground is where the
            // triplanar cliff projection takes over and where a top-down macro
            // lookup is stretched anyway, so the macro backs off there rather
            // than smearing a colour down the rock.
            let a = (1.0 - (grade - 0.6).max(0.0) / 0.9).clamp(0.15, 1.0);

            let enc = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
            texels.extend_from_slice(&[enc(r), enc(g), enc(b), enc(a)]);
        }
    }

    MacroMap { texels, size }
}

/// Upload a generated macro map as a single non-sRGB RGBA8 texture.
pub fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    map: &MacroMap,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Terrain Macro Map"),
        size: wgpu::Extent3d {
            width: map.size,
            height: map.size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // Not `Rgba8UnormSrgb`: see the module header. These are blend
        // operands already in a display-referred space, and linearising them
        // on the way in would put the overlay's 0.5 neutral point at 0.21.
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &map.texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(map.size * 4),
            rows_per_image: Some(map.size),
        },
        wgpu::Extent3d {
            width: map.size,
            height: map.size,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(n: u32) -> Vec<f32> {
        vec![10.0; (n * n) as usize]
    }

    /// Slope running up along +X, in raw units.
    fn ramp(n: u32, per_cell: f32) -> Vec<f32> {
        (0..n * n).map(|i| (i % n) as f32 * per_cell).collect()
    }

    /// A cone-shaped basin: lowest in the middle.
    fn basin(n: u32) -> Vec<f32> {
        let c = (n - 1) as f32 * 0.5;
        (0..n * n)
            .map(|i| {
                let x = (i % n) as f32 - c;
                let z = (i / n) as f32 - c;
                (x * x + z * z).sqrt()
            })
            .collect()
    }

    fn texel(map: &MacroMap, x: u32, z: u32) -> [u8; 4] {
        let i = ((z * map.size + x) * 4) as usize;
        [
            map.texels[i],
            map.texels[i + 1],
            map.texels[i + 2],
            map.texels[i + 3],
        ]
    }

    #[test]
    fn the_map_is_the_declared_size_and_fully_written() {
        let m = generate(&flat(64), 64, 64, 1.0, 1.0, 7);
        assert_eq!(m.size, MACRO_SIZE);
        assert_eq!(m.texels.len(), (MACRO_SIZE * MACRO_SIZE * 4) as usize);
    }

    #[test]
    fn a_flat_terrain_stays_near_the_neutral_value() {
        // With no landform to describe, only the noise speaks, and it must stay
        // small — 0.5 is "leave the detail alone" and a macro tier that drifts
        // far from it on featureless ground is tinting the whole world.
        let m = generate(&flat(64), 64, 64, 1.0, 1.0, 11);
        let mut worst = 0.0f32;
        for i in (0..m.texels.len()).step_by(4) {
            for c in 0..3 {
                worst = worst.max((f32::from(m.texels[i + c]) / 255.0 - 0.5).abs());
            }
        }
        assert!(worst < 0.12, "flat terrain drifted {worst} from neutral");
    }

    #[test]
    fn hollows_come_out_darker_than_ridges() {
        // The whole claim of the tier: variation that correlates with the
        // landform. In a basin the centre is a hollow and the rim is exposed.
        let n = 256;
        let m = generate(&basin(n), n, n, 1.0, 1.0, 3);
        let lum = |t: [u8; 4]| f32::from(t[0]) + f32::from(t[1]) + f32::from(t[2]);

        let centre = lum(texel(&m, MACRO_SIZE / 2, MACRO_SIZE / 2));
        let rim = lum(texel(&m, MACRO_SIZE - 12, MACRO_SIZE / 2));
        assert!(
            centre < rim,
            "basin centre {centre} not darker than rim {rim}"
        );
    }

    #[test]
    fn steep_ground_backs_the_macro_off() {
        let n = 128;
        let flat_map = generate(&flat(n), n, n, 1.0, 1.0, 5);
        let steep = generate(&ramp(n, 4.0), n, n, 1.0, 1.0, 5);
        let a = |m: &MacroMap| texel(m, MACRO_SIZE / 2, MACRO_SIZE / 2)[3];
        assert_eq!(
            a(&flat_map),
            255,
            "flat ground should take the macro in full"
        );
        assert!(
            a(&steep) < 200,
            "steep ground alpha {} did not back off",
            a(&steep)
        );
    }

    #[test]
    fn the_same_terrain_and_seed_give_the_same_map() {
        // Both sides of every A/B capture depend on this.
        let h = basin(64);
        let a = generate(&h, 64, 64, 1.0, 1.0, 9);
        let b = generate(&h, 64, 64, 1.0, 1.0, 9);
        assert_eq!(a.texels, b.texels);
    }

    #[test]
    fn the_slope_term_does_not_depend_on_cell_size() {
        // A 1 km terrain at 1 m cells and the same shape at 2 m cells describe
        // the same hillside, so they must get the same macro treatment. This is
        // what the metre conversions in `generate` are for.
        let n = 128;
        let coarse = generate(&ramp(n, 2.0), n, n, 2.0, 1.0, 4);
        let fine = generate(&ramp(n, 1.0), n, n, 1.0, 1.0, 4);
        let (ca, fa) = (
            texel(&coarse, MACRO_SIZE / 2, MACRO_SIZE / 2)[3],
            texel(&fine, MACRO_SIZE / 2, MACRO_SIZE / 2)[3],
        );
        assert_eq!(ca, fa, "same grade, different cell size, different alpha");
    }
}
