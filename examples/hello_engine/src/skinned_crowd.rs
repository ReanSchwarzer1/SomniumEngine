//! Skinned bodies for crowds: many entities sharing one skinned glTF.
//!
//! A **Skinned Body** (`hello.SkinnedBody`) draws a skinned glTF at its
//! entity, playing one of its clips (looping) on the body's own clock. Unlike
//! the Animated Mesh Preview, the file is read and its rest meshes uploaded
//! once per source, however many bodies use it; each body adds only its own
//! posed vertices. Distance thins the work the way `somnium.AnimationLod`
//! does: poses are sampled every frame near the camera, less often further
//! out (the skipped time is kept), and past the cull distance a body is not
//! drawn at all and gives its posed vertices back, so the GPU skins nothing
//! for it. With **LOD 1** and **LOD 2** set (the same skeleton and clips, fewer
//! triangles: WS3's `<name>_lod1.glb`, `<name>_lod2.glb`), a body draws them
//! beyond their distances, switching a couple of metres past each line and
//! back a couple short of it so a body on the line does not flicker.
//!
//! **Create > Examples > Skinned Crowd** lays out a field of them (its Count
//! sets how many) to see and measure; the crowd's numbers are in the MCP
//! authoring state (`skinned`) and the log every ten seconds.

use glam::{Mat4, Vec3};
use serde_json::{Value, json};
use somnium_anim::Playback;
use somnium_asset::{animated::AnimatedAsset, database::AssetId};
use somnium_core::animation_lod::{AnimationLod, AnimationStep};
use somnium_core::{EngineContext, Name, Parent, Transform, WorldTransform};
use somnium_ecs::{Component, Entity, World, component_schema, reflect::TypeRegistry};
use somnium_renderer::{
    SomniumRenderer, animated_geometry::AnimatedMeshId, geometry::MeshAllocation,
};
use std::collections::HashMap;
use std::time::Instant;
use tracing::{info, warn};

/// A skinned body: which glTF, which clip, how fast, and where in it to start.
#[derive(Clone, Debug, PartialEq)]
pub struct SkinnedBody {
    pub source: AssetId,
    pub clip: String,
    pub speed: f32,
    pub time: f32,
    pub lod1: AssetId,
    pub lod2: AssetId,
    pub lod1_beyond: f32,
    pub lod2_beyond: f32,
}

impl Component for SkinnedBody {}

impl Default for SkinnedBody {
    fn default() -> Self {
        Self {
            source: AssetId::from_relative_path("models/animation_demo.gltf"),
            clip: String::new(),
            speed: 1.0,
            time: 0.0,
            lod1: AssetId::NONE,
            lod2: AssetId::NONE,
            lod1_beyond: 25.0,
            lod2_beyond: 60.0,
        }
    }
}

fn schema(r: &mut TypeRegistry) {
    r.register(component_schema! {
        SkinnedBody as "hello.SkinnedBody", display "Skinned Body", version 1,
        fields {
            source { group: "Body", asset_kind_mask: somnium_asset::database::ASSET_KIND_MESH, doc: "A skinned glTF/GLB (one skeleton, baked clips). Read once however many bodies share it." },
            clip { group: "Body", doc: "Clip name; empty plays the first." },
            speed { group: "Body", min: -4.0, max: 4.0 },
            time { group: "Body", min: 0.0, max: 3600.0, unit: "s", doc: "Where in the clip this body starts, so a crowd is not in step." },
            lod1 { group: "LOD", asset_kind_mask: somnium_asset::database::ASSET_KIND_MESH, display_name: "LOD 1", doc: "Fewer triangles, the same skeleton and clips; drawn beyond LOD 1 Beyond. Empty: the source everywhere." },
            lod2 { group: "LOD", asset_kind_mask: somnium_asset::database::ASSET_KIND_MESH, display_name: "LOD 2", doc: "Fewer still; drawn beyond LOD 2 Beyond (only with LOD 1 set)." },
            lod1_beyond { group: "LOD", min: 0.0, unit: "m", display_name: "LOD 1 Beyond" },
            lod2_beyond { group: "LOD", min: 0.0, unit: "m", display_name: "LOD 2 Beyond" },
        }
    });
}

pub fn register(r: &mut somnium_core::authoring::GameRegistration) {
    r.components.push(schema);
    r.presets.push(somnium_core::authoring::GamePreset {
        id: "hello.skinned_crowd",
        label: "Skinned Crowd",
        category: "Examples",
        schema: json!({"type":"object","properties":{
            "position":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},
            "count":{"type":"integer","minimum":1,"maximum":1000,"description":"How many bodies."}
        },"additionalProperties":false}),
        build: spawn,
    });
}

fn spawn(world: &mut World, args: &Value) -> Result<Vec<Entity>, String> {
    let at: [f32; 3] =
        serde_json::from_value(args.get("position").cloned().unwrap_or(json!([0, 0, 0])))
            .map_err(|_| "position needs [x,y,z]")?;
    if at.iter().any(|v| !v.is_finite()) {
        return Err("position must be finite".into());
    }
    let count = args
        .get("count")
        .and_then(Value::as_u64)
        .unwrap_or(100)
        .clamp(1, 1000) as usize;
    Ok(spawn_crowd(world, Vec3::from(at), count))
}

/// A square of `count` bodies 2.5 m apart, under one root.
fn spawn_crowd(world: &mut World, at: Vec3, count: usize) -> Vec<Entity> {
    let root = world.spawn((
        Name::new("Skinned Crowd"),
        Transform::from_translation(at),
        WorldTransform::identity(),
    ));
    let side = (count as f32).sqrt().ceil() as usize;
    let mut out = vec![root];
    for i in 0..count {
        let (x, z) = ((i % side) as f32, (i / side) as f32);
        let half = side as f32 * 0.5;
        out.push(world.spawn((
            Name::new(&format!("Body {}", i + 1)),
            Transform::from_translation(Vec3::new((x - half) * 2.5, 0.0, (z - half) * 2.5)),
            WorldTransform::identity(),
            Parent { entity: root },
            SkinnedBody {
                time: (i as f32 * 0.37) % 4.0,
                ..Default::default()
            },
        )));
    }
    out
}

/// Build the crowd at the origin outside the Create menu (a capture hook):
/// `SOMNIUM_SKINNED_CROWD=<count>`.
pub fn spawn_at_startup(world: &mut World) -> Option<usize> {
    let count = std::env::var("SOMNIUM_SKINNED_CROWD")
        .ok()?
        .parse::<usize>()
        .ok()?;
    Some(spawn_crowd(world, Vec3::ZERO, count.clamp(1, 1000)).len() - 1)
}

/// One glTF shared by every body that draws it.
struct Shared {
    asset: AnimatedAsset,
    /// Each skinned mesh: its index in the scene, its rest allocation, its material.
    meshes: Vec<(usize, MeshAllocation, u32)>,
    users: usize,
}

struct Body {
    /// The source of the level it draws now.
    source: AssetId,
    level: usize,
    /// Its posed copy of each shared mesh, while within the cull distance.
    skins: Vec<(AnimatedMeshId, u32)>,
    clock: f32,
    owed: f32,
    palette: Vec<Mat4>,
    /// The palette holds a sampled pose.
    posed: bool,
}

#[derive(Default)]
struct Stats {
    bodies: usize,
    drawn: usize,
    sampled: usize,
    sample_ms: f64,
    frames: u32,
    reported: f32,
}

/// The bodies, and the glTFs they share.
#[derive(Default)]
pub struct SkinnedBodies {
    shared: HashMap<AssetId, Shared>,
    bodies: HashMap<Entity, Body>,
    failed: HashMap<AssetId, String>,
    lod: AnimationLod,
    stats: Stats,
}

fn world_matrix(world: &World, e: Entity) -> Mat4 {
    world
        .get::<WorldTransform>(e)
        .map(|w| w.0)
        .or_else(|| world.get::<Transform>(e).map(Transform::to_matrix))
        .unwrap_or(Mat4::IDENTITY)
}

fn load(
    gpu: &somnium_renderer::RenderContext,
    r: &mut SomniumRenderer,
    path: &std::path::Path,
) -> Result<Shared, String> {
    let asset = AnimatedAsset::load(path).map_err(|e| e.to_string())?;
    if asset.clips.is_empty() {
        return Err("no skeletal clips".into());
    }
    let materials = r.upload_scene_materials(gpu, &asset.scene);
    let mut meshes = Vec::new();
    for (index, mesh) in asset.scene.meshes.iter().enumerate() {
        if mesh.skin.is_none() {
            continue;
        }
        let material = asset
            .scene
            .nodes
            .iter()
            .find(|n| n.mesh_index == Some(index))
            .and_then(|n| n.material_index)
            .and_then(|m| materials.get(m))
            .copied()
            .unwrap_or(0);
        let rest =
            r.geometry
                .upload_mesh_pooled(&gpu.queue, &mesh.vertices, &mesh.indices, material);
        if rest.vertex_count as usize != mesh.vertices.len()
            || rest.index_count as usize != mesh.indices.len()
        {
            r.geometry.free_mesh(rest);
            for (_, rest, _) in meshes {
                r.geometry.free_mesh(rest);
            }
            return Err("rest geometry pool is full".into());
        }
        meshes.push((index, rest, material));
    }
    if meshes.is_empty() {
        return Err("no skinned meshes".into());
    }
    Ok(Shared {
        asset,
        meshes,
        users: 0,
    })
}

/// A body's levels: its source, then LOD 1 and LOD 2 while each is set.
fn levels(b: &SkinnedBody) -> Vec<AssetId> {
    std::iter::once(b.source)
        .chain(
            [b.lod1, b.lod2]
                .into_iter()
                .take_while(|id| *id != AssetId::NONE),
        )
        .collect()
}

/// Metres past a line before a body switches level (and short of it before it
/// switches back).
const LOD_SLACK: f32 = 2.0;

/// The level a body at `distance` draws, of `count`, given the one it draws.
fn level_for(distance: f32, current: usize, lines: [f32; 2], count: usize) -> usize {
    let current = current.min(count.saturating_sub(1));
    let want = lines
        .iter()
        .filter(|line| distance > **line)
        .count()
        .min(count.saturating_sub(1));
    if want > current && distance <= lines[want - 1] + LOD_SLACK {
        return current;
    }
    if want < current && distance >= lines[current - 1] - LOD_SLACK {
        return current;
    }
    want
}

/// Give a body's posed vertices back.
fn unregister(r: &mut SomniumRenderer, body: &mut Body) {
    for (skin, _) in body.skins.drain(..) {
        r.animated_geometry.remove(skin, &mut r.geometry);
    }
    body.posed = false;
}

impl SkinnedBodies {
    /// For MCP and automation.
    pub fn state(&self) -> Value {
        let s = &self.stats;
        json!({
            "bodies": s.bodies,
            "drawn": s.drawn,
            "sampled": s.sampled,
            "sample_ms": s.sample_ms,
            "sources": self.shared.len(),
            "errors": self.failed.values().collect::<Vec<_>>(),
        })
    }

    /// Sample, skin and draw every body this frame.
    pub fn render(&mut self, ctx: &mut EngineContext) {
        let Some(gpu) = ctx.render_ctx else { return };
        let Some(r) = ctx.renderer.as_deref_mut() else {
            return;
        };
        let camera = r.camera_pos;
        let dt = ctx.time.dt().min(0.1);
        let running = ctx.simulation.state != somnium_core::SimulationState::Paused;
        let wanted: Vec<(Entity, SkinnedBody, Mat4)> = ctx
            .world
            .iter_with::<SkinnedBody>()
            .map(|(e, b)| (e, b.clone(), world_matrix(ctx.world, e)))
            .collect();
        // Bodies whose entity went, or that now draw another source.
        let now: HashMap<Entity, Vec<AssetId>> =
            wanted.iter().map(|(e, b, _)| (*e, levels(b))).collect();
        let gone: Vec<Entity> = self
            .bodies
            .iter()
            .filter(|(e, b)| {
                !now.get(e)
                    .is_some_and(|ids| ids.get(b.level) == Some(&b.source))
            })
            .map(|(e, _)| *e)
            .collect();
        for e in gone {
            if let Some(mut body) = self.bodies.remove(&e) {
                unregister(r, &mut body);
                if let Some(shared) = self.shared.get_mut(&body.source) {
                    shared.users -= 1;
                }
            }
        }
        let unused: Vec<AssetId> = self
            .shared
            .iter()
            .filter(|(_, s)| s.users == 0)
            .map(|(id, _)| *id)
            .collect();
        for id in unused {
            if let Some(shared) = self.shared.remove(&id) {
                for (_, rest, _) in shared.meshes {
                    r.geometry.free_mesh(rest);
                }
            }
        }
        let started = Instant::now();
        let (mut drawn, mut sampled) = (0, 0);
        for (e, settings, matrix) in wanted {
            let distance = matrix.w_axis.truncate().distance(camera);
            let ids = levels(&settings);
            let current = self.bodies.get(&e).map_or(0, |b| b.level);
            let level = level_for(
                distance,
                current,
                [settings.lod1_beyond, settings.lod2_beyond],
                ids.len(),
            );
            let id = ids[level];
            if !self.shared.contains_key(&id) {
                if self.failed.contains_key(&id) {
                    continue;
                }
                let Some(record) = ctx.ui.asset_record(id) else {
                    continue;
                };
                match load(gpu, r, &record.absolute_path) {
                    Ok(shared) => {
                        info!(clips = ?shared.asset.clips.keys().collect::<Vec<_>>(), "skinned body source loaded");
                        self.shared.insert(id, shared);
                    }
                    Err(error) => {
                        warn!(%error, "skinned body source failed");
                        self.failed.insert(id, error);
                        continue;
                    }
                }
            }
            // Moving to another level: give the old one's posed vertices back.
            if let Some(body) = self.bodies.get_mut(&e)
                && body.source != id
            {
                unregister(r, body);
                if let Some(old) = self.shared.get_mut(&body.source) {
                    old.users -= 1;
                }
                let shared = self.shared.get_mut(&id).expect("loaded above");
                shared.users += 1;
                body.source = id;
                body.level = level;
                body.palette = vec![Mat4::IDENTITY; shared.asset.skeleton.len()];
            }
            let shared = self.shared.get_mut(&id).expect("loaded above");
            let body = self.bodies.entry(e).or_insert_with(|| {
                shared.users += 1;
                Body {
                    source: id,
                    level,
                    skins: Vec::new(),
                    clock: settings.time,
                    owed: 0.0,
                    palette: vec![Mat4::IDENTITY; shared.asset.skeleton.len()],
                    posed: false,
                }
            });
            if running {
                body.clock += dt * settings.speed;
                body.owed += dt;
            }
            if distance > self.lod.cull {
                if !body.skins.is_empty() {
                    unregister(r, body);
                }
                continue;
            }
            if body.skins.is_empty() {
                for (index, rest, _) in &shared.meshes {
                    let mesh = &shared.asset.scene.meshes[*index];
                    let skin = mesh.skin.as_ref().expect("kept only skinned meshes");
                    match r.animated_geometry.register(
                        gpu,
                        &mut r.geometry,
                        &mut r.skin_pass,
                        &shared.asset.skeleton,
                        skin,
                        rest.vertex_offset,
                        &mesh.vertices,
                    ) {
                        Ok(posed) => body.skins.push(posed),
                        Err(error) => {
                            warn!(%error, "skinned body could not be posed");
                            break;
                        }
                    }
                }
                if body.skins.len() != shared.meshes.len() {
                    unregister(r, body);
                    continue;
                }
            }
            let due = matches!(
                self.lod.step(distance, &mut body.owed),
                AnimationStep::Evaluate { .. }
            );
            if due || !body.posed {
                let clip = if settings.clip.is_empty() {
                    shared.asset.clips.values().next()
                } else {
                    shared.asset.clips.get(&settings.clip)
                };
                if let Some(clip) = clip
                    && let Ok(pose) =
                        clip.sample(&shared.asset.skeleton, body.clock, Playback::LOOPING)
                    && pose.to_palette(&shared.asset.skeleton, &mut body.palette)
                {
                    for (skin, _) in &body.skins {
                        r.animated_geometry.update(*skin, &body.palette);
                    }
                    body.posed = true;
                    sampled += 1;
                }
            }
            for ((_, rest, material), (_, posed)) in shared.meshes.iter().zip(&body.skins) {
                r.submit(somnium_renderer::command::DrawCommand {
                    paint: 0,
                    sort_key: somnium_renderer::command::SortKey::new(0, *material as u16, *posed),
                    vertex_offset: *posed,
                    index_offset: rest.index_offset,
                    index_count: rest.index_count,
                    material_id: *material,
                    transform: matrix,
                    casts_shadow: true,
                });
            }
            drawn += 1;
        }
        if drawn > 0 {
            r.invalidate_shadow_casters();
        }
        let s = &mut self.stats;
        s.bodies = self.bodies.len();
        s.drawn = drawn;
        s.sampled = sampled;
        s.frames += 1;
        let k = 1.0 / f64::from(s.frames.min(120));
        s.sample_ms += (started.elapsed().as_secs_f64() * 1000.0 - s.sample_ms) * k;
        s.reported += dt;
        if s.bodies > 0 && s.reported >= 10.0 {
            s.reported = 0.0;
            info!(
                bodies = s.bodies,
                drawn = s.drawn,
                sampled = s.sampled,
                cpu_ms = s.sample_ms,
                "skinned crowd"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The preset lays out as many bodies as asked, under one root, out of
    /// step with each other.
    #[test]
    fn the_crowd_preset_lays_out_its_bodies() {
        let mut world = World::new();
        let made = spawn(&mut world, &json!({"count": 50})).unwrap();
        assert_eq!(made.len(), 51);
        let bodies: Vec<_> = world
            .iter_with::<SkinnedBody>()
            .map(|(_, b)| b.time)
            .collect();
        assert_eq!(bodies.len(), 50);
        let mut starts = bodies.clone();
        starts.dedup();
        assert!(starts.len() > 10, "not in step");
        assert!(spawn(&mut world, &json!({"position": [f32::NAN, 0, 0]})).is_err());
    }

    /// Levels switch past each line with some slack either way, and never to
    /// a level that is not set.
    #[test]
    fn levels_switch_past_their_lines_without_flicker() {
        let lines = [25.0, 60.0];
        assert_eq!(level_for(10.0, 0, lines, 3), 0);
        assert_eq!(level_for(26.0, 0, lines, 3), 0, "within the slack");
        assert_eq!(level_for(28.0, 0, lines, 3), 1);
        assert_eq!(
            level_for(24.0, 1, lines, 3),
            1,
            "within the slack coming back"
        );
        assert_eq!(level_for(22.0, 1, lines, 3), 0);
        assert_eq!(level_for(100.0, 0, lines, 3), 2);
        assert_eq!(level_for(100.0, 0, lines, 2), 1, "no LOD 2");
        assert_eq!(level_for(100.0, 0, lines, 1), 0, "no LODs");
        assert_eq!(level_for(10.0, 2, lines, 1), 0, "lost its LODs");
        let body = SkinnedBody {
            lod2: AssetId::from_relative_path("x_lod2.glb"),
            ..Default::default()
        };
        assert_eq!(levels(&body).len(), 1, "LOD 2 without LOD 1 is not used");
    }
}
