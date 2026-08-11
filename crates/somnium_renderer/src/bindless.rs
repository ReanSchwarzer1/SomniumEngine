//! Bindless resource management.
//!
//! True bindless rendering leverages large arrays of textures and buffers
//! bound once per frame (or per pass), and indexed dynamically by the
//! shaders (e.g., using a material ID or instance ID).
//!
//! ## Reference Architecture
//!
//! Inspired by O3DE's Atom RHI (`Bindless.md`).
//! Instead of binding individual `ShaderResourceGroup`s per material,
//! we allocate descriptors from a global pool and pass the descriptor
//! indices to the shader via push constants or storage buffers.
//!
//! ## Phase 13C additions
//!
//! Bindings 7–10 for clustered lighting:
//! - binding 7: `array<GpuLocalLight>`  — local light data (point/spot)
//! - binding 8: `array<u32>`            — flattened light index list
//! - binding 9: `array<ClusterOffset>`  — per-froxel (offset, count)
//! - binding 10: `ClusterParams`        — grid dimensions, shading mode

use crate::cluster::ClusterGrid;
use wgpu;

/// Maximum number of sampled textures in the bindless array.
pub const MAX_BINDLESS_TEXTURES: u32 = 1024;

/// A global pool of resources mapped to a single bind group.
///
/// In a complete implementation, this handles dynamic allocation
/// and updates of texture and buffer views into the bindless arrays.
pub struct GlobalResourcePool {
    pub layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,

    // Keep the buffers alive
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub instance_buffer: wgpu::Buffer,
    pub view_proj_buffer: wgpu::Buffer,
    pub material_buffer: wgpu::Buffer,
    /// Directional light buffer (binding 6) — 320 bytes, `GpuDirectionalLight`.
    pub light_buffer: wgpu::Buffer,
    /// Phase 25A-2: `array<GpuTerrainMaterial>` (binding 11).
    pub terrain_material_buffer: wgpu::Buffer,

    /// Phase 13C: Cluster grid buffers (bindings 7–10).
    pub cluster_grid: ClusterGrid,

    // Storage for views to recreate bind group
    pub texture_views: Vec<wgpu::TextureView>,
}

impl GlobalResourcePool {
    pub fn new(
        device: &wgpu::Device,
        vertex_buffer: &wgpu::Buffer,
        index_buffer: &wgpu::Buffer,
        instance_buffer: &wgpu::Buffer,
        view_proj_buffer: &wgpu::Buffer,
        material_buffer: &wgpu::Buffer,
        light_buffer: &wgpu::Buffer,
        terrain_material_buffer: &wgpu::Buffer,
    ) -> Self {
        let cluster_grid = ClusterGrid::new(device);

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("GlobalResourcePool_Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX
                        | wgpu::ShaderStages::FRAGMENT
                        | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX
                        | wgpu::ShaderStages::FRAGMENT
                        | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX
                        | wgpu::ShaderStages::FRAGMENT
                        | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX
                        | wgpu::ShaderStages::FRAGMENT
                        | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: std::num::NonZeroU32::new(MAX_BINDLESS_TEXTURES),
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // binding 6: GpuDirectionalLight (320 bytes) — used by shadow.wgsl + shading.wgsl
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::VERTEX
                        | wgpu::ShaderStages::FRAGMENT
                        | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // ── Phase 13C: Clustered lighting bindings ──────────────────
                // binding 7: array<GpuLocalLight> — local light data
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // binding 8: array<u32> — flattened light index list
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // binding 9: array<ClusterOffset> — per-froxel (offset, count)
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // binding 10: ClusterParams — grid dimensions + shading mode
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Phase 25A-2, binding 11: array<GpuTerrainMaterial>. Terrain
                // shades in `shading.wgsl` now, and its splat/layer parameters
                // are too large to fit in `Material`.
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let dummy_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Dummy Bindless Texture"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let dummy_view = dummy_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut views = Vec::with_capacity(MAX_BINDLESS_TEXTURES as usize);
        for _ in 0..MAX_BINDLESS_TEXTURES {
            views.push(&dummy_view);
        }

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GlobalResourcePool_BindGroup"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: vertex_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: index_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: instance_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: view_proj_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureViewArray(&views),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: material_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: cluster_grid.light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: cluster_grid.index_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: cluster_grid.offset_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: cluster_grid.params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: terrain_material_buffer.as_entire_binding(),
                },
            ],
        });

        Self {
            layout,
            bind_group,
            vertex_buffer: vertex_buffer.clone(),
            index_buffer: index_buffer.clone(),
            instance_buffer: instance_buffer.clone(),
            view_proj_buffer: view_proj_buffer.clone(),
            material_buffer: material_buffer.clone(),
            light_buffer: light_buffer.clone(),
            terrain_material_buffer: terrain_material_buffer.clone(),
            cluster_grid,
            texture_views: (0..MAX_BINDLESS_TEXTURES)
                .map(|_| dummy_view.clone())
                .collect(),
        }
    }

    pub fn update_textures(&mut self, device: &wgpu::Device) {
        let views: Vec<&wgpu::TextureView> = self.texture_views.iter().collect();

        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GlobalResourcePool_BindGroup"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.vertex_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.index_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.instance_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.view_proj_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureViewArray(&views),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.material_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: self.light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: self.cluster_grid.light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: self.cluster_grid.index_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: self.cluster_grid.offset_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: self.cluster_grid.params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: self.terrain_material_buffer.as_entire_binding(),
                },
            ],
        });
    }
}
