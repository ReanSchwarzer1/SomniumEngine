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
            sensitivity: 0.1,
            is_rmb_down: false,
            move_forward: false, move_backward: false,
            move_left: false, move_right: false,
            move_up: false, move_down: false,
            is_shifting: false,
        }
    }

    /// `base_speed` is the editor camera speed from the viewport toolbar
    /// (Phase 20B) — the camera no longer owns it, so the slider and the
    /// RMB+wheel shortcut both drive movement directly.
    fn update(&mut self, dt: f32, base_speed: f32) {
        if !self.is_rmb_down { return; }
        let speed = if self.is_shifting { base_speed * 3.0 } else { base_speed };
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
#[allow(dead_code)] // Paused is a planned play-mode state, not yet wired up.
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
                normal_map: -1, metallic_roughness_map: -1, alpha_cutoff: 0.0,
                flags: 0,
                occlusion_map: -1,
                transmission: 0.0,
                emissive: [0.0; 3],
                emissive_map: -1,
                terrain_index: -1,
                _pad: [0.0; 2],
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

    /// Release every chunk's GPU allocation back to the geometry pool.
    ///
    /// Called when the voxel-terrain entity is deleted — without this the
    /// chunk meshes would leak the pool until the app exits.
    fn free_all(&mut self, renderer: &mut somnium_renderer::SomniumRenderer) {
        for (_, entry) in self.chunks.drain() {
            if let Some(alloc) = entry {
                renderer.geometry.free_mesh(alloc);
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

    /// Keep the voxel streaming driver in sync with the ECS.
    ///
    /// The voxel world is created from **Create > Voxel Terrain**, which spawns
    /// an entity carrying `VoxelTerrainComponent`. Chunks themselves are not
    /// entities (they stream constantly), so the driver lives here in game code
    /// and is built/torn down to follow that single entity.
    fn sync_voxel_terrain(&mut self, ctx: &mut EngineContext) {
        let wants_voxel = ctx
            .world
            .entities()
            .any(|e| ctx.world.get::<somnium_core::VoxelTerrainComponent>(e).is_some());

        match (wants_voxel, self.voxel_terrain.is_some()) {
            // Entity appeared — spin up the streaming driver.
            (true, false) => {
                if let (Some(renderer), Some(render_ctx)) = (&mut ctx.renderer, &ctx.render_ctx) {
                    let vt = VoxelTerrain::new(renderer, render_ctx);
                    info!(
                        "Voxel terrain created (radius {} chunks)",
                        vt.world.config().radius_chunks
                    );
                    self.voxel_terrain = Some(vt);
                }
            }
            // Entity deleted — free the chunk meshes and drop the driver.
            (false, true) => {
                if let (Some(vt), Some(renderer)) = (&mut self.voxel_terrain, &mut ctx.renderer) {
                    vt.free_all(renderer);
                }
                self.voxel_terrain = None;
                info!("Voxel terrain removed");
            }
            _ => {}
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



        // Phase 14: the voxel world is no longer spawned automatically — create
        // it from the editor via Create > Voxel Terrain. `sync_voxel_terrain`
        // builds the streaming driver when that entity appears.

        // Phase 24K: shadow smoke test. A wide ground plane with a cube above
        // it, both through the visibility buffer — which is what the traced
        // path can currently see, since terrain and water write depth in their
        // own later passes.
        //
        // In code rather than driven through the editor UI: building this by
        // synthesising clicks was attempted twice and produced scenes that
        // looked plausible but were not what they claimed, which is worse than
        // no test. Spawned with MeshKind only, so the same auto-attach path the
        // editor's Create menu uses supplies the mesh and material.
        if std::env::var("SOMNIUM_SHADOWTEST").is_ok() {
            // Remove the demo's water and helmet. Both sit at Y=0 and both have
            // confounded this test repeatedly — the water plane is coplanar with
            // any ground at Y=0 and z-fights it, and the helmet sits below a
            // raised ground so it cannot cast onto it. What is left is a plane
            // and a cube and nothing else, so a shadow either appears or does not.
            let name_req = ComponentSet::from_ids(vec![ComponentId::of::<Name>()]);
            let mut doomed = Vec::new();
            for archetype in ctx.world.query_archetypes(&name_req, &ComponentSet::empty()) {
                let n_col = archetype.column_index(ComponentId::of::<Name>()).unwrap();
                for row in 0..archetype.len() {
                    let n = unsafe { archetype.column(n_col).get::<Name>(row) };
                    let s = n.as_str();
                    if s == "WaterPlane" || s.contains("Helmet") || s.contains("helmet") {
                        doomed.push(archetype.entities()[row]);
                    }
                }
            }
            for e in doomed {
                ctx.world.despawn(e);
            }

            ctx.world.spawn((
                Transform {
                    // Y=6, not 0: the demo's WaterPlane also sits at Y=0, and
                    // two coplanar surfaces z-fight — which made every capture
                    // of this scene a mix of both.
                    translation: Vec3::new(0.0, 0.0, 0.0),
                    rotation: glam::Quat::IDENTITY,
                    scale: Vec3::new(40.0, 1.0, 40.0),
                },
                Name::new("ShadowGround"),
                WorldTransform::identity(),
                MeshKind::Plane,
            ));
            ctx.world.spawn((
                Transform {
                    translation: Vec3::new(0.0, 3.0, 0.0),
                    rotation: glam::Quat::IDENTITY,
                    scale: Vec3::splat(2.0),
                },
                Name::new("ShadowCaster"),
                WorldTransform::identity(),
                MeshKind::Cube,
            ));

            // A second cube high against the sky. If this one is visible and the
            // low one is not, the low cube is rendering but has no contrast
            // against the plane; if neither shows, it is not being drawn.
            ctx.world.spawn((
                Transform {
                    translation: Vec3::new(-6.0, 14.0, 0.0),
                    rotation: glam::Quat::IDENTITY,
                    scale: Vec3::splat(2.0),
                },
                Name::new("SkyCube"),
                WorldTransform::identity(),
                MeshKind::Cube,
            ));

            // Look down at the ground so the cast shadow fills the frame.
            self.camera.position = Vec3::new(0.0, 3.5, 12.0);
            self.camera.yaw = -90.0;
            self.camera.pitch = -3.0;
        }

        // Phase 14 (SSS): heightmap terrain smoke test — exercises chunk
        // meshing, LODs, sculpt brushes, and auto-splat without editor input.
        // Normally terrain is created via Create > Terrain in the editor.
        // `SOMNIUM_TERRAIN=flat` reproduces **Create > Terrain** instead: the
        // default 16x16-chunk descriptor, no sculpting, spawned at y = 0. The
        // sculpted 4x4 variant is not a substitute — it was the only thing
        // 25A-2 was verified against, and the editor-created terrain turned out
        // not to render even though that one did.
        // Terrain is part of the default scene (Phase 25L). `SOMNIUM_TERRAIN`
        // now selects a variant rather than enabling it:
        //   unset / "flat" — the editor's own **Create > Terrain** geometry: the
        //       default 16x16-chunk descriptor at y = 0, with a heightmap and
        //       all eight materials auto-splatted by altitude and slope.
        //   "1"            — the legacy sculpted 4x4 smoke test, kept because it
        //       is what exercises the brush paths without editor input.
        //   "0" / "none"   — no terrain, for isolating everything else.
        //
        // The sculpted variant is deliberately not the default: it was the only
        // thing 25A-2 was verified against, and the editor-created terrain
        // turned out not to render at all even though that one did.
        let terrain_mode = std::env::var("SOMNIUM_TERRAIN").unwrap_or_default();
        let flat_terrain = terrain_mode != "1";
        if terrain_mode != "0" && terrain_mode != "none" {
            if let (Some(renderer), Some(render_ctx)) = (&mut ctx.renderer, &ctx.render_ctx) {
                use somnium_renderer::terrain::{brush, TerrainDescriptor};
                let desc = if flat_terrain {
                    TerrainDescriptor::default()
                } else {
                    TerrainDescriptor { grid_size: [4, 4], ..Default::default() }
                };
                let terrain_id = renderer.create_terrain(render_ctx, desc);
                let [wx, wz] = desc.world_size();

                // Phase 25L: real relief, so the eight materials have altitudes
                // and slopes to be assigned against. `SOMNIUM_HEIGHTMAP=<path>`
                // loads a file (16-bit PNG, or CDLOD's `.tbmp`); otherwise the
                // terrain gets procedural FBM hills, which is still landscape
                // rather than the flat plain every earlier test scene used.
                let mut foliage_camera: Option<Vec3> = None;
                if flat_terrain {
                    let amplitude = std::env::var("SOMNIUM_TERRAIN_RELIEF")
                        .ok()
                        .and_then(|v| v.parse::<f32>().ok())
                        .unwrap_or(somnium_renderer::terrain::DEFAULT_RELIEF_METRES);
                    if let Some(terrain) = renderer.terrain_mut(terrain_id) {
                        // The same path Create > Terrain takes, so the demo
                        // scene and an editor-created terrain cannot diverge.
                        terrain.apply_default_relief(amplitude);
                        // Assign all eight materials by altitude and slope.
                        brush::auto_splat(terrain, amplitude * 0.62);

                        // `SOMNIUM_FOLIAGE=1` scatters foliage without the
                        // editor (Phase 17E). Painting by hand was the only way
                        // to get a plant on screen, which meant the foliage
                        // shading work could not be *seen*, let alone measured
                        // — the reason the 17E remainder sat open. Strokes are
                        // deterministic, so an A/B of a shading change is a
                        // like-for-like comparison.
                        if std::env::var("SOMNIUM_FOLIAGE").as_deref() == Ok("1") {
                            use somnium_renderer::terrain::foliage_paint::{self, FoliageBrush};
                            let [wx, wz] = desc.world_size();
                            let mut painted = std::mem::take(&mut terrain.painted_foliage);
                            let mut stroke = 0u32;

                            // Ground cover over a patch in front of the camera,
                            // then a few trees standing in it.
                            for (kind, radius, density, single, count) in
                                [(0u8, 26.0f32, 2.5f32, false, 12), (1u8, 1.0, 1.0, true, 6)]
                            {
                                let brush = FoliageBrush {
                                    kind,
                                    radius,
                                    density,
                                    single,
                                    max_slope_deg: 35.0,
                                    ..Default::default()
                                };
                                for i in 0..count {
                                    let t = i as f32 / count as f32;
                                    let cx = wx * (0.34 + 0.30 * t);
                                    let cz = wz * (0.40 + 0.16 * ((i % 4) as f32 / 4.0));
                                    stroke += 1;
                                    foliage_paint::paint(
                                        &mut painted, &brush, [cx, cz], stroke,
                                        |x, z| terrain.ground_sample(x, z),
                                    );
                                }
                            }
                            info!("Foliage scattered: {} instances", painted.len());
                            terrain.painted_foliage = painted;

                            // Stand in it. Foliage is culled past 120 m (17G)
                            // and a tuft is sub-pixel from the landscape camera,
                            // so judging leaf shading needs eye level.
                            let (lx, lz) = (wx * 0.34, wz * 0.40 + 14.0);
                            let ground = terrain.world_height_at(lx, lz);
                            foliage_camera = Some(Vec3::new(lx - wx * 0.5, ground + 1.6, lz - wz * 0.5));
                        }
                    }
                    // The default camera sits at y = 2, which is now inside a
                    // hillside rather than above a plain. Put it over the
                    // terrain looking down the slope.
                    self.camera.position = foliage_camera.unwrap_or_else(|| {
                        Vec3::new(0.0, amplitude * 1.15 + 30.0, wz * 0.45)
                    });
                    self.camera.yaw = -90.0;
                    self.camera.pitch = if foliage_camera.is_some() { -6.0 } else { -22.0 };
                }

                if let Some(terrain) = renderer.terrain_mut(terrain_id).filter(|_| !flat_terrain) {
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
                    Transform::from_translation(Vec3::new(
                        -wx * 0.5,
                        if flat_terrain { 0.0 } else { -6.0 },
                        -wz * 0.5,
                    )),
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
                    // Phase 17E: painted foliage only submits when the
                    // terrain entity carries an enabled FoliageComponent.
                    somnium_core::FoliageComponent {
                        enabled: foliage_camera.is_some(),
                        ..Default::default()
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
            LightComponent::directional(somnium_core::light_units::lux::DIRECT_SUNLIGHT),
            Name::new("SunLight"),
            WorldTransform::identity(),
        ));

        // Phase 13C/13E: a point and a spot light so clustered local lighting is
        // visible in the demo — and so the light gizmos (L) have something to
        // draw beyond the sun. Both sit near the origin scene.
        ctx.world.spawn((
            Transform::from_translation(Vec3::new(4.0, 3.0, 2.0)),
            LightComponent::point(somnium_core::light_units::lumens::BULB_100W, 12.0),
            Name::new("PointLight"),
            WorldTransform::identity(),
        ));
        ctx.world.spawn((
            Transform {
                translation: Vec3::new(-4.0, 6.0, 1.0),
                // -Z forward rotated to aim (mostly) straight down.
                rotation: glam::Quat::from_rotation_x((-75.0_f32).to_radians()),
                scale: Vec3::ONE,
            },
            LightComponent::spot(
                somnium_core::light_units::lumens::FLOODLIGHT, 20.0,
                20.0_f32.to_radians(),
                30.0_f32.to_radians(),
            ),
            Name::new("SpotLight"),
            WorldTransform::identity(),
        ));

        // Test hook: import a model at startup without clicking through the
        // UI. `SOMNIUM_IMPORT=<path> cargo run -p hello_engine`.
        if let Ok(import_path) = std::env::var("SOMNIUM_IMPORT") {
            if let (Some(renderer), Some(render_ctx)) = (&mut ctx.renderer, &ctx.render_ctx) {
                match somnium_asset::load_gltf(&import_path) {
                    Ok(scene) => {
                        let uploaded = renderer.upload_scene(render_ctx, &scene);
                        info!("IMPORT: {} nodes from {}", uploaded.len(), import_path);
                        for n in uploaded.iter() {
                            let (sc, rot, tr) = n.transform.to_scale_rotation_translation();
                            ctx.world.spawn((
                                Transform { translation: tr, rotation: rot, scale: sc },
                                Name::new(&n.entity_name),
                                WorldTransform::identity(),
                                MeshComponent {
                                    vertex_offset: n.vertex_offset,
                                    index_offset: n.index_offset,
                                    index_count: n.index_count,
                                },
                                MaterialComponent { id: n.material_id },
                            ));
                        }
                    }
                    Err(e) => info!("IMPORT failed: {e}"),
                }
            }
        }

        // Phase 15A1: scene-wide post-processing settings, selectable in the
        // outliner. All effects start off — the viewport shows the raw image
        // until a look is dialled in.
        ctx.world.spawn((
            Transform::from_translation(Vec3::ZERO),
            Name::new("Post Processing"),
            WorldTransform::identity(),
            somnium_core::PostProcessComponent::default(),
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
                    // F9: A/B the GPU-driven indirect draw path (Phase 15A).
                    // Both paths must render identically — if the image changes,
                    // the indirect arguments are wrong.
                    KeyCode::F9 if pressed => {
                        if let Some(renderer) = &mut ctx.renderer {
                            if renderer.supports_gpu_driven() {
                                let on = renderer.toggle_gpu_driven();
                                info!(
                                    "Draw path: {}",
                                    if on { "GPU-driven (multi-draw indirect)" } else { "CPU (per-draw)" }
                                );
                            } else {
                                info!("GPU-driven draw path not supported on this device");
                            }
                        }
                    }
                    // F10: A/B GPU frustum culling (Phase 15B). A correct cull
                    // is invisible — if geometry pops, the cull is wrong.
                    KeyCode::F10 if pressed => {
                        if let Some(renderer) = &mut ctx.renderer {
                            let on = renderer.toggle_culling();
                            info!("GPU frustum culling: {}", if on { "ON" } else { "off" });
                        }
                    }
                    // L: toggle light gizmos (Phase 13E)
                    KeyCode::KeyL if pressed => {
                        if let Some(renderer) = &mut ctx.renderer {
                            let on = renderer.toggle_light_gizmos();
                            info!("Light gizmos: {}", if on { "ON" } else { "off" });
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

            // RMB + scroll wheel adjusts fly speed, matching UE5's muscle
            // memory. Steps are multiplicative so the feel is the same at
            // 0.5 m/s and at 500 m/s.
            EngineEvent::MouseWheel { delta_y } => {
                if self.camera.is_rmb_down {
                    let current = ctx.camera_speed;
                    let scaled = current * 1.15_f32.powf(*delta_y as f32);
                    let norm = somnium_core::normalized_from_camera_speed(scaled);
                    ctx.set_camera_speed(norm);
                }
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

        self.camera.update(dt, ctx.camera_speed);
        self.log_timer += dt;

        // Phase 14: create/destroy the voxel driver to match the ECS, then
        // stream chunks around the camera (async generation; finished meshes
        // are uploaded here, freed allocations recycled).
        self.sync_voxel_terrain(ctx);
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
                        
                        // Two different conventions, easy to mix up:
                        //  * `forward` — the direction light TRAVELS (entity -Z).
                        //    This is the spot cone's axis: the shader tests
                        //    dot(-L, direction_ws) with -L pointing light→surface.
                        //  * `to_light` — the direction TOWARD the light, which is
                        //    what the directional BRDF wants for N·L.
                        // Passing `to_light` as the spot axis aimed the cone 180°
                        // away from where the gizmo (correctly) drew it.
                        let forward = transform.rotation.mul_vec3(glam::Vec3::NEG_Z);
                        let to_light = -forward;

                        match light.light_type {
                            LightType::Directional => {
                                renderer.set_directional_light(to_light, light.photometric_color());
                            }
                            LightType::Point | LightType::Spot => {
                                let l_type = if light.light_type == LightType::Point { 0 } else { 1 };
                                renderer.submit_local_light(somnium_renderer::cluster::GpuLocalLight {
                                    position_ws: transform.translation.to_array(),
                                    range: light.range,
                                    color: light.photometric_color().to_array(),
                                    light_type: l_type,
                                    // Spot axis = travel direction. Unused for point lights.
                                    direction_ws: forward.to_array(),
                                    spot_cos_outer: light.outer_angle.cos(),
                                    spot_cos_inner: light.inner_angle.cos(),
                                    radius: light.source_radius,
                                    _pad: [0.0; 2],
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
                        albedo_map: -1, normal_map: -1, metallic_roughness_map: -1, alpha_cutoff: 0.0,
                flags: 0,
                occlusion_map: -1,
                transmission: 0.0,
                emissive: [0.0; 3],
                emissive_map: -1,
                terrain_index: -1,
                _pad: [0.0; 2],
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
                    
                    if std::env::var("SOMNIUM_SHADOWTEST").is_ok() {
                        tracing::info!(
                            "shadowtest attach: kind={:?} vtx_off={} idx_off={} idx_count={} mat={}",
                            kind, alloc.vertex_offset, alloc.index_offset, alloc.index_count, default_mat
                        );
                    }

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
            albedo_map: -1, normal_map: -1, metallic_roughness_map: -1, alpha_cutoff: 0.0,
                flags: 0,
                occlusion_map: -1,
                transmission: 0.0,
                emissive: [0.0; 3],
                emissive_map: -1,
                terrain_index: -1,
                _pad: [0.0; 2],
        },
    );
    let mat_red = renderer.materials_pool.add_material(
        &render_ctx.queue,
        somnium_renderer::material::pool::GpuMaterial {
            base_color: [0.8, 0.1, 0.1, 1.0],
            roughness: 0.2, metallic: 0.8,
            albedo_map: -1, normal_map: -1, metallic_roughness_map: -1, alpha_cutoff: 0.0,
                flags: 0,
                occlusion_map: -1,
                transmission: 0.0,
                emissive: [0.0; 3],
                emissive_map: -1,
                terrain_index: -1,
                _pad: [0.0; 2],
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
                albedo_map: -1, normal_map: -1, metallic_roughness_map: -1, alpha_cutoff: 0.0,
                flags: 0,
                occlusion_map: -1,
                transmission: 0.0,
                emissive: [0.0; 3],
                emissive_map: -1,
                terrain_index: -1,
                _pad: [0.0; 2],
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
