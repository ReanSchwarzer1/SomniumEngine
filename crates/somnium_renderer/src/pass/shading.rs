//! Shading pass that reads the visibility buffer and outputs final color.
//!
//! Phase 11 additions:
//!   @group(1) binding 2 — shadow_atlas (texture_depth_2d)
//!   @group(1) binding 3 — shadow_sampler (sampler_comparison for PCF)
use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Arc, Mutex};
use wgpu;

/// One compiled shading pipeline: its spec, vertex entry and depth split.
pub type ShadingKey = (ShadingSpec, &'static str, Option<wgpu::CompareFunction>);

/// Compile-time feature set for the shading PSO.
///
/// Runtime uniforms cannot delete hex/POM/PCSS from the shader, so occupancy
/// stayed at the union of every path. Recreating the pipeline with these
/// overrides is what actually drops Shading ms when the features are off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShadingSpec {
    pub hex: bool,
    pub pom: bool,
    pub pcss: bool,
    pub contact: bool,
    pub clipmap: bool,
    pub debug: bool,
    pub terrain_scan: u32,
    /// Union of uploaded painted layers across live terrain draws. Unpainted
    /// slots can leave the shader without changing the authored material mix.
    pub terrain_active_layers: u32,
    /// Keep `evaluate_terrain_material` in the module (Phase DF audit).
    ///
    /// False only when every terrain queued this frame shades through its
    /// clipmap, which is the one case where nothing can reach the live path.
    pub live_terrain: bool,
    /// Phase DOOM-B ablation: shade only one class of pixel and return black
    /// for the rest. `0` is normal rendering. See [`Ablate`].
    ///
    /// **This is a measuring instrument, not a feature.** It makes the image
    /// wrong on purpose so the timer can say what a class of pixel costs to
    /// execute. Driven by `SOMNIUM_SHADE_ABLATE`; never set from the UI.
    pub ablate: u32,
}

/// Values for [`ShadingSpec::ablate`], matching `shading.wgsl`.
///
/// The numbers are shared with the shader by hand, so they are named here
/// rather than written as literals at each call site.
pub mod ablate {
    /// Normal rendering.
    pub const OFF: u32 = 0;
    /// Only the sky/background branch runs.
    pub const SKY: u32 = 1;
    /// Only opaque non-terrain, non-cutout surfaces.
    pub const MESH: u32 = 2;
    /// Only cutout (foliage) materials.
    pub const FOLIAGE: u32 = 3;
    /// Only terrain.
    pub const TERRAIN: u32 = 4;

    /// Parse `SOMNIUM_SHADE_ABLATE`. Unset or unrecognised means [`OFF`].
    #[must_use]
    pub fn from_env() -> u32 {
        match std::env::var("SOMNIUM_SHADE_ABLATE")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "1" | "sky" => SKY,
            "2" | "mesh" => MESH,
            "3" | "foliage" => FOLIAGE,
            "4" | "terrain" => TERRAIN,
            _ => OFF,
        }
    }
}

impl ShadingSpec {
    pub const COMPACT: Self = Self {
        hex: false,
        pom: false,
        pcss: false,
        contact: false,
        clipmap: false,
        debug: false,
        terrain_scan: 16,
        terrain_active_layers: u32::MAX,
        live_terrain: true,
        ablate: ablate::OFF,
    };

    fn constants(self) -> [(&'static str, f64); 12] {
        static BRANCHLESS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let branchless = *BRANCHLESS.get_or_init(|| {
            std::env::var("SOMNIUM_TERRAIN_BRANCHLESS_SELECT").as_deref() == Ok("1")
        });
        static STORAGE_READS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let storage_reads = *STORAGE_READS.get_or_init(|| {
            // Native A/B/A reduced terrain shading from ~17 to ~7.4 ms with
            // the same material values, coordinates and quality. Keep an
            // explicit reference opt-out for regression diagnosis.
            std::env::var("SOMNIUM_TERRAIN_STORAGE_READS").as_deref() != Ok("0")
        });
        [
            ("enable_hex", f64::from(u32::from(self.hex))),
            ("enable_pom", f64::from(u32::from(self.pom))),
            ("enable_pcss", f64::from(u32::from(self.pcss))),
            ("enable_contact", f64::from(u32::from(self.contact))),
            ("enable_clipmap", f64::from(u32::from(self.clipmap))),
            ("enable_debug", f64::from(u32::from(self.debug))),
            ("terrain_scan", f64::from(self.terrain_scan)),
            (
                "enable_live_terrain",
                f64::from(u32::from(self.live_terrain)),
            ),
            ("shade_ablate", f64::from(self.ablate)),
            ("terrain_active_layers", f64::from(self.terrain_active_layers)),
            ("terrain_branchless_select", f64::from(u32::from(branchless))),
            ("terrain_storage_reads", f64::from(u32::from(storage_reads))),
        ]
    }
}

/// Per-bin vertex constants (Phase DOOM-C). Mirrors `TileParams` in
/// `shading.wgsl`; the two are 32 bytes and must stay that way.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TileParams {
    pub tiles_x: u32,
    pub tile_size: u32,
    pub bin_offset: u32,
    pub width: u32,
    pub height: u32,
    /// Phase DOOM-E: clip-space depth of the aerial split.
    pub split_depth: f32,
    _pad: [u32; 2],
}

/// Stride between per-bin slices of the tile-params buffer.
///
/// 256 is wgpu's `min_uniform_buffer_offset_alignment` on every backend we
/// target; the struct itself is 32 bytes and the rest is padding.
const TILE_PARAMS_STRIDE: u64 = 256;

/// Shading variants compiling on worker threads, ahead of the frames that will want them (`ShadingPass::warm`).
///
/// A cold compile of this shader is about 2.4 s on an RTX 5080 Laptop (22 Oct 2026), and a chapter's first frames used
/// to pay two or three of them on the main thread. The driver keeps what it compiled on disk, so the next run finds
/// them in milliseconds; warming ahead is what makes the *first* run not stall.
#[derive(Default)]
struct Warm {
    queue: Arc<Mutex<WarmQueue>>,
    rx: Option<mpsc::Receiver<(ShadingKey, wgpu::RenderPipeline)>>,
    tx: Option<mpsc::Sender<(ShadingKey, wgpu::RenderPipeline)>>,
    /// Asked for, queued or compiling, and not arrived yet.
    pending: HashSet<ShadingKey>,
    total: usize,
    done: usize,
}

#[derive(Default)]
struct WarmQueue {
    keys: std::collections::VecDeque<ShadingKey>,
    workers: usize,
}

/// Gives a worker's place back if it panics (a validation error in a pipeline), so a frame waiting on its variant
/// stops waiting and compiles it itself.
struct WorkerGuard(Arc<Mutex<WarmQueue>>, bool);

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if !self.1 {
            self.0.lock().unwrap_or_else(|e| e.into_inner()).workers -= 1;
        }
    }
}

pub struct ShadingPass {
    /// The fullscreen pipeline for `spec`. `None` until the first frame's `ensure_pipeline`, so startup does not wait
    /// on a compile the workers are already doing.
    pub pipeline: Option<wgpu::RenderPipeline>,
    /// Every variant compiled so far. A spec that comes back (PCSS goes off and on as traced shadows come and go, a
    /// scene with terrain and then one without) is free the second time, and what `warm` compiled ahead is found here.
    variants: HashMap<ShadingKey, wgpu::RenderPipeline>,
    warm: Warm,
    shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    /// Phase DOOM-C.
    tile_layout: wgpu::BindGroupLayout,
    tile_params: wgpu::Buffer,
    tile_bind_group: Option<wgpu::BindGroup>,
    /// One pipeline per bin, each with the spec that bin needs. Indexed by bin;
    /// grown to `BIN_COUNT` on first use.
    bin_pipelines: Vec<(ShadingSpec, wgpu::RenderPipeline)>,
    /// Phase DOOM-E: the near/aerial pair, selected by a depth test rather
    /// than by a tile list. `None` until the first frame that wants them.
    split_near: Option<(ShadingSpec, wgpu::RenderPipeline)>,
    split_aerial: Option<(ShadingSpec, wgpu::RenderPipeline)>,
    hdr_format: wgpu::TextureFormat,
    spec: ShadingSpec,
    /// Composition options are part of the live pipeline and survive hot reload.
    defines: somnium_shader::Defines,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
    // Stored for bind-group recreation on resize / shadow atlas change.
    sampler: wgpu::Sampler,
    shadow_atlas_view: wgpu::TextureView,
    shadow_sampler: wgpu::Sampler,
    /// Phase 19: environment cubemap + its sampler, kept so the bind group can
    /// be rebuilt on resize.
    env_view: wgpu::TextureView,
    env_sampler: wgpu::Sampler,
    /// Phase 24I: GTAO output, kept for the same reason.
    gtao_view: wgpu::TextureView,
    /// Phase 24X: scene depth, for contact shadows.
    depth_view: wgpu::TextureView,
    /// Phase 24K: traced sun visibility.
    restir_view: wgpu::TextureView,
    /// Phase 24L: traced indirect diffuse.
    restir_gi_view: wgpu::TextureView,
    /// Shared scene structure; absent on devices without ray queries.
    local_shadow_tlas: Option<wgpu::Tlas>,
    /// Phases 24U/25I: froxel volume, its sampler, and the range uniform.
    volumetric_view: wgpu::TextureView,
    volumetric_sampler: wgpu::Sampler,
    volumetric_range: wgpu::Buffer,
    lighting_aux_view: wgpu::TextureView,
    world_volume_view: wgpu::TextureView,
    lighting_extra: wgpu::Buffer,
    sh_probes: wgpu::Buffer,
    /// CONTROL-M's world-XZ cloud shadow, kept so a resize can rebuild the
    /// bind group without threading it back through the caller.
    cloud_shadow_view: wgpu::TextureView,
    cloud_shadow_params: wgpu::Buffer,
    /// CONTROL-N: `[wet_diffuse, wet_specular, puddles, unused]`. All zero
    /// when no weather is driving, which makes the wetness path free rather
    /// than merely cheap.
    weather: wgpu::Buffer,
    /// CONTROL-O: the decal grid's four buffers, cloned so a resize can
    /// rebuild the bind group without threading the grid back through.
    decal_buffers: [wgpu::Buffer; 4],
    /// MORROWIND-Z sparse shadow cache. Dummy/disabled until a scene asks for VSM.
    virtual_shadow_view: wgpu::TextureView,
    virtual_shadow_sampler: wgpu::Sampler,
    virtual_shadow_page_table: wgpu::Buffer,
    virtual_shadow_params: wgpu::Buffer,
    /// DREAMS-B's shared stochastic sampling atlas.
    grain_packed: wgpu::Buffer,
    /// Vertex paint words (see `crate::vertex_paint`), grown on demand.
    paint_buffer: wgpu::Buffer,
    paint_capacity_words: u64,
    /// Where the finished surface goes while the path tracer runs (bindings 31-33).
    path_surface: [wgpu::TextureView; 3],
    /// Kept so growing the paint buffer can rebuild the bind group.
    visibility_view: wgpu::TextureView,
    _virtual_shadow_dummy: wgpu::Texture,
    /// Phase DF: sampled 2D arrays of the material clipmap (group 2). Dummy
    /// 1×1 until `set_clipmap_arrays` after a terrain is created.
    clipmap_layout: wgpu::BindGroupLayout,
    /// Bilinear + Repeat, anisotropy 1. See [`ShadingPass::clipmap_sampler`].
    clipmap_sampler: wgpu::Sampler,
    pub clipmap_bind_group: wgpu::BindGroup,
    _clipmap_dummy_detail: wgpu::Texture,
    _clipmap_dummy_macro: wgpu::Texture,
}

impl ShadingPass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        global_bind_group_layout: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        visibility_view: &wgpu::TextureView,
        shadow_atlas_view: &wgpu::TextureView,
        shadow_sampler: &wgpu::Sampler,
        env_view: &wgpu::TextureView,
        env_sampler: &wgpu::Sampler,
        gtao_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        restir_view: &wgpu::TextureView,
        restir_gi_view: &wgpu::TextureView,
        volumetric_view: &wgpu::TextureView,
        volumetric_sampler: &wgpu::Sampler,
        lighting_aux_view: &wgpu::TextureView,
        world_volume_view: &wgpu::TextureView,
        sh_probes: &wgpu::Buffer,
        cloud_shadow_view: &wgpu::TextureView,
        cloud_shadow_params: &wgpu::Buffer,
        decals: &crate::pass::decal::DecalGrid,
        grain_packed: &wgpu::Buffer,
        local_shadow_tlas: Option<&wgpu::Tlas>,
        path_surface: [&wgpu::TextureView; 3],
    ) -> Self {
        let mut layout_entries = vec![
                // binding 0: vis_buffer (R32Uint)
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // binding 1: default filtering sampler for PBR texture lookups
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // binding 2: shadow atlas depth texture
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // binding 3: PCF comparison sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                // binding 4: prefiltered environment cubemap (Phase 19)
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                    count: None,
                },
                // binding 5: trilinear sampler for the environment map
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // Phase 24K: traced sun visibility, replacing the shadow
                // map when ReSTIR is active.
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Phase 24L: traced indirect diffuse, replacing the
                // environment map's diffuse half when GI is active.
                wgpu::BindGroupLayoutEntry {
                    binding: 12,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Phase 24X: scene depth, for the contact-shadow march.
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Phases 24U/25I: the froxel volume, its sampler, and the
                // distance it spans (0 when volumetrics are off).
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Phase 24I: GTAO visibility + bent normal.
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 13,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 14,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 15,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 16,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // CONTROL-M: the world-XZ cloud shadow field, and the vec4
                // that says where it is. Terrain and water both read it here
                // rather than each growing their own copy, which is what makes
                // a cloud's shadow cross a beach onto the sea.
                wgpu::BindGroupLayoutEntry {
                    binding: 17,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 18,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // CONTROL-N: `[wet_diffuse, wet_specular, puddles, unused]`.
                wgpu::BindGroupLayoutEntry {
                    binding: 19,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // CONTROL-O: the decal list, its per-froxel table and its
                // index list, then the count. The froxel *geometry* is the
                // light grid's, read from `cluster_params`, so decals cost
                // four buffers and not a second grid.
                wgpu::BindGroupLayoutEntry {
                    binding: 20,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 21,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 22,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 23,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 24,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 25,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 26,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 27,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 28,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // binding 30: vertex paint (header, masks, instance table)
                wgpu::BindGroupLayoutEntry {
                    binding: 30,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ];
        // bindings 31-33: the finished surface, for the path tracer while it runs (path_tracer.rs `surface_views`).
        for binding in 31..34 {
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba16Float,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            });
        }
        if local_shadow_tlas.is_some() {
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 29,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::AccelerationStructure { vertex_return: false },
                count: None,
            });
        }
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shading Pass Bind Group Layout"), entries: &layout_entries,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Default Shading Sampler"),
            // glTF's default wrap is REPEAT. wgpu's Default is ClampToEdge,
            // which smears the edge texel across everything whose UVs leave
            // 0..1 — the cause of the stretched/streaked look on imported
            // models with tiled materials.
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            // Phase 25K. O3DE's terrain detail sampler runs at MaxAnisotropy 16
            // (`TerrainMaterialSrg.azsli`) and the reason is terrain-shaped:
            // ground is the one surface always seen at a grazing angle, where
            // an isotropic mip is chosen for the *shorter* axis and smears
            // everything along the longer one. Trilinear alone turns a
            // photographed layer to mush a few metres out.
            anisotropy_clamp: 16,
            ..Default::default()
        });

        // 16 bytes: `vec4` is the smallest uniform WGSL will align, and only x
        // is used — the volume's range in metres, 0 when it is switched off.
        let volumetric_range = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Volumetric Range"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let lighting_extra = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Lighting extra params"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let weather = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Weather wetness params"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let virtual_shadow_dummy = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Virtual Shadow Dummy"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let virtual_shadow_view =
            virtual_shadow_dummy.create_view(&wgpu::TextureViewDescriptor::default());
        let virtual_shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Virtual Shadow Dummy Sampler"),
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let virtual_shadow_page_table = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Virtual Shadow Dummy Page Table"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let virtual_shadow_params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Virtual Shadow Dummy Params"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });

        let decal_buffers = [
            decals.decal_buffer.clone(),
            decals.offset_buffer.clone(),
            decals.index_buffer.clone(),
            decals.params_buffer.clone(),
        ];

        let paint_capacity_words = 1024;
        let paint_buffer = Self::make_paint_buffer(device, paint_capacity_words);
        let bind_group = Self::make_bind_group(
            device,
            &bind_group_layout,
            visibility_view,
            &sampler,
            shadow_atlas_view,
            shadow_sampler,
            env_view,
            env_sampler,
            gtao_view,
            depth_view,
            restir_view,
            restir_gi_view,
            volumetric_view,
            volumetric_sampler,
            &volumetric_range,
            lighting_aux_view,
            world_volume_view,
            &lighting_extra,
            sh_probes,
            cloud_shadow_view,
            cloud_shadow_params,
            &weather,
            &decal_buffers,
            &virtual_shadow_view,
            &virtual_shadow_sampler,
            &virtual_shadow_page_table,
            &virtual_shadow_params,
            grain_packed,
            &paint_buffer,
            local_shadow_tlas,
            path_surface,
        );

        // MORROWIND-C: composition is declared in `shading.wgsl` and
        // resolved by `somnium_shader`; this site no longer knows the order.
        let mut defines = crate::shaders::define::DREAMS_STF;
        if local_shadow_tlas.is_some() {
            defines = defines.with(crate::shaders::define::LOCAL_SHADOWS);
        }
        let shader_source = shaders
            .source("shading.wgsl", defines)
            .unwrap_or_else(|error| panic!("shading.wgsl composition failed: {error}"));
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shading Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let clipmap_layout = Self::clipmap_bind_group_layout(device);
        let clipmap_sampler = Self::clipmap_sampler(device);
        let (dummy_detail, dummy_detail_view) = Self::dummy_clipmap_array(device, 8);
        let (dummy_macro, dummy_macro_view) = Self::dummy_clipmap_array(device, 4);
        let clipmap_bind_group = Self::make_clipmap_bind_group(
            device,
            &clipmap_layout,
            &dummy_detail_view,
            &dummy_detail_view,
            &dummy_macro_view,
            &dummy_macro_view,
            &clipmap_sampler,
        );

        // Phase DOOM-C: group 3 carries the tile list and the per-bin offset.
        // Present in the layout even on the fullscreen path — `vs_main` never
        // reads it, but wgpu requires every group in the layout to be bound
        // before a draw, and one always-bound group is simpler than two
        // pipeline layouts that have to be kept in step.
        let tile_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shading Tile BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        // One buffer, one aligned slice per bin, selected at
                        // draw time. Six separate buffers would work and would
                        // mean six bind groups to rebuild whenever the tile
                        // buffer is reallocated.
                        has_dynamic_offset: true,
                        min_binding_size: std::num::NonZeroU64::new(
                            std::mem::size_of::<TileParams>() as u64,
                        ),
                    },
                    count: None,
                },
            ],
        });
        let tile_params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Shading Tile Params"),
            size: TILE_PARAMS_STRIDE * crate::pass::classify::BIN_COUNT as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Shading Pipeline Layout"),
            bind_group_layouts: &[
                Some(global_bind_group_layout),
                Some(&bind_group_layout),
                Some(&clipmap_layout),
                Some(&tile_layout),
            ],
            immediate_size: 0,
        });

        let spec = ShadingSpec::COMPACT;
        let mut pass = Self {
            pipeline: None,
            variants: HashMap::new(),
            warm: Warm::default(),
            shader,
            pipeline_layout,
            tile_layout,
            tile_params,
            tile_bind_group: None,
            bin_pipelines: Vec::new(),
            split_near: None,
            split_aerial: None,
            hdr_format: surface_format,
            spec,
            defines,
            bind_group_layout,
            bind_group,
            sampler,
            shadow_atlas_view: shadow_atlas_view.clone(),
            shadow_sampler: shadow_sampler.clone(),
            env_view: env_view.clone(),
            env_sampler: env_sampler.clone(),
            gtao_view: gtao_view.clone(),
            depth_view: depth_view.clone(),
            restir_view: restir_view.clone(),
            restir_gi_view: restir_gi_view.clone(),
            local_shadow_tlas: local_shadow_tlas.cloned(),
            volumetric_view: volumetric_view.clone(),
            volumetric_sampler: volumetric_sampler.clone(),
            volumetric_range,
            lighting_aux_view: lighting_aux_view.clone(),
            world_volume_view: world_volume_view.clone(),
            lighting_extra,
            sh_probes: sh_probes.clone(),
            cloud_shadow_view: cloud_shadow_view.clone(),
            cloud_shadow_params: cloud_shadow_params.clone(),
            weather,
            decal_buffers,
            virtual_shadow_view,
            virtual_shadow_sampler,
            virtual_shadow_page_table,
            virtual_shadow_params,
            grain_packed: grain_packed.clone(),
            paint_buffer,
            paint_capacity_words,
            path_surface: path_surface.map(Clone::clone),
            visibility_view: visibility_view.clone(),
            _virtual_shadow_dummy: virtual_shadow_dummy,
            clipmap_layout,
            clipmap_sampler,
            clipmap_bind_group,
            _clipmap_dummy_detail: dummy_detail,
            _clipmap_dummy_macro: dummy_macro,
        };
        // The first frame's likeliest pipeline, compiled while the rest of startup goes on.
        pass.warm(device, [(spec, "vs_main", None)]);
        pass
    }

    /// Rebuild the shader module and every pipeline built from it.
    ///
    /// MORROWIND-C's hot-reload swap, for the acceptance case: `brdf.wgsl` is
    /// composed into `shading.wgsl`, so editing it lands here.
    ///
    /// **Nothing is mutated until the new module exists.** `shaders.source`
    /// resolves and `create_shader_module` compiles before the first `self.`
    /// assignment, so a composition failure returns with the pass exactly as it
    /// was and the old pipeline still bound — which is the rule the plan is
    /// emphatic about and the specific check Appendix A.7 names for this
    /// sub-phase.
    ///
    /// Note what is *not* guarded here: naga rejecting the source. wgpu reports
    /// that through an error scope rather than a `Result`, so the renderer
    /// validates before calling this (see `SomniumRenderer::reload_shaders`),
    /// and by the time control arrives the source is known to parse.
    pub fn reload(
        &mut self,
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
    ) -> Result<(), somnium_shader::ShaderError> {
        let source = shaders.source("shading.wgsl", self.defines)?;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shading Shader"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = Self::make_pipeline(
            device,
            &shader,
            &self.pipeline_layout,
            self.hdr_format,
            self.spec,
            "vs_main",
            None,
        );

        // Every variant of the old module goes, and so does anything still compiling from it (its workers stop at
        // their next send).
        self.warm = Warm::default();
        self.variants.clear();
        self.variants.insert((self.spec, "vs_main", None), pipeline.clone());
        self.pipeline = Some(pipeline);
        self.shader = shader;
        // The per-bin and split pipelines are built lazily from `self.shader`
        // on first use, so dropping them is the whole of their reload. Rebuilding
        // them eagerly would compile up to eight variants for a scene that may
        // want none of them this frame.
        self.bin_pipelines.clear();
        self.split_near = None;
        self.split_aerial = None;
        Ok(())
    }

    fn make_pipeline(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        layout: &wgpu::PipelineLayout,
        hdr_format: wgpu::TextureFormat,
        spec: ShadingSpec,
        vertex_entry: &str,
        depth_split: Option<wgpu::CompareFunction>,
    ) -> wgpu::RenderPipeline {
        // The depth split's pipelines leave out the path tracer's surface writes, which would cost them early-Z
        // (shading.wgsl `path_surface`); the split never runs under the tracer (renderer.rs).
        let mut constants = spec.constants().to_vec();
        constants.push(("path_surface", f64::from(u32::from(depth_split.is_none()))));
        let compilation_options = wgpu::PipelineCompilationOptions {
            constants: &constants,
            zero_initialize_workgroup_memory: true,
        };
        let started = std::time::Instant::now();
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Shading Pipeline"),
            layout: Some(layout),
            multiview_mask: None,
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(vertex_entry),
                buffers: &[],
                compilation_options: compilation_options.clone(),
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: hdr_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options,
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            // Phase DOOM-E: the depth split.
            //
            // The fullscreen triangle is emitted at the clip-space depth of the
            // aerial distance, and the test against the scene's own depth buffer
            // decides which half of the screen this pipeline covers: `Greater`
            // keeps everything nearer than the split, `LessEqual` everything at
            // or beyond it (sky included, at the cleared far value). Two draws,
            // no overlap, complete coverage — and early-Z rejects the other half
            // before a single fragment of the expensive shader runs.
            //
            // Never writes. The depth buffer belongs to the visibility pass and
            // several later passes read it.
            depth_stencil: depth_split.map(|compare| wgpu::DepthStencilState {
                format: crate::pass::visibility::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(compare),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            cache: None,
        });
        tracing::info!(ms = started.elapsed().as_millis() as u64, vertex_entry, split = ?depth_split, ?spec,
            "shading pipeline compiled");
        pipeline
    }

    /// The pipeline for `key`: from the variants compiled so far, from a worker already compiling it (waited for, not
    /// compiled twice), or compiled here.
    fn variant(&mut self, device: &wgpu::Device, key: ShadingKey) -> wgpu::RenderPipeline {
        self.collect();
        if let Some(pipeline) = self.variants.get(&key) {
            return pipeline.clone();
        }
        if self.warm.pending.contains(&key) {
            let queued = {
                let mut queue = self.warm.queue.lock().unwrap_or_else(|e| e.into_inner());
                let at = queue.keys.iter().position(|k| *k == key);
                at.and_then(|at| queue.keys.remove(at)).is_some()
            };
            if queued {
                // No worker has started it: quicker to compile it here than to wait for one to come free.
                self.warm.pending.remove(&key);
                self.warm.done += 1;
            } else {
                let started = std::time::Instant::now();
                while !self.variants.contains_key(&key) {
                    let Some(rx) = &self.warm.rx else { break };
                    match rx.recv_timeout(std::time::Duration::from_millis(50)) {
                        Ok(arrived) => self.arrive(arrived),
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            // Every worker gone without it (one panicked): compile it here.
                            if self.warm.queue.lock().unwrap_or_else(|e| e.into_inner()).workers == 0 {
                                break;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                tracing::info!(ms = started.elapsed().as_millis() as u64, "waited for a shading variant compiling ahead");
                if let Some(pipeline) = self.variants.get(&key) {
                    return pipeline.clone();
                }
                // Its worker died: compiled here, and counted, so the warm still reaches its total.
                if self.warm.pending.remove(&key) {
                    self.warm.done += 1;
                }
            }
        }
        let pipeline =
            Self::make_pipeline(device, &self.shader, &self.pipeline_layout, self.hdr_format, key.0, key.1, key.2);
        self.variants.insert(key, pipeline.clone());
        pipeline
    }

    fn arrive(&mut self, (key, pipeline): (ShadingKey, wgpu::RenderPipeline)) {
        if self.warm.pending.remove(&key) {
            self.warm.done += 1;
        }
        self.variants.entry(key).or_insert(pipeline);
    }

    /// Take in whatever the workers have finished.
    fn collect(&mut self) {
        loop {
            let Some(arrived) = self.warm.rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { break };
            self.arrive(arrived);
        }
    }

    /// Compile `keys` on worker threads, in order, skipping any compiled or on the way. Returns at once; the frame
    /// that wants one finds it in the variants, or waits for it if it is compiling.
    pub fn warm(&mut self, device: &wgpu::Device, keys: impl IntoIterator<Item = ShadingKey>) {
        self.collect();
        let mut fresh = Vec::new();
        for key in keys {
            if !self.variants.contains_key(&key) && !self.warm.pending.contains(&key) && !fresh.contains(&key) {
                fresh.push(key);
            }
        }
        if fresh.is_empty() {
            return;
        }
        if self.warm.tx.is_none() {
            let (tx, rx) = mpsc::channel();
            self.warm.tx = Some(tx);
            self.warm.rx = Some(rx);
        }
        self.warm.pending.extend(fresh.iter().copied());
        self.warm.total += fresh.len();
        // Half the cores: the scene loading beside it decodes and uploads on the rest.
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get() / 2).clamp(1, 8);
        let spawn = {
            let mut queue = self.warm.queue.lock().unwrap_or_else(|e| e.into_inner());
            queue.keys.extend(fresh);
            let spawn = threads.saturating_sub(queue.workers).min(queue.keys.len());
            queue.workers += spawn;
            spawn
        };
        for _ in 0..spawn {
            let (queue, tx) = (self.warm.queue.clone(), self.warm.tx.clone().expect("made above"));
            let (device, shader, layout, format) =
                (device.clone(), self.shader.clone(), self.pipeline_layout.clone(), self.hdr_format);
            let worker = move || {
                let mut guard = WorkerGuard(queue.clone(), false);
                loop {
                    let key = {
                        let mut queue = queue.lock().unwrap_or_else(|e| e.into_inner());
                        match queue.keys.pop_front() {
                            Some(key) => key,
                            None => {
                                // Leave under the same lock that saw the queue empty, so `warm` adding keys now
                                // starts a worker for them.
                                queue.workers -= 1;
                                guard.1 = true;
                                return;
                            }
                        }
                    };
                    let pipeline = Self::make_pipeline(&device, &shader, &layout, format, key.0, key.1, key.2);
                    if tx.send((key, pipeline)).is_err() {
                        return; // the pass reloaded or went: nobody wants the rest
                    }
                }
            };
            if std::thread::Builder::new().name("shading warm".into()).spawn(worker).is_err() {
                self.warm.queue.lock().unwrap_or_else(|e| e.into_inner()).workers -= 1;
            }
        }
    }

    /// Variants warmed so far, of all asked for: `(done, total)`.
    pub fn warm_progress(&mut self) -> (usize, usize) {
        self.collect();
        (self.warm.done, self.warm.total)
    }

    /// Recreate the PSO when hex/POM/PCSS/clipmap/debug/layer-scan change.
    /// A hitch only the first time a spec is seen and was not warmed (`warm`).
    pub fn ensure_pipeline(&mut self, device: &wgpu::Device, spec: ShadingSpec) {
        if self.spec == spec && self.pipeline.is_some() {
            return;
        }
        if self.spec != spec {
            tracing::info!(
                hex = spec.hex,
                pom = spec.pom,
                pcss = spec.pcss,
                contact = spec.contact,
                clipmap = spec.clipmap,
                debug = spec.debug,
                terrain_scan = spec.terrain_scan,
                live_terrain = spec.live_terrain,
                "shading pipeline spec changed"
            );
        }
        self.pipeline = Some(self.variant(device, (spec, "vs_main", None)));
        self.spec = spec;
    }

    // ── Phase DOOM-E: the depth split ────────────────────────────────────────

    /// Compile (or reuse) the near and aerial pipelines.
    ///
    /// Both draw the same fullscreen triangle and differ in two things: the
    /// depth comparison that decides which half of the screen they cover, and
    /// the spec they were compiled with. The aerial one is where the saving is —
    /// DOOM-B measured a terrain pixel at walking height costing 9.58 ns against
    /// 4.88 ns from the overview, which is what hex tiling and the parallax
    /// march cost when they are still resolving.
    pub fn ensure_split_pipelines(
        &mut self,
        device: &wgpu::Device,
        near: ShadingSpec,
        aerial: ShadingSpec,
    ) {
        if self.split_near.as_ref().is_none_or(|(s, _)| *s != near) {
            let pipeline = self.variant(device, (near, "vs_main", Some(wgpu::CompareFunction::Greater)));
            self.split_near = Some((near, pipeline));
        }
        if self.split_aerial.as_ref().is_none_or(|(s, _)| *s != aerial) {
            tracing::info!(
                hex = aerial.hex,
                pom = aerial.pom,
                terrain_scan = aerial.terrain_scan,
                "aerial shading pipeline spec changed"
            );
            let pipeline = self.variant(device, (aerial, "vs_main", Some(wgpu::CompareFunction::LessEqual)));
            self.split_aerial = Some((aerial, pipeline));
        }
    }

    pub fn split_near_pipeline(&self) -> Option<&wgpu::RenderPipeline> {
        self.split_near.as_ref().map(|(_, p)| p)
    }

    pub fn split_aerial_pipeline(&self) -> Option<&wgpu::RenderPipeline> {
        self.split_aerial.as_ref().map(|(_, p)| p)
    }

    // ── Phase DOOM-C: binned drawing ─────────────────────────────────────────

    /// Compile (or reuse) the pipeline for one bin.
    ///
    /// Each bin holds its own spec, so a terrain change swaps only the bins
    /// that reach terrain; the pipelines themselves come from the shared
    /// variants, so a spec two bins agree on is compiled once.
    pub fn ensure_bin_pipeline(&mut self, device: &wgpu::Device, bin: usize, spec: ShadingSpec) {
        if self.bin_pipelines.get(bin).is_some_and(|(s, _)| *s == spec) {
            return;
        }
        if bin < self.bin_pipelines.len() {
            tracing::info!(
                bin,
                name = crate::pass::classify::BIN_NAMES
                    .get(bin)
                    .copied()
                    .unwrap_or("?"),
                hex = spec.hex,
                pom = spec.pom,
                pcss = spec.pcss,
                terrain_scan = spec.terrain_scan,
                live_terrain = spec.live_terrain,
                "shading bin pipeline spec changed"
            );
        }
        let pipeline = self.variant(device, (spec, "vs_tile", None));
        // Slots below `bin` not asked for yet hold this one, under its own spec, so their own call replaces them.
        while self.bin_pipelines.len() <= bin {
            self.bin_pipelines.push((spec, pipeline.clone()));
        }
        self.bin_pipelines[bin] = (spec, pipeline);
    }

    pub fn bin_pipeline(&self, bin: usize) -> Option<&wgpu::RenderPipeline> {
        self.bin_pipelines.get(bin).map(|(_, p)| p)
    }

    /// Rebuild the tile bind group. Cheap, but the caller should only do it
    /// when the classifier reports its tile buffer was reallocated.
    pub fn set_tile_buffer(&mut self, device: &wgpu::Device, tiles: &wgpu::Buffer) {
        self.tile_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shading Tile BG"),
            layout: &self.tile_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: tiles.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.tile_params,
                        offset: 0,
                        size: std::num::NonZeroU64::new(std::mem::size_of::<TileParams>() as u64),
                    }),
                },
            ],
        }));
    }

    pub fn tile_bind_group(&self) -> Option<&wgpu::BindGroup> {
        self.tile_bind_group.as_ref()
    }

    /// Write one slice of tile constants per bin.
    pub fn write_tile_params(
        &self,
        queue: &wgpu::Queue,
        tiles_x: u32,
        tile_size: u32,
        tile_capacity: u32,
        width: u32,
        height: u32,
    ) {
        for bin in 0..crate::pass::classify::BIN_COUNT {
            let params = TileParams {
                tiles_x,
                tile_size,
                bin_offset: tile_capacity * bin as u32,
                width,
                height,
                // The tile path has no depth attachment, so the value is
                // ignored; it is written anyway so the two paths cannot drift.
                split_depth: 0.0,
                _pad: [0; 2],
            };
            queue.write_buffer(
                &self.tile_params,
                TILE_PARAMS_STRIDE * bin as u64,
                bytemuck::bytes_of(&params),
            );
        }
    }

    /// Dynamic offset for bin `bin`'s slice.
    pub fn tile_params_offset(bin: usize) -> u32 {
        (TILE_PARAMS_STRIDE * bin as u64) as u32
    }

    /// Write slice 0 for the depth-split path (Phase DOOM-E).
    ///
    /// Only `split_depth` matters here — `vs_main` reads nothing else — but the
    /// whole struct is written so a stale tile-path value cannot survive in the
    /// same slice.
    pub fn write_split_params(
        &self,
        queue: &wgpu::Queue,
        split_depth: f32,
        width: u32,
        height: u32,
    ) {
        let params = TileParams {
            tiles_x: 1,
            tile_size: 1,
            bin_offset: 0,
            width,
            height,
            split_depth,
            _pad: [0; 2],
        };
        queue.write_buffer(&self.tile_params, 0, bytemuck::bytes_of(&params));
    }

    fn make_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        visibility_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        shadow_atlas_view: &wgpu::TextureView,
        shadow_sampler: &wgpu::Sampler,
        env_view: &wgpu::TextureView,
        env_sampler: &wgpu::Sampler,
        gtao_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        restir_view: &wgpu::TextureView,
        restir_gi_view: &wgpu::TextureView,
        volumetric_view: &wgpu::TextureView,
        volumetric_sampler: &wgpu::Sampler,
        volumetric_range: &wgpu::Buffer,
        lighting_aux_view: &wgpu::TextureView,
        world_volume_view: &wgpu::TextureView,
        lighting_extra: &wgpu::Buffer,
        sh_probes: &wgpu::Buffer,
        cloud_shadow_view: &wgpu::TextureView,
        cloud_shadow_params: &wgpu::Buffer,
        weather: &wgpu::Buffer,
        decals: &[wgpu::Buffer; 4],
        virtual_shadow_view: &wgpu::TextureView,
        virtual_shadow_sampler: &wgpu::Sampler,
        virtual_shadow_page_table: &wgpu::Buffer,
        virtual_shadow_params: &wgpu::Buffer,
        grain_packed: &wgpu::Buffer,
        paint: &wgpu::Buffer,
        local_shadow_tlas: Option<&wgpu::Tlas>,
        path_surface: [&wgpu::TextureView; 3],
    ) -> wgpu::BindGroup {
        let mut entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(visibility_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(shadow_atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(env_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(env_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(gtao_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(restir_view),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: wgpu::BindingResource::TextureView(restir_gi_view),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(volumetric_view),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::Sampler(volumetric_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: volumetric_range.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 13,
                    resource: wgpu::BindingResource::TextureView(lighting_aux_view),
                },
                wgpu::BindGroupEntry {
                    binding: 14,
                    resource: wgpu::BindingResource::TextureView(world_volume_view),
                },
                wgpu::BindGroupEntry {
                    binding: 15,
                    resource: lighting_extra.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 16,
                    resource: sh_probes.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 17,
                    resource: wgpu::BindingResource::TextureView(cloud_shadow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 18,
                    resource: cloud_shadow_params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 19,
                    resource: weather.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 20,
                    resource: decals[0].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 21,
                    resource: decals[1].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 22,
                    resource: decals[2].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 23,
                    resource: decals[3].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 24,
                    resource: wgpu::BindingResource::TextureView(virtual_shadow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 25,
                    resource: wgpu::BindingResource::Sampler(virtual_shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 26,
                    resource: virtual_shadow_page_table.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 27,
                    resource: virtual_shadow_params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 28,
                    resource: grain_packed.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 30,
                    resource: paint.as_entire_binding(),
                },
            ];
        if let Some(tlas) = local_shadow_tlas {
            entries.push(wgpu::BindGroupEntry {
                binding: 29, resource: wgpu::BindingResource::AccelerationStructure(tlas),
            });
        }
        for (binding, view) in (31..).zip(path_surface) {
            entries.push(wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(view) });
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shading Pass Bind Group"), layout, entries: &entries,
        })
    }

    fn clipmap_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        let array_tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        };
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shading clipmap arrays"),
            entries: &[
                array_tex(0),
                array_tex(1),
                array_tex(2),
                array_tex(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        })
    }

    /// Bilinear + Repeat, **no anisotropy**, for the toroidal caches.
    ///
    /// Deliberately not `default_sampler`: that one runs at anisotropy 16 for
    /// terrain's grazing angles, and an anisotropic footprint across the clipmap
    /// wrap seam is what produced the ring-edge streaking the manual four-tap
    /// bilinear was written to avoid.
    fn clipmap_sampler(device: &wgpu::Device) -> wgpu::Sampler {
        device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Shading clipmap sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            anisotropy_clamp: 1,
            ..Default::default()
        })
    }

    fn dummy_clipmap_array(
        device: &wgpu::Device,
        layers: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Shading clipmap dummy"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("Shading clipmap dummy view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        (texture, view)
    }

    fn make_clipmap_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        detail_albedo: &wgpu::TextureView,
        detail_surface: &wgpu::TextureView,
        macro_albedo: &wgpu::TextureView,
        macro_normal: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shading clipmap arrays"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(detail_albedo),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(detail_surface),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(macro_albedo),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(macro_normal),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    /// Bind shade group 2 to the clipmap 2D-array views. Generate paints the
    /// same images as color attachments.
    pub fn set_clipmap_arrays(
        &mut self,
        device: &wgpu::Device,
        detail_albedo: &wgpu::TextureView,
        detail_surface: &wgpu::TextureView,
        macro_albedo: &wgpu::TextureView,
        macro_normal: &wgpu::TextureView,
    ) {
        self.clipmap_bind_group = Self::make_clipmap_bind_group(
            device,
            &self.clipmap_layout,
            detail_albedo,
            detail_surface,
            macro_albedo,
            macro_normal,
            &self.clipmap_sampler,
        );
    }

    /// Rebuild the bind group after a resize.
    ///
    /// **Every resolution-dependent view has to be passed in again.** This used
    /// to take only the visibility view and rebuild the rest from the clones
    /// captured at construction — but GTAO, the depth buffer and the ReSTIR
    /// target all recreate their textures on resize, so those clones pointed at
    /// textures nothing wrote to any more.
    ///
    /// The result was silent and total: `gtao.a` read 0, which zeroes
    /// `surface.occlusion`, which zeroes both terms of `evaluate_ibl` — so
    /// after the first window resize *no surface in the scene received any
    /// indirect light*, contact shadows marched a dead depth buffer, and ReSTIR
    /// visibility read as "not run". The demo resizes three times during
    /// startup, so this was the state in every session.
    #[allow(clippy::too_many_arguments)]
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        visibility_view: &wgpu::TextureView,
        gtao_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        restir_view: &wgpu::TextureView,
        restir_gi_view: &wgpu::TextureView,
        lighting_aux_view: &wgpu::TextureView,
        world_volume_view: &wgpu::TextureView,
    ) {
        self.gtao_view = gtao_view.clone();
        self.depth_view = depth_view.clone();
        self.restir_view = restir_view.clone();
        self.restir_gi_view = restir_gi_view.clone();
        self.lighting_aux_view = lighting_aux_view.clone();
        self.world_volume_view = world_volume_view.clone();
        self.visibility_view = visibility_view.clone();
        self.rebind(device);
    }

    /// Read another target as the lighting aux (the path tracer's output while it runs, lighting_extra's otherwise),
    /// and write the finished surface to the path tracer's current targets (`PathTracerPass::surface_views`).
    pub fn set_lighting_aux(
        &mut self,
        device: &wgpu::Device,
        view: &wgpu::TextureView,
        path_surface: [&wgpu::TextureView; 3],
    ) {
        self.lighting_aux_view = view.clone();
        self.path_surface = path_surface.map(Clone::clone);
        self.rebind(device);
    }

    fn rebind(&mut self, device: &wgpu::Device) {
        self.bind_group = Self::make_bind_group(
            device,
            &self.bind_group_layout,
            &self.visibility_view,
            &self.sampler,
            &self.shadow_atlas_view,
            &self.shadow_sampler,
            &self.env_view,
            &self.env_sampler,
            &self.gtao_view,
            &self.depth_view,
            &self.restir_view,
            &self.restir_gi_view,
            &self.volumetric_view,
            &self.volumetric_sampler,
            &self.volumetric_range,
            &self.lighting_aux_view,
            &self.world_volume_view,
            &self.lighting_extra,
            &self.sh_probes,
            &self.cloud_shadow_view,
            &self.cloud_shadow_params,
            &self.weather,
            &self.decal_buffers,
            &self.virtual_shadow_view,
            &self.virtual_shadow_sampler,
            &self.virtual_shadow_page_table,
            &self.virtual_shadow_params,
            &self.grain_packed,
            &self.paint_buffer,
            self.local_shadow_tlas.as_ref(),
            [&self.path_surface[0], &self.path_surface[1], &self.path_surface[2]],
        );
    }

    /// The vertex-paint words (the post-process reads the instance table's `OUTSIDE_RETRO` bits).
    pub fn paint_buffer(&self) -> &wgpu::Buffer {
        &self.paint_buffer
    }

    fn make_paint_buffer(device: &wgpu::Device, words: u64) -> wgpu::Buffer {
        // Created zeroed, so word 0 already reads "nothing painted".
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Vertex Paint"),
            size: words * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Grow the vertex paint buffer to at least `words`. True when it was
    /// recreated, in which case everything must be uploaded again.
    pub fn ensure_paint_capacity(&mut self, device: &wgpu::Device, words: u64) -> bool {
        if words <= self.paint_capacity_words {
            return false;
        }
        self.paint_capacity_words = words.next_power_of_two();
        self.paint_buffer = Self::make_paint_buffer(device, self.paint_capacity_words);
        self.rebind(device);
        true
    }

    pub fn write_paint(&self, queue: &wgpu::Queue, word: u32, data: &[u32]) {
        if !data.is_empty() {
            queue.write_buffer(&self.paint_buffer, u64::from(word) * 4, bytemuck::cast_slice(data));
        }
    }

    /// Bind the renderer-owned sparse shadow cache into opaque/terrain shading.
    pub fn set_virtual_shadow_resources(
        &mut self,
        device: &wgpu::Device,
        visibility_view: &wgpu::TextureView,
        gpu: &crate::shadow::virtual_map::VirtualShadowGpu,
    ) {
        self.virtual_shadow_view = gpu.physical_atlas_view.clone();
        self.virtual_shadow_sampler = gpu.comparison_sampler.clone();
        self.virtual_shadow_page_table = gpu.page_table.clone();
        self.virtual_shadow_params = gpu.params.clone();
        self.visibility_view = visibility_view.clone();
        self.rebind(device);
    }

    /// Publish this frame's wetness. All zero leaves shading bit-identical to
    /// what it was before CONTROL-N, which is what "off by default" has to
    /// mean for a surface property.
    pub fn set_weather(
        &self,
        queue: &wgpu::Queue,
        wet_diffuse: f32,
        wet_specular: f32,
        puddles: f32,
    ) {
        queue.write_buffer(
            &self.weather,
            0,
            bytemuck::bytes_of(&[
                wet_diffuse.clamp(0.0, 1.0),
                wet_specular.clamp(0.0, 1.0),
                puddles.clamp(0.0, 1.0),
                0.0_f32,
            ]),
        );
    }

    /// Publish the volume's range for this frame. 0 disables the lookup.
    /// The volume's range uniform (x = metres, 0 when off), shared with water.
    pub fn volumetric_range_buffer(&self) -> &wgpu::Buffer {
        &self.volumetric_range
    }

    pub fn set_volumetric_range(&self, queue: &wgpu::Queue, range: f32) {
        queue.write_buffer(
            &self.volumetric_range,
            0,
            bytemuck::bytes_of(&[range, 0.0, 0.0, 0.0]),
        );
    }

    pub fn set_lighting_extra(&self, queue: &wgpu::Queue, params: [f32; 4]) {
        queue.write_buffer(&self.lighting_extra, 0, bytemuck::bytes_of(&params));
    }
}

#[cfg(test)]
mod tests {
    use super::ShadingSpec;

    #[test]
    fn compact_spec_zeros_the_expensive_overrides() {
        let c = ShadingSpec::COMPACT.constants();
        assert_eq!(c[0], ("enable_hex", 0.0));
        assert_eq!(c[1], ("enable_pom", 0.0));
        assert_eq!(c[2], ("enable_pcss", 0.0));
        assert_eq!(c[3], ("enable_contact", 0.0));
        assert_eq!(c[4], ("enable_clipmap", 0.0));
        assert_eq!(c[5], ("enable_debug", 0.0));
        assert_eq!(c[6], ("terrain_scan", 16.0));
        // The live path stays compiled by default. Deleting it is only safe
        // once the CPU has confirmed every terrain shades from its clipmap.
        assert_eq!(c[7], ("enable_live_terrain", 1.0));
    }

    #[test]
    fn a_clipmap_only_spec_drops_the_live_terrain_path() {
        let spec = ShadingSpec {
            clipmap: true,
            live_terrain: false,
            ..ShadingSpec::COMPACT
        };
        let c = spec.constants();
        assert_eq!(c[4], ("enable_clipmap", 1.0));
        assert_eq!(c[7], ("enable_live_terrain", 0.0));
    }
}
