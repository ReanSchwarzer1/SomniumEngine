//! Material data management for the shading pass.

use bytemuck::{Pod, Zeroable};

/// Material structure that matches the GPU layout in shading.wgsl.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
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
    /// Explicit tail padding to a 16-byte multiple.
    ///
    /// WGSL requires the array stride of a storage-buffer element to be a
    /// multiple of its alignment, which is 16 here because of `base_color`.
    /// Adding a single f32 took the struct from 48 to 52 bytes, so the padding
    /// is spelled out rather than left to the compiler to insert silently.
    pub _pad: [f32; 3],
}

/// `GpuMaterial::flags` bit 0 — the material renders from both sides.
pub const MATERIAL_FLAG_DOUBLE_SIDED: u32 = 1;

/// Manages a pool of materials in a GPU storage buffer.
pub struct MaterialPool {
    pub buffer: wgpu::Buffer,
    materials: Vec<GpuMaterial>,
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
        }
    }

    /// Add a material to the pool and return its ID.
    pub fn add_material(&mut self, queue: &wgpu::Queue, material: GpuMaterial) -> u32 {
        let id = self.materials.len() as u32;
        self.materials.push(material);
        
        // Update the buffer
        queue.write_buffer(&self.buffer, (id as usize * std::mem::size_of::<GpuMaterial>()) as u64, bytemuck::bytes_of(&material));
        
        id
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
}
