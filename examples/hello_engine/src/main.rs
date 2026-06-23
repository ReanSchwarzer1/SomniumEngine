//! # Hello Somnium Engine — glTF Demo
//!
//! Loads a glTF scene from `assets/test_scene.glb`, uploads it to the
//! Visibility Buffer pipeline, and renders it with PBR + sky lighting.
//!
//! Falls back to a procedural cube scene when no GLB file is found.
//!
//! ## Running
//!
//! ```bash
//! cargo run -p hello_engine
//! ```
//!
//! Phase 11.5 additions:
//! - Parent/Children/WorldTransform hierarchy (11.5A)
//! - Editable inspector (11.5C)
//! - Create menu and procedural meshes (11.5D)
//! - Undo/redo (11.5E)
//! - Scene save/load (11.5F)
//! - Editor mode (Play/Pause/Stop) (11.5L)

use somnium_core::{
    Component, Engine, EngineConfig, EngineContext, EngineEvent, Entity, GameApp, InputState,
    KeyCode, MeshComponent, MaterialComponent, MeshKind, Name, Transform, LightComponent, LightType,
    Parent, WorldTransform, propagate_transforms,
    ComponentId, ComponentSet,
};
use somnium_physics::body::{RigidBodyDescriptor, MotionType, BodyId};
use somnium_physics::shape::ColliderShape;
use somnium_physics::layer::{LAYER_NON_MOVING, LAYER_MOVING};
use glam::Vec3;
use tracing::info;
use serde::Serialize;

fn downsample_wrap(img: &image::RgbaImage) -> image::RgbaImage {
    let w = (img.width() / 2).max(1);
    let h = (img.height() / 2).max(1);
    let mut out = image::RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let sx = x * 2;
            let sy = y * 2;
            
            let x0 = sx % img.width();
            let x1 = (sx + 1) % img.width();
            let y0 = sy % img.height();
            let y1 = (sy + 1) % img.height();
            
            let p00 = img.get_pixel(x0, y0);
            let p10 = img.get_pixel(x1, y0);
            let p01 = img.get_pixel(x0, y1);
            let p11 = img.get_pixel(x1, y1);
            
            let mut res = [0u32; 4];
            for i in 0..4 {
                res[i] = p00[i] as u32 + p10[i] as u32 + p01[i] as u32 + p11[i] as u32;
            }
            out.put_pixel(x, y, image::Rgba([
                (res[0] / 4) as u8,
                (res[1] / 4) as u8,
                (res[2] / 4) as u8,
                (res[3] / 4) as u8,
            ]));
        }
    }
    out
}

fn load_texture_from_path(device: &wgpu::Device, queue: &wgpu::Queue, path: &str, format: wgpu::TextureFormat) -> wgpu::TextureView {
    let img = image::open(path).unwrap_or_else(|e| panic!("Failed to load texture at {}: {}", path, e));
    let rgba = img.to_rgba8();
    let dimensions = rgba.dimensions();

    let mip_level_count = (dimensions.0.max(dimensions.1) as f32).log2().floor() as u32 + 1;

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(path),
        size: wgpu::Extent3d { width: dimensions.0, height: dimensions.1, depth_or_array_layers: 1 },
        mip_level_count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    let mut current_img = rgba;
    for level in 0..mip_level_count {
        let level_dims = current_img.dimensions();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &current_img,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * level_dims.0),
                rows_per_image: Some(level_dims.1),
            },
            wgpu::Extent3d { width: level_dims.0, height: level_dims.1, depth_or_array_layers: 1 },
        );
        
        if level < mip_level_count - 1 {
            current_img = downsample_wrap(&current_img);
        }
    }

    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Components
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Copy)]
struct Position { x: f32, y: f32 }
impl Component for Position {}

#[derive(Debug, Clone, Copy)]
struct PhysicsBody { id: BodyId }
impl Component for PhysicsBody {}

#[derive(Serialize)]
struct OutlinerEntity {
    name:   String,
    index:  u32,
    parent: Option<u32>,
    depth:  u32,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Camera
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

struct EditorCamera {
    position:      Vec3,
    yaw:           f32,
    pitch:         f32,
    speed:         f32,
    sensitivity:   f32,
    is_rmb_down:   bool,
    move_forward:  bool,
    move_backward: bool,
    move_left:     bool,
    move_right:    bool,
    move_up:       bool,
    move_down:     bool,
    is_shifting:   bool,
}

impl EditorCamera {
    fn new(position: Vec3) -> Self {
        Self {
            position, yaw: -90.0, pitch: -20.0,
            speed: 5.0, sensitivity: 0.1,
            is_rmb_down: false,
            move_forward: false, move_backward: false,
            move_left: false, move_right: false,
            move_up: false, move_down: false,
            is_shifting: false,
        }
    }

    fn update(&mut self, dt: f32) {
        if !self.is_rmb_down { return; }
        let speed = if self.is_shifting { self.speed * 3.0 } else { self.speed };
        let forward = self.forward_vector();
        let right = forward.cross(Vec3::Y).normalize();
        if self.move_forward  { self.position += forward * speed * dt; }
        if self.move_backward { self.position -= forward * speed * dt; }
        if self.move_right    { self.position += right * speed * dt; }
        if self.move_left     { self.position -= right * speed * dt; }
        if self.move_up       { self.position += Vec3::Y * speed * dt; }
        if self.move_down     { self.position -= Vec3::Y * speed * dt; }
    }

    fn forward_vector(&self) -> Vec3 {
        let yaw   = self.yaw.to_radians();
        let pitch = self.pitch.to_radians();
        Vec3::new(yaw.cos() * pitch.cos(), pitch.sin(), yaw.sin() * pitch.cos()).normalize()
    }

    fn view_matrix(&self) -> glam::Mat4 {
        glam::Mat4::look_at_rh(self.position, self.position + self.forward_vector(), Vec3::Y)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Editor mode
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorMode { Editing, Playing, Paused }

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Voxel terrain driver (Phase 14)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Bridges the streaming `somnium_voxel::VoxelWorld` to the renderer.
///
/// Chunks are deliberately NOT ECS entities: the world streams hundreds of
/// them in and out as the camera moves, which would flood the outliner and
/// undo stack. Instead each uploaded chunk is submitted as a raw
/// `DrawCommand` through the regular visibility-buffer pipeline, so it still
/// gets shadows, PBR shading, and clustered lights for free.
struct VoxelTerrain {
    world: somnium_voxel::VoxelWorld,
    /// GPU allocation per loaded chunk. `None` = chunk has no visible faces.
    chunks: std::collections::HashMap<somnium_voxel::ChunkCoord,
        Option<somnium_renderer::geometry::MeshAllocation>>,
    /// Shared material: white base color + palette texture (albedo).
    material_id: u32,
}

impl VoxelTerrain {
    /// Create the palette texture/material and seed a few `set_voxel` edits
    /// that prove the edit-overlay → remesh path works.
    fn new(
        renderer: &mut somnium_renderer::SomniumRenderer,
        render_ctx: &somnium_renderer::context::RenderContext,
    ) -> Self {
        use somnium_voxel::{Voxel, PALETTE_SIZE};

        // 1-D palette texture: one texel per voxel type, sampled at the texel
        // center by the constant per-face UV the mesher writes.
        let palette_bytes: Vec<u8> = Voxel::ALL.iter()
            .flat_map(|v| v.palette_color())
            .collect();
        let texture = render_ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Voxel Palette"),
            size: wgpu::Extent3d { width: PALETTE_SIZE, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        render_ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &palette_bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: None, // single row — no alignment requirement
                rows_per_image: None,
            },
            wgpu::Extent3d { width: PALETTE_SIZE, height: 1, depth_or_array_layers: 1 },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let palette_tex = renderer.add_texture(render_ctx, view);

        let material_id = renderer.materials_pool.add_material(
            &render_ctx.queue,
            somnium_renderer::material::pool::GpuMaterial {
                base_color: [1.0, 1.0, 1.0, 1.0],
                roughness: 0.9, metallic: 0.0,
                albedo_map: palette_tex as i32,
                normal_map: -1, metallic_roughness_map: -1, _padding: [0; 3],
            },
        );

        let mut world = somnium_voxel::VoxelWorld::new(somnium_voxel::VoxelWorldConfig::default());

        // Demo edits: a small stone cairn east of the basin, proving that the
        // set_voxel overlay is honored by chunk generation and remeshing.
        let cairn_x = 20;
        let cairn_z = 6;
        let surface = (0..40).rev()
            .map(|y| y - 8)
            .find(|&y| world.get_voxel(glam::IVec3::new(cairn_x, y, cairn_z)).is_solid())
            .unwrap_or(0);
        for dy in 1..=3 {
            world.set_voxel(
                glam::IVec3::new(cairn_x, surface + dy, cairn_z),
                somnium_voxel::Voxel::Stone,
            );
        }

        Self { world, chunks: Default::default(), material_id }
    }

    /// Per-frame: stream chunks around the camera, upload finished meshes,
    /// and recycle GPU memory of despawned chunks.
    fn update(
        &mut self,
        camera_pos: Vec3,
        renderer: &mut somnium_renderer::SomniumRenderer,
        render_ctx: &somnium_renderer::context::RenderContext,
    ) {
        let upd = self.world.update(camera_pos);

        for coord in upd.despawned {
            if let Some(Some(alloc)) = self.chunks.remove(&coord) {
                renderer.geometry.free_mesh(alloc);
            }
        }

        for ready in upd.ready {
            let new_alloc = ready.mesh.as_ref().map(|m| {
                renderer.geometry.upload_mesh_pooled(
                    &render_ctx.queue, &m.vertices, &m.indices, self.material_id,
                )
            });
            if let Some(Some(old)) = self.chunks.insert(ready.coord, new_alloc) {
                renderer.geometry.free_mesh(old);
            }
        }
    }

    /// Submit one DrawCommand per non-empty chunk.
    fn submit_draws(&self, renderer: &mut somnium_renderer::SomniumRenderer) {
        for (coord, entry) in &self.chunks {
            let Some(alloc) = entry else { continue };
            let origin = somnium_voxel::chunk_origin(*coord);
            renderer.submit(somnium_renderer::command::DrawCommand {
                sort_key: somnium_renderer::command::SortKey::new(
                    0, self.material_id as u16, alloc.vertex_offset,
                ),
                vertex_offset: alloc.vertex_offset,
                index_offset:  alloc.index_offset,
                index_count:   alloc.index_count,
                material_id:   self.material_id,
                transform:     glam::Mat4::from_translation(origin),
            });
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Game struct
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

struct HelloGame {
    log_timer:     f32,
    camera:        EditorCamera,
    cascade_debug: bool,
    editor_mode:   EditorMode,
    /// Default material ID for newly created entities.
    default_material_id: Option<u32>,
    /// Geometry allocation for the procedural cube (shared by create_entity).
    default_cube_alloc: Option<somnium_renderer::geometry::MeshAllocation>,
    /// Phase 14: streaming voxel terrain.
    voxel_terrain: Option<VoxelTerrain>,
}

impl HelloGame {
    fn new() -> Self {
        Self {
            log_timer:           0.0,
            camera:              EditorCamera::new(Vec3::new(0.0, 2.0, 8.0)),
            cascade_debug:       false,
            editor_mode:         EditorMode::Editing,
            default_material_id: None,
            default_cube_alloc:  None,
            voxel_terrain:       None,
        }
    }
}

impl GameApp for HelloGame {
    fn on_init(&mut self, ctx: &mut EngineContext) {
        info!("HelloGame initialised — loading scene...");

        let gltf_loaded = if let (Some(renderer), Some(render_ctx)) = (&mut ctx.renderer, &ctx.render_ctx) {
            match somnium_asset::load_gltf("assets/test_scene.glb") {
                Ok(scene) => {
                    info!("glTF loaded; uploading to GPU...");
                    let uploaded = renderer.upload_scene(render_ctx, &scene);
                    info!("{} nodes uploaded from glTF", uploaded.len());
                    for node in &uploaded {
                        let (scale, rotation, translation) =
                            node.transform.to_scale_rotation_translation();
                        ctx.world.spawn((
                            Transform { translation, rotation, scale },
                            MeshComponent {
                                vertex_offset: node.vertex_offset,
                                index_offset:  node.index_offset,
                                index_count:   node.index_count,
                            },
                            MaterialComponent { id: node.material_id },
                            Name::new(&node.entity_name),
                            WorldTransform::identity(),
                        ));
                    }
                    true
                }
                Err(e) => {
                    tracing::warn!("glTF load failed ({e}); using procedural cube scene");
                    false
                }
            }
        } else {
            false
        };

        if !gltf_loaded {
            let (mat_id, alloc) = spawn_procedural_scene(ctx);
            self.default_material_id = Some(mat_id);
            self.default_cube_alloc  = Some(alloc);
        }

        // Phase 13: Water Plane (spawned alongside the helmet)
        let (plane_verts, plane_idxs) = somnium_asset::generate_plane(20.0, 10);
        let plane_alloc = ctx.renderer.as_mut().unwrap().geometry.upload_mesh(
            &ctx.render_ctx.as_ref().unwrap().queue, &plane_verts, &plane_idxs, 0
        );
        ctx.world.spawn((
            Transform::from_translation(glam::Vec3::new(0.0, -0.5, 0.0)),
            MeshComponent { 
                vertex_offset: plane_alloc.vertex_offset, 
                index_offset: plane_alloc.index_offset, 
                index_count: plane_alloc.index_count 
            },
            somnium_core::WaterComponent::default(),
            Name::new("WaterPlane"),
            WorldTransform::identity(),
            MeshKind::Plane,
        ));

        // Load Water PBR Textures
        let render_ctx = ctx.render_ctx.as_ref().unwrap();
        let base_color = load_texture_from_path(&render_ctx.device, &render_ctx.queue, "assets/ocean_pbr/BaseColor.png", wgpu::TextureFormat::Rgba8UnormSrgb);
        let normal = load_texture_from_path(&render_ctx.device, &render_ctx.queue, "assets/ocean_pbr/Normal_DX.png", wgpu::TextureFormat::Rgba8Unorm);
        let orm = load_texture_from_path(&render_ctx.device, &render_ctx.queue, "assets/ocean_pbr/ORM_RAO_GROUGH_BMETAL.png", wgpu::TextureFormat::Rgba8Unorm);
        
        let sampler = render_ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Water Sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let water_tex_bg = render_ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Water Textures Bind Group"),
            layout: &ctx.renderer.as_ref().unwrap().water_pass.tex_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&base_color) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&normal) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&orm) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });

        ctx.renderer.as_mut().unwrap().water_textures_bind_group = Some(water_tex_bg);



        // Phase 14: Voxel terrain — hills surrounding the central basin.
        if let (Some(renderer), Some(render_ctx)) = (&mut ctx.renderer, &ctx.render_ctx) {
            self.voxel_terrain = Some(VoxelTerrain::new(renderer, render_ctx));
            info!("Voxel terrain initialised (radius {} chunks)",
                self.voxel_terrain.as_ref().unwrap().world.config().radius_chunks);
        }

        // Phase 14 (SSS): heightmap terrain smoke test — exercises chunk
        // meshing, LODs, sculpt brushes, and auto-splat without editor input.
        // Normally terrain is created via Create > Terrain in the editor.
        if std::env::var("SOMNIUM_TERRAIN").is_ok() {
            if let (Some(renderer), Some(render_ctx)) = (&mut ctx.renderer, &ctx.render_ctx) {
                use somnium_renderer::terrain::{brush, TerrainDescriptor};
                let desc = TerrainDescriptor {
                    grid_size: [4, 4],
                    ..Default::default()
                };
                let terrain_id = renderer.create_terrain(render_ctx, desc);
                let [wx, wz] = desc.world_size();

                if let Some(terrain) = renderer.terrain_mut(terrain_id) {
                    // Sculpt a hill and a valley to exercise the brush paths.
                    let raise = brush::TerrainBrush {
                        mode: brush::BrushMode::Raise,
                        radius: 30.0,
                        strength: 1.0,
                        ..Default::default()
                    };
                    for _ in 0..40 {
                        brush::apply_sculpt(terrain, &raise, wx * 0.3, wz * 0.3, 0.1);
                    }
                    let lower = brush::TerrainBrush {
                        mode: brush::BrushMode::Lower,
                        ..raise
                    };
                    for _ in 0..20 {
                        brush::apply_sculpt(terrain, &lower, wx * 0.7, wz * 0.7, 0.1);
                    }
                    brush::auto_splat(terrain, 10.0);
                }

                ctx.world.spawn((
                    Transform::from_translation(Vec3::new(-wx * 0.5, -6.0, -wz * 0.5)),
                    Name::new("Terrain"),
                    WorldTransform::identity(),
                    somnium_core::TerrainComponent {
                        terrain_id,
                        chunk_cells: desc.chunk_cells,
                        grid_x: desc.grid_size[0],
                        grid_z: desc.grid_size[1],
                        cell_size: desc.cell_size,
                        height_scale: desc.height_scale,
                    },
                ));
                info!("Heightmap terrain smoke test active ({}x{} m)", wx, wz);
            }
        }

        // Phase 11A: Spawn the directional light entity.
        let light_rot = glam::Quat::from_euler(
            glam::EulerRot::YXZ,
            (-30.0_f32).to_radians(),
            (-35.0_f32).to_radians(),
            0.0,
        );
        ctx.world.spawn((
            Transform { translation: Vec3::ZERO, rotation: light_rot, scale: Vec3::ONE },
            LightComponent::directional(5.0),
            Name::new("SunLight"),
            WorldTransform::identity(),
        ));

        ctx.physics.optimize_broad_phase();

        // Send initial content browser listing
        ctx.ui.send_message("update_content_browser", serde_json::json!({
            "path": "assets",
            "entries": list_assets_dir(),
        }));
    }

    fn on_event(&mut self, ctx: &mut EngineContext, event: &EngineEvent) {
        match event {
            EngineEvent::KeyInput { key, state } => {
                let pressed = *state == InputState::Pressed;
                match key {
                    KeyCode::Escape if pressed => ctx.exit(),
                    KeyCode::KeyW  if !self.camera.is_rmb_down && pressed => {
                        if let Some(r) = &mut ctx.renderer { r.set_gizmo_mode(somnium_renderer::GizmoMode::Translate); }
                    }
                    KeyCode::KeyW  => self.camera.move_forward  = pressed,
                    KeyCode::KeyS  => self.camera.move_backward = pressed,
                    KeyCode::KeyA  => self.camera.move_left     = pressed,
                    KeyCode::KeyD  => self.camera.move_right    = pressed,
                    KeyCode::KeyE  if !self.camera.is_rmb_down && pressed => {
                        if let Some(r) = &mut ctx.renderer { r.set_gizmo_mode(somnium_renderer::GizmoMode::Rotate); }
                    }
                    KeyCode::KeyE  => self.camera.move_up       = pressed,
                    KeyCode::KeyQ  if !self.camera.is_rmb_down && pressed => {
                        if let Some(r) = &mut ctx.renderer { r.set_gizmo_mode(somnium_renderer::GizmoMode::Translate); }
                    }
                    KeyCode::KeyQ  => self.camera.move_down     = pressed,
                    KeyCode::KeyR  if !self.camera.is_rmb_down && pressed => {
                        if let Some(r) = &mut ctx.renderer { r.set_gizmo_mode(somnium_renderer::GizmoMode::Scale); }
                    }
                    KeyCode::ShiftLeft | KeyCode::ShiftRight => self.camera.is_shifting = pressed,
                    // C: toggle cascade debug overlay
                    KeyCode::KeyC if pressed => {
                        self.cascade_debug = !self.cascade_debug;
                        if let Some(renderer) = &mut ctx.renderer {
                            renderer.set_cascade_debug(self.cascade_debug);
                        }
                    }
                    _ => {}
                }
            }

            EngineEvent::MouseButton {
                button: somnium_core::MouseButton::Right, state,
            } => {
                self.camera.is_rmb_down = *state == InputState::Pressed;
            }

            EngineEvent::MouseMotion { delta_x, delta_y } => {
                if self.camera.is_rmb_down {
                    self.camera.yaw   += delta_x * self.camera.sensitivity;
                    self.camera.pitch -= delta_y * self.camera.sensitivity;
                    self.camera.pitch = self.camera.pitch.clamp(-89.0, 89.0);
                }
            }

            EngineEvent::WindowResized { width, height } => {
                info!("Window resized to {width}×{height}");
            }

            _ => {}
        }
    }

    fn on_update(&mut self, ctx: &mut EngineContext) {
        let dt = ctx.dt();

        // Phase 11.5A-2: Propagate parent→child transforms, writing WorldTransform.
        propagate_transforms(ctx.world);

        // Sync physics → ECS transforms (only in playing/paused mode to keep editor stable)
        if self.editor_mode == EditorMode::Playing {
            let required = ComponentSet::from_ids(vec![
                ComponentId::of::<Transform>(),
                ComponentId::of::<PhysicsBody>(),
            ]);
            for archetype in ctx.world.query_archetypes_mut(&required, &ComponentSet::empty()) {
                let t_col = archetype.column_index(ComponentId::of::<Transform>()).unwrap();
                let b_col = archetype.column_index(ComponentId::of::<PhysicsBody>()).unwrap();
                for row in 0..archetype.len() {
                    let body      = unsafe { *archetype.column(b_col).get::<PhysicsBody>(row) };
                    let transform = unsafe { archetype.column_mut(t_col).get_mut::<Transform>(row) };
                    transform.translation = ctx.physics.get_position(body.id);
                }
            }
        }

        self.camera.update(dt);
        self.log_timer += dt;

        // Phase 14: Stream voxel chunks around the camera (async generation;
        // finished meshes are uploaded here, freed allocations recycled).
        if let (Some(vt), Some(renderer), Some(render_ctx)) =
            (&mut self.voxel_terrain, &mut ctx.renderer, &ctx.render_ctx)
        {
            vt.update(self.camera.position, renderer, render_ctx);
        }

        // Handle mesh-creating IPC commands that require renderer access.
        // These arrive after the engine-level IPC loop (which handles non-mesh commands).
        // We peek at the renderer here since create_entity for meshes needs GPU upload.
        // (The engine IPC loop in app.rs already drained non-mesh commands.)
        // Mesh entity creation is triggered by the "create_entity_mesh" internal message.
    }

    fn on_render(&mut self, ctx: &mut EngineContext) {
        if let (Some(renderer), Some(render_ctx)) = (&mut ctx.renderer, &ctx.render_ctx) {
            let aspect   = render_ctx.config.width as f32 / render_ctx.config.height as f32;
            let view_mat = self.camera.view_matrix();
            let proj     = glam::Mat4::perspective_rh(45.0f32.to_radians(), aspect, 0.1, 1000.0);
            renderer.set_view(view_mat, proj, self.camera.position);

            // Sync the lights from ECS LightComponent.
            {
                let light_req = ComponentSet::from_ids(vec![
                    ComponentId::of::<Transform>(),
                    ComponentId::of::<LightComponent>(),
                ]);
                for archetype in ctx.world.query_archetypes(&light_req, &ComponentSet::empty()) {
                    let t_col = archetype.column_index(ComponentId::of::<Transform>()).unwrap();
                    let l_col = archetype.column_index(ComponentId::of::<LightComponent>()).unwrap();
                    for row in 0..archetype.len() {
                        let transform = unsafe { archetype.column(t_col).get::<Transform>(row) };
                        let light     = unsafe { archetype.column(l_col).get::<LightComponent>(row) };
                        
                        let forward = transform.rotation.mul_vec3(glam::Vec3::NEG_Z);
                        let dir = -forward;
                        
                        match light.light_type {
                            LightType::Directional => {
                                renderer.set_directional_light(dir, light.color * light.intensity);
                            }
                            LightType::Point | LightType::Spot => {
                                let l_type = if light.light_type == LightType::Point { 0 } else { 1 };
                                renderer.submit_local_light(somnium_renderer::cluster::GpuLocalLight {
                                    position_ws: transform.translation.to_array(),
                                    range: light.range,
                                    color: (light.color * light.intensity).to_array(),
                                    light_type: l_type,
                                    direction_ws: dir.to_array(),
                                    spot_cos_outer: light.outer_angle.cos(),
                                    spot_cos_inner: light.inner_angle.cos(),
                                    _pad: [0.0; 3],
                                });
                            }
                        }
                    }
                }
            }
            
            // Phase 13: Auto-attach MeshComponent/MaterialComponent to editor-spawned primitives.
            let kind_req = ComponentSet::from_ids(vec![ComponentId::of::<MeshKind>()]);
            let mesh_req = ComponentSet::from_ids(vec![ComponentId::of::<MeshComponent>()]);
            let mut pending_meshes = Vec::new();
            for archetype in ctx.world.query_archetypes(&kind_req, &mesh_req) {
                let k_col = archetype.column_index(ComponentId::of::<MeshKind>()).unwrap();
                for row in 0..archetype.len() {
                    let kind = unsafe { archetype.column(k_col).get::<MeshKind>(row) };
                    pending_meshes.push((archetype.entities()[row], *kind));
                }
            }
            
            if !pending_meshes.is_empty() {
                // For now, use the same default blue material
                let default_mat = renderer.materials_pool.add_material(
                    &ctx.render_ctx.as_ref().unwrap().queue,
                    somnium_renderer::material::pool::GpuMaterial {
                        base_color: [0.8, 0.8, 0.8, 1.0],
                        roughness: 0.5, metallic: 0.0,
                        albedo_map: -1, normal_map: -1, metallic_roughness_map: -1, _padding: [0; 3],
                    },
                );

                for (entity, kind) in pending_meshes {
                    let (verts, idxs) = match kind {
                        MeshKind::Cube => somnium_asset::generate_cube(1.0),
                        MeshKind::Plane => somnium_asset::generate_plane(1.0, 1),
                        MeshKind::Sphere => somnium_asset::generate_sphere(0.5, 16, 16),
                        MeshKind::Cylinder => somnium_asset::generate_cylinder(0.5, 1.0, 16),
                    };
                    let alloc = renderer.geometry.upload_mesh(
                        &ctx.render_ctx.as_ref().unwrap().queue,
                        &verts,
                        &idxs,
                        default_mat,
                    );
                    
                    // Read old components
                    let t = ctx.world.get::<Transform>(entity).copied().unwrap_or(Transform::from_translation(glam::Vec3::ZERO));
                    let n = ctx.world.get::<Name>(entity).cloned().unwrap_or(Name::new("Mesh"));
                    let wt = ctx.world.get::<WorldTransform>(entity).copied().unwrap_or(WorldTransform::identity());
                    
                    // Respawn
                    ctx.world.despawn(entity);
                    let new_entity = ctx.world.spawn((
                        t, n, wt, kind,
                        MeshComponent {
                            vertex_offset: alloc.vertex_offset,
                            index_offset: alloc.index_offset,
                            index_count: alloc.index_count,
                        },
                        MaterialComponent { id: default_mat }
                    ));
                    
                    // Fix selection if needed
                    if *ctx.selected_entity == Some(entity) {
                        *ctx.selected_entity = Some(new_entity);
                    }
                }
            }

            // Phase 11.5A-2: Use WorldTransform for rendering instead of Transform::to_matrix().
            let required = ComponentSet::from_ids(vec![
                ComponentId::of::<WorldTransform>(),
                ComponentId::of::<MeshComponent>(),
                ComponentId::of::<MaterialComponent>(),
            ]);
            for archetype in ctx.world.query_archetypes(&required, &ComponentSet::empty()) {
                let wt_col  = archetype.column_index(ComponentId::of::<WorldTransform>()).unwrap();
                let m_col   = archetype.column_index(ComponentId::of::<MeshComponent>()).unwrap();
                let mat_col = archetype.column_index(ComponentId::of::<MaterialComponent>()).unwrap();
                for row in 0..archetype.len() {
                    let wt       = unsafe { archetype.column(wt_col).get::<WorldTransform>(row) };
                    let mesh     = unsafe { archetype.column(m_col).get::<MeshComponent>(row) };
                    let material = unsafe { archetype.column(mat_col).get::<MaterialComponent>(row) };
                    let entity   = archetype.entities()[row];
                    renderer.submit(somnium_renderer::command::DrawCommand {
                        sort_key:     somnium_renderer::command::SortKey::new(0, material.id as u16, entity.index()),
                        vertex_offset: mesh.vertex_offset,
                        index_offset:  mesh.index_offset,
                        index_count:   mesh.index_count,
                        material_id:   material.id,
                        transform:     wt.0,
                    });
                }
            }

            // Phase 14: Submit voxel chunk draws (visibility buffer pipeline).
            if let Some(vt) = &self.voxel_terrain {
                vt.submit_draws(renderer);
            }

            // Phase 13: Submit water meshes.
            let water_req = ComponentSet::from_ids(vec![
                ComponentId::of::<WorldTransform>(),
                ComponentId::of::<MeshComponent>(),
                ComponentId::of::<somnium_core::WaterComponent>(),
            ]);
            for archetype in ctx.world.query_archetypes(&water_req, &ComponentSet::empty()) {
                let wt_col = archetype.column_index(ComponentId::of::<WorldTransform>()).unwrap();
                let m_col  = archetype.column_index(ComponentId::of::<MeshComponent>()).unwrap();
                let w_col  = archetype.column_index(ComponentId::of::<somnium_core::WaterComponent>()).unwrap();
                for row in 0..archetype.len() {
                    let wt    = unsafe { archetype.column(wt_col).get::<WorldTransform>(row) };
                    let mesh  = unsafe { archetype.column(m_col).get::<MeshComponent>(row) };
                    let water = unsafe { archetype.column(w_col).get::<somnium_core::WaterComponent>(row) };
                    renderer.add_water(
                        wt.0,
                        somnium_renderer::pass::water::WaterMaterialData {
                            deep_color: water.deep_color,
                            shallow_color: water.shallow_color,
                            edge_color: water.edge_color,
                            clarity: water.clarity,
                            edge_scale: water.edge_scale,
                            amplitude: water.amplitude,
                            _pad0: 0.0,
                            coord_scale: water.coord_scale,
                            coord_offset: water.coord_offset,
                            wave_dir_a: water.wave_dir_a,
                            wave_dir_b: water.wave_dir_b,
                            wave_blend: water.wave_blend,
                            _pad1: [0.0; 3],
                        },
                        mesh.vertex_offset,
                        mesh.index_offset,
                        mesh.index_count,
                    );
                }
            }
        }

        // FPS counter
        ctx.ui.send_message("update_fps", ctx.time.fps());

        // Phase 11.5A-3: Outliner with depth/parent hierarchy (every 60 frames).
        if ctx.time.frame_count() % 60 == 0 {
            let entities_payload = build_outliner_payload(ctx);
            ctx.ui.send_message("update_outliner", serde_json::json!({ "entities": entities_payload }));
        }

        // Phase 11.5C: Selection sync with component details.
        if let Some(selected) = *ctx.selected_entity {
            let name_req = ComponentSet::from_ids(vec![ComponentId::of::<Name>()]);
            let mut display_name = format!("Entity_{}", selected.index());
            'outer: for archetype in ctx.world.query_archetypes(&name_req, &ComponentSet::empty()) {
                let n_col = archetype.column_index(ComponentId::of::<Name>()).unwrap();
                for row in 0..archetype.len() {
                    if archetype.entities()[row] == selected {
                        let name = unsafe { archetype.column(n_col).get::<Name>(row) };
                        display_name = name.as_str().to_string();
                        break 'outer;
                    }
                }
            }

            // Build detailed component data for the inspector.
            let transform_data = ctx.world.get::<Transform>(selected).map(|t| {
                let euler = t.rotation.to_euler(glam::EulerRot::XYZ);
                serde_json::json!({
                    "translation": [t.translation.x, t.translation.y, t.translation.z],
                    "rotation":    [euler.0.to_degrees(), euler.1.to_degrees(), euler.2.to_degrees()],
                    "scale":       [t.scale.x, t.scale.y, t.scale.z],
                })
            });

            let light_data = ctx.world.get::<LightComponent>(selected).map(|lc| {
                serde_json::json!({
                    "light_type": format!("{:?}", lc.light_type),
                    "color":      [lc.color.x, lc.color.y, lc.color.z],
                    "intensity":  lc.intensity,
                })
            });

            let mesh_data = ctx.world.get::<MeshComponent>(selected).map(|mc| {
                serde_json::json!({
                    "vertex_offset": mc.vertex_offset,
                    "index_offset":  mc.index_offset,
                    "index_count":   mc.index_count,
                })
            });

            ctx.ui.send_message("update_selection", serde_json::json!({
                "index":     selected.index(),
                "name":      display_name,
                "transform": transform_data,
                "light":     light_data,
                "mesh":      mesh_data,
            }));
        } else {
            ctx.ui.send_message("update_selection", serde_json::Value::Null);
        }
    }

    fn on_shutdown(&mut self) {
        info!("HelloGame shutting down — goodbye!");
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Outliner payload builder (Phase 11.5A-3)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn build_outliner_payload(ctx: &EngineContext) -> Vec<OutlinerEntity> {
    // Collect name map and parent map.
    let mut name_map:   std::collections::HashMap<u32, String>      = Default::default();
    let mut parent_map: std::collections::HashMap<u32, Option<u32>> = Default::default();

    let name_req = ComponentSet::from_ids(vec![ComponentId::of::<Name>()]);
    for arch in ctx.world.query_archetypes(&name_req, &ComponentSet::empty()) {
        let n_col = arch.column_index(ComponentId::of::<Name>()).unwrap();
        for row in 0..arch.len() {
            let name   = unsafe { arch.column(n_col).get::<Name>(row) };
            let entity = arch.entities()[row];
            name_map.insert(entity.index(), name.as_str().to_string());
        }
    }

    let parent_req = ComponentSet::from_ids(vec![ComponentId::of::<Parent>()]);
    for arch in ctx.world.query_archetypes(&parent_req, &ComponentSet::empty()) {
        let p_col = arch.column_index(ComponentId::of::<Parent>()).unwrap();
        for row in 0..arch.len() {
            let parent = unsafe { arch.column(p_col).get::<Parent>(row) };
            let entity = arch.entities()[row];
            // Only record if parent entity is not DANGLING sentinel.
            if parent.entity != Entity::DANGLING {
                parent_map.insert(entity.index(), Some(parent.entity.index()));
            } else {
                parent_map.insert(entity.index(), None);
            }
        }
    }

    // BFS: roots first, then children in order.
    let mut depth_map: std::collections::HashMap<u32, u32> = Default::default();
    let mut result: Vec<OutlinerEntity> = Vec::new();

    // Collect all entity indices, sorted for determinism.
    let mut all_indices: Vec<u32> = ctx.world.entities().map(|e| e.index()).collect();
    all_indices.sort_unstable();

    // Roots first.
    for &idx in &all_indices {
        if parent_map.get(&idx).copied().flatten().is_none() {
            depth_map.insert(idx, 0);
            result.push(OutlinerEntity {
                name:   name_map.get(&idx).cloned().unwrap_or_else(|| format!("Entity_{idx}")),
                index:  idx,
                parent: None,
                depth:  0,
            });
        }
    }

    // Children (up to 8 levels deep).
    for _ in 0..8 {
        let prev_len = result.len();
        for &idx in &all_indices {
            if let Some(Some(p_idx)) = parent_map.get(&idx) {
                if depth_map.contains_key(p_idx) && !depth_map.contains_key(&idx) {
                    let d = depth_map[p_idx] + 1;
                    depth_map.insert(idx, d);
                    result.push(OutlinerEntity {
                        name:   name_map.get(&idx).cloned().unwrap_or_else(|| format!("Entity_{idx}")),
                        index:  idx,
                        parent: Some(*p_idx),
                        depth:  d,
                    });
                }
            }
        }
        if result.len() == prev_len { break; }
    }

    result
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Procedural fallback scene
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn spawn_procedural_scene(ctx: &mut EngineContext) -> (u32, somnium_renderer::geometry::MeshAllocation) {
    let renderer   = ctx.renderer.as_mut().unwrap();
    let render_ctx = ctx.render_ctx.as_ref().unwrap();

    let mat_blue = renderer.materials_pool.add_material(
        &render_ctx.queue,
        somnium_renderer::material::pool::GpuMaterial {
            base_color: [0.1, 0.1, 0.15, 1.0],
            roughness: 0.8, metallic: 0.0,
            albedo_map: -1, normal_map: -1, metallic_roughness_map: -1, _padding: [0; 3],
        },
    );
    let mat_red = renderer.materials_pool.add_material(
        &render_ctx.queue,
        somnium_renderer::material::pool::GpuMaterial {
            base_color: [0.8, 0.1, 0.1, 1.0],
            roughness: 0.2, metallic: 0.8,
            albedo_map: -1, normal_map: -1, metallic_roughness_map: -1, _padding: [0; 3],
        },
    );

    // Use the procedural cube from somnium_asset (Phase 11.5D-2).
    let (cube_verts, cube_idxs) = somnium_asset::generate_cube(1.0);
    let cube_alloc = renderer.geometry.upload_mesh(&render_ctx.queue, &cube_verts, &cube_idxs, mat_blue);

    // Floor
    ctx.physics.create_body(RigidBodyDescriptor {
        shape: ColliderShape::Box { half_extents: Vec3::new(10.0, 0.1, 10.0) },
        position: Vec3::new(0.0, -1.0, 0.0),
        motion_type: MotionType::Static,
        object_layer: LAYER_NON_MOVING,
        ..Default::default()
    });
    ctx.world.spawn((
        Transform::from_translation(Vec3::new(0.0, -1.0, 0.0)),
        MeshComponent { vertex_offset: cube_alloc.vertex_offset, index_offset: cube_alloc.index_offset, index_count: cube_alloc.index_count },
        MaterialComponent { id: mat_blue },
        Name::new("Floor"),
        WorldTransform::identity(),
        MeshKind::Cube,
    ));

    // Player cube (physics-driven)
    let player_body = ctx.physics.create_body(RigidBodyDescriptor {
        shape: ColliderShape::Box { half_extents: Vec3::new(0.5, 0.5, 0.5) },
        position: Vec3::new(0.0, 5.0, 0.0),
        motion_type: MotionType::Dynamic,
        object_layer: LAYER_MOVING,
        ..Default::default()
    });
    ctx.world.spawn((
        Transform::from_translation(Vec3::new(0.0, 5.0, 0.0)),
        PhysicsBody { id: player_body },
        MeshComponent { vertex_offset: cube_alloc.vertex_offset, index_offset: cube_alloc.index_offset, index_count: cube_alloc.index_count },
        MaterialComponent { id: mat_red },
        Name::new("Player"),
        WorldTransform::identity(),
        MeshKind::Cube,
    ));

    // Static PBR metal cube
    let pbr_mat = {
        let renderer   = ctx.renderer.as_mut().unwrap();
        let render_ctx = ctx.render_ctx.as_ref().unwrap();
        renderer.materials_pool.add_material(
            &render_ctx.queue,
            somnium_renderer::material::pool::GpuMaterial {
                base_color: [1.0, 1.0, 1.0, 1.0],
                roughness: 0.1, metallic: 0.9,
                albedo_map: -1, normal_map: -1, metallic_roughness_map: -1, _padding: [0; 3],
            },
        )
    };
    ctx.world.spawn((
        Transform::from_translation(Vec3::new(2.0, 1.0, 0.0)),
        MeshComponent { vertex_offset: cube_alloc.vertex_offset, index_offset: cube_alloc.index_offset, index_count: cube_alloc.index_count },
        MaterialComponent { id: pbr_mat },
        Name::new("MetalCube"),
        WorldTransform::identity(),
        MeshKind::Cube,
    ));

    (mat_blue, cube_alloc)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Content browser helper
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn list_assets_dir() -> Vec<serde_json::Value> {
    let Ok(entries) = std::fs::read_dir("assets") else { return Vec::new() };
    entries.filter_map(|e| {
        let e = e.ok()?;
        let name = e.file_name().to_string_lossy().to_string();
        let is_dir = e.file_type().ok()?.is_dir();
        let ext = std::path::Path::new(&name).extension()
            .and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
        Some(serde_json::json!({ "name": name, "is_dir": is_dir, "ext": ext }))
    }).collect()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Entry point
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn main() -> Result<(), somnium_core::EngineError> {
    let config = EngineConfig {
        window_title: "Somnium Engine — glTF Demo".into(),
        window_size: (1280, 720),
        target_fps: Some(60),
        ..Default::default()
    };
    Engine::run(config, HelloGame::new())
}
