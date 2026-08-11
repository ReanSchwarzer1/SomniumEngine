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
        };
        let (size, mask, depth, sdf) = load_assets(descriptor).expect("baked assets");
        assert_eq!(size, [1024, 1024]);
        assert_eq!(mask.len(), depth.len());
        assert_eq!(mask.len(), sdf.len());
        assert!(mask.iter().any(|&v| v == 0) && mask.iter().any(|&v| v != 0));
        assert!(depth.iter().any(|&v| v > 0));
        assert!(depth.iter().zip(&mask).all(|(&d, &m)| m != 0 || d == 0));
    }
}
