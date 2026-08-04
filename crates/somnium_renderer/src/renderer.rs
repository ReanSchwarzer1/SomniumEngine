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
    pub vignette_strength: f32,

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

        let vis_pass = VisibilityBufferPass::new(
            &ctx.device, ctx.config.width, ctx.config.height, &global_pool.layout,
        );
        let shading_pass = ShadingPass::new(
            &ctx.device,
            &global_pool.layout,
            HDR_FORMAT,   // shading writes to the Rgba16Float HDR texture
            &vis_pass.view,
            &shadow_resources.atlas_depth_view,
            &shadow_resources.comparison_sampler,
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
            light_color: default_color,
            cascade_debug: false,
            shading_mode: 0,
            local_lights: Vec::new(),
            grid_pass,
            grid_enabled: false,
            postprocess_pass,
            exposure: 1.0,
            vignette_strength: 1.0,
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
            let row_bytes  = tex.width * 4;
            let align      = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let padded_row = (row_bytes + align - 1) / align * align;

            let upload_data: std::borrow::Cow<[u8]> = if padded_row == row_bytes {
                std::borrow::Cow::Borrowed(&tex.data)
            } else {
                let mut buf = vec![0u8; padded_row as usize * tex.height as usize];
                for row in 0..tex.height as usize {
                    let src = row * row_bytes as usize;
                    let dst = row * padded_row as usize;
                    buf[dst..dst + row_bytes as usize]
                        .copy_from_slice(&tex.data[src..src + row_bytes as usize]);
                }
                std::borrow::Cow::Owned(buf)
            };

            let wgpu_tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Scene Texture"),
                size: wgpu::Extent3d {
                    width: tex.width, height: tex.height, depth_or_array_layers: 1,
                },
                mip_level_count:  1,
                sample_count:     1,
                dimension:        wgpu::TextureDimension::D2,
                format:           wgpu::TextureFormat::Rgba8UnormSrgb,
                usage:            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats:     &[],
            });

            ctx.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture:   &wgpu_tex,
                    mip_level: 0,
                    origin:    wgpu::Origin3d::ZERO,
                    aspect:    wgpu::TextureAspect::All,
                },
                &upload_data,
                wgpu::TexelCopyBufferLayout {
                    offset:         0,
                    bytes_per_row:  Some(padded_row),
                    rows_per_image: Some(tex.height),
                },
                wgpu::Extent3d { width: tex.width, height: tex.height, depth_or_array_layers: 1 },
            );

            let view = wgpu_tex.create_view(&wgpu::TextureViewDescriptor::default());
            Some(self.add_texture(ctx, view) as i32)
        }).collect();

        // 2. Materials -------------------------------------------------------
        let resolve_tex = |opt: Option<usize>| -> i32 {
            opt.and_then(|i| texture_indices.get(i).and_then(|&t| t)).unwrap_or(-1)
        };

        let material_ids: Vec<u32> = scene.materials.iter().map(|mat| {
            self.materials_pool.add_material(&ctx.queue, GpuMaterial {
                base_color:             mat.base_color,
                roughness:              mat.roughness,
                metallic:               mat.metallic,
                albedo_map:             resolve_tex(mat.albedo_map),
                normal_map:             resolve_tex(mat.normal_map),
                metallic_roughness_map: resolve_tex(mat.metallic_roughness_map),
                _padding:               [0; 3],
            })
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
            self.outline_pass.resize(&ctx.device, width, height);
        }
    }

    /// Submit a draw command to the queue.
    pub fn submit(&mut self, cmd: DrawCommand) {
        self.draw_queue.push(cmd);
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
            _pad2: [0.0; 3],
        };
        ctx.queue.write_buffer(
            &self.global_pool.light_buffer,
            0,
            bytemuck::bytes_of(&gpu_light),
        );

        // ── 2. Build and upload instance buffer ──────────────────────────────
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
        self.instances.upload(&ctx.queue);

        // ── 3. Sort draw queue ───────────────────────────────────────────────
        self.draw_queue.sort_by_key(|cmd| cmd.sort_key);

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

            for (inst_id, cmd) in self.draw_queue.iter().enumerate() {
                rpass.draw(0..cmd.index_count, inst_id as u32..(inst_id as u32 + 1));
            }
        }

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
            self.water_pass.record(
                &ctx.device,
                &ctx.queue,
                &mut encoder,
                &self.postprocess_pass.hdr_view,
                &self.vis_pass.depth_view,
                &self.global_pool.view_proj_buffer,
                &self.vis_pass.depth_view,
                &self.geometry.vertex_buffer,
                &self.geometry.index_buffer,
                self.water_textures_bind_group.as_ref(),
                &self.water_queue,
            );
        }

        // ── 7.7 Grid Overlay → HDR texture ───────────────────────────────────
        if self.grid_enabled {
            self.grid_pass.record(&mut encoder, &self.postprocess_pass.hdr_view);
        }

        // ── 8. Post-process Pass: HDR → swapchain (ACES + vignette) ──────────
        self.postprocess_pass.set_params(&ctx.queue, self.exposure, self.vignette_strength);
        self.postprocess_pass.record(&mut encoder, &surface_view);

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
        self.light_gizmo_queue.clear();
    }
}
