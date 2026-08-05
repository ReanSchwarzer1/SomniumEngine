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
    /// Exposure multiplier applied before ACES tone mapping (default 1.0).
    pub exposure: f32,
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

    /// Phase 14: Heightmap terrain pass + terrain storage.
    pub terrain_pass: crate::pass::terrain::TerrainPass,
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
    /// each frame. Consumed by the two-phase occlusion cull in 15E2.
    pub hiz_pass: crate::pass::hiz::HiZPass,

    /// Phase 15B: GPU frustum-culling compute pass.
    cull_pass: crate::pass::cull::CullPass,
    /// Per-draw local AABBs for culling, rebuilt each frame.
    cull_aabbs: Vec<crate::culling::GpuCullAabb>,
    /// When false the cull shader keeps every draw (useful for A/B checks).
    pub culling_enabled: bool,

    /// Phase 21: forward pass for alpha-blended materials.
    transparent_pass: crate::pass::transparent::TransparentPass,
    /// Blended draws submitted this frame (routed automatically by material).
    transparent_queue: Vec<DrawCommand>,
    /// Per-material flag: true when the material is alpha-blended. Lets
    /// `submit` route draws without any call site needing to know.
    material_blend: Vec<bool>,

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

        let global_pool = GlobalResourcePool::new(
            &ctx.device,
            &geometry.vertex_buffer,
            &geometry.index_buffer,
            &instances.buffer,
            &view_buffer,
            &materials_pool.buffer,
            &light_buffer,
        );

        let materials = MaterialSystem::new();

        // Phase 11.5H: Editor infinite-grid overlay (renders to HDR target).
        let grid_pass = GridPass::new(&ctx.device, HDR_FORMAT, &global_pool.view_proj_buffer);

        // Phase 11.5K: Post-process pass owns the Rgba16Float HDR render target.
        let postprocess_pass = PostProcessPass::new(
            &ctx.device, ctx.config.format, ctx.config.width, ctx.config.height,
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
        let ibl_pass = crate::pass::ibl::IblPass::new(&ctx.device);

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

        // Phase 14: terrain pass shares the view/light/cluster buffers and the
        // shadow atlas; terrains themselves are created on demand.
        let terrain_pass = crate::pass::terrain::TerrainPass::new(
            &ctx.device,
            HDR_FORMAT,
            &global_pool.view_proj_buffer,
            &global_pool.light_buffer,
            &shadow_resources.atlas_depth_view,
            &shadow_resources.comparison_sampler,
            &global_pool.cluster_grid,
        );

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
            exposure: 1.0,
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
            ibl_pass,
            hiz_pass,
            cull_pass: crate::pass::cull::CullPass::new(&ctx.device),
            cull_aabbs: Vec::new(),
            culling_enabled: true,
            gpu_driven: ctx.supports_gpu_driven(),
            supports_gpu_driven: ctx.supports_gpu_driven(),
            water_pass,
            water_queue: Vec::new(),
            terrain_pass,
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
                _padding:               [0; 3],
            });
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
            .map(|mesh| self.geometry.upload_mesh(&ctx.queue, &mesh.vertices, &mesh.indices, 0))
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
        self.view_proj   = proj * view;
        self.camera_pos  = camera_pos;
    }

    /// Scene-wide indirect-light strength (Phase 22C), uploaded with the sun.
    pub fn set_ibl_intensity(&mut self, intensity: f32) {
        self.ibl_intensity = intensity.max(0.0);
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
    pub fn resize(&mut self, ctx: &RenderContext, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.vis_pass.resize(&ctx.device, width, height, &self.global_pool.layout);
            self.shading_pass.resize(&ctx.device, &self.vis_pass.view);
            self.postprocess_pass.resize(&ctx.device, width, height);
            self.fxaa_pass.resize(&ctx.device, ctx.config.format, width, height);
            self.outline_pass.resize(&ctx.device, width, height);
            // Must follow vis_pass: the level-0 bind group references its depth view.
            self.hiz_pass.resize(&ctx.device, width, height, &self.vis_pass.depth_view);
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
    pub fn create_terrain(
        &mut self,
        ctx: &RenderContext,
        desc: crate::terrain::TerrainDescriptor,
    ) -> u32 {
        let terrain = crate::terrain::TerrainData::new(
            &ctx.device,
            &ctx.queue,
            &self.terrain_pass.terrain_bgl,
            &self.terrain_pass.sampler,
            desc,
        );
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
        self.ibl_pass.generate_if_needed(
            &ctx.device,
            &ctx.queue,
            self.light_direction,
            self.light_color,
        );

        // ── 1. Compute cascades and upload light buffer ───────────────────────
        let cascades = compute_cascades(self.light_direction, inv_view_proj);

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
            _pad2: [0.0; 2],
        };
        ctx.queue.write_buffer(
            &self.global_pool.light_buffer,
            0,
            bytemuck::bytes_of(&gpu_light),
        );

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
            self.indirect.update(&ctx.device, &ctx.queue, &self.draw_queue);

            // Phase 15B: parallel array of local bounds for the cull shader.
            // A draw whose mesh has no recorded AABB gets an infinite box, so
            // it is never culled — safer than guessing at its extent.
            self.cull_aabbs.clear();
            self.cull_aabbs.extend(self.draw_queue.iter().map(|cmd| {
                match self.geometry.mesh_aabb(cmd.vertex_offset) {
                    Some((min, max)) => crate::culling::GpuCullAabb {
                        min: [min[0], min[1], min[2], 0.0],
                        max: [max[0], max[1], max[2], 0.0],
                    },
                    None => crate::culling::GpuCullAabb {
                        min: [f32::MIN, f32::MIN, f32::MIN, 0.0],
                        max: [f32::MAX, f32::MAX, f32::MAX, 0.0],
                    },
                }
            }));
            self.cull_pass.update(
                &ctx.device,
                &ctx.queue,
                &self.cull_aabbs,
                self.view_proj,
                !self.culling_enabled,
            );
        }

        // ── 4. Acquire swapchain texture ─────────────────────────────────────
        let output = match ctx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(tex)    => tex,
            wgpu::CurrentSurfaceTexture::Suboptimal(tex) => tex,
            _ => {
                tracing::warn!("Failed to acquire surface texture");
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

        // ── 5.5 Phase 15B: GPU instance culling ──────────────────────────────
        // Writes instance_count = 0 into the indirect args for off-screen
        // draws, so the visibility pass skips them at no cost.
        if self.gpu_driven && !self.indirect.is_empty() {
            self.cull_pass.record(
                &ctx.device,
                &mut encoder,
                &self.instances.buffer,
                &self.indirect.buffer,
                self.indirect.len(),
            );
        }

        // ── 6. Visibility Pass ───────────────────────────────────────────────
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Visibility Pass"),
                multiview_mask: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.vis_pass.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.vis_pass.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            rpass.set_pipeline(&self.vis_pass.pipeline);
            rpass.set_bind_group(0, &self.global_pool.bind_group, &[]);

            if self.gpu_driven && !self.indirect.is_empty() {
                // Phase 15A: the whole scene in one call. Culled draws (Phase
                // 15B) simply carry instance_count = 0 and cost nothing.
                rpass.multi_draw_indirect(&self.indirect.buffer, 0, self.indirect.len() as u32);
            } else {
                // Fallback for devices without multi-draw indirect.
                for (inst_id, cmd) in self.draw_queue.iter().enumerate() {
                    rpass.draw(0..cmd.index_count, inst_id as u32..(inst_id as u32 + 1));
                }
            }
        }

        // ── 6.9 Phase 15E: Hi-Z pyramid from this frame's depth ──────────────
        // Built right after the visibility pass, while the depth buffer holds
        // exactly the opaque geometry — the only thing that may occlude.
        self.hiz_pass.record(&mut encoder);

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
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.07, g: 0.07, b: 0.07, a: 1.0 }),
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

        // ── 7.3 Terrain Pass → HDR texture (Phase 14) ────────────────────────
        // Opaque, depth-writing draws against the visibility depth buffer, so
        // the water pass (7.5) correctly tests against terrain.
        if !self.terrain_queue.is_empty() {
            let camera_world = self.camera_pos;
            for &(id, model) in &self.terrain_queue {
                let terrain = &mut self.terrains[id as usize];
                terrain.model = model;
                let local_cam = model.inverse().transform_point3(camera_world);
                terrain.select_lods(local_cam);
                terrain.rebuild_dirty_chunks(&ctx.queue);
                terrain.ensure_index_buffers(&ctx.device);
                terrain.splatmap.upload_dirty(&ctx.queue);
                terrain.upload_uniforms(&ctx.queue);
            }
            let queued: Vec<&crate::terrain::TerrainData> = self
                .terrain_queue
                .iter()
                .map(|&(id, _)| &self.terrains[id as usize])
                .collect();
            self.terrain_pass.record(
                &mut encoder,
                &self.postprocess_pass.hdr_view,
                &self.vis_pass.depth_view,
                &queued,
            );
        }

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

        // ── 8. Post-process Pass: HDR → swapchain (ACES + vignette) ──────────
        self.postprocess_pass.set_params(
            &ctx.queue,
            self.exposure,
            self.vignette_strength,
            self.chromatic_aberration,
        );
        // Phase 15A2: with FXAA on, tone-map into the LDR intermediate and let
        // FXAA resolve it to the swapchain. Editor overlays draw afterwards, so
        // gizmos and UI text stay pixel-sharp.
        if self.fxaa_enabled {
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

        ctx.queue.submit(std::iter::once(encoder.finish()));
        output.present();

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
                for c in 0..4 {
                    let sum = texel(x0, y0, c) + texel(x1, y0, c)
                            + texel(x0, y1, c) + texel(x1, y1, c);
                    next[((y * nw + x) * 4) as usize + c] = (sum / 4) as u8;
                }
            }
        }

        levels.push((nw, nh, next));
        w = nw;
        h = nh;
    }
    levels
}

#[cfg(test)]
mod mip_tests {
    use super::build_mip_chain;

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
