//! Material data management for the shading pass.

use bytemuck::{Pod, Zeroable};

/// Material structure that matches the GPU layout in shading.wgsl.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
/// **The WGSL mirror of this struct must not use `vec3<f32>`.** WGSL aligns a
/// vec3 to 16 bytes; Rust's `repr(C)` aligns `[f32; 3]` to 4. `emissive` as a
/// vec3 in the shader therefore sat at offset 64 with a 96-byte stride, against
/// this struct's offset 52 and 80-byte stride, and every material past index 0
/// was decoded from the wrong bytes. Keep vector members as scalars in the
/// shader, or pad them to a 16-byte boundary on both sides.
pub struct GpuMaterial {
    pub base_color: [f32; 4],
    pub roughness: f32,
    pub metallic: f32,
    pub albedo_map: i32,
    pub normal_map: i32,
    pub metallic_roughness_map: i32,
    /// Phase 17D: glTF `alphaCutoff`. Fragments whose albedo alpha falls below
    /// this are discarded in the visibility pass. `0` means no cutout, which is
    /// what every OPAQUE and BLEND material uses.
    pub alpha_cutoff: f32,
    /// Bit 0: double-sided (glTF `doubleSided`). Kept as a bitfield rather than
    /// a bool so later flags cost no extra space.
    pub flags: u32,
    /// Bindless index of the glTF occlusion texture, or -1. Occupies what was
    /// padding, so the struct's size and alignment are unchanged.
    pub occlusion_map: i32,
    /// Fraction of light transmitted through the surface (Phase 24S).
    pub transmission: f32,
    /// Self-emitted light, linear RGB (Phase 24T).
    pub emissive: [f32; 3],
    /// Bindless index of the emissive texture, or -1.
    pub emissive_map: i32,
    /// Slot in the terrain-material buffer, or -1 for anything that is not
    /// terrain (Phase 25A-2).
    ///
    /// Terrain's material is a splatmap, four tiled PBR layers and a triplanar
    /// cliff projection — thirteen textures and a handful of parameters, which
    /// will not fit here and should not bloat every other material to try.
    /// Instead this indexes a parallel `GpuTerrainMaterial` array, and
    /// `shading.wgsl` takes the terrain path when it is non-negative. Occupies
    /// what was padding, so the struct's size and alignment are unchanged.
    pub terrain_index: i32,
    /// Phase CONTROL-N: how much water this surface takes up, `0..1`.
    ///
    /// Lagarde's discriminating channel. Not a wetness-only input — the same
    /// number governs ageing and pollution — which is why it lives on the
    /// material beside roughness rather than on the weather. Occupies what was
    /// padding, so the struct's size and alignment are unchanged.
    pub porosity: f32,
    /// Explicit tail padding to a 16-byte multiple.
    ///
    /// WGSL requires the array stride of a storage-buffer element to be a
    /// multiple of its alignment, which is 16 here because of `base_color`.
    /// Adding a single f32 took the struct from 48 to 52 bytes, so the padding
    /// is spelled out rather than left to the compiler to insert silently.
    pub _pad: f32,
}

/// `GpuMaterial::flags` bit 0 — the material renders from both sides.
pub const MATERIAL_FLAG_DOUBLE_SIDED: u32 = 1;

/// `GpuMaterial::flags` bit 1 — vegetation (Phase 17E).
///
/// Deliberately separate from `transmission`, which glass carries too: only a
/// leaf should get the curved-card normal treatment in `shading.wgsl`. The
/// shader tests `(flags & 2u)`.
pub const MATERIAL_FLAG_FOLIAGE: u32 = 1 << 1;

impl GpuMaterial {
    /// Rebuild the frozen 80-byte runtime payload from an authored material.
    /// Texture asset ids are resolved by the caller because only the runtime
    /// asset cache knows their current bindless slots.
    #[must_use]
    pub fn from_asset(
        asset: &somnium_asset::material::MaterialAsset,
        mut resolve_texture: impl FnMut(somnium_asset::database::AssetId) -> i32,
    ) -> Self {
        let emissive = glam::Vec3::from(asset.emissive.0) * asset.emissive_intensity;
        Self {
            base_color: [
                asset.base_color.0[0],
                asset.base_color.0[1],
                asset.base_color.0[2],
                asset.opacity,
            ],
            roughness: asset.roughness,
            metallic: asset.metallic,
            albedo_map: resolve_texture(asset.albedo_map),
            normal_map: resolve_texture(asset.normal_map),
            metallic_roughness_map: resolve_texture(asset.metallic_roughness_map),
            alpha_cutoff: cutout_threshold(asset.alpha_mode, asset.alpha_cutoff),
            flags: if asset.double_sided {
                MATERIAL_FLAG_DOUBLE_SIDED
            } else {
                0
            } | if asset.foliage {
                MATERIAL_FLAG_FOLIAGE
            } else {
                0
            },
            occlusion_map: resolve_texture(asset.occlusion_map),
            transmission: asset.transmission,
            emissive: emissive.to_array(),
            emissive_map: resolve_texture(asset.emissive_map),
            terrain_index: -1,
            porosity: asset.porosity,
            _pad: 0.0,
        }
    }
}

/// Manages a pool of materials in a GPU storage buffer.
pub struct MaterialPool {
    pub buffer: wgpu::Buffer,
    materials: Vec<GpuMaterial>,
    revision: u64,
}

impl MaterialPool {
    pub fn new(device: &wgpu::Device) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Global Material Buffer"),
            size: 1024 * 64, // 64KB
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            buffer,
            materials: Vec::new(),
            revision: 0,
        }
    }

    /// Add a material to the pool and return its ID.
    pub fn add_material(&mut self, queue: &wgpu::Queue, material: GpuMaterial) -> u32 {
        let id = self.materials.len() as u32;
        self.materials.push(material);
        self.revision = self.revision.wrapping_add(1);

        // Update the buffer
        queue.write_buffer(
            &self.buffer,
            (id as usize * std::mem::size_of::<GpuMaterial>()) as u64,
            bytemuck::bytes_of(&material),
        );

        id
    }
}

impl MaterialPool {
    pub fn get(&self, id: u32) -> Option<GpuMaterial> {
        self.materials.get(id as usize).copied()
    }

    pub fn set_material(&mut self, queue: &wgpu::Queue, id: u32, material: GpuMaterial) {
        let Some(slot) = self.materials.get_mut(id as usize) else {
            return;
        };
        if *slot == material {
            return;
        }
        *slot = material;
        self.revision = self.revision.wrapping_add(1);
        queue.write_buffer(
            &self.buffer,
            (id as usize * std::mem::size_of::<GpuMaterial>()) as u64,
            bytemuck::bytes_of(&material),
        );
    }

    /// Changes whenever material data visible to raster/ray hit shading changes.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

/// Storage buffer of `GpuTerrainMaterial`, bound at `@group(0) @binding(11)`.
///
/// Separate from `MaterialPool` because the two are written on different
/// schedules: ordinary materials are uploaded once at import, while a terrain's
/// entry carries the brush cursor and the model origin and is rewritten every
/// frame it is drawn.
pub struct TerrainMaterialPool {
    pub buffer: wgpu::Buffer,
    count: u32,
}

impl TerrainMaterialPool {
    /// Room for 16 terrains, which is well past what an editor session uses —
    /// each one costs 34 MB of vertex pool long before this buffer matters.
    const CAPACITY: u32 = 16;

    pub fn new(device: &wgpu::Device) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Terrain Material Buffer"),
            size: Self::CAPACITY as u64
                * std::mem::size_of::<crate::terrain::GpuTerrainMaterial>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { buffer, count: 0 }
    }

    /// Reclaim every slot. Map load creates a new terrain that allocates from zero.
    pub fn reset(&mut self) {
        self.count = 0;
    }

    /// Claim the next slot, or `None` when the buffer is full.
    pub fn allocate(&mut self) -> Option<u32> {
        if self.count >= Self::CAPACITY {
            tracing::error!("terrain material buffer full ({} entries)", Self::CAPACITY);
            return None;
        }
        let index = self.count;
        self.count += 1;
        Some(index)
    }

    /// Write one terrain's material for this frame.
    pub fn write(
        &self,
        queue: &wgpu::Queue,
        index: u32,
        material: &crate::terrain::GpuTerrainMaterial,
    ) {
        if index >= Self::CAPACITY {
            return;
        }
        queue.write_buffer(
            &self.buffer,
            index as u64 * std::mem::size_of::<crate::terrain::GpuTerrainMaterial>() as u64,
            bytemuck::bytes_of(material),
        );
    }
}

/// The cutout threshold a material should carry on the GPU (Phase 17D).
///
/// Only `MASK` cuts out. `OPAQUE` ignores alpha altogether — plenty of opaque
/// glTF textures carry a meaningless alpha channel, and honouring it would
/// punch holes in solid geometry. `BLEND` is drawn by the forward transparent
/// pass, where alpha fades rather than clips; cutting it out as well would
/// leave hard edges through the middle of glass.
///
/// A `MASK` material with a non-finite or non-positive cutoff falls back to the
/// glTF default of 0.5 rather than disabling the test, which is what a `0`
/// would silently mean everywhere else.
pub fn cutout_threshold(mode: somnium_asset::AlphaMode, cutoff: f32) -> f32 {
    match mode {
        somnium_asset::AlphaMode::Mask => {
            if cutoff.is_finite() && cutoff > 0.0 {
                cutoff.min(1.0)
            } else {
                0.5
            }
        }
        _ => 0.0,
    }
}

#[cfg(test)]
mod material_flag_tests {
    use super::*;
    use somnium_asset::AlphaMode;

    #[test]
    fn polished_asset_reconstructs_the_gpu_payload_without_authored_pool_ids() {
        let asset = somnium_asset::material::MaterialAsset {
            metallic: 1.0,
            roughness: 0.2,
            emissive: somnium_asset::material::LinearColor([0.25, 0.5, 1.0]),
            emissive_intensity: 4.0,
            double_sided: true,
            ..Default::default()
        };
        let gpu = GpuMaterial::from_asset(&asset, |_| -1);
        assert_eq!((gpu.metallic, gpu.roughness), (1.0, 0.2));
        assert_eq!(gpu.emissive, [1.0, 2.0, 4.0]);
        assert_eq!(gpu.flags & MATERIAL_FLAG_DOUBLE_SIDED, 1);
        assert_eq!(std::mem::size_of::<GpuMaterial>(), 80);
    }

    #[test]
    fn only_masked_materials_cut_out() {
        assert_eq!(cutout_threshold(AlphaMode::Mask, 0.5), 0.5);
        // An opaque texture's alpha channel is routinely garbage.
        assert_eq!(cutout_threshold(AlphaMode::Opaque, 0.5), 0.0);
        // Blended geometry fades in the forward pass; clipping it too would
        // leave a hard edge through the middle of glass.
        assert_eq!(cutout_threshold(AlphaMode::Blend, 0.5), 0.0);
    }

    #[test]
    fn a_masked_material_with_no_usable_cutoff_falls_back_to_the_gltf_default() {
        // 0 would read as "no cutout" everywhere else, so it cannot be passed
        // through — a MASK material must always clip something.
        assert_eq!(cutout_threshold(AlphaMode::Mask, 0.0), 0.5);
        assert_eq!(cutout_threshold(AlphaMode::Mask, -1.0), 0.5);
        assert_eq!(cutout_threshold(AlphaMode::Mask, f32::NAN), 0.5);
    }

    #[test]
    fn the_cutoff_is_clamped_to_a_meaningful_range() {
        // Above 1 nothing survives, which would delete the whole mesh.
        assert_eq!(cutout_threshold(AlphaMode::Mask, 4.0), 1.0);
        assert_eq!(cutout_threshold(AlphaMode::Mask, 0.25), 0.25);
    }

    #[test]
    fn the_double_sided_flag_is_bit_zero() {
        // The shader tests `(flags & 1u) != 0u`; changing this silently makes
        // every double-sided material light from the wrong side.
        assert_eq!(MATERIAL_FLAG_DOUBLE_SIDED, 1);
    }

    #[test]
    fn the_gpu_material_is_the_80_byte_shader_layout() {
        // Must match `Material` in shading.wgsl, visibility.wgsl, shadow.wgsl
        // and transparent.wgsl. A mismatch does not fail validation; the shader
        // simply reads the wrong words, which is why this is pinned.
        //
        // 48 bytes originally; 64 after Phase 24S added `transmission`; 80
        // after Phase 24T added emissive colour and its texture index.
        // Was 48 bytes until Phase 24S added `transmission`. That took the
        // struct to 52, and WGSL requires a storage-buffer array's stride to be
        // a multiple of the element alignment — 16 here, because of
        // `base_color` — so it rounds to 64. The padding is declared explicitly
        // rather than left implicit for the same reason this test exists.
        assert_eq!(std::mem::size_of::<GpuMaterial>(), 80);
        assert_eq!(std::mem::size_of::<GpuMaterial>() % 16, 0);
    }

    #[test]
    fn the_terrain_material_is_the_2048_byte_shader_layout() {
        // Must match `TerrainMaterial` in terrain_material.wgsl. Every vec4
        // member has to land on a 16-byte offset or WGSL and repr(C) disagree
        // and the shader silently decodes the wrong words — the failure mode
        // that cost a whole session when `emissive` was a vec3.
        //
        // Phase DF: clipmap addressing after the XV-Zeta 1664-byte body.
        // Phase TSUSHIMA-B/C: four scalar words for the baked horizon and sky
        // visibility, appended so every `array<vec4<_>>` above keeps its
        // offset. 2032 + 16 = 2048.
        use crate::terrain::GpuTerrainMaterial;
        assert_eq!(std::mem::size_of::<GpuTerrainMaterial>(), 2048);
        assert_eq!(std::mem::size_of::<GpuTerrainMaterial>() % 16, 0);

        let m = GpuTerrainMaterial::zeroed();
        let base = &m as *const _ as usize;
        let offset = |p: *const u8| p as usize - base;
        assert_eq!(offset(m.layer_tiling.as_ptr() as *const u8), 0);
        assert_eq!(offset(m.brush.as_ptr() as *const u8), 128);
        assert_eq!(offset(m.albedo_maps.as_ptr() as *const u8), 144);
        assert_eq!(offset(m.surface_maps.as_ptr() as *const u8), 272);
        assert_eq!(offset(m.terrain_origin.as_ptr() as *const u8), 400);
        assert_eq!(offset(m.inv_world_size.as_ptr() as *const u8), 408);
        assert_eq!(offset(m.splat_maps.as_ptr() as *const u8), 416);
        assert_eq!(offset(&m.cliff_layer as *const u32 as *const u8), 448);
        assert_eq!(offset(&m.hex_tiling as *const u32 as *const u8), 452);
        assert_eq!(offset(&m.height_blend as *const u32 as *const u8), 456);
        assert_eq!(offset(&m.macro_map as *const i32 as *const u8), 460);
        assert_eq!(offset(m.layer_height_scale.as_ptr() as *const u8), 464);
        assert_eq!(offset(m.layer_blend_width.as_ptr() as *const u8), 592);
        assert_eq!(offset(m.layer_weight_clamp.as_ptr() as *const u8), 720);
        assert_eq!(offset(m.layer_parallax.as_ptr() as *const u8), 848);
        assert_eq!(offset(&m.macro_mode as *const u32 as *const u8), 976);
        assert_eq!(offset(&m.macro_strength as *const f32 as *const u8), 980);
        assert_eq!(offset(&m.detail_fade_start as *const f32 as *const u8), 984);
        assert_eq!(offset(&m.detail_fade_end as *const f32 as *const u8), 988);
        assert_eq!(offset(m.layer_albedo.as_ptr() as *const u8), 992);
        assert_eq!(offset(&m.parallax_steps as *const u32 as *const u8), 1504);
        assert_eq!(
            offset(&m.projection_sharpness as *const f32 as *const u8),
            1512
        );
        assert_eq!(offset(m.layer_moisture.as_ptr() as *const u8), 1520);
        assert_eq!(offset(&m.wetness as *const f32 as *const u8), 1648);
        assert_eq!(offset(&m.wetness_f0 as *const f32 as *const u8), 1660);
        assert_eq!(offset(&m.clipmap_enabled as *const u32 as *const u8), 1664);
        assert_eq!(offset(m.clipmap_albedo.as_ptr() as *const u8), 1680);
        assert_eq!(offset(m.clipmap_center.as_ptr() as *const u8), 1744);
        assert_eq!(offset(m.clipmap_tpm.as_ptr() as *const u8), 1872);
        assert_eq!(
            offset(&m.clipmap_macro_rings as *const u32 as *const u8),
            2016
        );
        assert_eq!(
            offset(&m.clipmap_detail_ready as *const u32 as *const u8),
            2024
        );
    }
}
