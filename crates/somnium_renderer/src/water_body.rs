//! Renderer-owned heavy data for first-class ECS water bodies (Phase IV-C).

use crate::RenderContext;

pub const GREAT_LAKES_MASK: &str = "assets/terrain/great_lakes/water_mask.png";
pub const GREAT_LAKES_DEPTH: &str = "assets/terrain/great_lakes/water_depth.png";
pub const GREAT_LAKES_SHORE_SDF: &str = "assets/terrain/great_lakes/shore_sdf.png";

/// Stable, serializable description mirrored by `somnium_core::WaterComponent`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterBodyDescriptor {
    pub water_id: u32,
    pub terrain_id: u32,
    /// 1 = Great Lakes lake preset. Reserved values allow ocean/river later.
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
    if descriptor.preset != 1 {
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
        let mut indices = Vec::new();
        for z in 0..cells_z {
            for x in 0..cells_x {
                let sx0 = (x * self.size[0] / cells_x).min(self.size[0] - 1);
                let sx1 = (((x + 1) * self.size[0]).div_ceil(cells_x)).min(self.size[0]);
                let sz0 = (z * self.size[1] / cells_z).min(self.size[1] - 1);
                let sz1 = (((z + 1) * self.size[1]).div_ceil(cells_z)).min(self.size[1]);
                // Keep a coarse cell if any covered source texel is wet. The
                // fragment mask then owns the exact shore, so decimation can
                // add harmless overdraw but can never cut a narrow inlet out.
                let wet = (sz0..sz1).any(|source_z| {
                    (sx0..sx1).any(|source_x| {
                        self.mask[(source_z * self.size[0] + source_x) as usize] >= 128
                    })
                });
                if !wet {
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
        };
        let (size, mask, depth, sdf) = load_assets(descriptor).expect("baked assets");
        assert_eq!(size, [1024, 1024]);
        assert_eq!(mask.len(), depth.len());
        assert_eq!(mask.len(), sdf.len());
        assert!(mask.iter().any(|&v| v == 0) && mask.iter().any(|&v| v != 0));
        assert!(depth.iter().any(|&v| v > 0));
        assert!(depth.iter().zip(&mask).all(|(&d, &m)| m != 0 || d == 0));
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
        };
        let (displacement, normal, velocity) =
            gerstner(descriptor, glam::Vec2::new(123.0, 456.0), 3.0);
        assert!(displacement.is_finite() && normal.is_finite() && velocity.is_finite());
        assert!(normal.y > 0.5);
        assert!(displacement.y.abs() <= descriptor.amplitude + 1.0e-5);
    }
}
