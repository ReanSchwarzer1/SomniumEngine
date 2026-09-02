//! Horizon angles over the heightfield (Phase TSUSHIMA-B and TSUSHIMA-C).
//!
//! For each texel and each of eight azimuths, the maximum elevation angle at
//! which the terrain itself blocks the sky. Two consumers, one bake:
//!
//! * **TSUSHIMA-B, direct.** Is the sun below the stored angle in its azimuth?
//!   Then this texel is in shadow — at *any* distance, with no cascade
//!   involved. `SHADOW_DISTANCE` is a compile-time 100 m
//!   (`shadow/cascade.rs`), so before this the entire landscape past the
//!   nearest hundred metres was unshadowed, and the hills in every vista
//!   capture read as painted shapes rather than as land.
//! * **TSUSHIMA-C, indirect.** The integral of the eight angles over azimuth
//!   is how much sky the texel can see. That is the quantity nothing in the
//!   renderer had: GTAO is screen-space and radius-bounded, the per-layer AO
//!   is texture-scale, and the SH probe volume is 4×4×4 over the whole view.
//!   None of them knows a valley floor sees less sky than the ridge above it.
//!
//! # Why eight azimuths
//!
//! Eight is the published choice (Max 1988; Sloan & Cohen 2000) and it is also
//! the artifact source: a rotating sun makes the shadow edge snap between
//! compass bearings unless the two bracketing azimuths are interpolated. The
//! shader does interpolate them; see `terrain_horizon_shadow` in
//! `terrain_material.wgsl`.
//!
//! # Why the march is multi-resolution
//!
//! A naive march is `size² × 8 × steps`, and the steps have to reach across
//! the whole terrain or a mountain casts no shadow into the next valley. At
//! 1024² with a 512-step reach that is two billion samples and it does not
//! finish inside a load.
//!
//! Instead the occluder field is a **max-downsampled pyramid** and the march
//! strides by the mip it is reading: unit steps out to eight texels, then
//! doubling. Roughly forty samples per azimuth reach the far edge of a 1 km
//! terrain instead of forty-odd metres.
//!
//! Max-downsampling is what makes this safe rather than merely fast. A mean
//! would let a thin ridge vanish into its neighbours and stop casting; a max
//! keeps every occluder and can only ever *over*-shadow, by treating a wide
//! cell as though its tallest point filled it. At the distances where coarse
//! mips are read, the sun's own penumbra is far wider than that error.

use rayon::prelude::*;

/// Azimuths baked per texel. Four fit one RGBA8 texture, so eight is two.
pub const AZIMUTHS: usize = 8;

/// Texels per edge of the horizon and sky-visibility maps.
///
/// 1024 over a 1 km terrain is one texel per metre — finer than the terrain's
/// own vertex spacing is coarse, and far finer than the shadow it carries
/// needs, because this term only takes over past the last cascade at 100 m
/// where a metre is well under a pixel. Three RGBA8 maps at this size are
/// 12 MiB, against the virtual-texture atlas's fixed 64 MiB.
pub const HORIZON_SIZE: u32 = 1024;

/// Compass bearings, in level-0 texel steps. Index order is the packing order:
/// `angles_a` holds 0..3 in RGBA, `angles_b` holds 4..7.
///
/// Bearing `i` is at `i * 45°`, measured from +X toward +Z, which is what
/// `atan2(dir.z, dir.x) * 4/PI` recovers in the shader.
const DIRS: [(i32, i32); AZIMUTHS] = [
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
];

/// Unit XZ of each bearing, for the bent-direction accumulation.
const DIR_UNIT: [(f32, f32); AZIMUTHS] = [
    (1.0, 0.0),
    (std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2),
    (0.0, 1.0),
    (-std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2),
    (-1.0, 0.0),
    (
        -std::f32::consts::FRAC_1_SQRT_2,
        -std::f32::consts::FRAC_1_SQRT_2,
    ),
    (0.0, -1.0),
    (
        std::f32::consts::FRAC_1_SQRT_2,
        -std::f32::consts::FRAC_1_SQRT_2,
    ),
];

/// Baked terrain-space visibility, ready to upload.
///
/// Three RGBA8 maps rather than one wider format: RGBA8 is filterable
/// everywhere, and the shader wants bilinear on all three.
#[derive(Clone)]
pub struct HorizonMaps {
    /// Azimuths 0..3, one per channel. Angle / (π/2), quantised to 8 bits.
    pub angles_a: Vec<u8>,
    /// Azimuths 4..7.
    pub angles_b: Vec<u8>,
    /// RGB = bent direction × 0.5 + 0.5, A = cosine-weighted sky visibility.
    pub sky: Vec<u8>,
    pub size: u32,
}

/// Quantise an elevation angle in `[0, π/2)` to 8 bits.
///
/// A quarter of a degree per step, which is finer than the sun's own 0.53°
/// disc — so quantisation is never the limiting error in the shadow's edge.
fn pack_angle(angle: f32) -> u8 {
    ((angle / std::f32::consts::FRAC_PI_2).clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Cosine-weighted fraction of the hemisphere visible above `horizon`.
///
/// For one azimuth with horizon angle `a`, the cosine-weighted visible
/// fraction of that azimuthal slice is `cos²(a)` — the closed form of
/// `∫ sin θ cos θ dθ` from `a` to `π/2`, normalised. Averaging the eight
/// slices is the cheap quadrature.
pub fn sky_visibility(horizon: &[f32; AZIMUTHS]) -> f32 {
    let mut acc = 0.0;
    for &a in horizon {
        let c = a.cos();
        acc += c * c;
    }
    acc / AZIMUTHS as f32
}

/// The average unoccluded direction — the landscape-scale bent normal.
///
/// Each azimuth contributes a direction at the midpoint of its visible arc,
/// weighted by how much sky that arc holds. On flat ground every slice is
/// unoccluded, every midpoint is 45° up, the horizontal parts cancel and the
/// result is straight up — which is the property `flat_ground_looks_straight_up`
/// pins, and the reason this can be blended against the geometric normal
/// without a special case for "no occlusion".
pub fn bent_direction(horizon: &[f32; AZIMUTHS]) -> [f32; 3] {
    let (mut x, mut y, mut z) = (0.0f32, 0.0f32, 0.0f32);
    for (i, &a) in horizon.iter().enumerate() {
        let mid = (a + std::f32::consts::FRAC_PI_2) * 0.5;
        let c = a.cos();
        let w = c * c;
        let (dx, dz) = DIR_UNIT[i];
        x += dx * mid.cos() * w;
        y += mid.sin() * w;
        z += dz * mid.cos() * w;
    }
    let len = (x * x + y * y + z * z).sqrt();
    if len < 1e-5 {
        return [0.0, 1.0, 0.0];
    }
    [x / len, y / len, z / len]
}

/// A max-downsampled pyramid of world-space occluder heights.
struct MaxPyramid {
    /// `levels[0]` is `size × size`; each further level halves both axes.
    levels: Vec<Vec<f32>>,
    dims: Vec<(usize, usize)>,
}

impl MaxPyramid {
    fn build(level0: Vec<f32>, size: usize) -> Self {
        let mut levels = vec![level0];
        let mut dims = vec![(size, size)];
        let (mut w, mut h) = (size, size);
        while w > 1 && h > 1 {
            let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
            let prev = levels.last().expect("pyramid always has a level");
            let mut next = vec![f32::NEG_INFINITY; nw * nh];
            for z in 0..nh {
                for x in 0..nw {
                    let mut m = f32::NEG_INFINITY;
                    for (ox, oz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + ox).min(w - 1);
                        let sz = (z * 2 + oz).min(h - 1);
                        m = m.max(prev[sz * w + sx]);
                    }
                    next[z * nw + x] = m;
                }
            }
            levels.push(next);
            dims.push((nw, nh));
            w = nw;
            h = nh;
        }
        Self { levels, dims }
    }

    fn max_mip(&self) -> u32 {
        (self.levels.len() - 1) as u32
    }

    /// The occluder height covering level-0 texel `(x, z)` at `mip`.
    fn sample(&self, mip: u32, x: i64, z: i64) -> f32 {
        let m = mip as usize;
        let (w, h) = self.dims[m];
        let mx = ((x >> m).max(0) as usize).min(w - 1);
        let mz = ((z >> m).max(0) as usize).min(h - 1);
        self.levels[m][mz * w + mx]
    }
}

/// Bilinear world-space height at a fractional heightmap coordinate.
fn height_bilinear(heights: &[f32], total_x: u32, total_z: u32, x: f32, z: f32) -> f32 {
    let tx = total_x as usize;
    let tz = total_z as usize;
    let cx = x.clamp(0.0, (tx - 1) as f32);
    let cz = z.clamp(0.0, (tz - 1) as f32);
    let x0 = cx.floor() as usize;
    let z0 = cz.floor() as usize;
    let x1 = (x0 + 1).min(tx - 1);
    let z1 = (z0 + 1).min(tz - 1);
    let fx = cx - x0 as f32;
    let fz = cz - z0 as f32;
    let h00 = heights[z0 * tx + x0];
    let h10 = heights[z0 * tx + x1];
    let h01 = heights[z1 * tx + x0];
    let h11 = heights[z1 * tx + x1];
    let a = h00 + (h10 - h00) * fx;
    let b = h01 + (h11 - h01) * fx;
    a + (b - a) * fz
}

/// Bake the horizon and sky-visibility maps for one terrain.
///
/// `heights` is the raw row-major heightfield of `total_x × total_z` samples;
/// world height is `raw * height_scale`, matching `mesh::build_chunk_vertices`.
/// `cell_size` is metres between adjacent heightfield samples.
pub fn bake(
    heights: &[f32],
    total_x: u32,
    total_z: u32,
    cell_size: f32,
    height_scale: f32,
    size: u32,
) -> HorizonMaps {
    let n = size as usize;
    let count = n * n;
    if heights.is_empty() || total_x < 2 || total_z < 2 || n == 0 {
        return HorizonMaps {
            angles_a: vec![0; count * 4],
            angles_b: vec![0; count * 4],
            // Fully open sky, pointing straight up: the identity this whole
            // feature has to reduce to when there is nothing to bake.
            sky: [128u8, 255, 128, 255].repeat(count),
            size,
        };
    }

    // Metres per output texel. The two axes differ whenever `grid_size` is not
    // square, and a march that assumed they matched would stretch every
    // shadow along the shorter one.
    let world_x = (total_x - 1) as f32 * cell_size;
    let world_z = (total_z - 1) as f32 * cell_size;
    let step_x = world_x / n as f32;
    let step_z = world_z / n as f32;

    // Sample the heightfield at the centre of each output texel, twice: the
    // ground the query point stands on, and the occluder pyramid it marches
    // against. They are the same field read two ways — `ground` bilinear so a
    // point is not shadowed by the roughness it is standing on, `occluder`
    // max-pooled so nothing that could cast is ever averaged away.
    let to_height_coord = |i: usize, n: usize, total: u32| -> f32 {
        (i as f32 + 0.5) / n as f32 * (total - 1) as f32
    };
    let mut ground = vec![0.0f32; count];
    let mut occluder = vec![0.0f32; count];
    for z in 0..n {
        let hz = to_height_coord(z, n, total_z);
        for x in 0..n {
            let hx = to_height_coord(x, n, total_x);
            let g = height_bilinear(heights, total_x, total_z, hx, hz) * height_scale;
            ground[z * n + x] = g;
            // The max over the heightfield samples this texel covers. When the
            // output is finer than the heightfield this is just the bilinear
            // value; when it is coarser it is what stops ridges disappearing.
            let sx0 = (hx - 0.5 * (total_x - 1) as f32 / n as f32).floor().max(0.0) as usize;
            let sx1 = ((hx + 0.5 * (total_x - 1) as f32 / n as f32).ceil() as usize)
                .min(total_x as usize - 1);
            let sz0 = (hz - 0.5 * (total_z - 1) as f32 / n as f32).floor().max(0.0) as usize;
            let sz1 = ((hz + 0.5 * (total_z - 1) as f32 / n as f32).ceil() as usize)
                .min(total_z as usize - 1);
            let mut m = f32::NEG_INFINITY;
            for sz in sz0..=sz1 {
                for sx in sx0..=sx1 {
                    m = m.max(heights[sz * total_x as usize + sx]);
                }
            }
            occluder[z * n + x] = m * height_scale;
        }
    }

    let pyramid = MaxPyramid::build(occluder, n);
    let max_mip = pyramid.max_mip();

    // Per-azimuth step length in metres, for the tangent's denominator.
    let step_len: [f32; AZIMUTHS] = std::array::from_fn(|i| {
        let (dx, dz) = DIRS[i];
        let mx = dx as f32 * step_x;
        let mz = dz as f32 * step_z;
        (mx * mx + mz * mz).sqrt()
    });

    let mut angles_a = vec![0u8; count * 4];
    let mut angles_b = vec![0u8; count * 4];
    let mut sky = vec![0u8; count * 4];

    // One row per task. Rows are independent and each reads the shared
    // pyramid, so this is a pure fan-out with no synchronisation.
    angles_a
        .par_chunks_mut(n * 4)
        .zip(angles_b.par_chunks_mut(n * 4))
        .zip(sky.par_chunks_mut(n * 4))
        .enumerate()
        .for_each(|(z, ((row_a, row_b), row_sky))| {
            for x in 0..n {
                let g = ground[z * n + x];
                let mut horizon = [0.0f32; AZIMUTHS];
                for (a, &(dx, dz)) in DIRS.iter().enumerate() {
                    let mut best_tan = 0.0f32;
                    let mut s: i64 = 1;
                    loop {
                        let sx = x as i64 + dx as i64 * s;
                        let sz = z as i64 + dz as i64 * s;
                        if sx < 0 || sz < 0 || sx >= n as i64 || sz >= n as i64 {
                            break;
                        }
                        // Unit steps while the caster is close and its exact
                        // height matters; doubling once it is far enough that
                        // a coarser cell is smaller than the sun's penumbra
                        // at that range.
                        let mip = if s < 8 {
                            0
                        } else {
                            ((63 - s.leading_zeros()) as u32).saturating_sub(2).min(max_mip)
                        };
                        let h = pyramid.sample(mip, sx, sz);
                        let dh = h - g;
                        if dh > 0.0 {
                            let dist = s as f32 * step_len[a];
                            if dist > 1e-4 {
                                best_tan = best_tan.max(dh / dist);
                            }
                        }
                        s += 1i64 << mip;
                    }
                    horizon[a] = best_tan.atan();
                }

                for c in 0..4 {
                    row_a[x * 4 + c] = pack_angle(horizon[c]);
                    row_b[x * 4 + c] = pack_angle(horizon[c + 4]);
                }
                let bent = bent_direction(&horizon);
                let vis = sky_visibility(&horizon);
                row_sky[x * 4] = ((bent[0] * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
                row_sky[x * 4 + 1] = ((bent[1] * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
                row_sky[x * 4 + 2] = ((bent[2] * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
                row_sky[x * 4 + 3] = (vis.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        });

    HorizonMaps {
        angles_a,
        angles_b,
        sky,
        size,
    }
}

fn upload_one(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    texels: &[u8],
    size: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // Not sRGB: these are angles and a direction, not colours. Linearising
        // them on the way in would bend every shadow edge.
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
        texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size * 4),
            rows_per_image: Some(size),
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// GPU residency for one terrain's baked visibility.
pub struct HorizonGpu {
    pub angles_a: wgpu::Texture,
    pub angles_a_view: wgpu::TextureView,
    pub angles_b: wgpu::Texture,
    pub angles_b_view: wgpu::TextureView,
    pub sky: wgpu::Texture,
    pub sky_view: wgpu::TextureView,
}

pub fn upload(device: &wgpu::Device, queue: &wgpu::Queue, maps: &HorizonMaps) -> HorizonGpu {
    let (angles_a, angles_a_view) = upload_one(
        device,
        queue,
        "Terrain Horizon Angles A",
        &maps.angles_a,
        maps.size,
    );
    let (angles_b, angles_b_view) = upload_one(
        device,
        queue,
        "Terrain Horizon Angles B",
        &maps.angles_b,
        maps.size,
    );
    let (sky, sky_view) = upload_one(
        device,
        queue,
        "Terrain Sky Visibility",
        &maps.sky,
        maps.size,
    );
    HorizonGpu {
        angles_a,
        angles_a_view,
        angles_b,
        angles_b_view,
        sky,
        sky_view,
    }
}

/// Rewrite an already-uploaded set in place, after a sculpt.
///
/// The texture and its bindless index stay the same for the terrain's life —
/// the same contract `macro_map` keeps — so a rebake never invalidates a bind
/// group.
pub fn rewrite(queue: &wgpu::Queue, gpu: &HorizonGpu, maps: &HorizonMaps) {
    for (texture, texels) in [
        (&gpu.angles_a, &maps.angles_a),
        (&gpu.angles_b, &maps.angles_b),
        (&gpu.sky, &maps.sky),
    ] {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            texels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(maps.size * 4),
                rows_per_image: Some(maps.size),
            },
            wgpu::Extent3d {
                width: maps.size,
                height: maps.size,
                depth_or_array_layers: 1,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(total: u32) -> Vec<f32> {
        vec![0.0; (total * total) as usize]
    }

    #[test]
    fn flat_ground_has_no_horizon_and_sees_the_whole_sky() {
        let maps = bake(&flat(65), 65, 65, 1.0, 1.0, 32);
        assert!(
            maps.angles_a.iter().all(|&a| a == 0),
            "flat ground occludes nothing"
        );
        assert!(maps.angles_b.iter().all(|&a| a == 0));
        for texel in maps.sky.chunks(4) {
            assert_eq!(texel[3], 255, "flat ground sees the whole sky");
        }
    }

    #[test]
    fn flat_ground_looks_straight_up() {
        // The identity the whole feature reduces to. If this drifts, every
        // terrain pixel gets a bent normal that is subtly off-vertical and the
        // ambient term tilts across the whole map for no visible reason.
        let bent = bent_direction(&[0.0; AZIMUTHS]);
        assert!(bent[1] > 0.999, "up-ness {} should be ~1", bent[1]);
        assert!(bent[0].abs() < 1e-5 && bent[2].abs() < 1e-5);
    }

    #[test]
    fn a_wall_shadows_the_side_it_faces_and_not_the_other() {
        // A step in the middle: everything west of it (lower x) is looking at
        // a wall to its east, and vice versa.
        let total = 65u32;
        let mut h = flat(total);
        for z in 0..total as usize {
            for x in 32..total as usize {
                h[z * total as usize + x] = 40.0;
            }
        }
        let n = 32u32;
        let maps = bake(&h, total, total, 1.0, 1.0, n);
        let at = |x: usize, z: usize, azimuth: usize| -> u8 {
            let idx = (z * n as usize + x) * 4;
            if azimuth < 4 {
                maps.angles_a[idx + azimuth]
            } else {
                maps.angles_b[idx + azimuth - 4]
            }
        };
        // Azimuth 0 is +X, which is where the wall is.
        let west_of_wall = at(4, 16, 0);
        assert!(
            west_of_wall > 40,
            "a point west of a 40 m wall should have a high east horizon, got {west_of_wall}"
        );
        // Azimuth 4 is -X: nothing that way but flat ground.
        assert_eq!(at(4, 16, 4), 0, "the open side has no horizon");
    }

    #[test]
    fn a_pit_sees_less_sky_than_the_rim() {
        let total = 65u32;
        let mut h = vec![20.0f32; (total * total) as usize];
        for z in 24..40usize {
            for x in 24..40usize {
                h[z * total as usize + x] = 0.0;
            }
        }
        let n = 64u32;
        let maps = bake(&h, total, total, 1.0, 1.0, n);
        let vis = |x: usize, z: usize| maps.sky[(z * n as usize + x) * 4 + 3];
        let pit = vis(31, 31);
        let rim = vis(4, 4);
        assert!(
            pit < rim,
            "the pit floor ({pit}) must see less sky than open ground ({rim})"
        );
    }

    #[test]
    fn the_march_reaches_across_the_whole_map() {
        // The property the multi-resolution stride exists for. A single tall
        // spike at one edge must still raise the horizon at the far edge; a
        // fixed-step march with a sane step budget would never get there, and
        // the failure mode is a mountain that casts no shadow into the next
        // valley.
        let total = 257u32;
        let mut h = flat(total);
        let t = total as usize;
        for z in 0..8usize {
            for x in 0..8usize {
                h[z * t + x] = 600.0;
            }
        }
        let n = 128u32;
        let maps = bake(&h, total, total, 1.0, 1.0, n);
        // Far corner, looking back along azimuth 5 (-X, -Z) toward the spike.
        let idx = ((n as usize - 2) * n as usize + (n as usize - 2)) * 4;
        assert!(
            maps.angles_b[idx + 1] > 0,
            "a 600 m spike must be visible from the far corner"
        );
    }

    #[test]
    fn sky_visibility_is_monotone_in_the_horizon() {
        let open = sky_visibility(&[0.0; AZIMUTHS]);
        let half = sky_visibility(&[std::f32::consts::FRAC_PI_4; AZIMUTHS]);
        let closed = sky_visibility(&[std::f32::consts::FRAC_PI_2; AZIMUTHS]);
        assert!((open - 1.0).abs() < 1e-6);
        assert!(closed < 1e-6);
        assert!(closed < half && half < open);
    }

    #[test]
    fn an_empty_heightfield_bakes_the_identity() {
        let maps = bake(&[], 0, 0, 1.0, 1.0, 8);
        assert_eq!(maps.sky.len(), 8 * 8 * 4);
        for texel in maps.sky.chunks(4) {
            assert_eq!(texel[3], 255);
            assert_eq!(texel[1], 255, "bent direction is straight up");
        }
    }
}

#[cfg(test)]
mod timing {
    use super::*;

    /// Not an assertion about speed — a printed number, so the load-time cost
    /// of the bake is a measurement rather than a hope. Run with
    /// `cargo test -p somnium_renderer --release --lib horizon_bake_cost -- --nocapture`.
    #[test]
    fn horizon_bake_cost_at_shipped_size() {
        let total = 1025u32;
        let mut h = vec![0.0f32; (total * total) as usize];
        for z in 0..total as usize {
            for x in 0..total as usize {
                let fx = x as f32 * 0.01;
                let fz = z as f32 * 0.01;
                h[z * total as usize + x] = (fx.sin() * 30.0) + (fz.cos() * 25.0) + 40.0;
            }
        }
        let t = std::time::Instant::now();
        let maps = bake(&h, total, total, 1.0, 1.0, HORIZON_SIZE);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        println!("horizon bake {HORIZON_SIZE}^2 from {total}^2: {ms:.1} ms");
        assert_eq!(maps.size, HORIZON_SIZE);
    }
}
