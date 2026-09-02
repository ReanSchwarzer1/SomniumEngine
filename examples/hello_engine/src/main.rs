//! # Hello Somnium Engine â€” glTF Demo
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

mod dreams_fixture;

use glam::Vec3;
use serde::Serialize;
use somnium_core::{
    AudioAttenuationModel, AudioEmitterComponent, BuoyantVessel, CameraSettingsComponent, Children,
    Component, ComponentId, ComponentSet, EditorFlags, Engine, EngineConfig, EngineContext,
    EngineEvent, Entity, GameApp, GameUiFrame, InputState, KeyCode, LightComponent,
    LightShadowTechnique, LightType, MapKind, MapLoadResult, MaterialComponent, MeshComponent,
    MeshKind, Name, Parent, SimulationState, Transform, UiCanvasComponent, UiCanvasSpace,
    WorldTransform, camera_view_from_world, propagate_transforms,
};
use somnium_physics::body::{BodyId, MotionType, RigidBodyDescriptor};
use somnium_physics::layer::{LAYER_MOVING, LAYER_NON_MOVING};
use somnium_physics::shape::ColliderShape;
use tracing::info;

// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”
// Components
// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”

#[derive(Debug, Clone, Copy)]
struct PhysicsBody {
    id: BodyId,
}
impl Component for PhysicsBody {}

#[derive(Debug, Clone, Copy)]
struct BoatPart {
    vertex_offset: u32,
    index_offset: u32,
    index_count: u32,
    material_id: u32,
    local_transform: glam::Mat4,
}

#[derive(Debug, Clone)]
struct BoatRuntime {
    entity: Entity,
    body: BodyId,
    water_id: u32,
    water_origin: Vec3,
    initial_position: Vec3,
    initial_rotation: glam::Quat,
    parts: Vec<BoatPart>,
}

#[derive(Serialize)]
struct OutlinerEntity {
    name: String,
    index: u32,
    parent: Option<u32>,
    depth: u32,
}

// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”
// Camera
// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”

struct EditorCamera {
    position: Vec3,
    yaw: f32,
    pitch: f32,
    sensitivity: f32,
    is_rmb_down: bool,
    move_forward: bool,
    move_backward: bool,
    move_left: bool,
    move_right: bool,
    move_up: bool,
    move_down: bool,
    is_shifting: bool,
}

impl EditorCamera {
    fn new(position: Vec3) -> Self {
        Self {
            position,
            yaw: -90.0,
            pitch: -20.0,
            sensitivity: 0.1,
            is_rmb_down: false,
            move_forward: false,
            move_backward: false,
            move_left: false,
            move_right: false,
            move_up: false,
            move_down: false,
            is_shifting: false,
        }
    }

    /// `base_speed` is the editor camera speed from the viewport toolbar
    /// (Phase 20B) â€” the camera no longer owns it, so the slider and the
    /// RMB+wheel shortcut both drive movement directly.
    fn update(&mut self, dt: f32, base_speed: f32) {
        if !self.is_rmb_down {
            return;
        }
        let speed = if self.is_shifting {
            base_speed * 3.0
        } else {
            base_speed
        };
        let forward = self.forward_vector();
        let right = forward.cross(Vec3::Y).normalize();
        if self.move_forward {
            self.position += forward * speed * dt;
        }
        if self.move_backward {
            self.position -= forward * speed * dt;
        }
        if self.move_right {
            self.position += right * speed * dt;
        }
        if self.move_left {
            self.position -= right * speed * dt;
        }
        if self.move_up {
            self.position += Vec3::Y * speed * dt;
        }
        if self.move_down {
            self.position -= Vec3::Y * speed * dt;
        }
    }

    fn forward_vector(&self) -> Vec3 {
        let yaw = self.yaw.to_radians();
        let pitch = self.pitch.to_radians();
        Vec3::new(
            yaw.cos() * pitch.cos(),
            pitch.sin(),
            yaw.sin() * pitch.cos(),
        )
        .normalize()
    }

    fn view_matrix(&self) -> glam::Mat4 {
        glam::Mat4::look_at_rh(
            self.position,
            self.position + self.forward_vector(),
            Vec3::Y,
        )
    }

    /// Scene Camera actor pose: local `-Z` is the look direction.
    fn to_transform(&self) -> Transform {
        Transform {
            translation: self.position,
            rotation: somnium_core::look_rotation_neg_z(self.forward_vector()),
            scale: Vec3::ONE,
        }
    }

    fn look_at(&mut self, target: Vec3) {
        let d = target - self.position;
        if d.length_squared() < 1e-8 {
            return;
        }
        self.yaw = d.z.atan2(d.x).to_degrees();
        let horiz = (d.x * d.x + d.z * d.z).sqrt();
        self.pitch = d.y.atan2(horiz).to_degrees().clamp(-89.0, 89.0);
    }
}

fn play_session(ctx: &EngineContext) -> bool {
    ctx.simulation.state != SimulationState::Editing
}

/// World matrix of the Outliner Camera actor, if one exists.
fn scene_camera_world(world: &somnium_ecs::World) -> Option<glam::Mat4> {
    world.entities().find_map(|e| {
        world.get::<CameraSettingsComponent>(e)?;
        world
            .get::<WorldTransform>(e)
            .map(|wt| wt.0)
            .or_else(|| world.get::<Transform>(e).map(Transform::to_matrix))
    })
}

fn active_view(
    ctx: &EngineContext,
    editor: &EditorCamera,
    player: Option<&PlayerRuntime>,
) -> (glam::Mat4, Vec3) {
    if play_session(ctx) {
        // The scripted character's camera wins over the scene's Camera
        // actor. Both carry `CameraSettingsComponent`, so picking by
        // component alone would be a coin toss decided by archetype
        // order â€” this is explicit instead.
        if let Some(runtime) = player {
            if let Some(world) = ctx
                .world
                .get::<WorldTransform>(runtime.camera)
                .map(|wt| wt.0)
            {
                return camera_view_from_world(world);
            }
        }
        if let Some(world) = scene_camera_world(ctx.world) {
            return camera_view_from_world(world);
        }
    }
    (editor.view_matrix(), editor.position)
}

fn parse_vec3(s: &str) -> Option<Vec3> {
    let mut parts = s.split(',');
    Some(Vec3::new(
        parts.next()?.trim().parse().ok()?,
        parts.next()?.trim().parse().ok()?,
        parts.next()?.trim().parse().ok()?,
    ))
}

/// Place the editor camera for an XV-J kit capture (`SOMNIUM_KIT_VIEW`).
fn apply_kit_view(
    camera: &mut EditorCamera,
    terrain: &somnium_renderer::terrain::TerrainData,
    origin: Vec3,
    view: &str,
) {
    use somnium_renderer::terrain::DEFAULT_WATER_LEVEL_METRES;
    let [wx, wz] = terrain.desc.world_size();
    let water = DEFAULT_WATER_LEVEL_METRES;
    let to_world = |lx: f32, ly: f32, lz: f32| origin + Vec3::new(lx, ly, lz);

    let mut best = (f32::NEG_INFINITY, wx * 0.5, wz * 0.5, 0.0f32);
    let step = 8u32.max(1);
    let sw = terrain.splatmap.width;
    let sh = terrain.splatmap.height;
    for tz in (0..sh).step_by(step as usize) {
        for tx in (0..sw).step_by(step as usize) {
            let lx = (tx as f32 + 0.5) / sw as f32 * wx;
            let lz = (tz as f32 + 0.5) / sh as f32 * wz;
            let h = terrain.world_height_at(lx, lz);
            let slope = terrain.ground_sample(lx, lz).slope_cos;
            let idx = (tz * sw + tx) as usize;
            let texel = terrain.splatmap.data.get(idx).copied().unwrap_or([0; 32]);
            let w = |layer: usize| texel[layer] as f32;
            let score = match view {
                "walk" | "eye" => {
                    if h <= water + 1.5 || slope < 0.85 {
                        f32::NEG_INFINITY
                    } else {
                        1.0 - (h - (water + 8.0)).abs() * 0.02
                    }
                }
                "shore" => {
                    if h < water - 0.4 || h > water + 3.5 || slope < 0.7 {
                        f32::NEG_INFINITY
                    } else {
                        w(8) + w(9) + w(6) - (h - water).abs() * 40.0
                    }
                }
                "cliff" => {
                    if h < water + 4.0 {
                        f32::NEG_INFINITY
                    } else {
                        (1.0 - slope) * 200.0 + w(14) + w(18) + w(19)
                    }
                }
                "snow" => w(3) + w(31) + (h - 50.0).max(0.0),
                "forest" => w(1) + w(17) + w(23),
                "mud" => w(5) + w(10) + w(22),
                "talus" => w(15) + w(26) + w(7),
                "red_clay" => w(11),
                "lush" => w(16) + w(24),
                "ridge" | "ridge-look" => {
                    if h <= water + 12.0 || slope < 0.75 {
                        f32::NEG_INFINITY
                    } else {
                        h + (1.0 - slope) * 4.0
                    }
                }
                // Phase TSUSHIMA-A. Height, and only height: a vista wants the
                // camera high enough that the frame is mostly ground at a
                // hundred metres and beyond. `ridge` is the near relative of
                // this and stands *on* the ridge at eye height; this one
                // stands well above it, because the whole subject of the
                // phase is what the ground looks like past the last cascade.
                "vista" => {
                    if h <= water + 6.0 {
                        f32::NEG_INFINITY
                    } else {
                        h
                    }
                }
                _ => f32::NEG_INFINITY,
            };
            if score > best.0 {
                best = (score, lx, lz, h);
            }
        }
    }
    let (_, lx, lz, h) = best;
    match view {
        "walk" | "eye" => {
            camera.position = to_world(lx, h + 1.7, lz);
            camera.yaw = -90.0;
            camera.pitch = -8.0;
        }
        "ridge" | "ridge-look" => {
            camera.position = to_world(lx, h + 1.7, lz);
            camera.yaw = 20.0;
            camera.pitch = -22.0;
        }
        "vista" => {
            // Face whichever bearing has the most land in front of it.
            //
            // A fixed yaw works on one map and points at open water on the
            // other, and a capture of the sea is not a capture of terrain.
            // Eight bearings, counting above-water samples out to 400 m, is
            // enough to find the long axis of either map and costs nothing
            // next to the scoring sweep that just ran.
            let mut best_yaw = -90.0f32;
            let mut best_land = -1.0f32;
            for i in 0..8 {
                let yaw = i as f32 * 45.0;
                let r = yaw.to_radians();
                let (dx, dz) = (r.cos(), r.sin());
                let mut land = 0.0f32;
                for step in 1..=40 {
                    let d = step as f32 * 10.0;
                    let sx = lx + dx * d;
                    let sz = lz + dz * d;
                    if sx < 0.0 || sz < 0.0 || sx >= wx || sz >= wz {
                        continue;
                    }
                    if terrain.world_height_at(sx, sz) > water + 0.5 {
                        land += 1.0;
                    }
                }
                if land > best_land {
                    best_land = land;
                    best_yaw = yaw;
                }
            }
            // Above the summit rather than on it, so the near ground does not
            // fill the lower half of the frame and steal the shot from the
            // distance the phase is actually about.
            camera.position = to_world(lx, h + 22.0, lz);
            camera.yaw = best_yaw;
            camera.pitch = -12.0;
        }
        "shore" => {
            camera.position = to_world(lx, h.max(water) + 1.8, lz);
            camera.look_at(Vec3::new(0.0, water + 0.4, 0.0));
        }
        "cliff" => {
            let d = terrain.desc.cell_size;
            let hx = terrain.world_height_at(lx + d, lz) - terrain.world_height_at(lx - d, lz);
            let hz = terrain.world_height_at(lx, lz + d) - terrain.world_height_at(lx, lz - d);
            let n = Vec3::new(-hx, 2.0 * d, -hz).normalize_or_zero();
            let along = Vec3::new(n.x, 0.0, n.z).normalize_or_zero();
            camera.position = to_world(lx, h, lz) + along * 28.0 + Vec3::Y * 8.0;
            camera.look_at(to_world(lx, h + 2.0, lz));
        }
        "snow" | "forest" | "mud" | "talus" | "red_clay" | "lush" => {
            camera.position = to_world(lx, h + 4.0, lz - 12.0);
            camera.look_at(to_world(lx, h, lz));
        }
        _ => {}
    }
    info!(view, score = best.0, height = h, "XV-J kit view placed");
}

/// The canonical viewpoint a timing run asked for (Phase DOOM-A).
///
/// One name rather than a handful of coordinates, because a measurement whose
/// viewpoint is a pasted position is a measurement nobody can reproduce six
/// weeks later. The three names match the vocabulary the existing evidence
/// already uses â€” DF-A's overview and walk, and the Island recipe.
///
/// | `SOMNIUM_TIME_VIEW` | map | camera |
/// |---|---|---|
/// | `coastal-overview` | Coastal | the recipe's own seed (XV overview) |
/// | `coastal-ground` | Coastal | walk / eye height on the terrain |
/// | `island` | Island | the recipe's own seed, `(0, 28, 115)` |
/// | `island-ground` | Island | walk / eye height |
/// Pin the sun's elevation, in degrees above the horizon (Phase TSUSHIMA-A).
///
/// Without this the sun comes from the directional light's authored transform
/// and there is no way to ask for a *low* one, which is the single condition
/// where TSUSHIMA-B's long shadows, TSUSHIMA-E's grazing relief and
/// TSUSHIMA-F's specular all show most. It is also what makes a low-sun shot
/// reproducible six weeks later rather than "whatever the preset happened to
/// be".
///
/// The azimuth is left alone deliberately: the map's authored sun bearing is
/// what puts shadows where the terrain was built to receive them, so this
/// rotates the sun in elevation only.
fn pinned_sun_elevation() -> Option<f32> {
    let raw = std::env::var("SOMNIUM_SUN_ELEVATION").ok()?;
    let degrees = raw.trim().parse::<f32>().ok()?;
    if !degrees.is_finite() {
        tracing::warn!("SOMNIUM_SUN_ELEVATION={raw} is not a number; ignoring");
        return None;
    }
    Some(degrees.clamp(-90.0, 90.0).to_radians())
}

/// Re-aim `to_light` at `elevation` while keeping its compass bearing.
fn sun_at_elevation(to_light: glam::Vec3, elevation: f32) -> glam::Vec3 {
    let horizontal = glam::Vec2::new(to_light.x, to_light.z);
    // A sun directly overhead has no bearing to keep. Pick one rather than
    // normalising a zero vector into NaN and blacking out the frame.
    let bearing = if horizontal.length_squared() < 1e-8 {
        glam::Vec2::new(1.0, 0.0)
    } else {
        horizontal.normalize()
    };
    let (sin_e, cos_e) = elevation.sin_cos();
    glam::Vec3::new(bearing.x * cos_e, sin_e, bearing.y * cos_e).normalize()
}

fn timing_view() -> Option<String> {
    let raw = std::env::var("SOMNIUM_TIME_VIEW").ok()?;
    let name = raw.trim().to_ascii_lowercase();
    if name.is_empty() { None } else { Some(name) }
}

/// Map implied by `SOMNIUM_MAP`, or failing that by `SOMNIUM_TIME_VIEW`.
fn timing_view_map() -> Option<MapKind> {
    if let Ok(explicit) = std::env::var("SOMNIUM_MAP") {
        match MapKind::parse(&explicit) {
            Ok(kind) => return Some(kind),
            Err(e) => tracing::warn!("SOMNIUM_MAP ignored: {e}"),
        }
    }
    let view = timing_view()?;
    if view.starts_with("island") {
        Some(MapKind::Island)
    } else if view.starts_with("coastal") {
        Some(MapKind::Coastal)
    } else {
        tracing::warn!("SOMNIUM_TIME_VIEW={view} does not name a map; using the default");
        None
    }
}

fn apply_capture_camera_overrides(
    camera: &mut EditorCamera,
    terrain: Option<&somnium_renderer::terrain::TerrainData>,
    origin: Vec3,
) {
    // Before the explicit overrides below, so `SOMNIUM_CAMERA_POS` can still
    // pin a one-off viewpoint on top of a named one.
    if let Some(view) = timing_view() {
        if view.ends_with("-ground") || view.ends_with("-walk") {
            match terrain {
                Some(terrain) => apply_kit_view(camera, terrain, origin, "walk"),
                None => tracing::warn!("SOMNIUM_TIME_VIEW={view} wants terrain, and there is none"),
            }
        } else if view.ends_with("-vista") {
            // Phase TSUSHIMA-A.
            match terrain {
                Some(terrain) => apply_kit_view(camera, terrain, origin, "vista"),
                None => tracing::warn!("SOMNIUM_TIME_VIEW={view} wants terrain, and there is none"),
            }
        }
        tracing::info!(view, "DOOM-A timing viewpoint");
    }
    if let Ok(view) = std::env::var("SOMNIUM_KIT_VIEW") {
        if let Some(terrain) = terrain {
            apply_kit_view(camera, terrain, origin, view.trim());
        }
    }
    if std::env::var("SOMNIUM_TERRAIN_EYE").as_deref() == Ok("1") {
        if let Some(terrain) = terrain {
            apply_kit_view(camera, terrain, origin, "walk");
        }
    }
    if let Ok(s) = std::env::var("SOMNIUM_CAMERA_POS") {
        if let Some(p) = parse_vec3(&s) {
            camera.position = p;
        }
    }
    if let Ok(v) = std::env::var("SOMNIUM_CAMERA_YAW") {
        if let Ok(yaw) = v.parse() {
            camera.yaw = yaw;
        }
    }
    if let Ok(v) = std::env::var("SOMNIUM_CAMERA_PITCH") {
        if let Ok(pitch) = v.parse() {
            camera.pitch = pitch;
        }
    }
}

// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”
// Editor mode
// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”

// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”
// Voxel terrain driver (Phase 14)
// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”

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
    chunks: std::collections::HashMap<
        somnium_voxel::ChunkCoord,
        Option<somnium_renderer::geometry::MeshAllocation>,
    >,
    /// Shared material: white base color + palette texture (albedo).
    material_id: u32,
}

impl VoxelTerrain {
    /// Create the palette texture/material and seed a few `set_voxel` edits
    /// that prove the edit-overlay â†’ remesh path works.
    fn new(
        renderer: &mut somnium_renderer::SomniumRenderer,
        render_ctx: &somnium_renderer::context::RenderContext,
    ) -> Self {
        use somnium_voxel::{PALETTE_SIZE, Voxel};

        // 1-D palette texture: one texel per voxel type, sampled at the texel
        // center by the constant per-face UV the mesher writes.
        let palette_bytes: Vec<u8> = Voxel::ALL.iter().flat_map(|v| v.palette_color()).collect();
        let texture = render_ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Voxel Palette"),
            size: wgpu::Extent3d {
                width: PALETTE_SIZE,
                height: 1,
                depth_or_array_layers: 1,
            },
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
                bytes_per_row: None, // single row â€” no alignment requirement
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: PALETTE_SIZE,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let palette_tex = renderer.add_texture(render_ctx, view);

        let material_id = renderer.materials_pool.add_material(
            &render_ctx.queue,
            somnium_renderer::material::pool::GpuMaterial {
                base_color: [1.0, 1.0, 1.0, 1.0],
                roughness: 0.9,
                metallic: 0.0,
                albedo_map: palette_tex as i32,
                normal_map: -1,
                metallic_roughness_map: -1,
                alpha_cutoff: 0.0,
                flags: 0,
                occlusion_map: -1,
                transmission: 0.0,
                emissive: [0.0; 3],
                emissive_map: -1,
                terrain_index: -1,
                porosity: 0.5,
                _pad: 0.0,
            },
        );

        let mut world = somnium_voxel::VoxelWorld::new(somnium_voxel::VoxelWorldConfig::default());

        // Demo edits: a small stone cairn east of the basin, proving that the
        // set_voxel overlay is honored by chunk generation and remeshing.
        let cairn_x = 20;
        let cairn_z = 6;
        let surface = (0..40)
            .rev()
            .map(|y| y - 8)
            .find(|&y| {
                world
                    .get_voxel(glam::IVec3::new(cairn_x, y, cairn_z))
                    .is_solid()
            })
            .unwrap_or(0);
        for dy in 1..=3 {
            world.set_voxel(
                glam::IVec3::new(cairn_x, surface + dy, cairn_z),
                somnium_voxel::Voxel::Stone,
            );
        }

        Self {
            world,
            chunks: Default::default(),
            material_id,
        }
    }

    /// Per-frame: stream chunks around the camera, upload finished meshes,
    /// and recycle GPU memory of despawned chunks.
    fn update(
        &mut self,
        jobs: &mut somnium_core::jobs::JobSystem,
        camera_pos: Vec3,
        renderer: &mut somnium_renderer::SomniumRenderer,
        render_ctx: &somnium_renderer::context::RenderContext,
    ) {
        let upd = self.world.update(jobs, camera_pos);

        for coord in upd.despawned {
            if let Some(Some(alloc)) = self.chunks.remove(&coord) {
                renderer.geometry.free_mesh(alloc);
            }
        }

        for ready in upd.ready {
            let new_alloc = ready.mesh.as_ref().map(|m| {
                renderer.geometry.upload_mesh_pooled(
                    &render_ctx.queue,
                    &m.vertices,
                    &m.indices,
                    self.material_id,
                )
            });
            if let Some(Some(old)) = self.chunks.insert(ready.coord, new_alloc) {
                renderer.geometry.free_mesh(old);
            }
        }
    }

    /// Release every chunk's GPU allocation back to the geometry pool.
    ///
    /// Called when the voxel-terrain entity is deleted â€” without this the
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
                casts_shadow: true,
                sort_key: somnium_renderer::command::SortKey::new(
                    0,
                    self.material_id as u16,
                    alloc.vertex_offset,
                ),
                vertex_offset: alloc.vertex_offset,
                index_offset: alloc.index_offset,
                index_count: alloc.index_count,
                material_id: self.material_id,
                transform: glam::Mat4::from_translation(origin),
            });
        }
    }
}

// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”
// Game struct
// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”

struct HelloGame {
    log_timer: f32,
    camera: EditorCamera,
    dreams_rail: Option<dreams_fixture::DreamRail>,
    cascade_debug: bool,
    last_simulation_time: f32,
    boat: Option<BoatRuntime>,
    /// Default material ID for newly created entities.
    default_material_id: Option<u32>,
    /// Geometry allocation for the procedural cube (shared by create_entity).
    default_cube_alloc: Option<somnium_renderer::geometry::MeshAllocation>,
    /// Phase 14: streaming voxel terrain.
    voxel_terrain: Option<VoxelTerrain>,
    /// The scripted first-person character, while Play is running.
    player: Option<PlayerRuntime>,
    /// Whether the last frame was a play session, so the transitions in
    /// and out can be spotted without the engine having to announce them.
    was_playing: bool,
    /// The character scripts, imported once at startup.
    controller_asset: Option<somnium_script::ids::ScriptAssetId>,
    camera_asset: Option<somnium_script::ids::ScriptAssetId>,
    /// Visible proof that game code can own a retained canvas and submit it
    /// through the public frame hook. The ECS component controls whether this
    /// starter tree is drawn; the widget tree itself remains game-owned.
    runtime_ui: somnium_core::UiCanvas,
    runtime_ui_enabled: bool,
    /// MORROWIND-M2. `.somui` documents instantiated for the canvases that
    /// reference one, keyed by the entity's `document` asset so the editor can
    /// change the field and see the result without a restart.
    ///
    /// Registered by name with the engine, which is what makes them reachable
    /// from `ctx:setUiProperty` in a Luau script — the editor and the game get
    /// at an authored HUD the same way.
    authored_ui: somnium_core::somui_host::UiDocuments,
    /// Which asset `authored_ui` currently holds, so a Details edit or a
    /// Content Drawer drop reloads and an unchanged field does not.
    authored_ui_asset: Option<somnium_asset::database::AssetId>,
    /// An asset the inventory has not published yet, so the wait is logged once
    /// rather than once a frame.
    authored_ui_missing: Option<somnium_asset::database::AssetId>,
    /// Whether the canvas entity's Outliner eye is on. Kept apart from whether
    /// the document is *loaded*, so hiding does not unload.
    authored_ui_visible: bool,
}

impl HelloGame {
    fn spawn_map_audio(&self, ctx: &mut EngineContext, result: &MapLoadResult) {
        let wave_path = match result.kind {
            MapKind::Coastal => "audio/ambient/coastal_waves_cc0.flac",
            MapKind::Island => "audio/ambient/island_waves_cc0.flac",
        };
        let surface_y = result.water.map_or(result.camera_position.y, |water| {
            result.preset.terrain_translation.y + water.surface_level + 0.5
        });
        let wave_position = Vec3::new(
            result.camera_position.x + 12.0,
            surface_y,
            result.camera_position.z - 8.0,
        );
        ctx.world.spawn((
            Transform::from_translation(wave_position),
            WorldTransform::identity(),
            Name::new(match result.kind {
                MapKind::Coastal => "Coastal Surf (CC0)",
                _ => "Island Surf (CC0)",
            }),
            AudioEmitterComponent {
                audio: somnium_asset::database::AssetId::from_relative_path(wave_path),
                volume: 0.65,
                min_distance: 18.0,
                max_distance: 180.0,
                ..Default::default()
            },
        ));

        // A short, directional one-shot on the same beach gives the test map
        // two overlapping authored voices before footsteps are added by the
        // controller.
        ctx.world.spawn((
            Transform::from_translation(wave_position + Vec3::new(5.0, 0.0, 4.0)),
            WorldTransform::identity(),
            Name::new("Shore Splash Test (CC0)"),
            AudioEmitterComponent {
                audio: somnium_asset::database::AssetId::from_relative_path(
                    "audio/sfx/water_splash_cc0.wav",
                ),
                looping: false,
                volume: 0.8,
                attenuation: AudioAttenuationModel::Linear,
                min_distance: 2.0,
                max_distance: 30.0,
                cone_enabled: true,
                cone_inner_degrees: 55.0,
                cone_outer_degrees: 110.0,
                cone_outer_gain: 0.25,
                ..Default::default()
            },
        ));
    }

    fn new() -> Self {
        let mut runtime_ui = somnium_core::UiCanvas::new(640.0, 360.0);
        runtime_ui.add_pause_banner("Hello Engine - UI Canvas");
        Self {
            log_timer: 0.0,
            camera: EditorCamera::new(Vec3::new(0.0, 2.0, 8.0)),
            dreams_rail: None,
            cascade_debug: false,
            last_simulation_time: 0.0,
            boat: None,
            default_material_id: None,
            default_cube_alloc: None,
            voxel_terrain: None,
            player: None,
            was_playing: false,
            controller_asset: None,
            camera_asset: None,
            runtime_ui,
            runtime_ui_enabled: true,
            authored_ui: somnium_core::somui_host::UiDocuments::new(),
            authored_ui_asset: None,
            authored_ui_missing: None,
            authored_ui_visible: false,
        }
    }

    fn spawn_default_vessel(
        &mut self,
        renderer: &mut somnium_renderer::SomniumRenderer,
        render_ctx: &somnium_renderer::RenderContext,
        world: &mut somnium_ecs::World,
        physics: &mut somnium_physics::world::PhysicsWorld,
        preset: somnium_core::DefaultLandscapePreset,
        water: somnium_core::WaterComponent,
    ) {
        let scene = match somnium_asset::load_gltf(
            "assets/models/gislinge_viking_boat/gislinge_viking_boat.glb",
        ) {
            Ok(scene) => scene,
            Err(error) => {
                tracing::warn!("Gislinge Viking Boat load failed: {error}");
                return;
            }
        };
        let uploaded = renderer.upload_scene(render_ctx, &scene);
        if uploaded.is_empty() {
            tracing::warn!("Gislinge Viking Boat contains no renderable mesh");
            return;
        }
        let width = water.bounds[2] - water.bounds[0];
        let depth = water.bounds[3] - water.bounds[1];
        let preferred_xz = if width <= 640.0 {
            // Island: south of the landmass, in the surrounding ocean.
            glam::Vec2::new(
                water.bounds[0] + width * 0.5,
                water.bounds[1] + depth * 0.5 + width * 0.22,
            )
        } else {
            glam::Vec2::new(
                water.bounds[0] + width * 0.397,
                water.bounds[1] + depth * 0.716,
            )
        };
        let local_xz = if renderer
            .query_water_surface(water.water_id, preferred_xz, 0.0)
            .is_some()
        {
            preferred_xz
        } else {
            renderer.deepest_water_point(water.water_id).map_or_else(
                || {
                    glam::Vec2::new(
                        (water.bounds[0] + water.bounds[2]) * 0.5,
                        (water.bounds[1] + water.bounds[3]) * 0.5,
                    )
                },
                |point| point.0,
            )
        };
        let surface = renderer
            .query_water_surface(water.water_id, local_xz, 0.0)
            .map_or(water.surface_level, |sample| sample.height);
        let initial_position = Vec3::new(
            preset.terrain_translation.x + local_xz.x,
            preset.terrain_translation.y + surface,
            preset.terrain_translation.z + local_xz.y,
        );
        // The downloaded GLB is authored in centimetres with its 7.7 m length
        // on local Z. Scale to metres and turn that axis onto Somnium's +X
        // vessel-forward convention.
        let initial_rotation = glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2 - 0.28);
        let body = physics.create_body(RigidBodyDescriptor {
            // Stable low-frequency proxy for the documented 7.7 x 1.5 m hull.
            shape: ColliderShape::Box {
                half_extents: Vec3::new(3.55, 0.34, 0.72),
            },
            position: initial_position,
            rotation: initial_rotation,
            motion_type: MotionType::Dynamic,
            object_layer: LAYER_MOVING,
            friction: 0.25,
            restitution: 0.02,
            linear_damping: 0.08,
            angular_damping: 0.65,
            allow_sleeping: false,
            ..Default::default()
        });
        if !body.is_valid() {
            tracing::warn!("Gislinge Viking Boat physics hull could not be created");
            return;
        }
        let parts = uploaded
            .into_iter()
            .map(|node| BoatPart {
                vertex_offset: node.vertex_offset,
                index_offset: node.index_offset,
                index_count: node.index_count,
                material_id: node.material_id,
                local_transform: node.transform,
            })
            .collect::<Vec<_>>();
        let entity = world.spawn((
            Transform {
                translation: initial_position,
                rotation: initial_rotation,
                scale: Vec3::splat(0.01),
            },
            WorldTransform::identity(),
            PhysicsBody { id: body },
            BuoyantVessel {
                water_id: water.water_id,
                water_origin: preset.terrain_translation,
                ..BuoyantVessel::default()
            },
            Name::new("Gislinge Viking Boat (CC BY 4.0)"),
        ));
        self.boat = Some(BoatRuntime {
            entity,
            body,
            water_id: water.water_id,
            water_origin: preset.terrain_translation,
            initial_position,
            initial_rotation,
            parts,
        });
        info!(
            "Default vessel spawned: Gislinge Viking Boat (29,035 triangles, CC BY 4.0) at {:?}",
            initial_position
        );
    }

    fn apply_loaded_map(&mut self, ctx: &mut EngineContext, result: &MapLoadResult) {
        if let Some(boat) = self.boat.take() {
            ctx.physics.destroy_body(boat.body);
            // LoadScene already despawned every entity; the old handle may now
            // name Terrain/Sun/Camera. Only despawn if this is still the boat.
            if ctx
                .world
                .get::<PhysicsBody>(boat.entity)
                .is_some_and(|p| p.id == boat.body)
            {
                ctx.world.despawn(boat.entity);
            }
        }
        self.camera.position = result.camera_position;
        self.camera.yaw = result.camera_yaw_degrees;
        self.camera.pitch = result.camera_pitch_degrees;
        if let (Ok(viewpoint), Some(water_component)) =
            (std::env::var("SOMNIUM_WATER_VIEW"), result.water)
        {
            if let Some(renderer) = ctx.renderer.as_mut() {
                if let Some((local_xz, depth)) =
                    renderer.deepest_water_point(water_component.water_id)
                {
                    let surface = renderer
                        .query_water_surface(water_component.water_id, local_xz, 0.0)
                        .map_or(water_component.surface_level, |sample| sample.height);
                    let eye_y = match viewpoint.as_str() {
                        "underwater" => surface - (depth * 0.22).clamp(1.5, 4.0),
                        "waterline" => surface,
                        _ => surface + 2.0,
                    };
                    self.camera.position = Vec3::new(
                        result.preset.terrain_translation.x + local_xz.x,
                        result.preset.terrain_translation.y + eye_y,
                        result.preset.terrain_translation.z + local_xz.y,
                    );
                    self.camera.yaw = std::env::var("SOMNIUM_WATER_YAW")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(-35.0);
                    self.camera.pitch = std::env::var("SOMNIUM_WATER_PITCH")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(if viewpoint == "underwater" { 5.0 } else { 0.0 });
                    info!(%viewpoint, depth, "Deterministic water validation viewpoint active");
                }
            }
        }
        if matches!(result.kind, MapKind::Coastal | MapKind::Island) {
            // DOOM-D's zero-work acceptance needs a genuinely static scene.
            // Both canonical maps normally spawn a buoyant dynamic vessel, so
            // the timing harness can suppress that demo-only caster explicitly.
            // This is deliberately opt-in and does not alter normal play.
            if std::env::var("SOMNIUM_TIME_STATIC").as_deref() != Ok("1")
                && let (Some(renderer), Some(render_ctx), Some(water_component)) =
                    (ctx.renderer.as_mut(), ctx.render_ctx.as_ref(), result.water)
            {
                self.spawn_default_vessel(
                    renderer,
                    render_ctx,
                    ctx.world,
                    ctx.physics,
                    result.preset.clone(),
                    water_component,
                );
            }
            self.spawn_map_audio(ctx, result);
        }
        // DOOM-H needs voxel chunk streaming to be present in a *reproducible*
        // run. It is normally created by hand from Create > Voxel Terrain, so
        // a timing harness could never see the work that was just moved onto
        // the shared scheduler. Opt-in, and it spawns exactly what the menu
        // item spawns, so the measured path is the shipped one.
        if std::env::var("SOMNIUM_VOXEL").as_deref() == Ok("1") {
            ctx.world.spawn((
                Transform::from_translation(Vec3::ZERO),
                WorldTransform::identity(),
                Name::new("Voxel Terrain"),
                somnium_core::VoxelTerrainComponent::default(),
            ));
            info!("SOMNIUM_VOXEL=1 - voxel terrain spawned for the timing harness");
        }
        info!(
            "Map {:?} preset v{} active",
            result.kind, result.preset.version
        );
        apply_capture_camera_overrides(
            &mut self.camera,
            result
                .terrain_id
                .and_then(|id| ctx.renderer.as_ref().and_then(|r| r.terrain(id))),
            result.terrain_origin,
        );
        self.dreams_rail = std::env::var("SOMNIUM_DREAMS_RAIL").ok().and_then(|name| {
            // A rail is frame-indexed from the frame the map finished loading,
            // and `SOMNIUM_CAPTURE_FRAME` counts from process start. Those are
            // two clocks: a slower load moves the capture to a different point
            // on the rail, and two runs that look like the same recipe are not
            // the same experiment. Log the offset so a capture pair can be
            // checked rather than assumed.
            let start = ctx.time.frame_count();
            let rail = dreams_fixture::DreamRail::named(
                name.trim(),
                dreams_fixture::CameraPose {
                    position: self.camera.position,
                    yaw: self.camera.yaw,
                    pitch: self.camera.pitch,
                },
                start,
                dreams_fixture::flyover_stop_frames(),
            );
            if rail.is_some() {
                tracing::info!(
                    rail = name.trim(),
                    start_frame = start,
                    anchor = ?self.camera.position,
                    yaw = self.camera.yaw,
                    pitch = self.camera.pitch,
                    "DREAMS rail armed"
                );
            }
            rail
        });
        let pose = self.camera.to_transform();
        let camera_entity = ctx.world.entities().find(|&e| {
            ctx.world
                .get::<Name>(e)
                .is_some_and(|n| n.as_str() == "Camera")
        });
        if let Some(entity) = camera_entity {
            if let Some(transform) = ctx.world.get_mut::<Transform>(entity) {
                *transform = pose;
            }
        }
    }

    /// Keep the voxel streaming driver in sync with the ECS.
    ///
    /// The voxel world is created from **Create > Voxel Terrain**, which spawns
    /// an entity carrying `VoxelTerrainComponent`. Chunks themselves are not
    /// entities (they stream constantly), so the driver lives here in game code
    /// and is built/torn down to follow that single entity.
    fn sync_voxel_terrain(&mut self, ctx: &mut EngineContext) {
        let wants_voxel = ctx.world.entities().any(|e| {
            ctx.world
                .get::<somnium_core::VoxelTerrainComponent>(e)
                .is_some()
        });

        match (wants_voxel, self.voxel_terrain.is_some()) {
            // Entity appeared â€” spin up the streaming driver.
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
            // Entity deleted â€” free the chunk meshes and drop the driver.
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
    /// MORROWIND-M2. Where the engine delivers a script's `setUiProperty`.
    ///
    /// This is what makes `assets/ui/hello_hud.somui` a *runtime* asset rather
    /// than a picture: attach a script to anything and
    /// `ctx:setUiProperty("canvas", "Score", "text", "…")` rewrites the HUD the
    /// editor is showing, with no editor-private path between the two.
    fn ui_documents(&mut self) -> Option<&mut dyn somnium_core::script_host::UiDocumentSink> {
        Some(&mut self.authored_ui)
    }

    fn on_init(&mut self, ctx: &mut EngineContext) {
        info!("HelloGame initialised â€” loading scene...");

        let gltf_loaded =
            if let (Some(renderer), Some(render_ctx)) = (&mut ctx.renderer, &ctx.render_ctx) {
                match somnium_asset::load_gltf("assets/test_scene.glb") {
                    Ok(scene) => {
                        info!("glTF loaded; uploading to GPU...");
                        let uploaded = renderer.upload_scene(render_ctx, &scene);
                        info!("{} nodes uploaded from glTF", uploaded.len());
                        for node in &uploaded {
                            let (scale, rotation, translation) =
                                node.transform.to_scale_rotation_translation();
                            ctx.world.spawn((
                                Transform {
                                    translation,
                                    rotation,
                                    scale,
                                },
                                MeshComponent {
                                    vertex_offset: node.vertex_offset,
                                    index_offset: node.index_offset,
                                    index_count: node.index_count,
                                },
                                MaterialComponent {
                                    asset: somnium_asset::database::AssetId::NONE,
                                    runtime_id: node.material_id,
                                },
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
            self.default_cube_alloc = Some(alloc);
        }

        // Phase 14: the voxel world is no longer spawned automatically â€” create
        // it from the editor via Create > Voxel Terrain. `sync_voxel_terrain`
        // builds the streaming driver when that entity appears.

        // Phase 24K: shadow smoke test. A wide ground plane with a cube above
        // it, both through the visibility buffer â€” which is what the traced
        // path can currently see, since terrain and water write depth in their
        // own later passes.
        //
        // In code rather than driven through the editor UI: building this by
        // synthesising clicks was attempted twice and produced scenes that
        // looked plausible but were not what they claimed, which is worse than
        // no test. Spawned with MeshKind only, so the same auto-attach path the
        // editor's Create menu uses supplies the mesh and material.
        if std::env::var("SOMNIUM_SHADOWTEST").is_ok() {
            // Remove the helmet, which sits below a raised ground and therefore
            // cannot cast onto it. What is left is a plane
            // and a cube and nothing else, so a shadow either appears or does not.
            let name_req = ComponentSet::from_ids(vec![ComponentId::of::<Name>()]);
            let mut doomed = Vec::new();
            for archetype in ctx
                .world
                .query_archetypes(&name_req, &ComponentSet::empty())
            {
                let n_col = archetype.column_index(ComponentId::of::<Name>()).unwrap();
                for row in 0..archetype.len() {
                    let n = unsafe { archetype.column(n_col).get::<Name>(row) };
                    let s = n.as_str();
                    if s.contains("Helmet") || s.contains("helmet") {
                        doomed.push(archetype.entities()[row]);
                    }
                }
            }
            for e in doomed {
                ctx.world.despawn(e);
            }

            ctx.world.spawn((
                Transform {
                    // The isolated ground stays at the origin for a stable test.
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

        // Phase 14 (SSS): heightmap terrain smoke test â€” exercises chunk
        // meshing, LODs, sculpt brushes, and auto-splat without editor input.
        // Normally terrain is created via Create > Terrain in the editor.
        // `SOMNIUM_TERRAIN=flat` reproduces **Create > Terrain** instead: the
        // default 16x16-chunk descriptor, no sculpting, spawned at y = 0. The
        // sculpted 4x4 variant is not a substitute â€” it was the only thing
        // 25A-2 was verified against, and the editor-created terrain turned out
        // not to render even though that one did.
        // Terrain is part of the default scene (Phase 25L). `SOMNIUM_TERRAIN`
        // now selects a variant rather than enabling it:
        //   unset / "flat" â€” load `assets/Maps/Coastal.somnium` (Great Lakes +
        //       32-layer Appalachia). Double-click Island in the Content Drawer
        //       for the 512 m / 16-material map.
        //   "1"            â€” the legacy sculpted 4x4 smoke test, kept because it
        //       is what exercises the brush paths without editor input.
        //   "0" / "none"   â€” no terrain, for isolating everything else.
        //
        // The sculpted variant is deliberately not the default: it was the only
        // thing 25A-2 was verified against, and the editor-created terrain
        // turned out not to render at all even though that one did.
        let terrain_mode = std::env::var("SOMNIUM_TERRAIN").unwrap_or_default();
        let flat_terrain = terrain_mode != "1";
        let mut loaded_map: Option<MapLoadResult> = None;
        if terrain_mode != "0" && terrain_mode != "none" {
            if let (Some(renderer), Some(render_ctx)) = (&mut ctx.renderer, &ctx.render_ctx) {
                if flat_terrain {
                    // Phase DOOM-A: `SOMNIUM_MAP=coastal|island` picks the
                    // startup recipe, and `SOMNIUM_TIME_VIEW` implies it, so a
                    // timing run can name a canonical viewpoint in one variable
                    // instead of three. Explicit `SOMNIUM_MAP` wins.
                    let kind = timing_view_map()
                        .or_else(|| {
                            somnium_core::parse_map_file(somnium_core::DEFAULT_MAP_PATH).ok()
                        })
                        .unwrap_or(MapKind::Coastal);
                    match somnium_core::spawn_map(ctx.world, renderer, render_ctx, kind) {
                        Ok(result) => loaded_map = Some(result),
                        Err(error) => tracing::warn!("Default map spawn failed: {error}"),
                    }
                } else {
                    use somnium_renderer::terrain::{TerrainDescriptor, brush};
                    let desc = if flat_terrain {
                        TerrainDescriptor::default()
                    } else {
                        TerrainDescriptor {
                            grid_size: [4, 4],
                            ..Default::default()
                        }
                    };
                    let terrain_id = renderer.create_terrain(render_ctx, desc);
                    let [wx, wz] = desc.world_size();

                    // Phase 25L: real relief, so the sixteen biome materials have
                    // altitudes and slopes to be assigned against. `SOMNIUM_HEIGHTMAP=<path>`
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
                            brush::auto_splat(terrain, amplitude * 0.48);

                            // `SOMNIUM_FOLIAGE=1` scatters foliage without the
                            // editor (Phase 17E). Painting by hand was the only way
                            // to get a plant on screen, which meant the foliage
                            // shading work could not be *seen*, let alone measured
                            // â€” the reason the 17E remainder sat open. Strokes are
                            // deterministic, so an A/B of a shading change is a
                            // like-for-like comparison.
                            if std::env::var("SOMNIUM_FOLIAGE").as_deref() == Ok("1") {
                                use somnium_renderer::terrain::foliage_paint::{
                                    self, FoliageBrush,
                                };
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
                                            &mut painted,
                                            &brush,
                                            [cx, cz],
                                            stroke,
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
                                foliage_camera =
                                    Some(Vec3::new(lx - wx * 0.5, ground + 1.6, lz - wz * 0.5));
                            }

                            // The same eye-level stance without the foliage, for
                            // judging the ground itself. Every terrain texturing
                            // phase since 25F has had to be judged on a hillside a
                            // kilometre away, where a material transition is a few
                            // pixels wide and a height blend is invisible â€” the
                            // features live at metres and the demo camera does not.
                            if std::env::var("SOMNIUM_TERRAIN_EYE").as_deref() == Ok("1") {
                                let [wx, wz] = desc.world_size();
                                let (lx, lz) = (wx * 0.34, wz * 0.40 + 14.0);
                                let ground = terrain.world_height_at(lx, lz);
                                foliage_camera =
                                    Some(Vec3::new(lx - wx * 0.5, ground + 1.6, lz - wz * 0.5));
                            }
                        }
                        // The default camera sits at y = 2, which is now inside a
                        // hillside rather than above a plain. Put it over the
                        // terrain looking down the slope.
                        self.camera.position = foliage_camera
                            .unwrap_or_else(|| Vec3::new(0.0, amplitude * 1.15 + 30.0, wz * 0.45));
                        self.camera.yaw = -90.0;
                        self.camera.pitch = if foliage_camera.is_some() {
                            -6.0
                        } else {
                            -22.0
                        };
                    }

                    if let Some(terrain) =
                        renderer.terrain_mut(terrain_id).filter(|_| !flat_terrain)
                    {
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
                        brush::auto_splat(
                            terrain,
                            somnium_renderer::terrain::DEFAULT_RELIEF_METRES * 0.48,
                        );
                    }

                    let _terrain_entity = ctx.world.spawn((
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
                            ..somnium_core::TerrainComponent::default()
                        },
                        // Phase 17E: painted foliage only submits when the
                        // terrain entity carries an enabled FoliageComponent.
                        somnium_core::FoliageComponent {
                            enabled: foliage_camera.is_some(),
                            ..Default::default()
                        },
                        Children::empty(),
                    ));
                    info!("Heightmap terrain smoke test active ({}x{} m)", wx, wz);
                }
            }
        }
        let map_actors_spawned = loaded_map.is_some();
        if let Some(result) = loaded_map {
            self.apply_loaded_map(ctx, &result);
        }

        // Phase 11A: Spawn the directional light entity.
        //
        // Phase 25M: `SOMNIUM_SUN_ELEVATION` (degrees above the horizon) and
        // `SOMNIUM_SUN_AZIMUTH` place the sun at startup. Rotating it by hand
        // with the gizmo is how the night bug was found, and reproducing that by
        // hand is not a test â€” this makes dusk and night a capture like any
        // other. It is also the only way to give 24U's light shafts the low sun
        // behind a ridge they have never been verified against.
        if !map_actors_spawned {
            let elevation = std::env::var("SOMNIUM_SUN_ELEVATION")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(35.0);
            let azimuth = std::env::var("SOMNIUM_SUN_AZIMUTH")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(-30.0);
            // Pitch is negated: the light's forward is -Z, so a *positive* elevation
            // has to tilt the forward vector downward for `to_light` to point up.
            let light_rot = glam::Quat::from_euler(
                glam::EulerRot::YXZ,
                azimuth.to_radians(),
                -elevation.to_radians(),
                0.0,
            );
            let mut sun =
                LightComponent::directional(somnium_core::light_units::lux::DIRECT_SUNLIGHT);
            // Track-7 timing harness. The authored/editor route remains the
            // Light Details enum; this switch only makes unattended CSM/VSM
            // A/B runs reproducible without rewriting a shipped scene.
            if std::env::var("SOMNIUM_VIRTUAL_SHADOWS").as_deref() == Ok("1") {
                sun.shadow_technique = LightShadowTechnique::Virtual;
            }
            ctx.world.spawn((
                Transform {
                    translation: Vec3::ZERO,
                    rotation: light_rot,
                    scale: Vec3::ONE,
                },
                sun,
                Name::new("SunLight"),
                WorldTransform::identity(),
            ));
        }

        // Phase 13C/13E: a point and a spot light so clustered local lighting is
        // visible in the demo â€” and so the light gizmos (L) have something to
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
                somnium_core::light_units::lumens::FLOODLIGHT,
                20.0,
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
                                Transform {
                                    translation: tr,
                                    rotation: rot,
                                    scale: sc,
                                },
                                Name::new(&n.entity_name),
                                WorldTransform::identity(),
                                MeshComponent {
                                    vertex_offset: n.vertex_offset,
                                    index_offset: n.index_offset,
                                    index_count: n.index_count,
                                },
                                MaterialComponent {
                                    asset: somnium_asset::database::AssetId::NONE,
                                    runtime_id: n.material_id,
                                },
                            ));
                        }
                    }
                    Err(e) => info!("IMPORT failed: {e}"),
                }
            }
        }

        // Phase 15A1: scene-wide post-processing settings, selectable in the
        // outliner. All effects start off â€” the viewport shows the raw image
        // until a look is dialled in.
        if !map_actors_spawned {
            ctx.world.spawn((
                Transform::from_translation(Vec3::ZERO),
                Name::new("Post Processing"),
                WorldTransform::identity(),
                somnium_core::DefaultLandscapePreset::current().post_process,
            ));

            ctx.world.spawn((
                self.camera.to_transform(),
                Name::new("Camera"),
                WorldTransform::identity(),
                somnium_core::CameraSettingsComponent::from_env(),
            ));
        }

        // A selectable authored root makes the runtime canvas discoverable in
        // the Outliner and Details on first launch. Create > UI Canvas uses the
        // same component, so delete/create/undo immediately controls the
        // visible game-owned tree below without an editor-private UI path.
        if !ctx
            .world
            .entities()
            .any(|entity| ctx.world.get::<UiCanvasComponent>(entity).is_some())
        {
            ctx.world.spawn((
                Transform::from_translation(Vec3::ZERO),
                Name::new("Hello UI Canvas"),
                WorldTransform::identity(),
                UiCanvasComponent {
                    // MORROWIND-M2. Pointed at the shipped document on first
                    // launch rather than left blank: an authoring feature you
                    // have to already know about in order to find is one nobody
                    // finds. Clear the field in Details to get the old
                    // code-built canvas back, or point it at your own `.somui`.
                    document: somnium_asset::database::AssetId::from_relative_path(
                        "ui/hello_hud.somui",
                    ),
                    ..UiCanvasComponent::default()
                },
                // Hidden on first launch. A HUD drawn over the viewport before
                // anybody asked for one is chrome in the way of the scene; the
                // eye in the Outliner is how you ask. This is also the shipped
                // example of the eye doing something you can see immediately.
                EditorFlags {
                    hidden: true,
                    locked: false,
                },
            ));
        }

        ctx.physics.optimize_broad_phase();

        // Phase 16-C: import the demo script and attach it to a cube.
        setup_scripting(self, ctx);

        // Send initial content browser listing
        ctx.ui.send_message(
            "update_content_browser",
            serde_json::json!({
                "path": "assets",
                "entries": list_assets_dir(),
            }),
        );

        // CONTROL-A capture harness: select one real, already-spawned entity
        // by its displayed name so the populated Details surface is captured
        // without inventing fixture data or synthesising an Outliner click.
        if let Ok(name) = std::env::var("SOMNIUM_AUDIT_SELECT_ENTITY") {
            *ctx.selected_entity = ctx.world.entities().find(|&entity| {
                ctx.world
                    .get::<Name>(entity)
                    .is_some_and(|n| n.as_str() == name)
            });
            if ctx.selected_entity.is_none() {
                tracing::warn!("SOMNIUM_AUDIT_SELECT_ENTITY={name} did not match an entity");
            }
        }
    }

    fn on_event(&mut self, ctx: &mut EngineContext, event: &EngineEvent) {
        match event {
            EngineEvent::KeyInput { key, state } => {
                let pressed = *state == InputState::Pressed;
                match key {
                    KeyCode::Escape if pressed => ctx.exit(),
                    KeyCode::KeyW if !self.camera.is_rmb_down && pressed => {
                        if let Some(r) = &mut ctx.renderer {
                            r.set_gizmo_mode(somnium_renderer::GizmoMode::Translate);
                        }
                    }
                    KeyCode::KeyW => self.camera.move_forward = pressed,
                    KeyCode::KeyS => self.camera.move_backward = pressed,
                    KeyCode::KeyA => self.camera.move_left = pressed,
                    KeyCode::KeyD => self.camera.move_right = pressed,
                    KeyCode::KeyE if !self.camera.is_rmb_down && pressed => {
                        if let Some(r) = &mut ctx.renderer {
                            r.set_gizmo_mode(somnium_renderer::GizmoMode::Rotate);
                        }
                    }
                    KeyCode::KeyE => self.camera.move_up = pressed,
                    KeyCode::KeyQ if !self.camera.is_rmb_down && pressed => {
                        if let Some(r) = &mut ctx.renderer {
                            r.set_gizmo_mode(somnium_renderer::GizmoMode::Translate);
                        }
                    }
                    KeyCode::KeyQ => self.camera.move_down = pressed,
                    KeyCode::KeyR if !self.camera.is_rmb_down && pressed => {
                        if let Some(r) = &mut ctx.renderer {
                            r.set_gizmo_mode(somnium_renderer::GizmoMode::Scale);
                        }
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
                    // Both paths must render identically â€” if the image changes,
                    // the indirect arguments are wrong.
                    KeyCode::F9 if pressed => {
                        if let Some(renderer) = &mut ctx.renderer {
                            if renderer.supports_gpu_driven() {
                                let on = renderer.toggle_gpu_driven();
                                info!(
                                    "Draw path: {}",
                                    if on {
                                        "GPU-driven (multi-draw indirect)"
                                    } else {
                                        "CPU (per-draw)"
                                    }
                                );
                            } else {
                                info!("GPU-driven draw path not supported on this device");
                            }
                        }
                    }
                    // F10: A/B GPU frustum culling (Phase 15B). A correct cull
                    // is invisible â€” if geometry pops, the cull is wrong.
                    // CPU Frustum Cull is the Camera Details toggle (Phase CR-C).
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
                button: somnium_core::MouseButton::Right,
                state,
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
                if self.camera.is_rmb_down && !play_session(ctx) {
                    self.camera.yaw += delta_x * self.camera.sensitivity;
                    self.camera.pitch -= delta_y * self.camera.sensitivity;
                    self.camera.pitch = self.camera.pitch.clamp(-89.0, 89.0);
                }
            }

            EngineEvent::WindowResized { width, height } => {
                info!("Window resized to {width}Ã—{height}");
            }

            _ => {}
        }
    }

    fn on_fixed_update(&mut self, ctx: &mut EngineContext) {
        let Some(boat) = self.boat.as_ref() else {
            return;
        };
        let Some(vessel) = ctx.world.get::<BuoyantVessel>(boat.entity).copied() else {
            return;
        };
        let position = ctx.physics.get_position(boat.body);
        let rotation = ctx.physics.get_rotation(boat.body);
        let linear_velocity = ctx.physics.get_linear_velocity(boat.body);
        let angular_velocity = ctx.physics.get_angular_velocity(boat.body);
        // Eight samples along the 7.7 m hull: port/starboard pairs at four
        // stations. Slightly deeper midships samples give the keel more bite
        // when a wave rolls under, which is what makes the boat lean into the
        // swell instead of hovering flat above it.
        let samples = [
            Vec3::new(-3.2, -0.20, -0.48),
            Vec3::new(-3.2, -0.20, 0.48),
            Vec3::new(-1.1, -0.28, -0.62),
            Vec3::new(-1.1, -0.28, 0.62),
            Vec3::new(1.1, -0.28, -0.62),
            Vec3::new(1.1, -0.28, 0.62),
            Vec3::new(3.2, -0.20, -0.48),
            Vec3::new(3.2, -0.20, 0.48),
        ];
        let draft = vessel.draft.max(0.1);
        let mut wet_samples = 0.0;
        for local_offset in samples {
            let offset = rotation * local_offset;
            let point = position + offset;
            let terrain_local = glam::Vec2::new(
                point.x - vessel.water_origin.x,
                point.z - vessel.water_origin.z,
            );
            let surface = ctx.renderer.as_deref().and_then(|renderer| {
                renderer.query_water_surface(
                    vessel.water_id,
                    terrain_local,
                    ctx.simulation.elapsed_seconds,
                )
            });
            let Some(surface) = surface else { continue };
            let surface_y = vessel.water_origin.y + surface.height;
            let submerged = ((surface_y - point.y) / draft).clamp(0.0, 1.0);
            if submerged <= 0.0 {
                continue;
            }
            wet_samples += submerged;
            let point_velocity = linear_velocity + angular_velocity.cross(offset);
            let water_velocity =
                Vec3::new(surface.velocity.x, surface.velocity.y, surface.velocity.z);
            let relative_velocity = water_velocity - point_velocity;
            // Archimedes is mostly vertical; a little surface normal lets the
            // hull follow the wave face without tipping over on every crest.
            let buoyancy_dir = (surface.normal * 0.35 + Vec3::Y * 0.65).normalize_or_zero();
            let buoyancy = buoyancy_dir * vessel.buoyancy_per_sample * submerged;
            let drag = relative_velocity * vessel.linear_drag * submerged;
            // Damp the rotational part of the sample velocity separately so a
            // rolling hull settles instead of oscillating forever.
            let rotational = angular_velocity.cross(offset);
            let angular_drag = -rotational * vessel.angular_drag * submerged;
            ctx.physics
                .add_force_at_position(boat.body, buoyancy + drag + angular_drag, point);
        }

        if wet_samples > 0.5 {
            // Righting: push the keel back under the centre of mass whenever
            // the mast leans. Strength scales with how wet the hull is so a
            // beached boat is not yanked upright.
            let local_up = rotation * Vec3::Y;
            let lean = Vec3::new(local_up.x, 0.0, local_up.z);
            if lean.length_squared() > 1.0e-4 {
                let restore = -lean.normalize()
                    * vessel.righting
                    * (wet_samples / 8.0).clamp(0.0, 1.0)
                    * lean.length().min(1.0);
                ctx.physics.add_force_at_position(
                    boat.body,
                    restore,
                    position + rotation * Vec3::new(0.0, -0.32, 0.0),
                );
            }

            // The imported hull's bow points along +X after its root transform.
            // A modest constant thrust makes the default scene immediately
            // demonstrate wakes; later input/vehicle work can replace this
            // with throttle control.
            let forward_3d = rotation * Vec3::X;
            let forward = Vec3::new(forward_3d.x, 0.0, forward_3d.z).normalize_or_zero();
            let thrust = forward * vessel.propulsion_force * (wet_samples / 8.0).clamp(0.0, 1.0);
            ctx.physics.add_force_at_position(
                boat.body,
                thrust,
                position + rotation * Vec3::new(-2.8, -0.18, 0.0),
            );
        }
    }

    fn on_update(&mut self, ctx: &mut EngineContext) {
        let dt = ctx.dt();

        // CONTROL-F: the editor asked to frame something. The camera lives
        // here, so the engine hands over a centre and a radius and this is
        // where it becomes a pose — keeping the current viewing direction so
        // `F` reframes rather than reorienting.
        // CONTROL-G: an exact pose — a view preset or a recalled bookmark —
        // is applied before a focus request, because it says *both* where to
        // be and which way to look, so a focus in the same frame would only
        // half-overwrite it.
        if let Some((position, yaw, pitch)) = ctx.take_camera_pose() {
            self.camera.position = position;
            self.camera.yaw = yaw;
            self.camera.pitch = pitch;
        }
        if let Some((centre, radius)) = ctx.take_camera_focus() {
            let direction = (self.camera.position - centre).normalize_or_zero();
            let direction = if direction == Vec3::ZERO {
                Vec3::new(0.0, 0.4, 1.0).normalize()
            } else {
                direction
            };
            self.camera.position = centre + direction * (radius * 3.0);
            let look = (centre - self.camera.position).normalize_or_zero();
            if look != Vec3::ZERO {
                self.camera.yaw = look.z.atan2(look.x).to_degrees();
                self.camera.pitch = look.y.clamp(-1.0, 1.0).asin().to_degrees();
            }
        }

        // Stop rolls the shared simulation clock back to zero. Restore the
        // demonstrator vessel at that point, while ordinary editor preview
        // continues to advance water and rigid-body simulation.
        if ctx.simulation.elapsed_seconds + f32::EPSILON < self.last_simulation_time {
            if let Some(boat) = self.boat.as_ref() {
                ctx.physics
                    .set_position(boat.body, boat.initial_position, true);
                ctx.physics
                    .set_rotation(boat.body, boat.initial_rotation, true);
                ctx.physics.set_linear_velocity(boat.body, Vec3::ZERO);
                ctx.physics.set_angular_velocity(boat.body, Vec3::ZERO);
                if let Some(transform) = ctx.world.get_mut::<Transform>(boat.entity) {
                    transform.translation = boat.initial_position;
                    transform.rotation = boat.initial_rotation;
                }
            }
        }

        let required = ComponentSet::from_ids(vec![
            ComponentId::of::<Transform>(),
            ComponentId::of::<PhysicsBody>(),
        ]);
        for archetype in ctx
            .world
            .query_archetypes_mut(&required, &ComponentSet::empty())
        {
            let t_col = archetype
                .column_index(ComponentId::of::<Transform>())
                .unwrap();
            let b_col = archetype
                .column_index(ComponentId::of::<PhysicsBody>())
                .unwrap();
            for row in 0..archetype.len() {
                let body = unsafe { *archetype.column(b_col).get::<PhysicsBody>(row) };
                let transform = unsafe { archetype.column_mut(t_col).get_mut::<Transform>(row) };
                transform.translation = ctx.physics.get_position(body.id);
                transform.rotation = ctx.physics.get_rotation(body.id);
            }
        }
        self.last_simulation_time = ctx.simulation.elapsed_seconds;

        // The scripted first-person character exists only during a play
        // session, and spawns where the editor camera was standing.
        // Watched here rather than announced by the engine, because
        // "spawn my player on Play" is a game-layer decision and every
        // game will want a different one.
        let playing = play_session(ctx);
        if playing && !self.was_playing {
            spawn_player(self, ctx);
        } else if !playing && self.was_playing {
            despawn_player(self, ctx);
        }
        self.was_playing = playing;

        // Propagate after physics/editor synchronization so render transforms
        // reflect Play, Pause, and Stop in the same frame.
        propagate_transforms(ctx.world);

        // Flip a render switch mid-run, the way the Details panel does.
        //
        // A switch thrown at frame 200 is not the same experiment as the same
        // switch set before startup: the second decides how the terrain loads
        // its textures and starts every cache warm, while the first invalidates
        // a running cache under a camera that has been moving. The clipmap
        // artifact was reported on the *toggle*, and until this existed there
        // was no way to capture that without synthetic mouse input.
        if std::env::var("SOMNIUM_AUDIT_TOGGLE_FRAME")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            == Some(ctx.time.frame_count())
            && let Ok(id) = std::env::var("SOMNIUM_AUDIT_TOGGLE_SWITCH")
            && let Some(renderer) = ctx.renderer.as_mut()
        {
            // Deliberately the same three statements as
            // `Engine::toggle_render_switch`, minus the checkbox repaint.
            let next = !renderer.debug_toggles.is_on(id.trim());
            match renderer.debug_toggles.set(id.trim(), next) {
                Ok(()) => {
                    renderer.apply_debug_toggles();
                    info!(
                        frame = ctx.time.frame_count(),
                        switch = id.trim(),
                        on = next,
                        "audit render switch"
                    );
                }
                Err(reason) => tracing::warn!(switch = id.trim(), %reason, "audit toggle refused"),
            }
        }

        // Deterministic temporal-audit hook. It is inert in ordinary runs and
        // lets the path/reflection capture matrix make a one-frame camera cut
        // without synthetic mouse input or timing-sensitive automation.
        if std::env::var("SOMNIUM_AUDIT_YAW_JUMP_FRAME")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            == Some(ctx.time.frame_count())
        {
            let degrees = std::env::var("SOMNIUM_AUDIT_YAW_JUMP_DEGREES")
                .ok()
                .and_then(|value| value.parse::<f32>().ok())
                .unwrap_or(1.0);
            self.camera.yaw += degrees;
            info!(
                frame = ctx.time.frame_count(),
                degrees, "audit camera yaw jump"
            );
        }

        if !play_session(ctx) {
            self.camera.update(dt, ctx.camera_speed);
        }
        if let Some(rail) = self.dreams_rail {
            let pose = rail.pose(ctx.time.frame_count());
            self.camera.position = pose.position;
            self.camera.yaw = pose.yaw;
            self.camera.pitch = pose.pitch;
        }
        self.log_timer += dt;

        // Phase 14: create/destroy the voxel driver to match the ECS, then
        // stream chunks around the camera (async generation; finished meshes
        // are uploaded here, freed allocations recycled).
        self.sync_voxel_terrain(ctx);
        let eye = active_view(ctx, &self.camera, self.player.as_ref()).1;
        if let (Some(vt), Some(renderer), Some(render_ctx)) =
            (&mut self.voxel_terrain, &mut ctx.renderer, &ctx.render_ctx)
        {
            vt.update(ctx.jobs, eye, renderer, render_ctx);
        }

        // Handle mesh-creating IPC commands that require renderer access.
        // These arrive after the engine-level IPC loop (which handles non-mesh commands).
        // We peek at the renderer here since create_entity for meshes needs GPU upload.
        // (The engine IPC loop in app.rs already drained non-mesh commands.)
        // Mesh entity creation is triggered by the "create_entity_mesh" internal message.
    }

    fn on_render(&mut self, ctx: &mut EngineContext) {
        // **Which canvas** and **whether it draws** are two questions, and
        // conflating them costs more than it looks. `EditorFlags::hidden` is an
        // authoring state — *"not submitted for drawing"* — not an unload: a
        // hidden canvas still owns its document, and a script writing to that
        // document must not start failing because somebody clicked an eye.
        let authored_canvas = ctx.world.entities().find_map(|entity| {
            let canvas = ctx.world.get::<UiCanvasComponent>(entity).copied()?;
            let transform = ctx
                .world
                .get::<Transform>(entity)
                .copied()
                .unwrap_or_default();
            let hidden = ctx
                .world
                .get::<EditorFlags>(entity)
                .copied()
                .unwrap_or_default()
                .hidden;
            canvas.enabled.then_some((canvas, transform, hidden))
        });
        // The Outliner's eye applies to a canvas like it applies to a mesh: a
        // canvas that ignored it would be the one thing in the scene the eye
        // could not turn off, which reads as the eye being broken rather than
        // as canvases being special.
        self.runtime_ui_enabled = authored_canvas.is_some_and(|(_, _, hidden)| !hidden);
        self.authored_ui_visible = self.runtime_ui_enabled;

        // MORROWIND-M2. The canvas entity's `document` field is an ordinary
        // asset reference, so this is reached three ways without any of them
        // knowing about the others: typing a path in Details, picking one from
        // the asset dropdown, or dragging a `.somui` out of the Content Drawer
        // onto the row. Reloading only when the asset actually changed is what
        // keeps a per-frame check from rebuilding the widget tree every frame.
        // Deliberately keyed on the canvas and not on its visibility: hiding
        // it must not unload the document out from under a script.
        let wanted = authored_canvas
            .map(|(settings, _, _)| settings.document)
            .filter(|asset| asset.raw() != 0);
        if wanted != self.authored_ui_asset {
            self.authored_ui = somnium_core::somui_host::UiDocuments::new();
            match wanted {
                None => self.authored_ui_asset = None,
                Some(asset) => {
                    // The id is a hash of a path, so the database that hashed
                    // it is what turns it back into a file.
                    let record = ctx
                        .ui
                        .asset_record(asset)
                        .map(|record| (record.relative_path.clone(), record.absolute_path.clone()));
                    match record {
                        // **Not resolved yet is not the same as broken.** The
                        // asset inventory is a background job, so on the frames
                        // before it publishes, a perfectly good reference is
                        // simply unknown. Leaving `authored_ui_asset` alone
                        // makes the next frame try again; caching the failure
                        // here meant a HUD that never appeared and one
                        // confusing line about rescanning.
                        None => {
                            if self.authored_ui_missing != Some(asset) {
                                self.authored_ui_missing = Some(asset);
                                tracing::debug!(
                                    "UI document {asset} is not in the asset inventory yet"
                                );
                            }
                        }
                        Some((relative, absolute)) => {
                            // Resolved, so the outcome is final either way and
                            // the reference is recorded rather than retried.
                            self.authored_ui_asset = wanted;
                            self.authored_ui_missing = None;
                            match std::fs::read_to_string(&absolute) {
                                Ok(text) => match self.authored_ui.load("canvas", &text) {
                                    Ok(()) => tracing::info!("UI document {relative} loaded"),
                                    // Named, not swallowed. A `.somui` with a
                                    // typo in a widget kind is the common case,
                                    // and the Output Log is where somebody
                                    // editing one is looking.
                                    Err(errors) => {
                                        for error in errors {
                                            tracing::warn!("UI document {relative}: {error}");
                                        }
                                    }
                                },
                                Err(error) => tracing::warn!(
                                    "UI document {relative} could not be read: {error}"
                                ),
                            }
                        }
                    }
                }
            }
        }
        if let Some((settings, transform, _)) = authored_canvas {
            use somnium_ui::runtime::{Canvas, CanvasMode, Layer};

            let mut canvas = match settings.space {
                UiCanvasSpace::Screen => {
                    Canvas::scaled(glam::Vec2::new(settings.width, settings.height), 0.5)
                }
                UiCanvasSpace::World => {
                    let mut canvas = Canvas::world(
                        transform.to_matrix(),
                        glam::Vec2::new(settings.width, settings.height),
                    );
                    if let CanvasMode::World { billboard, .. } = &mut canvas.mode {
                        *billboard = settings.billboard;
                    }
                    canvas
                }
                UiCanvasSpace::Overlay => Canvas {
                    mode: CanvasMode::Overlay {
                        world_anchor: transform.translation,
                    },
                    ..Canvas::screen()
                },
            }
            .on_layer(Layer(i32::try_from(settings.layer).unwrap_or(
                if settings.layer < 0 {
                    i32::MIN
                } else {
                    i32::MAX
                },
            )));
            canvas.visible = settings.enabled;
            // MORROWIND-M2. The authored document's canvas gets the *same*
            // placement as the code-built one, because they are two contents of
            // one authored canvas entity and not two canvases. Without this the
            // document loads, resolves its anchors and draws into a canvas the
            // compositor has no placement for — which looks exactly like the
            // document being broken.
            for authored in self.authored_ui.iter_mut() {
                authored.canvas.set_canvas(canvas.clone());
                authored
                    .canvas
                    .set_world_pixels_per_unit(settings.pixels_per_unit);
            }
            self.runtime_ui.set_canvas(canvas);
            self.runtime_ui
                .set_world_pixels_per_unit(settings.pixels_per_unit);
        }

        if std::env::var("SOMNIUM_CAPTURE_QUIT").as_deref() == Ok("1")
            && somnium_renderer::capture::finished()
        {
            ctx.exit();
            return;
        }
        // Phase DOOM-A: same contract for a timing run. Default on rather than
        // opt-in â€” a run has a fixed frame count, so there is nothing left to
        // measure once it is written, and leaving the window up only invites
        // someone to read fps off it.
        if std::env::var("SOMNIUM_TIME_QUIT").as_deref() != Ok("0")
            && somnium_renderer::timing::finished()
        {
            ctx.exit();
            return;
        }
        let (wake_origin_direction, wake_params) =
            self.boat.as_ref().map_or(([0.0; 4], [0.0; 4]), |boat| {
                let position = ctx.physics.get_position(boat.body);
                let forward_3d = ctx.physics.get_rotation(boat.body) * Vec3::X;
                let forward = Vec3::new(forward_3d.x, 0.0, forward_3d.z).normalize_or_zero();
                let velocity = ctx.physics.get_linear_velocity(boat.body);
                let speed = velocity.dot(forward).max(0.0);
                let wake_strength = ((speed - 0.25) / 2.4).clamp(0.0, 1.0);
                (
                    [
                        position.x - boat.water_origin.x,
                        position.z - boat.water_origin.z,
                        forward.x,
                        forward.z,
                    ],
                    [speed, wake_strength, 110.0, 3.0],
                )
            });
        let (view_mat, eye) = active_view(ctx, &self.camera, self.player.as_ref());
        if let (Some(renderer), Some(render_ctx)) = (&mut ctx.renderer, &ctx.render_ctx) {
            let (rw, rh) = renderer.scene_extent();
            let aspect = rw as f32 / rh.max(1) as f32;
            let proj = glam::Mat4::perspective_rh(45.0f32.to_radians(), aspect, 0.1, 1000.0);
            renderer.set_view(view_mat, proj, eye);

            // Sync the lights from ECS LightComponent.
            {
                let light_req = ComponentSet::from_ids(vec![
                    ComponentId::of::<Transform>(),
                    ComponentId::of::<LightComponent>(),
                ]);
                for archetype in ctx
                    .world
                    .query_archetypes(&light_req, &ComponentSet::empty())
                {
                    let t_col = archetype
                        .column_index(ComponentId::of::<Transform>())
                        .unwrap();
                    let l_col = archetype
                        .column_index(ComponentId::of::<LightComponent>())
                        .unwrap();
                    // Hiding a light hides what it lights, not just its gizmo.
                    // One lookup per archetype, since most carry no flags.
                    let flags_col = archetype.column_index(ComponentId::of::<EditorFlags>());
                    for row in 0..archetype.len() {
                        if let Some(col) = flags_col
                            && unsafe { archetype.column(col).get::<EditorFlags>(row) }.hidden
                        {
                            continue;
                        }
                        let transform = unsafe { archetype.column(t_col).get::<Transform>(row) };
                        let light = unsafe { archetype.column(l_col).get::<LightComponent>(row) };

                        // Two different conventions, easy to mix up:
                        //  * `forward` â€” the direction light TRAVELS (entity -Z).
                        //    This is the spot cone's axis: the shader tests
                        //    dot(-L, direction_ws) with -L pointing lightâ†’surface.
                        //  * `to_light` â€” the direction TOWARD the light, which is
                        //    what the directional BRDF wants for NÂ·L.
                        // Passing `to_light` as the spot axis aimed the cone 180Â°
                        // away from where the gizmo (correctly) drew it.
                        let forward = transform.rotation.mul_vec3(glam::Vec3::NEG_Z);
                        let to_light = -forward;

                        match light.light_type {
                            LightType::Directional => {
                                let shadow_technique = if std::env::var("SOMNIUM_VIRTUAL_SHADOWS")
                                    .as_deref()
                                    == Ok("1")
                                {
                                    LightShadowTechnique::Virtual
                                } else {
                                    light.shadow_technique
                                };
                                if shadow_technique == LightShadowTechnique::Virtual
                                    && renderer.virtual_shadow_gpu.is_none()
                                {
                                    if let Err(error) = renderer.enable_virtual_shadow_resources(
                                        &render_ctx.device,
                                        &render_ctx.queue,
                                        somnium_renderer::shadow::virtual_map::VirtualShadowConfig::default(),
                                    ) {
                                        tracing::warn!(%error, "virtual shadow resources unavailable; using CSM");
                                    }
                                }
                                // Phase 25M: the sun's illuminance is what
                                // survives the trip through the atmosphere, so
                                // it reddens as the sun drops and reaches zero
                                // once it is below the horizon. Applied here,
                                // to the one value every consumer reads â€”
                                // shading, shadows, ReSTIR, the froxel volume
                                // and the sky's own moon blending all take the
                                // light buffer, so there is nowhere for them to
                                // disagree about whether the sun has set.
                                // Phase TSUSHIMA-A: the capture harness's sun
                                // pin. Applied here, before `transmittance`
                                // reads `to_light.y`, so a pinned low sun
                                // reddens exactly as an authored one would —
                                // the reddening is a function of elevation and
                                // must not be bypassed by overriding the
                                // direction after it.
                                let to_light = pinned_sun_elevation()
                                    .map_or(to_light, |e| sun_at_elevation(to_light, e));
                                let survives = somnium_core::sun::transmittance(
                                    to_light.y,
                                    transform.translation.y * 0.001,
                                );
                                renderer.set_directional_light(
                                    to_light,
                                    light.photometric_color() * survives,
                                );
                                renderer.set_directional_shadow_policy(
                                    somnium_renderer::shadow::virtual_map::ShadowLightPolicy {
                                        light_id: 0,
                                        technique: match shadow_technique {
                                            LightShadowTechnique::Cascaded => somnium_renderer::shadow::virtual_map::ShadowTechnique::Cascaded,
                                            LightShadowTechnique::Virtual => somnium_renderer::shadow::virtual_map::ShadowTechnique::Virtual,
                                        },
                                        csm_fallback: true,
                                    },
                                );
                                renderer.set_moon_intensity(light.moon_intensity);
                            }
                            LightType::Point
                            | LightType::Spot
                            | LightType::Rect
                            | LightType::Disc
                            | LightType::Tube => {
                                let l_type = match light.light_type {
                                    LightType::Point => 0,
                                    LightType::Spot => 1,
                                    LightType::Rect => 2,
                                    LightType::Disc => 3,
                                    LightType::Tube => 4,
                                    LightType::Directional => 0,
                                };
                                renderer.submit_local_light(
                                    somnium_renderer::cluster::GpuLocalLight {
                                        position_ws: transform.translation.to_array(),
                                        range: light.range,
                                        color: light.photometric_color().to_array(),
                                        light_type: l_type,
                                        // Spot/rect axis = travel direction. Unused for point lights.
                                        direction_ws: forward.to_array(),
                                        spot_cos_outer: light.outer_angle.cos(),
                                        spot_cos_inner: light.inner_angle.cos(),
                                        radius: light.source_radius,
                                        _pad: [light.area_width, light.area_height],
                                    },
                                );
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
                let k_col = archetype
                    .column_index(ComponentId::of::<MeshKind>())
                    .unwrap();
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
                        roughness: 0.5,
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
                        terrain_index: -1,
                        porosity: 0.5,
                        _pad: 0.0,
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
                    let t = ctx
                        .world
                        .get::<Transform>(entity)
                        .copied()
                        .unwrap_or(Transform::from_translation(glam::Vec3::ZERO));
                    let n = ctx
                        .world
                        .get::<Name>(entity)
                        .cloned()
                        .unwrap_or(Name::new("Mesh"));
                    let wt = ctx
                        .world
                        .get::<WorldTransform>(entity)
                        .copied()
                        .unwrap_or(WorldTransform::identity());

                    // Respawn
                    ctx.world.despawn(entity);
                    let new_entity = ctx.world.spawn((
                        t,
                        n,
                        wt,
                        kind,
                        MeshComponent {
                            vertex_offset: alloc.vertex_offset,
                            index_offset: alloc.index_offset,
                            index_count: alloc.index_count,
                        },
                        MaterialComponent {
                            asset: somnium_asset::database::AssetId::NONE,
                            runtime_id: default_mat,
                        },
                    ));

                    if std::env::var("SOMNIUM_SHADOWTEST").is_ok() {
                        tracing::info!(
                            "shadowtest attach: kind={:?} vtx_off={} idx_off={} idx_count={} mat={}",
                            kind,
                            alloc.vertex_offset,
                            alloc.index_offset,
                            alloc.index_count,
                            default_mat
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
            for archetype in ctx
                .world
                .query_archetypes(&required, &ComponentSet::empty())
            {
                let wt_col = archetype
                    .column_index(ComponentId::of::<WorldTransform>())
                    .unwrap();
                let m_col = archetype
                    .column_index(ComponentId::of::<MeshComponent>())
                    .unwrap();
                let mat_col = archetype
                    .column_index(ComponentId::of::<MaterialComponent>())
                    .unwrap();
                // CONTROL-F: hidden entities are skipped here rather than in
                // the renderer, because "hidden" is an authoring state and the
                // renderer has no business knowing about the Outliner. Most
                // archetypes carry no flags column at all, so this costs one
                // lookup per archetype, not per entity.
                let flags_col = archetype.column_index(ComponentId::of::<EditorFlags>());
                for row in 0..archetype.len() {
                    if let Some(col) = flags_col
                        && unsafe { archetype.column(col).get::<EditorFlags>(row) }.hidden
                    {
                        continue;
                    }
                    let wt = unsafe { archetype.column(wt_col).get::<WorldTransform>(row) };
                    let mesh = unsafe { archetype.column(m_col).get::<MeshComponent>(row) };
                    let material =
                        unsafe { archetype.column(mat_col).get::<MaterialComponent>(row) };
                    let entity = archetype.entities()[row];
                    renderer.submit(somnium_renderer::command::DrawCommand {
                        casts_shadow: true,
                        sort_key: somnium_renderer::command::SortKey::new(
                            0,
                            material.runtime_id as u16,
                            entity.index(),
                        ),
                        vertex_offset: mesh.vertex_offset,
                        index_offset: mesh.index_offset,
                        index_count: mesh.index_count,
                        material_id: material.runtime_id,
                        transform: wt.0,
                    });
                }
            }

            // The Viking boat is a single ECS/physics entity backed by the
            // original GLB's multi-node render hierarchy. Submit every part
            // against the shared rigid-body root without flooding the outliner.
            // The boat is one ECS entity drawn as many parts, so its flags are
            // asked once for the whole hierarchy rather than per part.
            if let Some(boat) = self
                .boat
                .as_ref()
                .filter(|boat| !somnium_core::is_hidden(ctx.world, boat.entity))
            {
                let root = glam::Mat4::from_scale_rotation_translation(
                    Vec3::splat(0.01),
                    ctx.physics.get_rotation(boat.body),
                    ctx.physics.get_position(boat.body),
                );
                for (part_index, part) in boat.parts.iter().enumerate() {
                    renderer.submit(somnium_renderer::command::DrawCommand {
                        casts_shadow: true,
                        sort_key: somnium_renderer::command::SortKey::new(
                            0,
                            part.material_id as u16,
                            boat.entity.index().saturating_add(part_index as u32),
                        ),
                        vertex_offset: part.vertex_offset,
                        index_offset: part.index_offset,
                        index_count: part.index_count,
                        material_id: part.material_id,
                        transform: root * part.local_transform,
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
            for archetype in ctx
                .world
                .query_archetypes(&water_req, &ComponentSet::empty())
            {
                let wt_col = archetype
                    .column_index(ComponentId::of::<WorldTransform>())
                    .unwrap();
                let m_col = archetype
                    .column_index(ComponentId::of::<MeshComponent>())
                    .unwrap();
                let w_col = archetype
                    .column_index(ComponentId::of::<somnium_core::WaterComponent>())
                    .unwrap();
                for row in 0..archetype.len() {
                    let wt = unsafe { archetype.column(wt_col).get::<WorldTransform>(row) };
                    let mesh = unsafe { archetype.column(m_col).get::<MeshComponent>(row) };
                    let water = unsafe {
                        archetype
                            .column(w_col)
                            .get::<somnium_core::WaterComponent>(row)
                    };
                    if !water.enabled {
                        continue;
                    }
                    renderer.add_water(
                        water.water_id,
                        wt.0,
                        somnium_renderer::pass::water::WaterMaterialData {
                            deep_color: water.deep_color,
                            shallow_color: water.shallow_color,
                            edge_color: water.edge_color,
                            absorption_roughness: [
                                water.absorption[0],
                                water.absorption[1],
                                water.absorption[2],
                                water.roughness,
                            ],
                            scattering_anisotropy: [
                                water.scattering[0],
                                water.scattering[1],
                                water.scattering[2],
                                water.anisotropy,
                            ],
                            bounds: water.bounds,
                            surface_params: [
                                water.clarity,
                                water.edge_scale,
                                water.amplitude,
                                water.ssr_strength,
                            ],
                            wave_dir_a: water.wave_dir_a,
                            wave_dir_b: water.wave_dir_b,
                            wave_params: [
                                water.wave_length_a,
                                water.wave_length_b,
                                water.wave_speed,
                                water.wave_steepness,
                            ],
                            simulation_params: [
                                water.spectrum_blend,
                                water.wind_speed,
                                water.foam_decay,
                                water.foam_threshold,
                            ],
                            volume_params: [
                                water.caustic_strength,
                                f32::from(u8::from(water.underwater_enabled)),
                                water.rt_reflect_strength,
                                water.reflect_debug,
                            ],
                            wake_origin_direction,
                            wake_params: if water.water_id
                                == self.boat.as_ref().map_or(u32::MAX, |b| b.water_id)
                            {
                                wake_params
                            } else {
                                [0.0; 4]
                            },
                            cascade_scales: [[0.0; 4]; 3],
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
            ctx.ui.send_message(
                "update_outliner",
                serde_json::json!({ "entities": entities_payload }),
            );
        }

        // Phase 11.5C: Selection sync with component details.
        if let Some(selected) = *ctx.selected_entity {
            let name_req = ComponentSet::from_ids(vec![ComponentId::of::<Name>()]);
            let mut display_name = format!("Entity_{}", selected.index());
            'outer: for archetype in ctx
                .world
                .query_archetypes(&name_req, &ComponentSet::empty())
            {
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

            ctx.ui.send_message(
                "update_selection",
                serde_json::json!({
                    "index":     selected.index(),
                    "name":      display_name,
                    "transform": transform_data,
                    "light":     light_data,
                    "mesh":      mesh_data,
                }),
            );
        } else {
            ctx.ui
                .send_message("update_selection", serde_json::Value::Null);
        }
    }

    fn on_render_ui(&mut self, frame: &mut GameUiFrame) {
        // Authored documents first, then the code-built canvas over them: the
        // pause banner is chrome and belongs on top of whatever the game drew.
        // Loaded but not drawn while the eye is off. The document stays
        // registered so a script can keep writing to it.
        let visible = self.authored_ui_visible;
        for authored in self.authored_ui.iter_mut() {
            if !visible {
                continue;
            }
            let viewport = authored.canvas.ui().screen_size;
            authored.relayout(viewport);
            frame.draw(&mut authored.canvas);
            if std::env::var("SOMNIUM_SOMUI_DEBUG").as_deref() == Ok("1")
                && let Some(handle) = authored.instance.handle("Title")
            {
                let probe = authored.canvas.ui().a11y_probe(handle);
                tracing::info!(
                    ?viewport,
                    bounds = ?probe.as_ref().map(|p| p.bounds),
                    visible = ?probe.as_ref().map(|p| p.visible),
                    name = ?probe.as_ref().map(|p| p.name.clone()),
                    "somui debug"
                );
            }
        }
        if self.runtime_ui_enabled {
            frame.draw(&mut self.runtime_ui);
        }
    }

    fn on_shutdown(&mut self) {
        info!("HelloGame shutting down â€” goodbye!");
    }

    fn on_map_loaded(&mut self, ctx: &mut EngineContext, result: &MapLoadResult) {
        self.apply_loaded_map(ctx, result);
    }
}

// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”
// Outliner payload builder (Phase 11.5A-3)
// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”

fn build_outliner_payload(ctx: &EngineContext) -> Vec<OutlinerEntity> {
    // Collect name map and parent map.
    let mut name_map: std::collections::HashMap<u32, String> = Default::default();
    let mut parent_map: std::collections::HashMap<u32, Option<u32>> = Default::default();

    let name_req = ComponentSet::from_ids(vec![ComponentId::of::<Name>()]);
    for arch in ctx
        .world
        .query_archetypes(&name_req, &ComponentSet::empty())
    {
        let n_col = arch.column_index(ComponentId::of::<Name>()).unwrap();
        for row in 0..arch.len() {
            let name = unsafe { arch.column(n_col).get::<Name>(row) };
            let entity = arch.entities()[row];
            name_map.insert(entity.index(), name.as_str().to_string());
        }
    }

    let parent_req = ComponentSet::from_ids(vec![ComponentId::of::<Parent>()]);
    for arch in ctx
        .world
        .query_archetypes(&parent_req, &ComponentSet::empty())
    {
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
                name: name_map
                    .get(&idx)
                    .cloned()
                    .unwrap_or_else(|| format!("Entity_{idx}")),
                index: idx,
                parent: None,
                depth: 0,
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
                        name: name_map
                            .get(&idx)
                            .cloned()
                            .unwrap_or_else(|| format!("Entity_{idx}")),
                        index: idx,
                        parent: Some(*p_idx),
                        depth: d,
                    });
                }
            }
        }
        if result.len() == prev_len {
            break;
        }
    }

    result
}

// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”
// Phase 16-C â€” the scripting gate
// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”

/// Where the demo script lives, relative to the working directory.
const DEMO_SCRIPT_PATH: &str = "assets/scripts/demo_rotator.luau";

/// The two scripts that make up the first-person character.
const CONTROLLER_SCRIPT_PATH: &str = "assets/scripts/first_person_controller.luau";
const CAMERA_SCRIPT_PATH: &str = "assets/scripts/first_person_camera.luau";

/// A capsule 1.8 m tall: `half_height` is the *cylinder* half, so the
/// total is `2 * (half_height + radius)`.
const PLAYER_HALF_HEIGHT: f32 = 0.6;
const PLAYER_RADIUS: f32 = 0.3;

/// Where the eye sits above the capsule's centre. Must match the camera
/// script's `eyeHeight` default, or pressing Play moves the view.
const PLAYER_EYE_HEIGHT: f32 = 0.72;

/// The player, its camera child, and the Jolt body underneath.
struct PlayerRuntime {
    player: Entity,
    camera: Entity,
    body: BodyId,
}

/// Spawn the first-person character where the editor camera is standing.
///
/// Called on the transition into Play. The capsule's centre goes an eye
/// height *below* the editor camera, so the view does not jump when you
/// press Play â€” you carry on looking from where you were.
///
/// Nothing here is scripting-specific except the two attachments: the
/// movement, the look and the jump all live in Luau, which is the point of
/// the exercise.
fn spawn_player(game: &mut HelloGame, ctx: &mut EngineContext) {
    let (Some(controller), Some(camera_script)) = (game.controller_asset, game.camera_asset) else {
        tracing::warn!("character scripts did not import; not spawning a player");
        return;
    };

    let eye = game.camera.position;
    let centre = eye - Vec3::Y * PLAYER_EYE_HEIGHT;

    let body = ctx.physics.create_body(RigidBodyDescriptor {
        shape: ColliderShape::Capsule {
            half_height: PLAYER_HALF_HEIGHT,
            radius: PLAYER_RADIUS,
        },
        position: centre,
        motion_type: MotionType::Dynamic,
        object_layer: LAYER_MOVING,
        // A character does not bounce and does not skid: the script owns
        // horizontal velocity outright, so friction and restitution would
        // only fight it.
        friction: 0.0,
        restitution: 0.0,
        linear_damping: 0.0,
        angular_damping: 1.0,
        // A sleeping character stops responding to the keyboard.
        allow_sleeping: false,
        ..Default::default()
    });

    let mut player_scripts = somnium_script::attachment::ScriptSet::new();
    player_scripts.attach(somnium_script::attachment::ScriptAttachment::new(
        controller,
    ));
    let player = ctx.world.spawn((
        Transform::from_translation(centre),
        WorldTransform::identity(),
        Name::new("Player"),
        somnium_core::RigidBodyComponent::driven(body),
        player_scripts,
        Children::empty(),
    ));

    let mut camera_scripts = somnium_script::attachment::ScriptSet::new();
    camera_scripts.attach(somnium_script::attachment::ScriptAttachment::new(
        camera_script,
    ));
    let camera = ctx.world.spawn((
        Transform::from_translation(Vec3::Y * PLAYER_EYE_HEIGHT),
        WorldTransform::identity(),
        Name::new("PlayerCamera"),
        // Makes it a camera the renderer will accept; `active_view`
        // prefers the player's when one exists.
        CameraSettingsComponent::from_env(),
        Parent { entity: player },
        camera_scripts,
    ));
    if let Some(children) = ctx.world.get_mut::<Children>(player) {
        children.push(camera);
    }

    game.player = Some(PlayerRuntime {
        player,
        camera,
        body,
    });
    info!("Player spawned at {centre:?} â€” WASD, Shift to run, Space to jump");
}

/// Tear the character down on Stop.
///
/// The *entities* are removed by the engine's play-session checkpoint â€”
/// anything spawned after Play was pressed is not part of the authored
/// world. The Jolt body is ours, and nothing else would ever free it.
fn despawn_player(game: &mut HelloGame, ctx: &mut EngineContext) {
    let Some(runtime) = game.player.take() else {
        return;
    };
    ctx.physics.destroy_body(runtime.body);
    // Belt and braces: if the checkpoint did not exist (a headless run,
    // say) the entities would otherwise outlive the body they refer to.
    for entity in [runtime.camera, runtime.player] {
        if ctx.world.is_alive(entity) {
            ctx.world.despawn(entity);
        }
    }
}

/// The same file, compiled in, so the gate runs from any working
/// directory. The on-disk copy is the one the editor's content drawer
/// lists and the one a hot reload watches; this is the fallback.
const DEMO_SCRIPT_SOURCE: &str = include_str!("../../../assets/scripts/demo_rotator.luau");

/// Import the demo script, teach the engine how to route a force, and
/// attach the script to a cube.
///
/// This is the whole of what game code has to do for scripting: the
/// lifecycle, the ordering, the command applier and the frame hooks are
/// the engine's. Press Play to run it.
fn setup_scripting(game: &mut HelloGame, ctx: &mut EngineContext) {
    for clip in [
        "audio/footsteps/footstep_01_cc0.ogg",
        "audio/footsteps/footstep_02_cc0.ogg",
        "audio/footsteps/footstep_03_cc0.ogg",
        "audio/footsteps/footstep_04_cc0.ogg",
    ] {
        ctx.scripts.register_audio(
            somnium_script::ids::ScriptAssetId::from_path(clip),
            std::path::Path::new("assets").join(clip).to_string_lossy(),
        );
    }
    // â”€â”€ The force router â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    //
    // `applyForce` names an entity; Jolt wants a body id. The mapping is
    // this game's `PhysicsBody` component, which `somnium_core` has never
    // heard of â€” so the engine asks rather than guessing.
    ctx.scripts
        .set_force_router(Box::new(|world, physics, entity, force, mode| {
            let Some(body) = world.get::<PhysicsBody>(entity).copied() else {
                return;
            };
            match mode {
                somnium_script::command::ForceMode::Impulse => {
                    physics.add_impulse(body.id, force);
                }
                somnium_script::command::ForceMode::Force => {
                    physics.add_force(body.id, force);
                }
            }
        }));

    // â”€â”€ The character scripts â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    //
    // Imported from disk so the file watcher picks up edits: change the
    // walk speed in the `.luau` and it takes effect without a restart.
    for (path, slot) in [(CONTROLLER_SCRIPT_PATH, 0), (CAMERA_SCRIPT_PATH, 1)] {
        match ctx.scripts.import_script_file(std::path::Path::new(path)) {
            Ok(id) => {
                if slot == 0 {
                    game.controller_asset = Some(id);
                } else {
                    game.camera_asset = Some(id);
                }
            }
            Err(diagnostics) => {
                tracing::error!("{path} failed to import:\n{diagnostics}");
            }
        }
    }

    // â”€â”€ The rotator demo asset â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    let text = std::fs::read_to_string(DEMO_SCRIPT_PATH)
        .unwrap_or_else(|_| DEMO_SCRIPT_SOURCE.to_string());
    let asset = somnium_script::ids::ScriptAssetId::mint();
    if let Err(diagnostics) = ctx.scripts.load_script(asset, DEMO_SCRIPT_PATH, text) {
        tracing::error!("demo script failed to compile:\n{diagnostics}");
        return;
    }

    // â”€â”€ The scripted entity â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    let mut transform = Transform::from_translation(Vec3::new(3.0, 1.5, 0.0));
    transform.scale = Vec3::splat(0.6);
    let entity = match (game.default_cube_alloc, game.default_material_id) {
        (Some(alloc), Some(material)) => ctx.world.spawn((
            transform,
            MeshComponent {
                vertex_offset: alloc.vertex_offset,
                index_offset: alloc.index_offset,
                index_count: alloc.index_count,
            },
            MaterialComponent {
                asset: somnium_asset::database::AssetId::NONE,
                runtime_id: material,
            },
            Name::new("Scripted Rotator"),
            WorldTransform::identity(),
            MeshKind::Cube,
            somnium_script::attachment::ScriptSet::new(),
        )),
        // The glTF scene loaded, so there is no procedural cube to borrow
        // geometry from. The entity is still real and the script still
        // runs on it; it simply has nothing to draw.
        _ => ctx.world.spawn((
            transform,
            Name::new("Scripted Rotator"),
            WorldTransform::identity(),
            somnium_script::attachment::ScriptSet::new(),
        )),
    };

    let mut attachment = somnium_script::attachment::ScriptAttachment::new(asset);
    attachment.properties.insert(
        "spinSpeed".into(),
        somnium_script::value::ScriptValue::F64(2.0),
    );
    if let Some(set) = ctx
        .world
        .get_mut::<somnium_script::attachment::ScriptSet>(entity)
    {
        set.attach(attachment);
    }

    info!("Phase 16-C: `Scripted Rotator` attached to {DEMO_SCRIPT_PATH} â€” press Play to run it");
}

// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”
// Procedural fallback scene
// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”

fn spawn_procedural_scene(
    ctx: &mut EngineContext,
) -> (u32, somnium_renderer::geometry::MeshAllocation) {
    let renderer = ctx.renderer.as_mut().unwrap();
    let render_ctx = ctx.render_ctx.as_ref().unwrap();

    let mat_blue = renderer.materials_pool.add_material(
        &render_ctx.queue,
        somnium_renderer::material::pool::GpuMaterial {
            base_color: [0.1, 0.1, 0.15, 1.0],
            roughness: 0.8,
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
            terrain_index: -1,
            porosity: 0.5,
            _pad: 0.0,
        },
    );
    let mat_red = renderer.materials_pool.add_material(
        &render_ctx.queue,
        somnium_renderer::material::pool::GpuMaterial {
            base_color: [0.8, 0.1, 0.1, 1.0],
            roughness: 0.2,
            metallic: 0.8,
            albedo_map: -1,
            normal_map: -1,
            metallic_roughness_map: -1,
            alpha_cutoff: 0.0,
            flags: 0,
            occlusion_map: -1,
            transmission: 0.0,
            emissive: [0.0; 3],
            emissive_map: -1,
            terrain_index: -1,
            porosity: 0.5,
            _pad: 0.0,
        },
    );

    // Use the procedural cube from somnium_asset (Phase 11.5D-2).
    let (cube_verts, cube_idxs) = somnium_asset::generate_cube(1.0);
    let cube_alloc =
        renderer
            .geometry
            .upload_mesh(&render_ctx.queue, &cube_verts, &cube_idxs, mat_blue);

    // Floor
    ctx.physics.create_body(RigidBodyDescriptor {
        shape: ColliderShape::Box {
            half_extents: Vec3::new(10.0, 0.1, 10.0),
        },
        position: Vec3::new(0.0, -1.0, 0.0),
        motion_type: MotionType::Static,
        object_layer: LAYER_NON_MOVING,
        ..Default::default()
    });
    ctx.world.spawn((
        Transform::from_translation(Vec3::new(0.0, -1.0, 0.0)),
        MeshComponent {
            vertex_offset: cube_alloc.vertex_offset,
            index_offset: cube_alloc.index_offset,
            index_count: cube_alloc.index_count,
        },
        MaterialComponent {
            asset: somnium_asset::database::AssetId::NONE,
            runtime_id: mat_blue,
        },
        Name::new("Floor"),
        WorldTransform::identity(),
        MeshKind::Cube,
    ));

    // Player cube (physics-driven)
    let player_body = ctx.physics.create_body(RigidBodyDescriptor {
        shape: ColliderShape::Box {
            half_extents: Vec3::new(0.5, 0.5, 0.5),
        },
        position: Vec3::new(0.0, 5.0, 0.0),
        motion_type: MotionType::Dynamic,
        object_layer: LAYER_MOVING,
        ..Default::default()
    });
    ctx.world.spawn((
        Transform::from_translation(Vec3::new(0.0, 5.0, 0.0)),
        PhysicsBody { id: player_body },
        MeshComponent {
            vertex_offset: cube_alloc.vertex_offset,
            index_offset: cube_alloc.index_offset,
            index_count: cube_alloc.index_count,
        },
        MaterialComponent {
            asset: somnium_asset::database::AssetId::NONE,
            runtime_id: mat_red,
        },
        Name::new("Player"),
        WorldTransform::identity(),
        MeshKind::Cube,
    ));

    // Static PBR metal cube
    let pbr_mat = {
        let renderer = ctx.renderer.as_mut().unwrap();
        let render_ctx = ctx.render_ctx.as_ref().unwrap();
        renderer.materials_pool.add_material(
            &render_ctx.queue,
            somnium_renderer::material::pool::GpuMaterial {
                base_color: [1.0, 1.0, 1.0, 1.0],
                roughness: 0.1,
                metallic: 0.9,
                albedo_map: -1,
                normal_map: -1,
                metallic_roughness_map: -1,
                alpha_cutoff: 0.0,
                flags: 0,
                occlusion_map: -1,
                transmission: 0.0,
                emissive: [0.0; 3],
                emissive_map: -1,
                terrain_index: -1,
                porosity: 0.5,
                _pad: 0.0,
            },
        )
    };
    ctx.world.spawn((
        Transform::from_translation(Vec3::new(2.0, 1.0, 0.0)),
        MeshComponent {
            vertex_offset: cube_alloc.vertex_offset,
            index_offset: cube_alloc.index_offset,
            index_count: cube_alloc.index_count,
        },
        MaterialComponent {
            asset: somnium_asset::database::AssetId::NONE,
            runtime_id: pbr_mat,
        },
        Name::new("MetalCube"),
        WorldTransform::identity(),
        MeshKind::Cube,
    ));

    (mat_blue, cube_alloc)
}

// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”
// Content browser helper
// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”

fn list_assets_dir() -> Vec<serde_json::Value> {
    let Ok(entries) = std::fs::read_dir("assets") else {
        return Vec::new();
    };
    entries
        .filter_map(|e| {
            let e = e.ok()?;
            let name = e.file_name().to_string_lossy().to_string();
            let is_dir = e.file_type().ok()?.is_dir();
            let ext = std::path::Path::new(&name)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            Some(serde_json::json!({ "name": name, "is_dir": is_dir, "ext": ext }))
        })
        .collect()
}

// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”
// Entry point
// â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”

/// Exact logical window extent for deterministic UI evidence.
///
/// Ordinary runs never consult a second configuration source. The override is
/// audit-only, explicit, and intentionally refuses zero or malformed extents.
fn audit_window_size() -> Option<(u32, u32)> {
    let raw = std::env::var("SOMNIUM_AUDIT_WINDOW_SIZE").ok()?;
    let (width, height) = raw.split_once(['x', 'X'])?;
    let size = (width.trim().parse().ok()?, height.trim().parse().ok()?);
    (size.0 > 0 && size.1 > 0).then_some(size)
}

fn main() -> Result<(), somnium_core::EngineError> {
    let config = EngineConfig {
        window_title: "Somnium Engine".into(),
        window_size: audit_window_size().unwrap_or((1280, 720)),
        target_fps: Some(60),
        ..Default::default()
    };
    Engine::run(config, HelloGame::new())
}

#[cfg(test)]
mod audit_harness_tests {
    #[test]
    fn audit_window_extent_parser_accepts_the_evidence_sizes() {
        // Keep environment mutation out of a parallel test: this verifies the
        // same grammar without setting a process-global variable.
        let parse = |raw: &str| {
            let (width, height) = raw.split_once(['x', 'X'])?;
            let size = (
                width.trim().parse::<u32>().ok()?,
                height.trim().parse::<u32>().ok()?,
            );
            (size.0 > 0 && size.1 > 0).then_some(size)
        };
        assert_eq!(parse("1280x720"), Some((1280, 720)));
        assert_eq!(parse("1920X1080"), Some((1920, 1080)));
        assert_eq!(parse("0x1080"), None);
        assert_eq!(parse("wide"), None);
    }
}
