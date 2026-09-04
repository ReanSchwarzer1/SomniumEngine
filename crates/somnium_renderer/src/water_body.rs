//! Renderer-owned heavy data for first-class ECS water bodies (Phase IV-C).

use crate::RenderContext;

pub const GREAT_LAKES_MASK: &str = "assets/terrain/great_lakes/water_mask.png";
pub const GREAT_LAKES_DEPTH: &str = "assets/terrain/great_lakes/water_depth.png";
pub const GREAT_LAKES_SHORE_SDF: &str = "assets/terrain/great_lakes/shore_sdf.png";

/// Coverage rules a body can be baked with. The *shape* of the water, which is
/// separate from what it is made of — that is `WaterComponent::body_kind`.
pub const WATER_PRESET_LAKE_BAKE: u32 = 1;
pub const WATER_PRESET_OPEN: u32 = 2;
/// Phase TSUSHIMA-I: a ribbon swept along an authored centreline.
pub const WATER_PRESET_CHANNEL: u32 = 3;

/// Control points a channel centreline may carry.
///
/// Fixed rather than a `Vec` because the descriptor is `Copy` and its equality
/// is what triggers a rebake: `ensure_water_body` compares the stored
/// descriptor with the incoming one, so a spline the author drags reaches the
/// GPU by being *different*, with no dirty flag to forget to set. A `Vec` would
/// cost that property, and sixteen control points is a long river reach.
pub const WATER_PATH_POINTS: usize = 16;

/// Stable, serializable description mirrored by `somnium_core::WaterComponent`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterBodyDescriptor {
    pub water_id: u32,
    pub terrain_id: u32,
    /// One of the `WATER_PRESET_*` constants above.
    pub preset: u32,
    pub surface_level: f32,
    pub max_depth: f32,
    /// Terrain-local `[min_x, min_z, max_x, max_z]`.
    pub bounds: [f32; 4],
    pub amplitude: f32,
    pub wave_dir_a: [f32; 2],
    pub wave_dir_b: [f32; 2],
    pub wave_length_a: f32,
    pub wave_length_b: f32,
    pub wave_speed: f32,
    pub wave_steepness: f32,
    /// Channel centreline in terrain-local XZ, `path_len` points used.
    pub path: [[f32; 2]; WATER_PATH_POINTS],
    pub path_len: u32,
    /// Half-width of the swept channel, metres. Ignored by other presets.
    pub half_width: f32,
}

impl Default for WaterBodyDescriptor {
    /// An unbound body with no coverage.
    ///
    /// Exists so test fixtures and future callers can write the handful of
    /// fields they care about and spread the rest. A `somnium_core` test that
    /// built `TerrainTextureIds` field by field broke on TSUSHIMA-B for exactly
    /// this reason, and the fix there was the same: a test about which ids get
    /// unbound has no opinion about the rest.
    fn default() -> Self {
        Self {
            water_id: u32::MAX,
            terrain_id: u32::MAX,
            preset: 0,
            surface_level: 0.0,
            max_depth: 0.0,
            bounds: [0.0; 4],
            amplitude: 1.0,
            wave_dir_a: [1.0, 0.0],
            wave_dir_b: [0.0, 1.0],
            wave_length_a: 18.0,
            wave_length_b: 11.0,
            wave_speed: 1.0,
            wave_steepness: 0.35,
            path: [[0.0; 2]; WATER_PATH_POINTS],
            path_len: 0,
            half_width: 3.0,
        }
    }
}

impl WaterBodyDescriptor {
    /// The centreline actually in use.
    #[must_use]
    pub fn path_points(&self) -> &[[f32; 2]] {
        &self.path[..(self.path_len as usize).min(WATER_PATH_POINTS)]
    }
}

/// Shortest distance from `p` to the polyline through `points`, in the same
/// units as the points.
///
/// Segment-wise rather than point-wise: the nearest *vertex* to a point beside
/// a long straight reach can be tens of metres away while the river itself is
/// two metres away, and a mask built from vertex distance is a string of beads.
fn distance_to_polyline(points: &[[f32; 2]], p: [f32; 2]) -> f32 {
    if points.is_empty() {
        return f32::INFINITY;
    }
    if points.len() == 1 {
        return ((p[0] - points[0][0]).powi(2) + (p[1] - points[0][1]).powi(2)).sqrt();
    }
    let mut best = f32::INFINITY;
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let len_sq = dx * dx + dz * dz;
        // A repeated control point is a zero-length segment; clamping `t` to 0
        // makes it degenerate to the vertex rather than divide by zero.
        let t = if len_sq > 1e-9 {
            (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / len_sq).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (cx, cz) = (a[0] + dx * t, a[1] + dz * t);
        best = best.min(((p[0] - cx).powi(2) + (p[1] - cz).powi(2)).sqrt());
    }
    best
}

/// Rasterise a channel's coverage, bed depth and shore distance.
///
/// Returns the same four arrays the baked lake loads from PNGs, so everything
/// downstream — the datum reprojection, the mask decimation, `finite_mesh` —
/// works on a river without knowing one exists.
fn bake_channel(descriptor: WaterBodyDescriptor) -> ([u32; 2], Vec<u8>, Vec<u16>, Vec<u16>) {
    const SIZE: u32 = 256;
    let size = [SIZE, SIZE];
    let n = (SIZE * SIZE) as usize;
    let points = descriptor.path_points();
    let [min_x, min_z, max_x, max_z] = descriptor.bounds;
    let span_x = (max_x - min_x).max(1.0);
    let span_z = (max_z - min_z).max(1.0);
    let half_width = descriptor.half_width.max(0.5);
    // The shore SDF is stored in *cells*, and a cell here is one texel of this
    // grid rather than one metro of world. Converting once keeps the two
    // presets' SDFs comparable, which is what lets the shader use one contour.
    let metres_per_cell = (span_x / SIZE as f32).max(span_z / SIZE as f32);

    let mut mask = vec![0u8; n];
    let mut depth = vec![0u16; n];
    let mut sdf = vec![0u16; n];
    if points.len() < 2 {
        // No path is an empty body, not a full one. A river whose spline was
        // deleted should vanish rather than flood the bounds.
        return (size, mask, depth, sdf);
    }

    for z in 0..SIZE {
        for x in 0..SIZE {
            let idx = (z * SIZE + x) as usize;
            let world = [
                min_x + (x as f32 + 0.5) / SIZE as f32 * span_x,
                min_z + (z as f32 + 0.5) / SIZE as f32 * span_z,
            ];
            let distance = distance_to_polyline(points, world);
            let across = (distance / half_width).clamp(0.0, 1.0);
            if distance <= half_width {
                mask[idx] = u8::MAX;
                // A rounded bed: deepest along the centreline, meeting the bank
                // tangentially. A flat-bottomed channel puts a step at the
                // waterline that the shore fade then has to hide.
                let profile = (1.0 - across * across).max(0.0).sqrt();
                depth[idx] = (profile * u16::MAX as f32) as u16;
            }
            // Signed, positive inside, in cells, encoded the way the baked
            // lake's PNG is: 32768 is the shoreline.
            let signed_cells = ((half_width - distance) / metres_per_cell).clamp(-128.0, 128.0);
            sdf[idx] = (((signed_cells / 128.0) * 0.5 + 0.5) * u16::MAX as f32) as u16;
        }
    }
    (size, mask, depth, sdf)
}

/// CPU result matching the deterministic Gerstner displacement used by WGSL.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterSurfaceSample {
    pub height: f32,
    pub normal: glam::Vec3,
    pub depth: f32,
    pub velocity: glam::Vec3,
}

/// CPU query copy plus GPU textures. ECS stores only the descriptor-sized handle.
pub struct WaterBodyData {
    pub descriptor: WaterBodyDescriptor,
    pub size: [u32; 2],
    pub mask: Vec<u8>,
    pub depth_metres: Vec<f32>,
    pub shore_distance_cells: Vec<f32>,
    pub mask_texture: wgpu::Texture,
    pub mask_view: wgpu::TextureView,
    pub depth_texture: wgpu::Texture,
    pub depth_view: wgpu::TextureView,
    pub shore_sdf_texture: wgpu::Texture,
    pub shore_sdf_view: wgpu::TextureView,
}

fn load_assets(
    descriptor: WaterBodyDescriptor,
) -> Result<([u32; 2], Vec<u8>, Vec<u16>, Vec<u16>), String> {
    if descriptor.preset == WATER_PRESET_CHANNEL {
        return Ok(bake_channel(descriptor));
    }
    if descriptor.preset == WATER_PRESET_OPEN {
        // Open ocean: fully wet rectangle. Terrain depth owns the island shore
        // (water.wgsl under-terrain guard). Same optical/Gerstner numbers live
        // on the ECS component; this is only coverage.
        let size = [256u32, 256u32];
        let n = (size[0] * size[1]) as usize;
        return Ok((size, vec![255u8; n], vec![u16::MAX; n], vec![u16::MAX; n]));
    }
    if descriptor.preset != WATER_PRESET_LAKE_BAKE {
        return Err(format!("unsupported water preset {}", descriptor.preset));
    }
    let resolve = |path: &str| {
        let direct = std::path::PathBuf::from(path);
        if direct.exists() {
            direct
        } else {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(path)
        }
    };
    let mask_path = resolve(GREAT_LAKES_MASK);
    let depth_path = resolve(GREAT_LAKES_DEPTH);
    let sdf_path = resolve(GREAT_LAKES_SHORE_SDF);
    let mask_image = image::open(&mask_path)
        .map_err(|e| format!("{GREAT_LAKES_MASK}: {e}"))?
        .to_luma8();
    let depth_image = image::open(&depth_path)
        .map_err(|e| format!("{GREAT_LAKES_DEPTH}: {e}"))?
        .to_luma16();
    let sdf_image = image::open(&sdf_path)
        .map_err(|e| format!("{GREAT_LAKES_SHORE_SDF}: {e}"))?
        .to_luma16();
    let size = [mask_image.width(), mask_image.height()];
    if [depth_image.width(), depth_image.height()] != size
        || [sdf_image.width(), sdf_image.height()] != size
    {
        return Err("water mask/depth/SDF dimensions differ".into());
    }
    Ok((
        size,
        mask_image.into_raw(),
        depth_image.into_raw(),
        sdf_image.into_raw(),
    ))
}

fn upload_r8(
    ctx: &RenderContext,
    label: &str,
    size: [u32; 2],
    bytes: &[u8],
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    ctx.queue.write_texture(
        texture.as_image_copy(),
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size[0]),
            rows_per_image: Some(size[1]),
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&Default::default());
    (texture, view)
}

fn upload_r32_float(
    ctx: &RenderContext,
    label: &str,
    size: [u32; 2],
    values: &[f32],
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // R16Unorm requires wgpu's optional TEXTURE_FORMAT_16BIT_NORM
        // feature. R32Float is core and keeps the authored precision; IV-D can
        // consume it with textureLoad or a non-filtering binding.
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    ctx.queue.write_texture(
        texture.as_image_copy(),
        bytemuck::cast_slice(values),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size[0] * 4),
            rows_per_image: Some(size[1]),
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&Default::default());
    (texture, view)
}

/// The surface datum the baked Great Lakes coverage was authored against.
///
/// `assets/terrain/great_lakes/{mask,depth,shore_sdf}.png` are not a heightmap —
/// they are a *shoreline*, solved once for a water plane at this height. The
/// depth texel is metres of water below it and the SDF is the contour where
/// that reaches zero.
pub const BAKED_DATUM_METRES: f32 = crate::terrain::GREAT_LAKES_BAKE_DATUM_METRES;

/// The depth the baked texture is normalised against, from the same
/// `recipe.json` (`"max_depth_metres": 12`).
///
/// **This is not `WaterBodyDescriptor::max_depth`**, and conflating the two is
/// the trap here. The descriptor's 18.6 is an *optical* path length, chosen
/// deliberately deeper than the bed so open water reaches full absorption
/// instead of staying thin and grey; `WaterBodyData::load` decodes the texel
/// with it, so the depth the shader reads is the true depth scaled by
/// 18.6/12 ≈ 1.55. That is fine for an extinction integral and wrong for
/// geometry: a datum shift is in real metres, so it has to be applied to real
/// depth. [`reproject_to_datum`] divides the scaling out, shifts, and puts it
/// back.
pub const BAKED_MAX_DEPTH_METRES: f32 = crate::terrain::DEFAULT_WATER_DEPTH_METRES;

/// Move baked coverage from [`BAKED_DATUM_METRES`] to the authored datum.
///
/// **The defect this fixes:** `surface_level` is an editable field, and moving
/// it moved the water *plane* while the mask, depth and SDF stayed where they
/// were baked. Lower the datum and the plane drops below a shoreline that still
/// thinks it is at the bake datum, so the surface ends in the wrong place and
/// gets an edge that does not follow the terrain it is supposedly meeting.
/// Raise it and water draws over ground that is now above it. Either way the
/// number in Details and the picture on screen disagree, which is the same
/// class of defect as a checked box that runs no pass.
///
/// The baked depth field is exactly what is needed to fix it, because depth
/// below one datum is depth below another plus a constant. Subtracting the
/// shift gives the true depth for the authored plane, and the wet set is where
/// that is still positive. The SDF is then re-solved for that set with an exact
/// Euclidean transform, so the antialiased contour `water.wgsl` reads keeps its
/// sub-cell quality instead of degrading to a stair-stepped mask.
///
/// **At the default datum this returns the baked data untouched.** That is
/// deliberate and load-bearing: `terrain_shading_occupancy_2026-08-14.md`
/// froze the Great Lakes look, so authoring the shipped datum must be
/// bit-identical, and it is — the work happens only once someone authors a
/// different number, which is exactly when the old behaviour was wrong.
fn reproject_to_datum(
    size: [u32; 2],
    mask: Vec<u8>,
    depth_metres: Vec<f32>,
    sdf_cells: Vec<f32>,
    surface_level: f32,
    optical_max_depth: f32,
) -> (Vec<u8>, Vec<f32>, Vec<f32>) {
    let shift = BAKED_DATUM_METRES - surface_level;
    if shift.abs() < 1.0e-3 {
        return (mask, depth_metres, sdf_cells);
    }
    // `depth_metres` arrives scaled by the optical/bake ratio — see
    // [`BAKED_MAX_DEPTH_METRES`]. The shift is in real metres, so undo the
    // scaling, shift, and reapply it; the optics are then unchanged and only
    // the waterline has moved.
    let optical = if BAKED_MAX_DEPTH_METRES > 0.0 {
        optical_max_depth / BAKED_MAX_DEPTH_METRES
    } else {
        1.0
    };
    let optical = if optical.is_finite() && optical > 0.0 {
        optical
    } else {
        1.0
    };
    let shifted: Vec<f32> = depth_metres.iter().map(|d| d / optical - shift).collect();
    let wet: Vec<bool> = shifted.iter().map(|d| *d > 0.0).collect();
    let mask = wet.iter().map(|w| if *w { 255u8 } else { 0 }).collect();
    // The shader reads depth as a positive optical path; a negative one is dry
    // ground it discards anyway, and clamping keeps the extinction integral
    // from being handed a negative length.
    let depth = shifted.iter().map(|d| d.max(0.0) * optical).collect();
    let sdf = signed_distance_cells(size, &wet);
    (mask, depth, sdf)
}

/// Exact Euclidean signed distance to the wet/dry boundary, in cells.
///
/// Positive inside the water and negative outside, which is the sign convention
/// `water.wgsl` reads: `coverage` rises with it, and `shore_distance` clamps it
/// at zero to measure foam *into* the body.
///
/// Felzenszwalb and Huttenlocher's lower-envelope transform, one pass per axis.
/// A chamfer approximation would be fewer lines and is what a shoreline this
/// smooth would usually tolerate, but the contour feeds an `fwidth`
/// antialiasing band — a third of a cell of error there becomes a visibly
/// wobbling waterline, so the exact transform is the cheaper choice in the end.
fn signed_distance_cells(size: [u32; 2], wet: &[bool]) -> Vec<f32> {
    let inside = squared_distance_to(size, wet, false);
    let outside = squared_distance_to(size, wet, true);
    wet.iter()
        .zip(inside.iter().zip(outside.iter()))
        .map(|(w, (d_in, d_out))| {
            let d = if *w { d_in.sqrt() } else { -d_out.sqrt() };
            d.clamp(-128.0, 128.0)
        })
        .collect()
}

/// Squared Euclidean distance from every cell to the nearest cell whose `wet`
/// flag equals `target`.
fn squared_distance_to(size: [u32; 2], wet: &[bool], target: bool) -> Vec<f32> {
    let (w, h) = (size[0] as usize, size[1] as usize);
    const FAR: f32 = 1.0e12;
    let mut grid: Vec<f32> = wet
        .iter()
        .map(|v| if *v == target { 0.0 } else { FAR })
        .collect();
    if w == 0 || h == 0 {
        return grid;
    }
    // Columns first, then rows: the 2D transform is separable, which is the
    // whole reason this is linear rather than quadratic.
    let mut column = vec![0.0f32; h];
    for x in 0..w {
        for y in 0..h {
            column[y] = grid[y * w + x];
        }
        let out = envelope(&column);
        for y in 0..h {
            grid[y * w + x] = out[y];
        }
    }
    for y in 0..h {
        let out = envelope(&grid[y * w..y * w + w]);
        grid[y * w..y * w + w].copy_from_slice(&out);
    }
    grid
}

/// 1D squared-distance transform: the lower envelope of the parabolas
/// `(x - i)^2 + f[i]`.
fn envelope(f: &[f32]) -> Vec<f32> {
    let n = f.len();
    if n == 0 {
        return Vec::new();
    }
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f32; n + 1];
    let mut k = 0usize;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;
    for q in 1..n {
        loop {
            let p = v[k];
            let s = ((f[q] + (q * q) as f32) - (f[p] + (p * p) as f32))
                / (2.0 * q as f32 - 2.0 * p as f32);
            if s <= z[k] && k > 0 {
                k -= 1;
            } else {
                k += 1;
                v[k] = q;
                z[k] = s;
                z[k + 1] = f32::INFINITY;
                break;
            }
        }
    }
    let mut out = vec![0.0f32; n];
    let mut k = 0usize;
    for q in 0..n {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let p = v[k];
        let d = q as f32 - p as f32;
        out[q] = d * d + f[p];
    }
    out
}

impl WaterBodyData {
    fn load(ctx: &RenderContext, descriptor: WaterBodyDescriptor) -> Result<Self, String> {
        let (size, mask, depth_raw, sdf_raw) = load_assets(descriptor)?;
        let depth_metres: Vec<f32> = depth_raw
            .iter()
            .map(|&v| v as f32 / u16::MAX as f32 * descriptor.max_depth)
            .collect();
        let shore_distance_cells: Vec<f32> = sdf_raw
            .iter()
            .map(|&v| (v as f32 / u16::MAX as f32 * 2.0 - 1.0) * 128.0)
            .collect();
        // Make `surface_level` mean something. Preset 2 is a wet rectangle with
        // no baked shoreline to move, so it is exempt.
        let (mask, depth_metres, shore_distance_cells) =
            if descriptor.preset == WATER_PRESET_LAKE_BAKE {
                reproject_to_datum(
                    size,
                    mask,
                    depth_metres,
                    shore_distance_cells,
                    descriptor.surface_level,
                    descriptor.max_depth,
                )
            } else {
                (mask, depth_metres, shore_distance_cells)
            };
        let (mask_texture, mask_view) = upload_r8(ctx, "Water body mask", size, &mask);
        let (depth_texture, depth_view) =
            upload_r32_float(ctx, "Water body depth", size, &depth_metres);
        let (shore_sdf_texture, shore_sdf_view) =
            upload_r32_float(ctx, "Water body shore SDF", size, &shore_distance_cells);
        Ok(Self {
            descriptor,
            size,
            mask,
            depth_metres,
            shore_distance_cells,
            mask_texture,
            mask_view,
            depth_texture,
            depth_view,
            shore_sdf_texture,
            shore_sdf_view,
        })
    }

    fn texel_index(&self, uv: glam::Vec2) -> usize {
        let x = (uv.x.clamp(0.0, 1.0) * (self.size[0] - 1) as f32).round() as u32;
        let z = (uv.y.clamp(0.0, 1.0) * (self.size[1] - 1) as f32).round() as u32;
        (z * self.size[0] + x) as usize
    }

    fn uv_for_local(&self, xz: glam::Vec2) -> Option<glam::Vec2> {
        let [min_x, min_z, max_x, max_z] = self.descriptor.bounds;
        if xz.x < min_x || xz.y < min_z || xz.x > max_x || xz.y > max_z {
            return None;
        }
        Some(glam::Vec2::new(
            (xz.x - min_x) / (max_x - min_x).max(f32::EPSILON),
            (xz.y - min_z) / (max_z - min_z).max(f32::EPSILON),
        ))
    }

    /// Whether terrain-local XZ lies inside the authored wet mask.
    pub fn contains_xz(&self, xz: glam::Vec2) -> bool {
        self.uv_for_local(xz)
            .is_some_and(|uv| self.mask[self.texel_index(uv)] >= 128)
    }

    /// Terrain-local location and depth of the deepest authored wet texel.
    /// Useful for deterministic underwater validation and gameplay spawn
    /// placement without exposing the registry's image storage.
    pub fn deepest_point(&self) -> Option<(glam::Vec2, f32)> {
        // Large authored basins commonly contain many texels at the same
        // maximum depth. Break that tie by signed shoreline distance so a
        // validation/gameplay spawn cannot land on the last raster-edge texel.
        let (index, &depth) = self
            .depth_metres
            .iter()
            .enumerate()
            .filter(|(index, _)| self.mask[*index] >= 128)
            .max_by(|(index_a, depth_a), (index_b, depth_b)| {
                depth_a.total_cmp(depth_b).then_with(|| {
                    self.shore_distance_cells[*index_a]
                        .total_cmp(&self.shore_distance_cells[*index_b])
                })
            })?;
        let x = index as u32 % self.size[0];
        let z = index as u32 / self.size[0];
        let uv = glam::Vec2::new(
            x as f32 / (self.size[0] - 1).max(1) as f32,
            z as f32 / (self.size[1] - 1).max(1) as f32,
        );
        let [min_x, min_z, max_x, max_z] = self.descriptor.bounds;
        Some((
            glam::Vec2::new(
                min_x + uv.x * (max_x - min_x),
                min_z + uv.y * (max_z - min_z),
            ),
            depth,
        ))
    }

    fn depth_at_uv(&self, uv: glam::Vec2) -> f32 {
        let fx = uv.x.clamp(0.0, 1.0) * (self.size[0] - 1) as f32;
        let fz = uv.y.clamp(0.0, 1.0) * (self.size[1] - 1) as f32;
        let x0 = fx.floor() as u32;
        let z0 = fz.floor() as u32;
        let x1 = (x0 + 1).min(self.size[0] - 1);
        let z1 = (z0 + 1).min(self.size[1] - 1);
        let at = |x: u32, z: u32| self.depth_metres[(z * self.size[0] + x) as usize];
        let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let a = mix(at(x0, z0), at(x1, z0), fx.fract());
        let b = mix(at(x0, z1), at(x1, z1), fx.fract());
        mix(a, b, fz.fract())
    }

    /// Surface height, normal, bed depth, and velocity in terrain-local space.
    ///
    /// This is the Gerstner tier only. The spectral cascades that dominate the
    /// drawn surface are GPU textures the CPU never reads, so gameplay — the
    /// viking boat's buoyancy samples included — rides the same analytic waves
    /// the shader still evaluates and adds on top of the FFT. Keep
    /// `wave_speed` in the vessel-tuned range; dialling it toward zero freezes
    /// the boat while the water keeps moving.
    pub fn sample_surface(&self, xz: glam::Vec2, time: f32) -> Option<WaterSurfaceSample> {
        let uv = self.uv_for_local(xz)?;
        if self.mask[self.texel_index(uv)] < 128 {
            return None;
        }
        let depth = self.depth_at_uv(uv);
        let shore = smoothstep(0.25, 2.0, depth);
        // Match the GPU exactly: the shader attenuates the amplitude before
        // evaluating both displacement and derivatives at the shore.
        let mut wave_descriptor = self.descriptor;
        wave_descriptor.amplitude *= shore;
        let (displacement, normal, velocity) = gerstner(wave_descriptor, xz, time);
        Some(WaterSurfaceSample {
            height: self.descriptor.surface_level + displacement.y,
            normal,
            depth,
            velocity,
        })
    }

    /// True when a terrain-local point is between the displaced surface and bed.
    pub fn contains_point(&self, point: glam::Vec3, time: f32) -> bool {
        self.sample_surface(glam::Vec2::new(point.x, point.z), time)
            .is_some_and(|surface| {
                point.y <= surface.height && point.y >= surface.height - surface.depth
            })
    }

    /// Build a compact 2 m grid containing only cells touched by the wet mask.
    /// The fragment shader samples the full-resolution mask for the exact shore.
    pub fn finite_mesh(&self, cell_metres: f32) -> (Vec<somnium_asset::Vertex>, Vec<u32>) {
        let [min_x, min_z, max_x, max_z] = self.descriptor.bounds;
        let width = max_x - min_x;
        let depth = max_z - min_z;
        let cells_x = (width / cell_metres.max(0.25)).ceil().max(1.0) as u32;
        let cells_z = (depth / cell_metres.max(0.25)).ceil().max(1.0) as u32;
        let mut vertices = Vec::with_capacity(((cells_x + 1) * (cells_z + 1)) as usize);
        for z in 0..=cells_z {
            for x in 0..=cells_x {
                let uv = glam::Vec2::new(x as f32 / cells_x as f32, z as f32 / cells_z as f32);
                vertices.push(somnium_asset::Vertex {
                    position: [uv.x * width - width * 0.5, 0.0, uv.y * depth - depth * 0.5],
                    normal: [0.0, 1.0, 0.0],
                    uv: uv.to_array(),
                });
            }
        }
        let row = cells_x + 1;
        let mut wet_cells = vec![false; (cells_x * cells_z) as usize];
        for z in 0..cells_z {
            for x in 0..cells_x {
                let sx0 = (x * self.size[0] / cells_x).min(self.size[0] - 1);
                let sx1 = (((x + 1) * self.size[0]).div_ceil(cells_x)).min(self.size[0]);
                let sz0 = (z * self.size[1] / cells_z).min(self.size[1] - 1);
                let sz1 = (((z + 1) * self.size[1]).div_ceil(cells_z)).min(self.size[1]);
                // Keep a coarse cell if any covered source texel is wet. The
                // fragment mask then owns the exact shore, so decimation can
                // add harmless overdraw but can never cut a narrow inlet out.
                wet_cells[(z * cells_x + x) as usize] = (sz0..sz1).any(|source_z| {
                    (sx0..sx1).any(|source_x| {
                        self.mask[(source_z * self.size[0] + source_x) as usize] >= 128
                    })
                });
            }
        }

        let mut indices = Vec::new();
        for z in 0..cells_z {
            for x in 0..cells_x {
                // Rasterize two dry coarse-cell guard rings around the wet set.
                // The SDF fragment contour still owns exact coverage. Without
                // this ring the sparse mesh itself became the visible shore,
                // producing the large 2 m square/triangle bites seen in IV-I.
                // The second ring supports a small shader-side dilation under
                // opaque terrain, mirroring Unreal's dilated WaterInfo mesh.
                let min_x = x.saturating_sub(2);
                let max_x = (x + 2).min(cells_x - 1);
                let min_z = z.saturating_sub(2);
                let max_z = (z + 2).min(cells_z - 1);
                let touches_water = (min_z..=max_z).any(|near_z| {
                    (min_x..=max_x).any(|near_x| wet_cells[(near_z * cells_x + near_x) as usize])
                });
                if !touches_water {
                    continue;
                }
                let base = z * row + x;
                indices.extend_from_slice(&[
                    base,
                    base + row,
                    base + 1,
                    base + 1,
                    base + row,
                    base + row + 1,
                ]);
            }
        }
        (vertices, indices)
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn gerstner(
    descriptor: WaterBodyDescriptor,
    p: glam::Vec2,
    time: f32,
) -> (glam::Vec3, glam::Vec3, glam::Vec3) {
    let normalize = |v: glam::Vec2| v.try_normalize().unwrap_or(glam::Vec2::X);
    let a = normalize(glam::Vec2::from_array(descriptor.wave_dir_a));
    let b = normalize(glam::Vec2::from_array(descriptor.wave_dir_b));
    let waves = [
        (a, descriptor.wave_length_a.max(0.5), 0.55),
        (b, descriptor.wave_length_b.max(0.5), 0.25),
        (
            normalize(a + glam::Vec2::new(-b.y, b.x) * 0.35),
            descriptor.wave_length_a.max(0.5) * 0.50,
            0.13,
        ),
        (
            normalize(b - glam::Vec2::new(-a.y, a.x) * 0.25),
            descriptor.wave_length_b.max(0.5) * 0.70,
            0.07,
        ),
    ];
    let mut displacement = glam::Vec3::ZERO;
    let mut dx = glam::Vec3::X;
    let mut dz = glam::Vec3::Z;
    let mut velocity = glam::Vec3::ZERO;
    for (dir, wavelength, weight) in waves {
        let amplitude = descriptor.amplitude * weight;
        let k = std::f32::consts::TAU / wavelength;
        let omega = (9.81 * k).sqrt() * descriptor.wave_speed;
        let phase = k * dir.dot(p) + omega * time;
        let (sin_phase, cos_phase) = phase.sin_cos();
        let q = descriptor.wave_steepness.clamp(0.0, 0.95);
        displacement += glam::Vec3::new(
            q * amplitude * dir.x * cos_phase,
            amplitude * sin_phase,
            q * amplitude * dir.y * cos_phase,
        );
        dx += glam::Vec3::new(
            -q * amplitude * k * dir.x * dir.x * sin_phase,
            amplitude * k * dir.x * cos_phase,
            -q * amplitude * k * dir.x * dir.y * sin_phase,
        );
        dz += glam::Vec3::new(
            -q * amplitude * k * dir.x * dir.y * sin_phase,
            amplitude * k * dir.y * cos_phase,
            -q * amplitude * k * dir.y * dir.y * sin_phase,
        );
        velocity += glam::Vec3::new(
            -q * amplitude * dir.x * omega * sin_phase,
            amplitude * omega * cos_phase,
            -q * amplitude * dir.y * omega * sin_phase,
        );
    }
    (displacement, dz.cross(dx).normalize_or_zero(), velocity)
}

#[derive(Default)]
pub struct WaterBodyRegistry {
    bodies: Vec<Option<WaterBodyData>>,
    next_id: u32,
}

impl WaterBodyRegistry {
    pub fn allocate_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    /// Drop every body and reuse ids from zero. Map load rebuilds water from a factory.
    pub fn clear(&mut self) {
        self.bodies.clear();
        self.next_id = 0;
    }

    pub fn contains(&self, id: u32) -> bool {
        self.bodies.get(id as usize).is_some_and(Option::is_some)
    }

    pub fn descriptor(&self, id: u32) -> Option<WaterBodyDescriptor> {
        self.bodies
            .get(id as usize)?
            .as_ref()
            .map(|body| body.descriptor)
    }

    pub fn get(&self, id: u32) -> Option<&WaterBodyData> {
        self.bodies.get(id as usize)?.as_ref()
    }

    pub fn sample_surface(&self, id: u32, xz: glam::Vec2, time: f32) -> Option<WaterSurfaceSample> {
        self.get(id)?.sample_surface(xz, time)
    }

    pub fn deepest_point(&self, id: u32) -> Option<(glam::Vec2, f32)> {
        self.get(id)?.deepest_point()
    }

    pub fn contains_point(&self, id: u32, point: glam::Vec3, time: f32) -> bool {
        self.get(id)
            .is_some_and(|body| body.contains_point(point, time))
    }

    pub fn active_count(&self) -> usize {
        self.bodies.iter().filter(|body| body.is_some()).count()
    }

    /// Active water descriptors associated with one terrain. Terrain LOD uses
    /// these small records to keep the actual land/water intersection at full
    /// geometry resolution, while open land and deep water retain distance LOD.
    pub fn shoreline_lod_regions(&self, terrain_id: u32) -> Vec<WaterBodyDescriptor> {
        self.bodies
            .iter()
            .flatten()
            .filter(|body| body.descriptor.terrain_id == terrain_id)
            .map(|body| body.descriptor)
            .collect()
    }

    pub fn create_or_replace(
        &mut self,
        ctx: &RenderContext,
        descriptor: WaterBodyDescriptor,
    ) -> Result<(), String> {
        let id = descriptor.water_id as usize;
        if self.bodies.len() <= id {
            self.bodies.resize_with(id + 1, || None);
        }
        self.bodies[id] = Some(WaterBodyData::load(ctx, descriptor)?);
        self.next_id = self.next_id.max(descriptor.water_id.saturating_add(1));
        Ok(())
    }

    pub fn remove(&mut self, id: u32) -> bool {
        self.bodies
            .get_mut(id as usize)
            .is_some_and(|slot| slot.take().is_some())
    }

    pub fn retain_ids(&mut self, active: &std::collections::HashSet<u32>) {
        for (id, body) in self.bodies.iter_mut().enumerate() {
            if !active.contains(&(id as u32)) {
                *body = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn great_lakes_assets_have_depth_and_matching_dimensions() {
        let descriptor = WaterBodyDescriptor {
            water_id: 0,
            terrain_id: 0,
            preset: 1,
            surface_level: 15.0,
            max_depth: 12.0,
            bounds: [0.0, 0.0, 1024.0, 1024.0],
            amplitude: 0.35,
            wave_dir_a: [1.0, 0.35],
            wave_dir_b: [-0.25, 1.0],
            wave_length_a: 18.0,
            wave_length_b: 7.0,
            wave_speed: 1.0,
            wave_steepness: 0.45,
            ..Default::default()
        };
        let (size, mask, depth, sdf) = load_assets(descriptor).expect("baked assets");
        assert_eq!(size, [2048, 2048]);
        assert_eq!(mask.len(), depth.len());
        assert_eq!(mask.len(), sdf.len());
        assert!(mask.iter().any(|&v| v == 0) && mask.iter().any(|&v| v != 0));
        assert!(depth.iter().any(|&v| v > 0));
        assert!(depth.iter().zip(&mask).all(|(&d, &m)| m != 0 || d == 0));
    }

    #[test]
    fn ocean_preset_is_fully_wet() {
        let descriptor = WaterBodyDescriptor {
            water_id: 0,
            terrain_id: 0,
            preset: 2,
            surface_level: BAKED_DATUM_METRES,
            max_depth: 18.6,
            bounds: [0.0, 0.0, 512.0, 512.0],
            amplitude: 0.57,
            wave_dir_a: [1.0, 0.35],
            wave_dir_b: [-0.25, 1.0],
            wave_length_a: 18.0,
            wave_length_b: 7.0,
            wave_speed: 0.85,
            wave_steepness: 0.42,
            ..Default::default()
        };
        let (size, mask, depth, sdf) = load_assets(descriptor).expect("ocean assets");
        assert_eq!(size, [256, 256]);
        assert!(mask.iter().all(|&v| v == 255));
        assert_eq!(mask.len(), depth.len());
        assert_eq!(mask.len(), sdf.len());
        assert!(depth.iter().all(|&v| v == u16::MAX));
    }

    #[test]
    fn gerstner_query_stays_finite_and_upward() {
        let descriptor = WaterBodyDescriptor {
            water_id: 0,
            terrain_id: 0,
            preset: 1,
            surface_level: 15.0,
            max_depth: 12.0,
            bounds: [0.0, 0.0, 1024.0, 1024.0],
            amplitude: 0.35,
            wave_dir_a: [1.0, 0.35],
            wave_dir_b: [-0.25, 1.0],
            wave_length_a: 18.0,
            wave_length_b: 7.0,
            wave_speed: 1.0,
            wave_steepness: 0.45,
            ..Default::default()
        };
        let (displacement, normal, velocity) =
            gerstner(descriptor, glam::Vec2::new(123.0, 456.0), 3.0);
        assert!(displacement.is_finite() && normal.is_finite() && velocity.is_finite());
        assert!(normal.y > 0.5);
        assert!(displacement.y.abs() <= descriptor.amplitude + 1.0e-5);
    }
}

#[cfg(test)]
mod datum_tests {
    use super::*;

    /// Brute force, for the transform to be checked against. O(n^2) and
    /// obviously correct, which is the only property wanted of it.
    fn brute_force(size: [u32; 2], wet: &[bool], target: bool) -> Vec<f32> {
        let (w, h) = (size[0] as usize, size[1] as usize);
        let mut out = vec![f32::INFINITY; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut best = f32::INFINITY;
                for yy in 0..h {
                    for xx in 0..w {
                        if wet[yy * w + xx] == target {
                            let dx = x as f32 - xx as f32;
                            let dy = y as f32 - yy as f32;
                            best = best.min(dx * dx + dy * dy);
                        }
                    }
                }
                out[y * w + x] = best;
            }
        }
        out
    }

    fn checker(w: usize, h: usize, seed: u64) -> Vec<bool> {
        // Deterministic pseudo-random blobs: a lattice of sines, thresholded.
        // Random-looking without a dependency, and reproducible.
        (0..w * h)
            .map(|i| {
                let x = (i % w) as f32;
                let y = (i / w) as f32;
                let s = seed as f32 * 0.37;
                ((x * 0.31 + s).sin() + (y * 0.23 - s).cos() + (x * 0.07 * y * 0.05).sin()) > 0.2
            })
            .collect()
    }

    #[test]
    fn the_distance_transform_matches_brute_force() {
        // The transform is separable and easy to get subtly wrong in a way that
        // still looks like a distance field. Only an exact comparison catches
        // that, and a shoreline contour is read through `fwidth`, so "subtly
        // wrong" is "visibly wobbling".
        for (w, h, seed) in [(13usize, 9usize, 1u64), (16, 16, 7), (9, 21, 3)] {
            let wet = checker(w, h, seed);
            for target in [true, false] {
                let fast = squared_distance_to([w as u32, h as u32], &wet, target);
                let slow = brute_force([w as u32, h as u32], &wet, target);
                for (i, (f, s)) in fast.iter().zip(slow.iter()).enumerate() {
                    if s.is_finite() {
                        assert!(
                            (f - s).abs() < 1.0e-3,
                            "{w}x{h} seed {seed} target {target} cell {i}: {f} vs {s}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_default_datum_returns_the_baked_data_untouched() {
        // The freeze in `terrain_shading_occupancy_2026-08-14.md` is the reason
        // this test exists: the shipped Great Lakes look must not move by one
        // texel because this function now exists.
        let size = [8u32, 8u32];
        let mask: Vec<u8> = (0..64).map(|i| (i * 3 % 256) as u8).collect();
        let depth: Vec<f32> = (0..64).map(|i| i as f32 * 0.25).collect();
        let sdf: Vec<f32> = (0..64).map(|i| i as f32 - 32.0).collect();
        let (m, d, s) = reproject_to_datum(
            size,
            mask.clone(),
            depth.clone(),
            sdf.clone(),
            BAKED_DATUM_METRES,
            BAKED_MAX_DEPTH_METRES,
        );
        assert_eq!(m, mask);
        assert_eq!(d, depth);
        assert_eq!(s, sdf);
    }

    #[test]
    fn lowering_the_datum_shrinks_the_wet_set_by_exactly_the_shift() {
        // A ramp from 0 m to 7 m of baked depth. Dropping the plane two metres
        // must dry out precisely the cells shallower than two metres — that
        // equivalence is the whole argument for reusing the baked depth field.
        let size = [8u32, 1u32];
        let depth: Vec<f32> = (0..8).map(|i| i as f32).collect();
        let (mask, out_depth, sdf) = reproject_to_datum(
            size,
            vec![255; 8],
            depth,
            vec![0.0; 8],
            BAKED_DATUM_METRES - 2.0,
            BAKED_MAX_DEPTH_METRES,
        );
        // Baked depths 0,1,2 are at or above the new plane; 3..7 stay wet.
        assert_eq!(
            mask,
            vec![0, 0, 0, 255, 255, 255, 255, 255],
            "the waterline should land between 2 m and 3 m of baked depth"
        );
        assert_eq!(out_depth[3], 1.0, "a 3 m cell is 1 m deep two metres down");
        assert!(
            out_depth[0] >= 0.0,
            "dry cells must not report negative depth"
        );
        // Sign convention: positive inside the water, negative outside.
        assert!(sdf[0] < 0.0 && sdf[7] > 0.0, "sdf = {sdf:?}");
    }

    #[test]
    fn raising_the_datum_grows_the_wet_set() {
        let size = [8u32, 1u32];
        let depth: Vec<f32> = (0..8).map(|i| i as f32).collect();
        let (mask, _, _) = reproject_to_datum(
            size,
            vec![0; 8],
            depth,
            vec![0.0; 8],
            BAKED_DATUM_METRES + 1.5,
            BAKED_MAX_DEPTH_METRES,
        );
        // Everything from the 0 m cell up is now under water.
        assert!(mask.iter().all(|m| *m == 255), "mask = {mask:?}");
    }

    /// The two constants above are claims about files on disk, so they are
    /// checked against those files rather than trusted.
    ///
    /// This is the test that would have caught the original defect: the runtime
    /// datum was 16.1 while `recipe.json` said the shoreline was baked at 15,
    /// and nothing anywhere compared the two.
    #[test]
    fn the_bake_constants_match_the_shipped_recipe() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/terrain/great_lakes/recipe.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            // A clone without the assets still builds; the asset tests beside
            // this one already skip on the same condition.
            return;
        };
        let number = |key: &str| -> f32 {
            let at = text.find(key).expect("recipe key");
            let rest = &text[at + key.len()..];
            let start = rest.find(|c: char| c.is_ascii_digit()).expect("digit");
            let end = start
                + rest[start..]
                    .find(|c: char| !c.is_ascii_digit() && c != '.')
                    .unwrap_or(rest.len() - start);
            rest[start..end].parse().expect("number")
        };
        assert_eq!(
            number("\"water_level_metres\""),
            BAKED_DATUM_METRES,
            "the runtime datum must be the one the shoreline was baked at"
        );
        assert_eq!(
            number("\"max_depth_metres\""),
            BAKED_MAX_DEPTH_METRES,
            "the depth texture's normalisation must match what decodes it"
        );
    }

    #[test]
    fn the_signed_field_is_zero_crossing_at_the_waterline() {
        let size = [6u32, 1u32];
        let wet = [false, false, false, true, true, true];
        let sdf = signed_distance_cells(size, &wet);
        assert!(sdf[2] < 0.0 && sdf[3] > 0.0);
        // One cell either side of the boundary is one cell from it.
        assert!((sdf[3] - 1.0).abs() < 1.0e-3, "sdf = {sdf:?}");
        assert!((sdf[2] + 1.0).abs() < 1.0e-3, "sdf = {sdf:?}");
    }
}

#[cfg(test)]
mod channel_tests {
    use super::*;

    fn descriptor(points: &[[f32; 2]], half_width: f32) -> WaterBodyDescriptor {
        let mut d = WaterBodyDescriptor {
            preset: WATER_PRESET_CHANNEL,
            max_depth: 2.0,
            bounds: [0.0, 0.0, 100.0, 100.0],
            half_width,
            ..Default::default()
        };
        for (slot, point) in points.iter().enumerate() {
            d.path[slot] = *point;
        }
        d.path_len = points.len() as u32;
        d
    }

    fn texel(size: [u32; 2], bounds: [f32; 4], world: [f32; 2]) -> usize {
        let x = ((world[0] - bounds[0]) / (bounds[2] - bounds[0]) * size[0] as f32) as u32;
        let z = ((world[1] - bounds[1]) / (bounds[3] - bounds[1]) * size[1] as f32) as u32;
        (z.min(size[1] - 1) * size[0] + x.min(size[0] - 1)) as usize
    }

    /// Distance is to the *segments*, not to the control points.
    ///
    /// A point beside the middle of a long straight reach is metres from the
    /// river and hundreds of metres from either vertex. Measuring to vertices
    /// builds a mask that is a string of beads with dry gaps between them.
    #[test]
    fn distance_is_measured_to_the_segments() {
        let line = [[0.0, 0.0], [100.0, 0.0]];
        // Beside the middle: 5 m from the segment, ~50 m from both ends.
        let beside = distance_to_polyline(&line, [50.0, 5.0]);
        assert!((beside - 5.0).abs() < 1e-3, "got {beside}");
        // Past the end clamps to the endpoint rather than running off the line.
        let past = distance_to_polyline(&line, [130.0, 0.0]);
        assert!((past - 30.0).abs() < 1e-3, "got {past}");
    }

    /// A repeated control point is a zero-length segment. Dividing by its
    /// length would put NaN through the whole mask.
    #[test]
    fn a_repeated_control_point_does_not_divide_by_zero() {
        let line = [[10.0, 10.0], [10.0, 10.0], [20.0, 10.0]];
        let d = distance_to_polyline(&line, [15.0, 13.0]);
        assert!(d.is_finite(), "distance went non-finite: {d}");
        assert!((d - 3.0).abs() < 1e-3, "got {d}");
    }

    /// The ribbon is wet along the line and dry away from it.
    #[test]
    fn the_channel_is_wet_along_its_path_and_dry_beside_it() {
        let d = descriptor(&[[20.0, 50.0], [80.0, 50.0]], 6.0);
        let (size, mask, depth, _) = bake_channel(d);

        let on = texel(size, d.bounds, [50.0, 50.0]);
        assert_eq!(mask[on], u8::MAX, "the centreline is dry");
        assert!(depth[on] > 0, "the centreline has no depth");

        // Well outside the half-width, and past the end of the reach.
        for dry in [[50.0, 75.0], [5.0, 50.0], [95.0, 50.0]] {
            let i = texel(size, d.bounds, dry);
            assert_eq!(mask[i], 0, "{dry:?} is wet and should not be");
        }
    }

    /// Deepest at the centreline, meeting the bank tangentially.
    ///
    /// A flat-bottomed channel puts a step at the waterline, and the shore fade
    /// then has to hide a discontinuity rather than describe one.
    #[test]
    fn the_bed_is_deepest_in_the_middle() {
        let d = descriptor(&[[20.0, 50.0], [80.0, 50.0]], 10.0);
        let (size, _, depth, _) = bake_channel(d);
        let centre = depth[texel(size, d.bounds, [50.0, 50.0])];
        let midway = depth[texel(size, d.bounds, [50.0, 55.0])];
        let bank = depth[texel(size, d.bounds, [50.0, 59.5])];
        assert!(
            centre > midway && midway > bank,
            "bed profile is not monotonic: {centre} {midway} {bank}"
        );
    }

    /// A channel whose spline was deleted vanishes rather than flooding.
    ///
    /// The opposite is the dangerous default: a body with no path whose mask
    /// defaults to "wet everywhere" fills its whole bounding rectangle, which
    /// on a river-sized body is a lake nobody asked for.
    #[test]
    fn a_channel_without_a_path_is_empty() {
        for points in [&[][..], &[[50.0, 50.0]][..]] {
            let (_, mask, _, _) = bake_channel(descriptor(points, 6.0));
            assert!(mask.iter().all(|&m| m == 0), "an empty path flooded");
        }
    }

    /// Inside the ribbon the shore distance is positive, outside it negative,
    /// encoded the way the baked lake PNG is so one shader contour serves both.
    #[test]
    fn the_shore_distance_changes_sign_at_the_bank() {
        let d = descriptor(&[[20.0, 50.0], [80.0, 50.0]], 8.0);
        let (size, _, _, sdf) = bake_channel(d);
        let decode = |raw: u16| (raw as f32 / u16::MAX as f32 * 2.0 - 1.0) * 128.0;
        let inside = decode(sdf[texel(size, d.bounds, [50.0, 50.0])]);
        let outside = decode(sdf[texel(size, d.bounds, [50.0, 70.0])]);
        assert!(inside > 0.0, "inside the ribbon read {inside}");
        assert!(outside < 0.0, "outside the ribbon read {outside}");
    }
}
