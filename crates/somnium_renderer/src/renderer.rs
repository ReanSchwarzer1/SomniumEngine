//! The main entry point for the Somnium Renderer.
//!
//! Orchestrates the `GlobalResourcePool`, `MaterialSystem`, and rendering passes.
//!
//! Phase 11 additions:
//! - `ShadowMapResources` + `ShadowPass` for cascade shadow maps
//! - `GpuDirectionalLight` upload each frame
//! - View buffer expanded to 208 bytes (adds raw view matrix for cascade selection)
//! - `set_view` now takes separate `view` and `proj` matrices
//! - `set_directional_light` / `set_cascade_debug` public APIs

use crate::{
    bindless::GlobalResourcePool,
    command::DrawCommand,
    context::RenderContext,
    material::{hlms::MaterialSystem, pool::{GpuMaterial, MaterialPool}},
    texture_pool::TexturePool,
    pass::{
        gizmo::{GizmoMode, GizmoPass},
        grid::GridPass,
        outline::OutlinePass,
        particle::{GpuParticle, ParticlePass},
        postprocess::{PostProcessPass, HDR_FORMAT},
        shading::ShadingPass,
        shadow::ShadowPass,
        visibility::VisibilityBufferPass,
    },
    geometry::GeometryPool,
    instance::InstancePool,
    shadow::{
        ShadowMapResources, GpuDirectionalLight, ATLAS_SIZE,
        cascade::compute_cascades,
    },
};
use somnium_ui::UiManager;
use winit::window::Window;

/// Above this many copies of the same mesh in one frame, cluster expansion is
/// abandoned for that mesh and each draw becomes a single whole-mesh argument.
///
/// Chosen low because the trade flips fast: the argument count is
/// `instances x clusters`, so anything genuinely instanced blows past what
/// per-cluster culling can ever recover.
const MAX_INSTANCES_FOR_CLUSTERING: u32 = 8;

/// Everything the ECS layer needs to spawn one entity from an uploaded scene node.
#[derive(Debug, Clone)]
pub struct UploadedNode {
    pub entity_name:   String,
    pub vertex_offset: u32,
    pub index_offset:  u32,
    pub index_count:   u32,
    pub material_id:   u32,
    pub transform:     glam::Mat4,
}

/// The primary renderer struct.
pub struct SomniumRenderer {
    /// Global descriptor pool (bindless arrays, includes light buffer at binding 6).
    pub global_pool: GlobalResourcePool,
    /// High level material system cache.
    pub materials: MaterialSystem,
    /// The visibility buffer render pass.
    pub vis_pass: VisibilityBufferPass,
    /// The final shading pass.
    pub shading_pass: ShadingPass,
    /// Shadow atlas + comparison sampler (shared between shadow pass and shading pass).
    pub shadow_resources: ShadowMapResources,
    /// Depth-only shadow render pass (4 cascades).
    pub shadow_pass: ShadowPass,

    /// Global geometry storage.
    pub geometry: GeometryPool,
    /// Global material storage.
    pub materials_pool: MaterialPool,
    /// Global texture storage.
    pub texture_pool: TexturePool,
    /// Per-frame instance storage.
    pub instances: InstancePool,

    /// The current camera view matrix (world → view).
    pub view_matrix: glam::Mat4,
    /// The current projection matrix.
    pub proj_matrix: glam::Mat4,
    /// The current combined view-projection matrix.
    pub view_proj: glam::Mat4,
    /// The current camera position in world space.
    pub camera_pos: glam::Vec3,
    /// The elapsed engine time in seconds.
    pub time: f32,

    /// Directional light direction (toward the light, world space, normalized).
    pub light_direction: glam::Vec3,
    /// Scene-wide indirect-light strength, driven by `PostProcessComponent`.
    ibl_intensity: f32,
    /// Directional light color, pre-multiplied by intensity.
    pub light_color: glam::Vec3,

    /// When true, the shading pass tints pixels by cascade index (debug overlay).
    cascade_debug: bool,
    
    /// Phase 13D: 0 = PBR, 1 = Cel-shading.
    pub shading_mode: u32,
    /// Phase 13C: Accumulated local lights for the frame.
    local_lights: Vec<crate::cluster::GpuLocalLight>,

    /// Water textures bind group.
    pub water_textures_bind_group: Option<wgpu::BindGroup>,

    /// Editor infinite-grid overlay pass.
    grid_pass: GridPass,
    /// When true, the grid overlay is composited into the HDR target.
    grid_enabled: bool,

    /// Post-processing pass: owns the HDR render target + tone-maps to swapchain.
    postprocess_pass: PostProcessPass,
    atmosphere_pass: crate::pass::atmosphere::AtmospherePass,
    auto_exposure_pass: crate::pass::auto_exposure::AutoExposurePass,
    pub taa_pass: crate::pass::taa::TaaPass,
    pub gtao_pass: crate::pass::gtao::GtaoPass,
    pub bloom_pass: crate::pass::bloom::BloomPass,
    pub dof_pass: crate::pass::dof::DofPass,
    pub raytrace_pass: crate::pass::raytrace::RaytracePass,
    rt_debug_pass: crate::pass::raytrace::RtDebugPass,
    pub restir_pass: crate::pass::restir::RestirPass,
    /// Geometry registered for tracing: (vertex_offset, vertex_count, index_offset, index_count).
    rt_geometry: Vec<(u32, u32, u32, u32)>,
    /// Exposure multiplier applied before ACES tone mapping (default 1.0).
    pub exposure: f32,
    /// Meter the frame and adapt exposure to it, rather than using `exposure`.
    pub auto_exposure: bool,
    /// Seconds since the previous frame, for exposure adaptation.
    pub frame_delta_time: f32,
    /// Stops applied on top of the metered exposure. Negative darkens.
    pub exposure_compensation: f32,
    /// Colour grading and lens settings (Phases 24Y / 24Z).
    pub grading: crate::pass::postprocess::Grading,
    /// View-projection without TAA's sub-pixel jitter, for reprojection.
    view_proj_unjittered: glam::Mat4,
    /// Current render target size, for computing the jitter offset.
    render_width: u32,
    render_height: u32,
    /// Half the sun's angular diameter, radians (Phase 24E). Drives the
    /// specular highlight's size and the shadow penumbra's width.
    pub sun_angular_radius: f32,
    /// Tone-mapping curve index; matches `Tonemapper::as_index`.
    pub tonemapper: u32,
    /// Radial vignette strength (0 = off, 1 = default, higher = stronger).
    ///
    /// Defaults to **off**: an always-on vignette darkens the viewport edges,
    /// which reads as a dirty screen in an editor. Enable it per-scene through
    /// the Post Processing entity (Phase 15A1).
    pub vignette_strength: f32,
    /// Chromatic-aberration strength in UV units at the screen edge (0 = off).
    pub chromatic_aberration: f32,
    /// Phase 15A2: FXAA anti-aliasing pass (runs before editor overlays).
    fxaa_pass: crate::pass::fxaa::FxaaPass,
    /// Whether FXAA is applied. When off, post-processing writes straight to
    /// the swapchain and the pass costs nothing.
    pub fxaa_enabled: bool,

    /// Editor transform gizmo pass.
    gizmo_pass: GizmoPass,

    /// Phase 13E: light visualization pass (point/spot/directional bounds).
    light_gizmo_pass: crate::pass::light_gizmo::LightGizmoPass,
    /// Light gizmos submitted this frame.
    light_gizmo_queue: Vec<crate::pass::light_gizmo::LightGizmoDesc>,
    /// When true, submitted light gizmos are drawn (toggle with `L`).
    light_gizmos_enabled: bool,
    /// Which gizmo operation is active.
    pub gizmo_mode: GizmoMode,
    /// World-space position of the selected entity (None when nothing selected).
    pub gizmo_world_pos: Option<glam::Vec3>,

    /// Phase 11.5I: Selection outline pass.
    outline_pass: OutlinePass,
    /// Mesh data for the currently selected entity (vertex_offset, index_offset,
    /// index_count, model matrix). None when no entity with a mesh is selected.
    outline_entity: Option<(u32, u32, u32, glam::Mat4)>,

    /// Phase 11.5J: GPU particle billboard pass.
    particle_pass: ParticlePass,
    /// Particle instances to render this frame (uploaded in render()).
    pending_particles: Vec<GpuParticle>,

    /// Phase 13: Water pass.
    pub water_pass: crate::pass::water::WaterPass,
    water_queue: Vec<(glam::Mat4, crate::pass::water::WaterMaterialData, u32, u32, u32)>,

    /// Phase 25A-2: per-terrain splat/layer parameters read by `shading.wgsl`.
    terrain_materials: crate::material::pool::TerrainMaterialPool,
    /// Deterministic HDR frame readback for A/B measurement. Inert unless
    /// `SOMNIUM_CAPTURE` or `SOMNIUM_CAPTURE_COMPARE` is set.
    capture: crate::capture::FrameCapture,
    /// Material ids belonging to a terrain, so a capture can label its pixels.
    terrain_material_ids: std::collections::HashSet<u32>,
    /// All created terrains, indexed by terrain id (`TerrainComponent::terrain_id`).
    pub terrains: Vec<crate::terrain::TerrainData>,
    /// Terrain ids (+ model matrices) submitted for the current frame.
    terrain_queue: Vec<(u32, glam::Mat4)>,

    /// Phase 15A: indirect draw arguments for the visibility pass.
    indirect: crate::indirect::IndirectDrawBuffer,
    /// Whether the GPU-driven indirect path is currently active.
    /// When false the renderer falls back to one `draw()` per object.
    gpu_driven: bool,
    /// Whether the device supports it at all (gates the runtime toggle).
    supports_gpu_driven: bool,

    /// Phase 19: environment cubemap for image-based lighting.
    ibl_pass: crate::pass::ibl::IblPass,

    /// Phase 15E: Hi-Z depth pyramid, rebuilt from the visibility depth buffer
    /// each frame and consumed by the two-phase occlusion cull.
    pub hiz_pass: crate::pass::hiz::HiZPass,
    /// False until a pyramid has been built from real geometry. Occlusion
    /// culling is held off until then — see the note at the cull dispatch.
    hiz_ready: bool,
    /// `SOMNIUM_CULL_STATS=1` reads the indirect args back after each cull
    /// phase and logs how many draws survived. Off by default: the readback
    /// stalls the pipeline waiting on the GPU.
    cull_stats: bool,
    /// Staging copies of the indirect args after phase one and phase two.
    cull_stats_buffers: Option<[wgpu::Buffer; 2]>,
    /// `SOMNIUM_NO_OCCLUSION=1` keeps frustum culling but skips the Hi-Z half,
    /// so the two can be measured apart.
    occlusion_off: bool,

    /// Phase 15B: GPU frustum-culling compute pass.
    cull_pass: crate::pass::cull::CullPass,
    /// Per-draw local AABBs for culling, rebuilt each frame.
    cull_aabbs: Vec<crate::culling::GpuCullAabb>,
    /// Phase 15F: this frame's indirect arguments, one per cluster.
    cluster_args: Vec<crate::indirect::DrawIndirectArgs>,
    /// How many times each mesh appears in this frame's draw queue, used to
    /// decide whether cluster expansion is worth it (Phase 17G).
    instanced_counts: std::collections::HashMap<u32, u32>,
    /// When true, a draw is expanded into one indirect argument per cluster so
    /// culling works below whole-object granularity. `SOMNIUM_NO_MESHLETS=1`
    /// forces the whole-mesh path, for A/B measurement.
    pub meshlet_draws: bool,
    /// When false the cull shader keeps every draw (useful for A/B checks).
    pub culling_enabled: bool,

    /// Phase 21: forward pass for alpha-blended materials.
    transparent_pass: crate::pass::transparent::TransparentPass,
    /// Blended draws submitted this frame (routed automatically by material).
    transparent_queue: Vec<DrawCommand>,
    /// Per-material flag: true when the material is alpha-blended. Lets
    /// `submit` route draws without any call site needing to know.
    material_blend: Vec<bool>,
    /// Phase 17D: per-material double-sided flag, used to split the visibility
    /// draws between the back-face-culled and two-sided pipelines.
    material_double_sided: Vec<bool>,
    /// Number of leading indirect arguments that belong to single-sided
    /// materials. Everything after it is drawn with culling off.
    single_sided_args: usize,

    /// The list of draw commands submitted this frame.
    draw_queue: Vec<DrawCommand>,
}

impl SomniumRenderer {
    /// Initialize the renderer using the provided `RenderContext`.
    pub fn new(ctx: &RenderContext) -> Self {
        let geometry      = GeometryPool::new(&ctx.device);
        let materials_pool = MaterialPool::new(&ctx.device);
        let instances     = InstancePool::new(&ctx.device);

        // Phase 11D/13: View buffer expanded to 224 bytes to include raw `view` matrix and `time`.
        let view_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("View Buffer"),
            size: 224,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Phase 11A: Directional light buffer (320 bytes).
        let light_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("DirectionalLight Buffer"),
            size: 320,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Phase 25A-2: terrain's splat/layer parameters, indexed by
        // `Material::terrain_index` from the shared shading pass.
        let terrain_materials = crate::material::pool::TerrainMaterialPool::new(&ctx.device);

        let global_pool = GlobalResourcePool::new(
            &ctx.device,
            &geometry.vertex_buffer,
            &geometry.index_buffer,
            &instances.buffer,
            &view_buffer,
            &materials_pool.buffer,
            &light_buffer,
            &terrain_materials.buffer,
        );

        let materials = MaterialSystem::new();

        // Phase 11.5H: Editor infinite-grid overlay (renders to HDR target).
        let grid_pass = GridPass::new(&ctx.device, HDR_FORMAT, &global_pool.view_proj_buffer);

        // Phase 24A-3: built before the post-process pass, which binds its
        // result buffer.
        let mut auto_exposure_pass =
            crate::pass::auto_exposure::AutoExposurePass::new(&ctx.device);

        // Phase 24J: acceleration structures. Constructed even when the device
        // lacks ray query — the pass then does nothing, which keeps the call
        // sites free of feature checks.
        let raytrace_pass = crate::pass::raytrace::RaytracePass::new(
            &ctx.device,
            ctx.features.contains(crate::context::RAY_TRACING_FEATURES),
        );

        // Phase 24K: reservoir-based direct lighting, on top of 24J.
        let restir_pass = crate::pass::restir::RestirPass::new(
            &ctx.device,
            raytrace_pass.supported(),
            ctx.config.width,
            ctx.config.height,
        );

        let rt_debug_pass =
            crate::pass::raytrace::RtDebugPass::new(&ctx.device, raytrace_pass.layout());

        // Phase 24Z: depth of field, driven by the same aperture as exposure.
        let dof_pass = crate::pass::dof::DofPass::new(
            &ctx.device, HDR_FORMAT, ctx.config.width, ctx.config.height,
        );

        // Phase 24T: built before the post-process pass, which samples its result.
        let bloom_pass = crate::pass::bloom::BloomPass::new(
            &ctx.device, HDR_FORMAT, ctx.config.width, ctx.config.height,
        );

        // Phase 11.5K: Post-process pass owns the Rgba16Float HDR render target.
        let postprocess_pass = PostProcessPass::new(
            &ctx.device,
            ctx.config.format,
            ctx.config.width,
            ctx.config.height,
            auto_exposure_pass.exposure_buffer(),
            bloom_pass.result_view(),
        );
        auto_exposure_pass.resize(&ctx.device, &postprocess_pass.hdr_view);

        // Phase 24I: screen-space occlusion, consumed by the shading pass.
        let gtao_pass = crate::pass::gtao::GtaoPass::new(
            &ctx.device, ctx.config.width, ctx.config.height,
        );

        // Phase 24F: resolves the jittered HDR frames into a stable image.
        let mut taa_pass = crate::pass::taa::TaaPass::new(
            &ctx.device,
            HDR_FORMAT,
            ctx.config.width,
            ctx.config.height,
            auto_exposure_pass.exposure_buffer(),
        );


        // Phase 11.5B: Transform gizmo (renders to swapchain after tone-mapping).
        let gizmo_pass = GizmoPass::new(
            &ctx.device, ctx.config.format, &global_pool.view_proj_buffer,
        );

        // Phase 13E: light gizmos (drawn to the swapchain like the transform gizmo).
        let light_gizmo_pass = crate::pass::light_gizmo::LightGizmoPass::new(
            &ctx.device, ctx.config.format, &global_pool.view_proj_buffer,
        );

        // Phase 11.5J: GPU billboard particle pass.
        let particle_pass = ParticlePass::new(&ctx.device, ctx.config.format);

        // Phase 11.5I: Selection outline (stencil-based, renders to swapchain).
        let outline_pass = OutlinePass::new(
            &ctx.device,
            ctx.config.format,
            &geometry.vertex_buffer,
            &geometry.index_buffer,
            ctx.config.width,
            ctx.config.height,
        );

        // Phase 11B: Shadow atlas and comparison sampler.
        // Shading pass output format is now Rgba16Float (HDR target).
        let shadow_resources = ShadowMapResources::new(&ctx.device);

        // Phase 11B: Depth-only shadow pass (4 cascades into the atlas).
        let shadow_pass = ShadowPass::new(&ctx.device, &ctx.queue, &global_pool.layout);

        // Phase 19: build the environment cubemap before the shading pass, which
        // binds it. Contents are generated on the first frame (and whenever the
        // sun changes), not here.
        // Phase 24C: the atmosphere LUTs must exist before the IBL pass, which
        // binds them to ray-march the sky into the environment cubemap.
        let atmosphere_pass = crate::pass::atmosphere::AtmospherePass::new(&ctx.device);
        let ibl_pass = crate::pass::ibl::IblPass::new(&ctx.device, &atmosphere_pass);

        let vis_pass = VisibilityBufferPass::new(
            &ctx.device, ctx.config.width, ctx.config.height, &global_pool.layout,
        );
        let hiz_pass = crate::pass::hiz::HiZPass::new(
            &ctx.device, ctx.config.width, ctx.config.height, &vis_pass.depth_view,
        );
        let shading_pass = ShadingPass::new(
            &ctx.device,
            &global_pool.layout,
            HDR_FORMAT,   // shading writes to the Rgba16Float HDR texture
            &vis_pass.view,
            &shadow_resources.atlas_depth_view,
            &shadow_resources.comparison_sampler,
            &ibl_pass.cube_view,
            &ibl_pass.sampler,
            gtao_pass.output_view(),
            &vis_pass.depth_view,
            restir_pass
                .visibility_view()
                .expect("ReSTIR always allocates its visibility target"),
        );

        // Phase 21: forward pass for blended materials. Built here because it
        // needs the global bind group layout and the environment cubemap.
        let transparent_pass = crate::pass::transparent::TransparentPass::new(
            &ctx.device,
            HDR_FORMAT,
            &global_pool.layout,
            &ibl_pass.cube_view,
            &ibl_pass.sampler,
        );

        let texture_pool = TexturePool::new(&ctx.device);

        // Default sun direction (normalized (1,2,-1)) and white light at intensity 5.
        let default_dir   = glam::Vec3::new(1.0, 2.0, -1.0).normalize();
        let default_color = glam::Vec3::splat(5.0);

        let water_pass = crate::pass::water::WaterPass::new(&ctx.device, HDR_FORMAT);

        Self {
            global_pool,
            materials,
            vis_pass,
            shading_pass,
            shadow_resources,
            shadow_pass,
            geometry,
            materials_pool,
            texture_pool,
            instances,
            view_matrix: glam::Mat4::IDENTITY,
            proj_matrix: glam::Mat4::IDENTITY,
            view_proj:   glam::Mat4::IDENTITY,
            camera_pos:  glam::Vec3::ZERO,
            time:        0.0,
            light_direction: default_dir,
            ibl_intensity: 0.35,
            light_color: default_color,
            cascade_debug: false,
            shading_mode: 0,
            local_lights: Vec::new(),
            grid_pass,
            grid_enabled: false,
            postprocess_pass,
            atmosphere_pass,
            auto_exposure_pass,
            taa_pass,
            gtao_pass,
            bloom_pass,
            dof_pass,
            raytrace_pass,
            rt_debug_pass,
            restir_pass,
            rt_geometry: Vec::new(),
            // EV100 15 (direct sunlight): 1 / (1.2 * 2^15). The renderer cannot
            // call into somnium_core for this — core depends on the renderer,
            // not the other way round — so the value is inlined.
            exposure: 1.0 / (1.2 * 32768.0),
            auto_exposure: true,
            frame_delta_time: 1.0 / 60.0,
            exposure_compensation: 0.0,
            grading: crate::pass::postprocess::Grading::default(),
            sun_angular_radius: 0.004_654,
            view_proj_unjittered: glam::Mat4::IDENTITY,
            render_width: ctx.config.width,
            render_height: ctx.config.height,
            tonemapper: 0,
            vignette_strength: 0.0,
            chromatic_aberration: 0.0,
            fxaa_pass: crate::pass::fxaa::FxaaPass::new(
                &ctx.device, ctx.config.format, ctx.config.width, ctx.config.height,
            ),
            fxaa_enabled: true,
            gizmo_pass,
            gizmo_mode: GizmoMode::Translate,
            gizmo_world_pos: None,
            light_gizmo_pass,
            light_gizmo_queue: Vec::new(),
            light_gizmos_enabled: true,
            outline_pass,
            outline_entity: None,
            particle_pass,
            pending_particles: Vec::new(),
            indirect: crate::indirect::IndirectDrawBuffer::new(&ctx.device),
            transparent_pass,
            transparent_queue: Vec::new(),
            material_blend: Vec::new(),
            material_double_sided: Vec::new(),
            single_sided_args: 0,
            ibl_pass,
            hiz_pass,
            hiz_ready: false,
            cull_stats: std::env::var("SOMNIUM_CULL_STATS").is_ok_and(|v| v == "1"),
            cull_stats_buffers: None,
            occlusion_off: std::env::var("SOMNIUM_NO_OCCLUSION").is_ok_and(|v| v == "1"),
            cull_pass: crate::pass::cull::CullPass::new(&ctx.device),
            cull_aabbs: Vec::new(),
            cluster_args: Vec::new(),
            instanced_counts: std::collections::HashMap::new(),
            meshlet_draws: !std::env::var("SOMNIUM_NO_MESHLETS").is_ok_and(|v| v == "1"),
            culling_enabled: true,
            gpu_driven: ctx.supports_gpu_driven(),
            supports_gpu_driven: ctx.supports_gpu_driven(),
            water_pass,
            water_queue: Vec::new(),
            terrain_materials,
            capture: crate::capture::FrameCapture::from_env(),
            terrain_material_ids: std::collections::HashSet::new(),
            terrains: Vec::new(),
            terrain_queue: Vec::new(),
            draw_queue: Vec::new(),

            water_textures_bind_group: None,
        }
    }

    /// Add a texture to the global bindless pool.
    pub fn add_texture(&mut self, ctx: &RenderContext, view: wgpu::TextureView) -> u32 {
        let index = self.texture_pool.add_texture(view.clone());
        self.global_pool.texture_views[index as usize] = view;
        self.global_pool.update_textures(&ctx.device);
        index
    }

    /// Upload a `LoadedScene` to the GPU pools and return one `UploadedNode` per
    /// renderable node (mesh_index is Some). The caller can then spawn ECS entities
    /// using the returned data.
    pub fn upload_scene(
        &mut self,
        ctx: &RenderContext,
        scene: &somnium_asset::LoadedScene,
    ) -> Vec<UploadedNode> {
        // 1. Textures --------------------------------------------------------
        let texture_indices: Vec<Option<i32>> = scene.textures.iter().map(|tex| {
            // Full mip chain. Without it, minified textures alias badly — the
            // sampler asks for trilinear filtering but a single level leaves
            // nothing to filter between, so detailed materials shimmer at
            // distance and read as noise.
            let levels = build_mip_chain(&tex.data, tex.width, tex.height);
            let mip_level_count = levels.len() as u32;

            let wgpu_tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Scene Texture"),
                size: wgpu::Extent3d {
                    width: tex.width, height: tex.height, depth_or_array_layers: 1,
                },
                mip_level_count,
                sample_count:     1,
                dimension:        wgpu::TextureDimension::D2,
                format:           wgpu::TextureFormat::Rgba8UnormSrgb,
                usage:            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats:     &[],
            });

            for (level, (lw, lh, data)) in levels.iter().enumerate() {
                // write_texture requires rows padded to COPY_BYTES_PER_ROW_ALIGNMENT.
                let row_bytes  = lw * 4;
                let align      = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
                let padded_row = row_bytes.div_ceil(align) * align;

                let upload: std::borrow::Cow<[u8]> = if padded_row == row_bytes {
                    std::borrow::Cow::Borrowed(data)
                } else {
                    let mut buf = vec![0u8; padded_row as usize * *lh as usize];
                    for row in 0..*lh as usize {
                        let src = row * row_bytes as usize;
                        let dst = row * padded_row as usize;
                        buf[dst..dst + row_bytes as usize]
                            .copy_from_slice(&data[src..src + row_bytes as usize]);
                    }
                    std::borrow::Cow::Owned(buf)
                };

                ctx.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture:   &wgpu_tex,
                        mip_level: level as u32,
                        origin:    wgpu::Origin3d::ZERO,
                        aspect:    wgpu::TextureAspect::All,
                    },
                    &upload,
                    wgpu::TexelCopyBufferLayout {
                        offset:         0,
                        bytes_per_row:  Some(padded_row),
                        rows_per_image: Some(*lh),
                    },
                    wgpu::Extent3d { width: *lw, height: *lh, depth_or_array_layers: 1 },
                );
            }

            let view = wgpu_tex.create_view(&wgpu::TextureViewDescriptor::default());
            Some(self.add_texture(ctx, view) as i32)
        }).collect();

        // 2. Materials -------------------------------------------------------
        let resolve_tex = |opt: Option<usize>| -> i32 {
            opt.and_then(|i| texture_indices.get(i).and_then(|&t| t)).unwrap_or(-1)
        };

        let material_ids: Vec<u32> = scene.materials.iter().map(|mat| {
            let id = self.materials_pool.add_material(&ctx.queue, GpuMaterial {
                base_color:             mat.base_color,
                roughness:              mat.roughness,
                metallic:               mat.metallic,
                albedo_map:             resolve_tex(mat.albedo_map),
                normal_map:             resolve_tex(mat.normal_map),
                metallic_roughness_map: resolve_tex(mat.metallic_roughness_map),
                occlusion_map: resolve_tex(mat.occlusion_map),
                transmission: mat.transmission,
                emissive: mat.emissive,
                emissive_map: resolve_tex(mat.emissive_map),
                terrain_index: -1,
                _pad: [0.0; 2],
                // Phase 17D: only MASK cuts out. OPAQUE ignores alpha entirely
                // and BLEND goes to the forward pass, so a cutoff on either
                // would punch holes in geometry that should be solid.
                alpha_cutoff: crate::material::pool::cutout_threshold(
                    mat.alpha_mode,
                    mat.alpha_cutoff,
                ),
                flags: if mat.double_sided {
                    crate::material::pool::MATERIAL_FLAG_DOUBLE_SIDED
                } else {
                    0
                },
            });
            // Phase 17D: remember double-sidedness so the visibility pass can
            // draw those instances with back-face culling switched off.
            self.set_material_double_sided(id, mat.double_sided);
            // Phase 21: remember which materials are blended so `submit` can
            // route their draws to the forward transparent pass.
            self.set_material_blend(
                id,
                mat.alpha_mode == somnium_asset::AlphaMode::Blend,
            );
            id
        }).collect();

        // 3. Meshes ----------------------------------------------------------
        let mesh_allocs: Vec<crate::geometry::MeshAllocation> = scene.meshes.iter()
            .map(|mesh| {
                let alloc = self.geometry.upload_mesh(
                    &ctx.queue, &mesh.vertices, &mesh.indices, 0,
                );
                // Phase 24J: a BLAS describes geometry in object space, so it
                // is built once here and then referenced by however many
                // instances place it in the world.
                self.raytrace_pass.register_mesh(
                    &ctx.device,
                    alloc.vertex_offset,
                    mesh.vertices.len() as u32,
                    alloc.index_offset,
                    alloc.index_count,
                );
                self.rt_geometry.push((
                    alloc.vertex_offset,
                    mesh.vertices.len() as u32,
                    alloc.index_offset,
                    alloc.index_count,
                ));
                alloc
            })
            .collect();

        // 4. Build UploadedNode list ----------------------------------------
        scene.nodes.iter().filter_map(|node| {
            let mesh_idx = node.mesh_index?;
            let alloc    = mesh_allocs.get(mesh_idx)?;
            let mat_idx  = node.material_index.unwrap_or(0);
            let mat_id   = material_ids.get(mat_idx).copied().unwrap_or(0);
            Some(UploadedNode {
                entity_name:   node.name.clone(),
                vertex_offset: alloc.vertex_offset,
                index_offset:  alloc.index_offset,
                index_count:   alloc.index_count,
                material_id:   mat_id,
                transform:     node.transform,
            })
        }).collect()
    }

    /// Set the camera matrices for this frame.
    ///
    /// `view`  — world-to-camera (look_at matrix)
    /// `proj`  — camera projection (perspective / orthographic)
    /// `camera_pos` — camera world-space position
    pub fn set_view(&mut self, view: glam::Mat4, proj: glam::Mat4, camera_pos: glam::Vec3) {
        self.view_matrix = view;
        self.proj_matrix = proj;
        // Unjittered, for TAA's own reprojection: it has to compare like with
        // like across frames, and each frame's jitter is different.
        self.view_proj_unjittered = proj * view;

        // Phase 24F: nudge the projection by a sub-pixel offset so successive
        // frames sample the scene at slightly different positions. Applied to
        // the clip-space translation, which shifts the whole image without
        // touching the projection's shape.
        let jitter = self.taa_pass.jitter_ndc(self.render_width, self.render_height);
        let mut jittered = proj;
        jittered.z_axis.x += jitter.x;
        jittered.z_axis.y += jitter.y;

        self.view_proj  = jittered * view;
        self.camera_pos = camera_pos;
    }

    /// Scene-wide indirect-light strength (Phase 22C), uploaded with the sun.
    pub fn set_ibl_intensity(&mut self, intensity: f32) {
        self.ibl_intensity = intensity.max(0.0);
    }

    /// Colour the HDR target is cleared to, in cd/m² (Phase 24A).
    ///
    /// This used to be a flat 0.07 grey, which was fine when light was an
    /// arbitrary multiplier and became black the moment exposure turned
    /// physical — 0.07 cd/m² against a 100 000 lux scene is night.
    ///
    /// Anywhere no geometry is drawn is sky, so the clear takes the same
    /// horizon colour and sun-driven luminance scale that `ibl_gen.wgsl` gives
    /// the environment cubemap. The background then matches the light the scene
    /// is actually receiving, and darkens with the sun instead of staying put.
    ///
    /// Interim: Phase 24C draws the atmosphere properly and this goes away.
    fn background_color(&self) -> wgpu::Color {
        // Matches `horizon_color` in ibl_gen.wgsl.
        const HORIZON: glam::Vec3 = glam::Vec3::new(0.5, 0.7, 0.9);
        const SKY_LUMINANCE_PER_LUX: f32 = 0.08;

        let illuminance = self
            .light_color
            .dot(glam::Vec3::new(0.2126, 0.7152, 0.0722));
        let sky = HORIZON * (illuminance * SKY_LUMINANCE_PER_LUX);
        wgpu::Color {
            r: f64::from(sky.x),
            g: f64::from(sky.y),
            b: f64::from(sky.z),
            a: 1.0,
        }
    }

    /// Set the directional light parameters for this frame.
    pub fn set_directional_light(&mut self, direction: glam::Vec3, color: glam::Vec3) {
        self.light_direction = direction.normalize();
        self.light_color     = color;
    }

    /// Add a local light (Point or Spot) for this frame (Phase 13C).
    pub fn submit_local_light(&mut self, light: crate::cluster::GpuLocalLight) {
        if self.local_lights.len() < crate::cluster::MAX_LOCAL_LIGHTS {
            self.local_lights.push(light);
        }
    }

    /// Toggle the cascade debug overlay (tints pixels by cascade index).
    /// Press `C` in hello_engine to activate.
    pub fn set_cascade_debug(&mut self, enabled: bool) {
        self.cascade_debug = enabled;
    }

    /// Show or hide the editor infinite-grid overlay (Phase 11.5H).
    /// The grid is off by default; toggle with `G` in the editor.
    pub fn set_grid_enabled(&mut self, enabled: bool) {
        self.grid_enabled = enabled;
    }

    /// Toggle the editor grid overlay and return the new state.
    pub fn toggle_grid(&mut self) -> bool {
        self.grid_enabled = !self.grid_enabled;
        self.grid_enabled
    }

    /// Set the active gizmo mode (Translate / Rotate / Scale).
    pub fn set_gizmo_mode(&mut self, mode: GizmoMode) {
        self.gizmo_mode = mode;
    }

    /// Toggle the Phase 15 GPU-driven indirect draw path, returning the new
    /// state. Returns `false` (and does nothing) if the device lacks support.
    ///
    /// Both paths must produce an identical image, so this doubles as an A/B
    /// check: flip it and the scene should not change.
    pub fn toggle_gpu_driven(&mut self) -> bool {
        if !self.supports_gpu_driven {
            return false;
        }
        self.gpu_driven = !self.gpu_driven;
        self.gpu_driven
    }

    /// Toggle GPU frustum culling, returning the new state (Phase 15B).
    ///
    /// A correct cull is invisible, so this is the way to check it: flip it and
    /// nothing on screen should change. Anything that pops in or out was being
    /// culled wrongly.
    pub fn toggle_culling(&mut self) -> bool {
        self.culling_enabled = !self.culling_enabled;
        self.culling_enabled
    }

    /// Whether the GPU-driven indirect path is currently in use.
    pub fn gpu_driven(&self) -> bool {
        self.gpu_driven
    }

    /// Whether this device supports the GPU-driven path at all.
    pub fn supports_gpu_driven(&self) -> bool {
        self.supports_gpu_driven
    }

    /// Submit one light's gizmo for this frame (Phase 13E).
    ///
    /// Cleared every frame like the draw queue; the editor re-submits each
    /// light it wants visualized.
    pub fn submit_light_gizmo(&mut self, desc: crate::pass::light_gizmo::LightGizmoDesc) {
        self.light_gizmo_queue.push(desc);
    }

    /// Show or hide light gizmos (on by default).
    pub fn set_light_gizmos_enabled(&mut self, enabled: bool) {
        self.light_gizmos_enabled = enabled;
    }

    /// Toggle light gizmos and return the new state.
    pub fn toggle_light_gizmos(&mut self) -> bool {
        self.light_gizmos_enabled = !self.light_gizmos_enabled;
        self.light_gizmos_enabled
    }

    /// Whether light gizmos are currently drawn.
    pub fn light_gizmos_enabled(&self) -> bool {
        self.light_gizmos_enabled
    }

    /// Update the world-space position the gizmo should appear at.
    pub fn set_gizmo_world_pos(&mut self, pos: glam::Vec3) {
        self.gizmo_world_pos = Some(pos);
    }

    /// Hide the gizmo (e.g. when no entity is selected).
    pub fn clear_gizmo(&mut self) {
        self.gizmo_world_pos = None;
    }

    /// Set the selected entity's mesh data for outline rendering (Phase 11.5I).
    pub fn set_outline_entity(
        &mut self,
        vertex_offset: u32,
        index_offset:  u32,
        index_count:   u32,
        model:         glam::Mat4,
    ) {
        self.outline_entity = Some((vertex_offset, index_offset, index_count, model));
    }

    /// Clear the selection outline (call when nothing is selected or selection has no mesh).
    pub fn clear_outline(&mut self) {
        self.outline_entity = None;
    }

    /// Replace this frame's particle list (Phase 11.5J).
    ///
    /// Called once per frame from `app.rs` after CPU simulation; the renderer
    /// uploads the data and draws instanced billboards in `render()`.
    pub fn set_particles(&mut self, particles: Vec<GpuParticle>) {
        self.pending_particles = particles;
    }

    /// Resize all internal renderer targets.
    /// Copy the indirect args into the phase's staging buffer.
    fn snapshot_indirect(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        phase: usize,
    ) {
        let bytes = (self.indirect.len() * 16) as u64;
        let needs_alloc = self
            .cull_stats_buffers
            .as_ref()
            .is_none_or(|b| b[phase].size() < bytes);
        if needs_alloc {
            let make = || {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Cull Stats Readback"),
                    size: bytes.max(16),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                })
            };
            self.cull_stats_buffers = Some([make(), make()]);
        }
        let dst = &self.cull_stats_buffers.as_ref().unwrap()[phase];
        encoder.copy_buffer_to_buffer(&self.indirect.buffer, 0, dst, 0, bytes);
    }

    /// Map both snapshots and log how many draws each phase left alive.
    ///
    /// `instance_count` doubles as the cull verdict, so counting the non-zero
    /// entries is exactly the number of draws that phase submitted.
    fn report_cull_stats(&self, ctx: &RenderContext, draw_count: usize) {
        let Some(buffers) = &self.cull_stats_buffers else { return };
        let bytes = (draw_count * 16) as u64;
        let mut alive = [0usize; 2];
        // Draw counts alone cannot be compared between the whole-mesh and
        // cluster paths — a mesh and a 128-triangle cluster are not the same
        // unit of work. Summing the indices actually submitted is.
        let mut indices = [0u64; 2];

        for (phase, buf) in buffers.iter().enumerate() {
            let slice = buf.slice(0..bytes);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
            {
                let data = slice.get_mapped_range();
                // DrawIndirectArgs: vertex_count, instance_count, first_vertex,
                // first_instance — instance_count is the second u32.
                for a in data.chunks_exact(16) {
                    if u32::from_le_bytes([a[4], a[5], a[6], a[7]]) != 0 {
                        alive[phase] += 1;
                        indices[phase] +=
                            u32::from_le_bytes([a[0], a[1], a[2], a[3]]) as u64;
                    }
                }
            }
            buf.unmap();
        }

        tracing::info!(
            "CULLSTATS total={draw_count} phase1_drawn={} phase2_drawn={} culled={} tris_drawn={}",
            alive[0],
            alive[1],
            draw_count - alive[0] - alive[1],
            (indices[0] + indices[1]) / 3,
        );
    }

    /// Record one visibility pass.
    ///
    /// `clear` distinguishes the two occlusion phases: phase one clears the
    /// targets, phase two loads them so it adds to what phase one drew rather
    /// than starting over.
    fn record_visibility(&self, encoder: &mut wgpu::CommandEncoder, clear: bool) {
        let color_load = if clear {
            wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 })
        } else {
            wgpu::LoadOp::Load
        };
        let depth_load = if clear { wgpu::LoadOp::Clear(1.0) } else { wgpu::LoadOp::Load };

        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Visibility Pass"),
            multiview_mask: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.vis_pass.view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations { load: color_load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.vis_pass.depth_view,
                depth_ops: Some(wgpu::Operations { load: depth_load, store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        rpass.set_bind_group(0, &self.global_pool.bind_group, &[]);
        rpass.set_bind_group(1, &self.vis_pass.cutout_bind_group, &[]);

        if self.gpu_driven && !self.indirect.is_empty() {
            // Phase 15A: the whole scene in one call per cull mode. Culled draws
            // simply carry instance_count = 0 and cost nothing.
            let total = self.indirect.len();
            let split = self.single_sided_args.min(total);
            if split > 0 {
                rpass.set_pipeline(&self.vis_pass.pipeline);
                rpass.multi_draw_indirect(&self.indirect.buffer, 0, split as u32);
            }
            if total > split {
                rpass.set_pipeline(&self.vis_pass.pipeline_two_sided);
                rpass.multi_draw_indirect(
                    &self.indirect.buffer,
                    (split as u64) * crate::indirect::ARGS_SIZE,
                    (total - split) as u32,
                );
            }
        } else if clear {
            rpass.set_pipeline(&self.vis_pass.pipeline);
            // Fallback for devices without multi-draw indirect. There are no
            // indirect args to cull, so everything is drawn in phase one and
            // phase two has nothing to add.
            for (inst_id, cmd) in self.draw_queue.iter().enumerate() {
                rpass.draw(0..cmd.index_count, inst_id as u32..(inst_id as u32 + 1));
            }
        }
    }

    pub fn resize(&mut self, ctx: &RenderContext, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.vis_pass.resize(&ctx.device, width, height, &self.global_pool.layout);
            self.postprocess_pass.resize(&ctx.device, width, height);
            self.auto_exposure_pass
                .resize(&ctx.device, &self.postprocess_pass.hdr_view);
            self.render_width = width;
            self.render_height = height;
            self.gtao_pass.resize(&ctx.device, width, height);
            self.bloom_pass.resize(&ctx.device, width, height);
            self.dof_pass.resize(&ctx.device, width, height);
            self.rt_debug_pass.invalidate();
            self.restir_pass.resize(&ctx.device, width, height);
            // After every pass that owns a resolution-dependent texture, never
            // before: the shading bind group has to reference the views those
            // resizes just created, not the ones they replaced.
            self.shading_pass.resize(
                &ctx.device,
                &self.vis_pass.view,
                self.gtao_pass.output_view(),
                &self.vis_pass.depth_view,
                self.restir_pass
                    .visibility_view()
                    .expect("ReSTIR always allocates its visibility target"),
            );
            self.taa_pass.resize(&ctx.device, HDR_FORMAT, width, height);
            self.taa_pass.rebuild(
                &ctx.device,
                &self.postprocess_pass.hdr_view,
                &self.vis_pass.depth_view,
            );
            self.fxaa_pass.resize(&ctx.device, ctx.config.format, width, height);
            self.outline_pass.resize(&ctx.device, width, height);
            // Must follow vis_pass: the level-0 bind group references its depth view.
            self.hiz_pass.resize(&ctx.device, width, height, &self.vis_pass.depth_view);
            // The new texture is zero-filled, i.e. everything at the near
            // plane, so occlusion has to stand down until it is rebuilt.
            self.hiz_ready = false;
        }
    }

    /// Submit a draw command.
    ///
    /// Blended materials are routed to the forward transparent pass instead of
    /// the visibility buffer, which can only resolve one triangle per pixel.
    /// Callers do not need to know which is which.
    pub fn submit(&mut self, cmd: DrawCommand) {
        let blended = self
            .material_blend
            .get(cmd.material_id as usize)
            .copied()
            .unwrap_or(false);
        if blended {
            self.transparent_queue.push(cmd);
        } else {
            self.draw_queue.push(cmd);
        }
    }

    /// Record whether a material is alpha-blended, so `submit` can route it.
    /// Materials default to opaque when never registered.
    /// Record that `material_id` renders from both sides (Phase 17D).
    pub fn set_material_double_sided(&mut self, material_id: u32, double_sided: bool) {
        let idx = material_id as usize;
        if self.material_double_sided.len() <= idx {
            self.material_double_sided.resize(idx + 1, false);
        }
        self.material_double_sided[idx] = double_sided;
    }

    /// Whether `material_id` renders from both sides.
    fn is_double_sided(&self, material_id: u32) -> bool {
        self.material_double_sided
            .get(material_id as usize)
            .copied()
            .unwrap_or(false)
    }

    pub fn set_material_blend(&mut self, material_id: u32, blended: bool) {
        let idx = material_id as usize;
        if self.material_blend.len() <= idx {
            self.material_blend.resize(idx + 1, false);
        }
        self.material_blend[idx] = blended;
    }

    /// Submit a water rendering command.
    pub fn add_water(
        &mut self,
        transform: glam::Mat4,
        water: crate::pass::water::WaterMaterialData,
        vertex_offset: u32,
        index_offset: u32,
        index_count: u32,
    ) {
        self.water_queue.push((transform, water, vertex_offset, index_offset, index_count));
    }

    /// Create a new heightmap terrain (Phase 14) and return its terrain id.
    ///
    /// Phase 25A-2 does three things here that the terrain pass used to do for
    /// itself: reserve a rewritable vertex span per chunk in the global pool,
    /// publish the splatmap and the twelve layer maps into the bindless texture
    /// array, and register a `GpuMaterial` so chunk draws look like every other
    /// draw to the rest of the renderer.
    pub fn create_terrain(
        &mut self,
        ctx: &RenderContext,
        desc: crate::terrain::TerrainDescriptor,
    ) -> u32 {
        let mut terrain = crate::terrain::TerrainData::new(&ctx.device, &ctx.queue, desc);
        terrain.reserve_pool_spans(&mut self.geometry);

        // The layer maps are `texture_2d_array`s and the bindless array is
        // `texture_2d`, so each layer is published as its own single-layer view
        // of the same texture. No copy, no second upload.
        let layer_view = |tex: &wgpu::Texture, layer: u32, label: &str| {
            tex.create_view(&wgpu::TextureViewDescriptor {
                label: Some(label),
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            })
        };
        let mut ids = crate::terrain::TerrainTextureIds::default();
        ids.splat_map = self.add_texture(ctx, terrain.splatmap.view.clone()) as i32;
        for layer in 0..crate::terrain::textures::TERRAIN_LAYER_COUNT {
            let i = layer as usize;
            ids.albedo[i] = self.add_texture(
                ctx,
                layer_view(&terrain.layer_textures.albedo, layer, "Terrain Layer Albedo"),
            ) as i32;
            ids.normal[i] = self.add_texture(
                ctx,
                layer_view(&terrain.layer_textures.normal, layer, "Terrain Layer Normal"),
            ) as i32;
            ids.roughness[i] = self.add_texture(
                ctx,
                layer_view(&terrain.layer_textures.roughness, layer, "Terrain Layer Roughness"),
            ) as i32;
        }
        terrain.texture_ids = ids;
        terrain.terrain_index = self.terrain_materials.allocate().unwrap_or(0);

        // The `GpuMaterial` itself is nearly empty: `terrain_index` is what
        // sends the shading pass down the splat path, and everything it would
        // otherwise read lives in the terrain-material entry.
        terrain.material_id = self.materials_pool.add_material(
            &ctx.queue,
            GpuMaterial {
                base_color: [1.0, 1.0, 1.0, 1.0],
                roughness: 0.9,
                metallic: 0.0,
                albedo_map: -1,
                normal_map: -1,
                metallic_roughness_map: -1,
                alpha_cutoff: 0.0,
                flags: 0,
                occlusion_map: -1,
                transmission: 0.0,
                emissive: [0.0; 3],
                emissive_map: -1,
                terrain_index: terrain.terrain_index as i32,
                _pad: [0.0; 2],
            },
        );
        // Opaque and single-sided, which is what an unregistered material
        // already defaults to — recorded explicitly so terrain does not depend
        // on that default staying put.
        self.set_material_blend(terrain.material_id, false);
        self.set_material_double_sided(terrain.material_id, false);
        self.terrain_material_ids.insert(terrain.material_id);

        self.terrains.push(terrain);
        (self.terrains.len() - 1) as u32
    }

    /// Queue a terrain for rendering this frame with the given model matrix.
    pub fn submit_terrain(&mut self, terrain_id: u32, model: glam::Mat4) {
        if (terrain_id as usize) < self.terrains.len() {
            self.terrain_queue.push((terrain_id, model));
        }
    }

    /// Mutable access to a terrain (sculpting, painting, raycasts).
    pub fn terrain_mut(&mut self, terrain_id: u32) -> Option<&mut crate::terrain::TerrainData> {
        self.terrains.get_mut(terrain_id as usize)
    }

    /// Shared access to a terrain.
    pub fn terrain(&self, terrain_id: u32) -> Option<&crate::terrain::TerrainData> {
        self.terrains.get(terrain_id as usize)
    }

    /// Execute the rendering pipeline for the current frame.
    pub fn render(&mut self, ctx: &RenderContext, ui: &mut UiManager, window: &Window) {
        // ── Phase 13C: Clustered lighting assignment ───────────────────────
        self.global_pool.cluster_grid.assign_and_upload(
            &ctx.queue,
            &self.local_lights,
            self.view_matrix,
            self.proj_matrix,
            ctx.config.width,
            ctx.config.height,
            0.1,    // near
            1000.0, // far
            self.shading_mode,
        );
        self.local_lights.clear();
        // ── 0. Upload view buffer (208 bytes) ────────────────────────────────
        // Layout: view_proj(64) | inv_view_proj(64) | view(64) | camera_pos(12) | cascade_debug_flag(4)
        let inv_view_proj = self.view_proj.inverse();
        let debug_flag    = if self.cascade_debug { 1.0f32 } else { 0.0f32 };

        let mut view_data = Vec::with_capacity(224);
        view_data.extend_from_slice(bytemuck::bytes_of(&self.view_proj.to_cols_array()));
        view_data.extend_from_slice(bytemuck::bytes_of(&inv_view_proj.to_cols_array()));
        view_data.extend_from_slice(bytemuck::bytes_of(&self.view_matrix.to_cols_array()));
        view_data.extend_from_slice(bytemuck::bytes_of(&self.camera_pos.to_array()));
        view_data.extend_from_slice(bytemuck::bytes_of(&debug_flag));
        view_data.extend_from_slice(bytemuck::bytes_of(&self.time));
        view_data.extend_from_slice(bytemuck::bytes_of(&[0.0f32; 3])); // _pad1
        ctx.queue.write_buffer(&self.global_pool.view_proj_buffer, 0, &view_data);

        // ── 0.5 Phase 19: refresh the environment cubemap ────────────────────
        // No-ops unless the sun actually moved, so this is free in the common
        // case. The sky is captured from the same procedural function the
        // background uses, keeping reflections consistent with what is drawn.
        // Phase 24C: the scattering LUTs the sky march reads. Built once —
        // they depend on the atmosphere's composition, not on sun or camera.
        self.atmosphere_pass.ensure_built(&ctx.device, &ctx.queue);

        self.ibl_pass.generate_if_needed(
            &ctx.device,
            &ctx.queue,
            self.light_direction,
            self.light_color,
        );

        // ── 1. Compute cascades and upload light buffer ───────────────────────
        // Fitted from the UNJITTERED inverse.
        //
        // `inv_view_proj` above is the jittered one, which is right for
        // reconstructing world position from a jittered depth buffer but wrong
        // here: it makes the cascade frusta shift by the sub-pixel jitter every
        // frame, so every shadow-map texel lands somewhere slightly different in
        // world space and every shadow edge crawls. TAA cannot average that
        // away, because it is a real change in the scene rather than a sampling
        // difference — which is why the shimmer vanished when TAA was switched
        // off: `jitter_ndc` returns zero when TAA is disabled, so the cascades
        // stopped moving.
        let cascades = compute_cascades(
            self.light_direction,
            self.view_proj_unjittered.inverse(),
        );

        let gpu_light = GpuDirectionalLight {
            direction: self.light_direction.to_array(),
            _pad0: 0.0,
            color: self.light_color.to_array(),
            _pad1: 0.0,
            view_proj: std::array::from_fn(|i| cascades[i].view_proj.to_cols_array_2d()),
            cascade_splits: [
                cascades[0].split_depth,
                cascades[1].split_depth,
                cascades[2].split_depth,
                cascades[3].split_depth,
            ],
            shadow_map_size: ATLAS_SIZE as f32,
            ibl_intensity: self.ibl_intensity,
            sun_angular_radius: self.sun_angular_radius,
            // TEMP: shadow_factor debug visualisation.
            _pad2: std::env::var("SOMNIUM_SHADOW_DEBUG")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0),
        };
        ctx.queue.write_buffer(
            &self.global_pool.light_buffer,
            0,
            bytemuck::bytes_of(&gpu_light),
        );

        // ── 1.5 Terrain becomes ordinary draws (Phase 25A-2) ─────────────────
        //
        // Before the sort, because from here on terrain is indistinguishable
        // from any other geometry: it goes into the same instance buffer, the
        // same indirect arguments, the same GPU frustum and Hi-Z culling, the
        // same shadow pass — which is also the first time terrain casts a
        // shadow — and the same visibility buffer, where `shading.wgsl` picks
        // it up. There is no terrain pass left to run at the end of the frame.
        for &(id, model) in &self.terrain_queue {
            let terrain = &mut self.terrains[id as usize];
            terrain.model = model;
            let local_cam = model.inverse().transform_point3(self.camera_pos);
            terrain.select_lods(local_cam);
            terrain.rebuild_dirty_chunks(&ctx.queue, &mut self.geometry);
            terrain.ensure_index_blocks(&ctx.queue, &mut self.geometry);
            terrain.splatmap.upload_dirty(&ctx.queue);
            self.terrain_materials
                .write(&ctx.queue, terrain.terrain_index, &terrain.gpu_material());

            let material_id = terrain.material_id;
            for chunk in &terrain.chunks {
                if chunk.vertex_offset == crate::terrain::UNALLOCATED {
                    continue;
                }
                let Some((index_offset, index_count)) =
                    terrain.index_block(chunk.lod, chunk.edge_mask)
                else {
                    continue;
                };
                // Pushed straight onto the opaque queue rather than through
                // `submit`, which would need a second mutable borrow of self —
                // terrain's material is registered opaque, so the routing
                // `submit` does would be a no-op anyway.
                self.draw_queue.push(DrawCommand {
                    sort_key: crate::command::SortKey::new(
                        0,
                        material_id as u16,
                        chunk.vertex_offset,
                    ),
                    vertex_offset: chunk.vertex_offset,
                    index_offset,
                    index_count,
                    material_id,
                    transform: model,
                });
            }
        }

        // ── 2. Sort draw queue ───────────────────────────────────────────────
        // This has to happen before the instance buffer is built. Instance `i`
        // is what draw `i` pulls its model matrix and geometry offsets from, so
        // reordering the queue afterwards pairs every draw with a different
        // mesh's offsets — which renders as triangles stretched between
        // unrelated parts of the geometry pool.
        self.draw_queue.sort_by_key(|cmd| cmd.sort_key);

        // ── 3. Build and upload instance buffer ──────────────────────────────
        self.instances.clear();
        for cmd in &self.draw_queue {
            self.instances.add_instance(crate::instance::GpuInstanceData {
                model_matrix:       cmd.transform.to_cols_array_2d(),
                material_id:        cmd.material_id,
                mesh_vertex_offset: cmd.vertex_offset,
                mesh_index_offset:  cmd.index_offset,
                _padding:           0,
            });
        }
        // Phase 21: blended draws share the same instance buffer, appended
        // after the opaque ones. The visibility pass only draws the opaque
        // range; the transparent pass indexes into the tail.
        let transparent_base = self.draw_queue.len() as u32;
        let mut transparent_draws: Vec<crate::pass::transparent::TransparentDraw> =
            Vec::with_capacity(self.transparent_queue.len());
        for (i, cmd) in self.transparent_queue.iter().enumerate() {
            self.instances.add_instance(crate::instance::GpuInstanceData {
                model_matrix:       cmd.transform.to_cols_array_2d(),
                material_id:        cmd.material_id,
                mesh_vertex_offset: cmd.vertex_offset,
                mesh_index_offset:  cmd.index_offset,
                _padding:           0,
            });
            let origin = cmd.transform.w_axis.truncate();
            transparent_draws.push(crate::pass::transparent::TransparentDraw {
                instance_index: transparent_base + i as u32,
                index_count: cmd.index_count,
                depth_sq: (origin - self.camera_pos).length_squared(),
            });
        }
        crate::pass::transparent::sort_back_to_front(&mut transparent_draws);
        self.instances.upload(&ctx.queue);

        // ── 3.5 Phase 15A: build this frame's indirect draw arguments ────────
        // Argument `i` lines up with instance `i`, which the sort above keeps true.
        if self.gpu_driven {
            // Phase 15F: each draw expands into one argument per cluster, with
            // a parallel array of bounds for the cull shader. A mesh with no
            // clusters (voxel chunks) or with no recorded AABB falls back to a
            // single whole-mesh argument that is never culled — safer than
            // guessing at its extent.
            // Phase 17D: single-sided draws first, then double-sided, with the
            // boundary recorded. The visibility pass issues one indirect call
            // per range so each gets the right cull mode. Argument order does
            // not have to match the draw queue — `first_instance` carries the
            // instance explicitly, and the cull shader reads it from there.
            // Phase 17G: clustering pays for a big mesh drawn once — parts of
            // it are meaningfully cullable. It is backwards for a small mesh
            // drawn thousands of times: a 6 400-triangle grass tuft expands to
            // 51 arguments, so a painted field costs 100 000 draws a frame to
            // cull sub-parts of things a few pixels across. Count how often
            // each mesh appears and fall back to one whole-mesh argument once
            // it is clearly being instanced.
            self.instanced_counts.clear();
            for cmd in &self.draw_queue {
                *self.instanced_counts.entry(cmd.vertex_offset).or_insert(0u32) += 1;
            }

            self.cluster_args.clear();
            self.cull_aabbs.clear();
            for pass_two_sided in [false, true] {
                if pass_two_sided {
                    self.single_sided_args = self.cluster_args.len();
                }
                for (i, cmd) in self.draw_queue.iter().enumerate() {
                    if self.is_double_sided(cmd.material_id) != pass_two_sided {
                        continue;
                    }
                    let heavily_instanced = self
                        .instanced_counts
                        .get(&cmd.vertex_offset)
                        .is_some_and(|n| *n > MAX_INSTANCES_FOR_CLUSTERING);
                    let meshlets = if self.meshlet_draws && !heavily_instanced {
                        self.geometry.mesh_meshlets(cmd.vertex_offset)
                    } else {
                        None
                    };
                    crate::indirect::push_cluster_args(
                        i as u32,
                        cmd.index_count,
                        meshlets,
                        self.geometry.mesh_aabb(cmd.vertex_offset),
                        &mut self.cluster_args,
                        &mut self.cull_aabbs,
                    );
                }
            }
            self.indirect.upload(&ctx.device, &ctx.queue, &self.cluster_args);
            self.cull_pass.update(
                &ctx.device,
                &ctx.queue,
                &self.cull_aabbs,
                // Un-jittered: a visibility decision must not depend on a
                // sub-pixel sampling offset. With the jittered matrix the
                // frustum planes — and the Hi-Z occlusion test behind them —
                // moved every frame, so any cluster sitting on the threshold
                // was culled on some frames and drawn on others. That is
                // geometry appearing and disappearing at jitter frequency,
                // which reads as the mesh vibrating, and it hits foliage
                // hardest because it is thousands of small clusters all near
                // the threshold at once.
                self.view_proj_unjittered,
                !self.culling_enabled,
                self.hiz_pass.size(),
                self.hiz_pass.mip_count(),
                self.hiz_ready && !self.occlusion_off,
                self.camera_pos,
            );
        }

        // ── 4. Acquire swapchain texture ─────────────────────────────────────
        let output = match ctx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(tex)    => tex,
            wgpu::CurrentSurfaceTexture::Suboptimal(tex) => tex,
            _ => {
                tracing::warn!("Failed to acquire surface texture");
                // The per-frame queues still have to be emptied. Returning
                // without clearing leaves this frame's submissions in place and
                // the next frame appends to them, so everything is drawn twice
                // — invisible for opaque geometry, but it double-blends the
                // transparent pass and wastes a whole frame of work.
                self.clear_frame_queues();
                return;
            }
        };
        let surface_view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Main Render Encoder"),
        });

        // ── 5. Shadow Pass (4 cascades into the atlas) ───────────────────────
        self.shadow_pass.record(
            &mut encoder,
            &self.shadow_resources.atlas_view,
            &self.global_pool.bind_group,
            &self.draw_queue,
        );

        // ── 5.5 Phase 15B/15E2: two-phase GPU instance culling ───────────────
        //
        //   cull phase 1   frustum, then occlusion against LAST frame's pyramid
        //   visibility     draw the survivors (clears the targets)
        //   Hi-Z build     pyramid now reflects what is actually on screen
        //   cull phase 2   re-test only what phase 1 rejected on occlusion
        //   visibility     draw whatever became visible (loads, never clears)
        //   Hi-Z build     final pyramid, which next frame's phase 1 reads
        //
        // Reprojecting the previous frame alone would drop geometry the moment
        // the camera moves. The second phase is what makes that safe: anything
        // wrongly rejected gets a look at fresh depth within the same frame.
        let cull_active = self.gpu_driven && !self.indirect.is_empty();

        if cull_active {
            self.cull_pass.record(
                &ctx.device,
                &mut encoder,
                &self.instances.buffer,
                &self.indirect.buffer,
                &self.hiz_pass.view,
                0,
                self.indirect.len(),
            );
        }

        if self.cull_stats && cull_active {
            self.snapshot_indirect(&ctx.device, &mut encoder, 0);
        }

        // ── 6. Visibility Pass (phase 1) ─────────────────────────────────────
        self.record_visibility(&mut encoder, true);

        // ── 6.5 Hi-Z pyramid from phase 1 depth ──────────────────────────────
        self.hiz_pass.record(&mut encoder);

        if cull_active {
            // ── 6.7 Cull phase 2 ─────────────────────────────────────────────
            self.cull_pass.record(
                &ctx.device,
                &mut encoder,
                &self.instances.buffer,
                &self.indirect.buffer,
                &self.hiz_pass.view,
                1,
                self.indirect.len(),
            );

            if self.cull_stats {
                self.snapshot_indirect(&ctx.device, &mut encoder, 1);
            }

            // ── 6.8 Visibility Pass (phase 2) — disocclusions ────────────────
            self.record_visibility(&mut encoder, false);

            // ── 6.9 Final pyramid, for the next frame's phase 1 ──────────────
            self.hiz_pass.record(&mut encoder);
        }

        // Occlusion culling stays off until a pyramid has been built from real
        // geometry. wgpu zero-fills a new texture, and zero is the near plane,
        // which would read as "everything is occluded" on the first frame.
        self.hiz_ready = true;

        // The Phase 25A-1 terrain depth prepass stood here. It is gone with
        // 25A-2: terrain is drawn by the visibility pass above, which fills the
        // same depth buffer before the acceleration-structure build, ReSTIR and
        // GTAO read it. Keeping the prepass as well would draw every chunk a
        // second time, from whatever LOD state the previous frame left behind.

        // ── 6.5 Acceleration structures (Phase 24J) ──────────────────────────
        // Rebuilt each frame from the same draw queue the raster path uses, so
        // the traced scene and the drawn one cannot drift apart.
        if self.raytrace_pass.supported() {
            let instances: Vec<(u32, glam::Mat4)> = self
                .draw_queue
                .iter()
                .map(|cmd| (cmd.vertex_offset, cmd.transform))
                .collect();
            let geometry = std::mem::take(&mut self.rt_geometry);
            self.raytrace_pass.build(
                &ctx.device,
                &mut encoder,
                &self.geometry.vertex_buffer,
                &self.geometry.index_buffer,
                &geometry,
                &instances,
            );
            self.rt_geometry = geometry;

            // Phase 24K: traced direct lighting. Here because it needs the TLAS
            // built above and the depth the visibility pass filled, and because
            // shading below consumes its result.
            if let Some(tlas) = self.raytrace_pass.tlas() {
                self.restir_pass.record(
                    &ctx.device,
                    &ctx.queue,
                    &mut encoder,
                    tlas,
                    &self.vis_pass.depth_view,
                    self.view_proj,
                    self.light_direction,
                    self.sun_angular_radius,
                    ctx.config.width,
                    ctx.config.height,
                );
            }
        }

        // Outside the ray-tracing guards on purpose: a pass that stopped running
        // still owns a stale target, and that is exactly the case that needs
        // clearing.
        self.restir_pass.clear_if_inactive(&mut encoder);

        // ── 6.9 GTAO (Phase 24I) ─────────────────────────────────────────────
        // After the visibility pass has filled depth, before shading reads it.
        self.gtao_pass
            .ensure_bind_groups(&ctx.device, &self.vis_pass.depth_view);
        self.gtao_pass.record(
            &mut encoder,
            &ctx.queue,
            self.proj_matrix,
            ctx.config.width,
            ctx.config.height,
            0.1,
        );

        // ── 7. Shading Pass → HDR texture ────────────────────────────────────
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shading Pass"),
                multiview_mask: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.postprocess_pass.hdr_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.background_color()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            rpass.set_pipeline(&self.shading_pass.pipeline);
            rpass.set_bind_group(0, &self.global_pool.bind_group, &[]);
            rpass.set_bind_group(1, &self.shading_pass.bind_group, &[]);
            rpass.draw(0..3, 0..1);
        }

        // ── 7.35 Frame capture, if one was asked for ─────────────────────────
        //
        // Here rather than at the end of the frame: this is the last point at
        // which the HDR target holds exactly the shading pass's own output,
        // before water, transparents, the editor grid, TAA and tone mapping.
        // An A/B of the shading path should not have to see through any of
        // those.
        let capture_now = self.capture.tick();
        if capture_now {
            // The switches an A/B is meant to be varying, recorded with the
            // capture so a null result can be told from a switch that never
            // reached the pass — which is what the first run of this test hit.
            tracing::info!(
                "CAPTURE-STATE gtao={} gtao_intensity={} ibl_intensity={} taa={} restir={}",
                self.gtao_pass.enabled,
                self.gtao_pass.intensity,
                self.ibl_intensity,
                self.taa_pass.enabled(),
                self.restir_pass.enabled,
            );
            self.capture.record(
                &ctx.device,
                &mut encoder,
                &self.postprocess_pass.hdr_texture,
                &self.vis_pass.texture,
                ctx.config.width,
                ctx.config.height,
            );
        }

        // The terrain pass stood here (7.3). Terrain now shades in the pass
        // above, with GTAO, contact shadows, traced visibility, IBL and aerial
        // perspective reaching it for the first time — and with one copy of
        // `sample_shadow` and the cluster lookup instead of two. It still
        // writes depth before the water pass, because the visibility pass does.

        // ── 7.5 Water Pass → HDR texture ─────────────────────────────────────
        if !self.water_queue.is_empty() {
            // Phase 22: water refracts what is behind it, so it needs the scene
            // colour as a texture. A pass cannot sample its own render target,
            // hence the copy — taken here so it holds opaque geometry, terrain
            // and transparents, i.e. everything that can legitimately be seen
            // through the surface.
            encoder.copy_texture_to_texture(
                self.postprocess_pass.hdr_texture.as_image_copy(),
                self.postprocess_pass.scene_copy_texture.as_image_copy(),
                self.postprocess_pass.hdr_texture.size(),
            );

            self.water_pass.record(
                &ctx.device,
                &ctx.queue,
                &mut encoder,
                &self.postprocess_pass.hdr_view,
                &self.vis_pass.depth_view,
                &self.global_pool.view_proj_buffer,
                &self.vis_pass.depth_view,
                &self.global_pool.light_buffer,
                &self.shadow_resources.atlas_depth_view,
                &self.shadow_resources.comparison_sampler,
                &self.ibl_pass.cube_view,
                &self.ibl_pass.sampler,
                &self.postprocess_pass.scene_copy_view,
                &self.geometry.vertex_buffer,
                &self.geometry.index_buffer,
                self.water_textures_bind_group.as_ref(),
                &self.water_queue,
            );
        }

        // ── 7.6 Phase 21: blended geometry → HDR texture ─────────────────────
        // After opaque shading, terrain and water have filled the target, so
        // blended surfaces composite over a complete image. Depth-tested
        // against the opaque depth, never writing it.
        self.transparent_pass.record(
            &mut encoder,
            &self.postprocess_pass.hdr_view,
            &self.vis_pass.depth_view,
            &self.global_pool.bind_group,
            &transparent_draws,
        );

        // ── 7.7 Grid Overlay → HDR texture ───────────────────────────────────
        if self.grid_enabled {
            self.grid_pass.record(&mut encoder, &self.postprocess_pass.hdr_view);
        }

        // ── 7.75 Ray-traced shadow debug (Phase 24J) ─────────────────────────
        // After shading, not before it. The shading pass clears the HDR target,
        // so a debug view written earlier is simply overwritten — which is
        // exactly what happened on the first attempt, and looks identical to
        // the acceleration structures silently not working.
        if self.raytrace_pass.supported() {
            if let Some(tlas) = self.raytrace_pass.tlas() {
                self.rt_debug_pass.record(
                    &ctx.device,
                    &ctx.queue,
                    &mut encoder,
                    tlas,
                    &self.vis_pass.depth_view,
                    &self.postprocess_pass.hdr_view,
                    self.view_proj,
                    self.light_direction,
                    ctx.config.width,
                    ctx.config.height,
                );
            }
        }

        // ── 7.8 TAA resolve (Phase 24F) ──────────────────────────────────────
        // Between the last thing that writes HDR and the metering, so exposure
        // is measured on the resolved image rather than on a jittered one.
        self.taa_pass.ensure_bind_groups(
            &ctx.device,
            &self.postprocess_pass.hdr_view,
            &self.vis_pass.depth_view,
        );
        // Both matrices: the jittered one matches the depth buffer this frame,
        // the unjittered one matches the resolved history. See `TaaPass::record`.
        if self.taa_pass.record(
            &mut encoder,
            &ctx.queue,
            self.view_proj,
            self.view_proj_unjittered,
            ctx.config.width,
            ctx.config.height,
        )
        .is_some()
        {
            // Copy back into the HDR target so every later pass keeps reading
            // one target, rather than a view that alternates each frame.
            let written = self.taa_pass.last_written();
            encoder.copy_texture_to_texture(
                self.taa_pass.resolved_texture(written).as_image_copy(),
                self.postprocess_pass.hdr_texture.as_image_copy(),
                self.postprocess_pass.hdr_texture.size(),
            );
        }

        // ── 7.9 Auto-exposure: meter the finished HDR frame ──────────────────
        // Runs after everything that writes HDR and before tone mapping, so it
        // meters exactly the image being exposed. The reading lands one frame
        // late by construction — that is what adaptation is.
        if self.auto_exposure {
            self.auto_exposure_pass.record(
                &mut encoder,
                &ctx.queue,
                ctx.config.width,
                ctx.config.height,
                self.frame_delta_time,
                self.exposure_compensation,
            );
        }

        // ── 7.93 Depth of field (Phase 24Z) ──────────────────────────────────
        // Before bloom, so the blurred highlights bloom rather than the sharp
        // ones — that ordering is what produces the soft circular flare a real
        // lens gives an out-of-focus light.
        self.dof_pass.ensure_bind_group(
            &ctx.device,
            &self.postprocess_pass.hdr_view,
            &self.vis_pass.depth_view,
        );
        if let Some(result) = self.dof_pass.record(
            &mut encoder,
            &ctx.queue,
            ctx.config.width,
            ctx.config.height,
            0.1,
            1000.0,
        ) {
            encoder.copy_texture_to_texture(
                result.as_image_copy(),
                self.postprocess_pass.hdr_texture.as_image_copy(),
                self.postprocess_pass.hdr_texture.size(),
            );
        }

        // ── 7.95 Bloom (Phase 24T) ───────────────────────────────────────────
        // After TAA, so the chain is built from a resolved image rather than a
        // jittered one; a blur of unstable input broadcasts that instability
        // across everything it touches.
        self.bloom_pass.record(
            &ctx.device,
            &ctx.queue,
            &mut encoder,
            &self.postprocess_pass.hdr_view,
            (ctx.config.width, ctx.config.height),
        );

        // ── 8. Post-process Pass: HDR → swapchain (tone map + vignette) ──────
        // A TAA debug view must reach the screen unmodified: exposure would
        // crush a 0/1 flag image to black, and a tone curve would grade the
        // very values being inspected.
        let debugging = self.taa_pass.debugging();
        self.postprocess_pass.set_params(
            &ctx.queue,
            if debugging { 1.0 } else { self.exposure },
            if debugging { 0.0 } else { self.vignette_strength },
            if debugging { 0.0 } else { self.chromatic_aberration },
            if debugging { 3 } else { self.tonemapper },
            self.auto_exposure && !debugging,
            if debugging { 0.0 } else { self.bloom_pass.intensity() },
            // Grading is a look, and a debug view is not one — it must reach
            // the screen as the numbers it holds.
            if debugging {
                crate::pass::postprocess::Grading::default()
            } else {
                self.grading
            },
        );
        // Phase 15A2: with FXAA on, tone-map into the LDR intermediate and let
        // FXAA resolve it to the swapchain. Editor overlays draw afterwards, so
        // gizmos and UI text stay pixel-sharp.
        // Phase 24F: TAA supersedes FXAA. Running both means edges get
        // processed twice — TAA resolves them from accumulated samples, then
        // FXAA blurs along whatever gradients remain, dragging dark pixels into
        // bright neighbours. That contributed to the rim around silhouettes,
        // and there is nothing left for FXAA to usefully do once TAA is on.
        let fxaa_active = self.fxaa_enabled && !self.taa_pass.enabled();
        if fxaa_active {
            self.fxaa_pass.update(&ctx.queue, ctx.config.width, ctx.config.height);
            self.postprocess_pass.record(&mut encoder, &self.fxaa_pass.ldr_view);
            self.fxaa_pass.record(&mut encoder, &surface_view);
        } else {
            self.postprocess_pass.record(&mut encoder, &surface_view);
        }

        // ── 8.5 Gizmo Pass → swapchain (after tone-mapping, before UI) ───────
        if let Some(gizmo_pos) = self.gizmo_world_pos {
            let dist = (self.camera_pos - gizmo_pos).length().max(0.5);
            let scale = dist * 0.15;
            let model = glam::Mat4::from_translation(gizmo_pos)
                * glam::Mat4::from_scale(glam::Vec3::splat(scale));
            self.gizmo_pass.update_transform(&ctx.queue, model);
            self.gizmo_pass.record(&mut encoder, &surface_view, self.gizmo_mode);
        }

        // ── 8.7 Selection outline → swapchain (Phase 11.5I) ─────────────────
        if let Some((v_off, i_off, i_cnt, model)) = self.outline_entity {
            self.outline_pass.record(
                &ctx.queue,
                &mut encoder,
                &surface_view,
                self.view_proj,
                model,
                v_off,
                i_off,
                i_cnt,
                [0.98, 0.58, 0.07, 1.0],  // orange highlight (#FA9412)
                0.007,                      // ~2-3 px at typical camera distance
            );
        }

        // ── 8.75 Light gizmos → swapchain (Phase 13E) ────────────────────────
        if self.light_gizmos_enabled && !self.light_gizmo_queue.is_empty() {
            let lines =
                crate::pass::light_gizmo::build_light_gizmo_lines(&self.light_gizmo_queue);
            self.light_gizmo_pass.record(
                &ctx.device,
                &ctx.queue,
                &mut encoder,
                &surface_view,
                &lines,
            );
        }

        // ── 8.8 Particle Pass → swapchain (Phase 11.5J) ──────────────────────
        if !self.pending_particles.is_empty() {
            self.particle_pass.record(
                &ctx.queue,
                &mut encoder,
                &surface_view,
                self.view_proj,
                self.view_matrix,
                &self.pending_particles,
            );
        }

        // ── 9. UI Overlay ────────────────────────────────────────────────────
        ui.end_frame(window, &ctx.device, &ctx.queue, &mut encoder, &surface_view);

        let stats_draws = if self.cull_stats { self.indirect.len() } else { 0 };
        ctx.queue.submit(std::iter::once(encoder.finish()));
        if stats_draws > 0 {
            self.report_cull_stats(ctx, stats_draws);
        }
        if capture_now {
            // After submit, so the copies recorded above have actually run.
            // Resolved before the draw queue is cleared, because labelling a
            // pixel as terrain means looking its instance back up in it.
            let terrain_ids = &self.terrain_material_ids;
            let draws = &self.draw_queue;
            self.capture.resolve(&ctx.device, |instance| {
                draws
                    .get(instance as usize)
                    .is_some_and(|d| terrain_ids.contains(&d.material_id))
            });
        }
        output.present();

        self.clear_frame_queues();
    }

    /// Empty every per-frame submission queue.
    ///
    /// Must run on *every* path out of `render`, including the ones that bail
    /// before drawing.
    fn clear_frame_queues(&mut self) {
        self.draw_queue.clear();
        self.water_queue.clear();
        self.terrain_queue.clear();
        self.transparent_queue.clear();
        self.light_gizmo_queue.clear();
    }
}

/// Build a full mip chain by repeated 2×2 box filtering.
///
/// Returns `(width, height, rgba8)` per level, starting with the original.
/// Done on the CPU because the imported data is already in system memory and
/// this runs once per texture at import; a GPU blit chain would be faster but
/// needs a render pass per level.
///
/// Colour is averaged **weighted by alpha**. A plain box filter is wrong for
/// cutout atlases: foliage diffuse maps carry blade colour only where the mask
/// is opaque and leave the rest black, so an unweighted average drags that
/// black into the blades and grass turns darker — and bluer, once ambient sky
/// is the brightest thing left — the further away it gets. Weighting by alpha
/// makes the transparent background contribute nothing, which is what the
/// artist's authored colour means. For a fully opaque texture every weight is
/// equal and this reduces exactly to the plain box filter.
///
/// Alpha itself is then rescaled per level to preserve *coverage* — the
/// fraction of texels that survive alpha testing. Averaging a binary mask
/// pushes alpha toward its mean, so a fixed 0.5 cutoff eats thin geometry as
/// it recedes and foliage visibly thins out with distance.
fn build_mip_chain(data: &[u8], width: u32, height: u32) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels = vec![(width, height, data.to_vec())];
    let (mut w, mut h) = (width, height);

    while w > 1 || h > 1 {
        let (pw, ph, prev) = levels.last().unwrap();
        let (pw, ph) = (*pw, *ph);
        let nw = (pw / 2).max(1);
        let nh = (ph / 2).max(1);
        let mut next = vec![0u8; (nw * nh * 4) as usize];

        for y in 0..nh {
            for x in 0..nw {
                // Source 2x2 block, clamped for odd dimensions.
                let x0 = (x * 2).min(pw - 1);
                let x1 = (x * 2 + 1).min(pw - 1);
                let y0 = (y * 2).min(ph - 1);
                let y1 = (y * 2 + 1).min(ph - 1);
                let texel = |tx: u32, ty: u32, c: usize| -> u32 {
                    prev[((ty * pw + tx) * 4) as usize + c] as u32
                };
                let block = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)];

                let alpha_sum: u32 = block.iter().map(|&(tx, ty)| texel(tx, ty, 3)).sum();
                let out = ((y * nw + x) * 4) as usize;
                for c in 0..3 {
                    // Fully transparent block: no authored colour to preserve,
                    // so fall back to the unweighted mean rather than emitting
                    // black, which would still bleed once alpha is rescaled.
                    next[out + c] = if alpha_sum == 0 {
                        (block.iter().map(|&(tx, ty)| texel(tx, ty, c)).sum::<u32>() / 4) as u8
                    } else {
                        let weighted: u32 = block
                            .iter()
                            .map(|&(tx, ty)| texel(tx, ty, c) * texel(tx, ty, 3))
                            .sum();
                        (weighted / alpha_sum) as u8
                    };
                }
                next[out + 3] = (alpha_sum / 4) as u8;
            }
        }

        levels.push((nw, nh, next));
        w = nw;
        h = nh;
    }

    preserve_alpha_coverage(&mut levels);
    levels
}

/// Alpha cutoff that [`preserve_alpha_coverage`] matches coverage against.
///
/// Mirrors the glTF default `alphaCutoff` and the value the loader assigns to
/// sidecar-masked materials.
const ALPHA_TEST_CUTOFF: u8 = 128;

/// Rescale each mip's alpha so the same fraction of texels passes alpha testing
/// as at mip 0 (Castaño's alpha-test mipmap scaling).
///
/// Left alone, downsampling a binary mask averages it toward grey, so fewer and
/// fewer texels clear the cutoff and masked geometry erodes with distance —
/// grass thins to nothing, tree canopies go patchy and then bald.
///
/// A no-op for textures that are opaque everywhere: coverage is already 1.0 at
/// every level, so the search settles on a scale of 1 and nothing moves.
fn preserve_alpha_coverage(levels: &mut [(u32, u32, Vec<u8>)]) {
    /// Fraction of texels at or above the cutoff once alpha is scaled.
    fn coverage(data: &[u8], scale: f32) -> f32 {
        let passing = data
            .chunks_exact(4)
            .filter(|t| (f32::from(t[3]) * scale) >= f32::from(ALPHA_TEST_CUTOFF))
            .count();
        passing as f32 / (data.len() / 4) as f32
    }

    let Some((_, _, base)) = levels.first() else {
        return;
    };
    let target = coverage(base, 1.0);
    // Nothing masked out (or nothing left): no coverage to defend.
    if target >= 1.0 || target <= 0.0 {
        return;
    }

    for (_, _, data) in levels.iter_mut().skip(1) {
        // Bisect on the scale factor. Coverage is monotonic in scale but a step
        // function, so solve numerically rather than in closed form; a handful
        // of iterations lands well inside one 8-bit quantisation step.
        let (mut lo, mut hi) = (0.0f32, 4.0f32);
        for _ in 0..12 {
            let mid = 0.5 * (lo + hi);
            if coverage(data, mid) < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let scale = 0.5 * (lo + hi);
        for texel in data.chunks_exact_mut(4) {
            texel[3] = (f32::from(texel[3]) * scale).round().clamp(0.0, 255.0) as u8;
        }
    }
}

#[cfg(test)]
mod mip_tests {
    use super::{build_mip_chain, ALPHA_TEST_CUTOFF};

    /// A 2x2 cutout block: one opaque green texel, three transparent black.
    /// Unweighted averaging would give a quarter-strength muddy green; weighting
    /// by alpha has to hand back the authored green untouched.
    #[test]
    fn transparent_black_does_not_darken_the_visible_colour() {
        let mut data = vec![0u8; 2 * 2 * 4];
        data[0..4].copy_from_slice(&[80, 120, 40, 255]);
        let levels = build_mip_chain(&data, 2, 2);
        let (_, _, mip1) = &levels[1];
        assert_eq!(
            &mip1[0..3],
            &[80, 120, 40],
            "alpha-weighted average let the transparent background bleed in",
        );
    }

    /// Downsampling a binary mask averages alpha toward grey, which without
    /// rescaling drops texels below the cutoff and erodes the shape. The
    /// property that matters is that coverage never *shrinks*.
    ///
    /// It can overshoot, and at the smallest levels it must: once a mip is
    /// uniform, every texel passes or none do, so there is no scale that lands
    /// on a fraction. Rounding up keeps distant grass visible; rounding down
    /// would make it disappear, which is the failure this rescaling exists to
    /// prevent.
    #[test]
    fn coverage_never_erodes_down_the_chain() {
        // 4x4 checkerboard: half the texels opaque, half fully transparent.
        let mut data = vec![0u8; 4 * 4 * 4];
        for (i, texel) in data.chunks_exact_mut(4).enumerate() {
            texel[0..3].copy_from_slice(&[200, 200, 200]);
            texel[3] = if i % 2 == 0 { 255 } else { 0 };
        }
        let base_cov = 0.5;
        for (w, h, level) in build_mip_chain(&data, 4, 4) {
            let passing = level
                .chunks_exact(4)
                .filter(|t| t[3] >= ALPHA_TEST_CUTOFF)
                .count() as f32;
            let cov = passing / (w * h) as f32;
            assert!(
                cov >= base_cov,
                "mip {w}x{h} coverage {cov} eroded below {base_cov}",
            );
        }
    }

    /// Without rescaling the same checkerboard would vanish: the 2x2 level
    /// averages to 127, one step under the cutoff, and nothing survives.
    /// This pins the regression that rescaling exists to prevent.
    #[test]
    fn an_unrescaled_mask_would_have_vanished() {
        let mut data = vec![0u8; 4 * 4 * 4];
        for (i, texel) in data.chunks_exact_mut(4).enumerate() {
            texel[3] = if i % 2 == 0 { 255 } else { 0 };
        }
        let levels = build_mip_chain(&data, 4, 4);
        let (_, _, mip1) = &levels[1];
        assert!(
            mip1.chunks_exact(4).all(|t| t[3] >= ALPHA_TEST_CUTOFF),
            "half-covered mask fell under the cutoff after downsampling",
        );
    }

    /// The opaque case must be untouched: no alpha weighting artefacts, no
    /// coverage rescaling, identical to the plain box filter it replaced.
    #[test]
    fn fully_opaque_textures_keep_full_alpha() {
        let mut data = vec![0u8; 8 * 8 * 4];
        for (i, texel) in data.chunks_exact_mut(4).enumerate() {
            texel[0] = i as u8;
            texel[3] = 255;
        }
        for (_, _, level) in build_mip_chain(&data, 8, 8) {
            assert!(
                level.chunks_exact(4).all(|t| t[3] == 255),
                "coverage rescaling touched an opaque texture",
            );
        }
    }

    #[test]
    fn chain_halves_down_to_one_by_one() {
        let data = vec![255u8; 8 * 8 * 4];
        let levels = build_mip_chain(&data, 8, 8);
        let dims: Vec<(u32, u32)> = levels.iter().map(|(w, h, _)| (*w, *h)).collect();
        assert_eq!(dims, vec![(8, 8), (4, 4), (2, 2), (1, 1)]);
    }

    #[test]
    fn a_flat_colour_survives_downsampling() {
        let data = vec![100u8; 4 * 4 * 4];
        for (_, _, level) in build_mip_chain(&data, 4, 4) {
            assert!(level.iter().all(|&v| v == 100), "box filter shifted a flat colour");
        }
    }

    #[test]
    fn non_square_and_odd_sizes_terminate() {
        // Odd dimensions must not loop forever or index out of bounds.
        let data = vec![7u8; 5 * 3 * 4];
        let levels = build_mip_chain(&data, 5, 3);
        assert_eq!(levels.last().unwrap().0, 1);
        assert_eq!(levels.last().unwrap().1, 1);
    }

    #[test]
    fn each_level_has_the_expected_byte_count() {
        let data = vec![0u8; 16 * 16 * 4];
        for (w, h, level) in build_mip_chain(&data, 16, 16) {
            assert_eq!(level.len(), (w * h * 4) as usize);
        }
    }
}
