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
///
/// 2048 since TSF's Town alone references ~880 imported images before
/// terrain, foliage and authored materials; at 1024 opening it after any other
/// level exhausted the pool, which panics. Scene switches now release imported
/// slots (`SomniumRenderer::release_uploads`), so this bounds one level, not
/// a session.
pub const MAX_BINDLESS_TEXTURES: u32 = 2048;

static CAPACITY: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(MAX_BINDLESS_TEXTURES);

/// How many textures the bindless pool holds: [`MAX_BINDLESS_TEXTURES`], unless `SOMNIUM_BINDLESS_TEXTURES` asked
/// for more and the device granted it (set at context creation, before any pool exists). An open world's terrains,
/// buildings and skinned crowds outgrow 2,048 (Rimeborn's Tyffir left ~400 textures blank).
#[must_use]
pub fn bindless_capacity() -> u32 {
    CAPACITY.load(std::sync::atomic::Ordering::Relaxed)
}

/// The binding-array size to request of a device whose adapter allows `adapter_max`: exactly
/// [`MAX_BINDLESS_TEXTURES`] when `SOMNIUM_BINDLESS_TEXTURES` is unset (as before), else that ask (never below the
/// default) within the adapter's limit. Remembered for [`bindless_capacity`].
pub(crate) fn request_bindless_capacity(adapter_max: u32) -> u32 {
    let asked = std::env::var("SOMNIUM_BINDLESS_TEXTURES").ok().and_then(|v| v.trim().parse::<u32>().ok());
    let capacity = asked.map_or(MAX_BINDLESS_TEXTURES, |n| n.max(MAX_BINDLESS_TEXTURES).min(adapter_max.max(MAX_BINDLESS_TEXTURES)));
    CAPACITY.store(capacity, std::sync::atomic::Ordering::Relaxed);
    capacity
}

/// A global pool of resources mapped to a single bind group.
///
/// In a complete implementation, this handles dynamic allocation
/// and updates of texture and buffer views into the bindless arrays.
pub struct GlobalResourcePool {
    pub layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
    /// The same layout and buffers with a one-view texture array, for passes
    /// whose shaders never index `textures` (census, classify, velocity, skin
    /// scatter). wgpu tracks every view of a bound group for each pass (each
    /// dispatch, in compute) at `finish`; this one costs one view, not the
    /// level's 1,500-odd. Its buffers never change, so it is built once.
    pub lean_bind_group: wgpu::BindGroup,

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
    dummy_view: wgpu::TextureView,
    /// Which slots hold a real texture ([`Self::set_slot`] / [`Self::clear_slot`]).
    live: Vec<bool>,
    /// A texture was given the pool's fallback slot (the top one): bind them all.
    bind_all: bool,
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
                    count: std::num::NonZeroU32::new(bindless_capacity()),
                },
                // Materials reach the vertex stage for foliage wind (`wind.wgsl`):
                // the visibility and shadow rasters displace by the material's
                // bend and flutter.
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
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

        let mut views = Vec::with_capacity(bindless_capacity() as usize);
        for _ in 0..bindless_capacity() {
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

        let mut pool = Self {
            layout,
            lean_bind_group: bind_group.clone(),
            bind_group,
            vertex_buffer: vertex_buffer.clone(),
            index_buffer: index_buffer.clone(),
            instance_buffer: instance_buffer.clone(),
            view_proj_buffer: view_proj_buffer.clone(),
            material_buffer: material_buffer.clone(),
            light_buffer: light_buffer.clone(),
            terrain_material_buffer: terrain_material_buffer.clone(),
            cluster_grid,
            dummy_view: dummy_view.clone(),
            texture_views: (0..bindless_capacity())
                .map(|_| dummy_view.clone())
                .collect(),
            live: vec![false; bindless_capacity() as usize],
            bind_all: false,
        };
        let dummy = pool.dummy_view.clone();
        pool.lean_bind_group = pool.rebuild_bind_group(device, &[&dummy]);
        // Nothing is live yet: bind the one slot rather than all of them.
        pool.update_textures(device);
        pool
    }

    /// Put a texture in a slot (the bind group follows at [`Self::update_textures`]).
    pub fn set_slot(&mut self, slot: usize, view: wgpu::TextureView) {
        if let Some(entry) = self.texture_views.get_mut(slot) {
            *entry = view;
            self.live[slot] = true;
        }
    }

    /// Return a slot to the dummy image.
    pub fn clear_slot(&mut self, slot: usize) {
        if let Some(entry) = self.texture_views.get_mut(slot) {
            *entry = self.dummy_view.clone();
            self.live[slot] = false;
        }
    }

    /// A texture now samples the pool's fallback slot, which sits at the top of
    /// the array: from here on every slot is bound.
    pub fn bind_every_slot(&mut self) {
        self.bind_all = true;
    }

    /// How many slots the bind group carries: up to the highest live one.
    ///
    /// wgpu tracks every view of a bound group each time a pass binds it, at
    /// `CommandEncoder::finish`: about 80 ns a view a bind on this machine. All
    /// 2,048 slots bound about 40 times a frame cost 6.5-6.9 ms of CPU even in
    /// a small level (`tests/bindless_finish_cost.rs`). Slots past the highest
    /// live one are never indexed (materials of released textures are reset to
    /// -1), and `PARTIALLY_BOUND_BINDING_ARRAY`, a required feature, allows a
    /// shorter array than the layout's.
    pub fn bound_len(&self) -> usize {
        if self.bind_all {
            return self.texture_views.len();
        }
        self.live.iter().rposition(|live| *live).map_or(1, |slot| slot + 1)
    }

    fn rebuild_bind_group(
        &self,
        device: &wgpu::Device,
        views: &[&wgpu::TextureView],
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
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
                    resource: wgpu::BindingResource::TextureViewArray(views),
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
        })
    }

    /// The instance buffer grew ([`crate::instance::InstancePool::reserve`]): bind the new one in both groups.
    pub fn set_instance_buffer(&mut self, device: &wgpu::Device, buffer: &wgpu::Buffer) {
        self.instance_buffer = buffer.clone();
        self.update_textures(device);
        let dummy = self.dummy_view.clone();
        self.lean_bind_group = self.rebuild_bind_group(device, &[&dummy]);
    }

    pub fn update_textures(&mut self, device: &wgpu::Device) {
        let views: Vec<&wgpu::TextureView> = self.texture_views[..self.bound_len()].iter().collect();
        self.bind_group = self.rebuild_bind_group(device, &views);
    }

    /// Bind group whose listed slots are the dummy view, so a compute dispatch
    /// can write those GPU images as storage without a sampled+storage clash.
    pub fn bind_group_hiding_slots(&self, device: &wgpu::Device, hide: &[u32]) -> wgpu::BindGroup {
        let mut views: Vec<&wgpu::TextureView> = self.texture_views[..self.bound_len()].iter().collect();
        for &id in hide {
            let slot = id as usize;
            if slot < views.len() {
                views[slot] = &self.dummy_view;
            }
        }
        self.rebuild_bind_group(device, &views)
    }
}
