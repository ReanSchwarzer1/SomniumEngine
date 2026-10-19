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
//! back a couple short of it so a body on the line does not flicker. A budget
//! of posed vertices holds whatever the distances (the renderer skins about
//! 2 M; WS3's crowd contract): nearest first, each body keeps the level its
//! distance wants only while what is left still covers everyone behind it at
//! their coarsest, so a packed market is coarsened from the back.
//!
//! A **Skinned Socket** (`hello.SkinnedSocket`) on an entity under a Skinned
//! Body makes it follow one of the body's joints (`weapon_r`, `sheath`,
//! `back`...): its own Transform is its offset from the joint, and it and
//! everything under it are placed the same frame the pose is, before meshes
//! are drawn, so a sword does not trail the hand.
//!
//! **Create > Examples > Skinned Crowd** lays out a field of them (its Count
//! sets how many) to see and measure; the crowd's numbers are in the MCP
//! authoring state (`skinned`) and the log every ten seconds.

use glam::{Mat4, Vec3};
use serde_json::{Value, json};
use somnium_anim::Playback;
use somnium_asset::{animated::AnimatedAsset, database::AssetId};
use somnium_core::animation_lod::{AnimationLod, AnimationStep};
use somnium_core::{
    EngineContext, MaterialComponent, MeshKind, Name, Parent, Transform, WorldTransform,
};
use somnium_ecs::{Component, Entity, World, component_schema, reflect::TypeRegistry};
use somnium_life::crowd::fit_levels as fit;
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

/// Follow a joint of the Skinned Body this entity is parented to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkinnedSocket {
    pub joint: String,
}

impl Component for SkinnedSocket {}

impl Default for SkinnedBody {
    fn default() -> Self {
        Self {
            source: AssetId::from_relative_path("models/animation_demo.gltf"),
            clip: String::new(),
            speed: 1.0,
            time: 0.0,
            lod1: AssetId::NONE,
            lod2: AssetId::NONE,
            lod1_beyond: 20.0,
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
    r.register(component_schema! {
        SkinnedSocket as "hello.SkinnedSocket", display "Skinned Socket", version 1,
        fields {
            joint { group: "Socket", doc: "A joint of the parent Skinned Body (weapon_r, weapon_l, spell_r, spell_l, sheath, back). This entity's own Transform is its offset from it." },
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

/// A square of `count` bodies 2.5 m apart, under one root, each with a ball
/// on its `tip` joint (a socket, to see that sockets keep up).
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
        let body = world.spawn((
            Name::new(&format!("Body {}", i + 1)),
            Transform::from_translation(Vec3::new((x - half) * 2.5, 0.0, (z - half) * 2.5)),
            WorldTransform::identity(),
            Parent { entity: root },
            SkinnedBody {
                time: (i as f32 * 0.37) % 4.0,
                ..Default::default()
            },
        ));
        let ball = world.spawn((
            Name::new(&format!("Body {} ball", i + 1)),
            // The demo rig's tip joint is its bend, a metre up a 2 m tube:
            // the ball sits on the tube's end, along the bent half.
            Transform {
                translation: Vec3::new(0.0, 1.05, 0.0),
                scale: Vec3::splat(0.5),
                ..Default::default()
            },
            WorldTransform::identity(),
            Parent { entity: body },
            SkinnedSocket {
                joint: "tip".into(),
            },
            MeshKind::Sphere,
            MaterialComponent {
                asset: AssetId::from_relative_path(std::path::Path::new(
                    "materials/combat/fire.sommat",
                )),
                runtime_id: 0,
            },
        ));
        out.extend([body, ball]);
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
    let count = count.clamp(1, 1000);
    spawn_crowd(world, Vec3::ZERO, count);
    Some(count)
}

/// One glTF shared by every body that draws it.
struct Shared {
    asset: AnimatedAsset,
    /// Each skinned mesh: its index in the scene, its rest allocation, its material.
    meshes: Vec<(usize, MeshAllocation, u32)>,
    users: usize,
    /// Posed vertices one body of it costs.
    vertices: u32,
}

struct Body {
    /// The source of the level it draws now.
    source: AssetId,
    level: usize,
    /// Its posed vertices were refused (the skinned budget is full): when to
    /// ask again.
    retry: Option<Instant>,
    /// Its posed copy of each shared mesh, while within the cull distance.
    skins: Vec<(AnimatedMeshId, u32)>,
    clock: f32,
    owed: f32,
    palette: Vec<Mat4>,
    /// Each joint in the body's space, for sockets.
    model: Vec<Mat4>,
    /// Where the body stands, as last drawn.
    matrix: Mat4,
    /// The palette holds a sampled pose.
    posed: bool,
}

#[derive(Default)]
struct Stats {
    bodies: usize,
    drawn: usize,
    sampled: usize,
    /// Posed vertices the drawn bodies hold.
    posed: u32,
    sample_ms: f64,
    /// Of it, loading sources and fitting levels to the budget.
    plan_ms: f64,
    frames: u32,
    reported: f32,
}

/// The bodies, and the glTFs they share.
pub struct SkinnedBodies {
    /// Posed vertices all skinned bodies may hold together.
    pub vertex_budget: u32,
    shared: HashMap<AssetId, Shared>,
    bodies: HashMap<Entity, Body>,
    failed: HashMap<AssetId, String>,
    lod: AnimationLod,
    stats: Stats,
    /// Each socket's descendants, parents first, and the entity count they
    /// were found at.
    subtrees: HashMap<Entity, Vec<Entity>>,
    subtrees_for: usize,
}

impl Default for SkinnedBodies {
    fn default() -> Self {
        Self {
            vertex_budget: VERTEX_BUDGET,
            shared: HashMap::new(),
            bodies: HashMap::new(),
            failed: HashMap::new(),
            lod: AnimationLod::default(),
            stats: Stats::default(),
            subtrees: HashMap::new(),
            subtrees_for: 0,
        }
    }
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
    let vertices = meshes
        .iter()
        .map(|(i, _, _)| asset.scene.meshes[*i].vertices.len() as u32)
        .sum();
    Ok(Shared {
        asset,
        meshes,
        users: 0,
        vertices,
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

/// Posed vertices for every skinned body together, by default: WS3's crowd
/// contract gives the renderer about 2 M, and the cast and fighters need the rest.
const VERTEX_BUDGET: u32 = 1_600_000;

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
    /// Put each socket (and everything under it) on its joint, as posed this
    /// frame; transforms have already been propagated, so this writes the
    /// world matrices the meshes are about to be drawn with.
    fn place_sockets(&mut self, world: &mut World) {
        if self.subtrees_for != world.entity_count() {
            self.subtrees_for = world.entity_count();
            self.subtrees.clear();
            let sockets: Vec<Entity> = world.iter_with::<SkinnedSocket>().map(|(e, _)| e).collect();
            if !sockets.is_empty() {
                let mut children: HashMap<Entity, Vec<Entity>> = HashMap::new();
                for (e, parent) in world.iter_with::<Parent>() {
                    children.entry(parent.entity).or_default().push(e);
                }
                for socket in sockets {
                    let mut below = Vec::new();
                    let mut next = vec![socket];
                    while let Some(e) = next.pop() {
                        for child in children.get(&e).into_iter().flatten() {
                            below.push(*child);
                            next.push(*child);
                        }
                    }
                    self.subtrees.insert(socket, below);
                }
            }
        }
        let sockets: Vec<(Entity, String, Entity, Mat4)> = world
            .iter_with::<SkinnedSocket>()
            .filter_map(|(e, s)| {
                let parent = world.get::<Parent>(e)?.entity;
                let local = world.get::<Transform>(e)?.to_matrix();
                Some((e, s.joint.clone(), parent, local))
            })
            .collect();
        for (socket, joint, parent, local) in sockets {
            let Some(body) = self.bodies.get(&parent).filter(|b| b.posed) else {
                continue;
            };
            let Some(joint) = self
                .shared
                .get(&body.source)
                .and_then(|s| s.asset.joint(&joint))
            else {
                continue;
            };
            let mut placed =
                HashMap::from([(socket, body.matrix * body.model[usize::from(joint)] * local)]);
            for e in self.subtrees.get(&socket).into_iter().flatten() {
                let (Some(parent), Some(local)) = (
                    world.get::<Parent>(*e).map(|p| p.entity),
                    world.get::<Transform>(*e).map(Transform::to_matrix),
                ) else {
                    continue;
                };
                if let Some(above) = placed.get(&parent).copied() {
                    placed.insert(*e, above * local);
                }
            }
            for (e, matrix) in placed {
                if let Some(w) = world.get_mut::<WorldTransform>(e) {
                    w.0 = matrix;
                }
            }
        }
    }

    /// For MCP and automation.
    pub fn state(&self) -> Value {
        let s = &self.stats;
        json!({
            "bodies": s.bodies,
            "drawn": s.drawn,
            "sampled": s.sampled,
            "sample_ms": s.sample_ms,
            "plan_ms": s.plan_ms,
            "sources": self.shared.len(),
            "posed_vertices": [s.posed, self.vertex_budget],
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
        // A source stays while any body could draw it: every level is kept, so
        // the budget knows each one's size.
        let wanted_ids: std::collections::HashSet<AssetId> =
            now.values().flatten().copied().collect();
        let unused: Vec<AssetId> = self
            .shared
            .keys()
            .filter(|id| !wanted_ids.contains(id))
            .copied()
            .collect();
        for id in unused {
            if let Some(shared) = self.shared.remove(&id) {
                for (_, rest, _) in shared.meshes {
                    r.geometry.free_mesh(rest);
                }
            }
        }
        let started = Instant::now();
        let (mut drawn, mut sampled, mut posed) = (0, 0, 0);
        for id in wanted_ids {
            if self.shared.contains_key(&id) || self.failed.contains_key(&id) {
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
                }
            }
        }
        // Each drawn body's level: its distance's, coarsened from the back until
        // the crowd fits the vertex budget.
        let mut plan: Vec<(f32, Entity, usize, Vec<u32>)> = wanted
            .iter()
            .filter_map(|(e, b, m)| {
                let distance = m.w_axis.truncate().distance(camera);
                let sizes: Vec<u32> = levels(b)
                    .iter()
                    .map_while(|id| self.shared.get(id).map(|s| s.vertices))
                    .collect();
                if distance > self.lod.cull || sizes.is_empty() {
                    return None;
                }
                let current = self.bodies.get(e).map_or(0, |x| x.level);
                let lines = [b.lod1_beyond, b.lod2_beyond];
                Some((
                    distance,
                    *e,
                    level_for(distance, current, lines, sizes.len()),
                    sizes,
                ))
            })
            .collect();
        plan.sort_by(|a, b| a.0.total_cmp(&b.0));
        let wants: Vec<usize> = plan.iter().map(|p| p.2).collect();
        let sizes: Vec<Vec<u32>> = plan.iter().map(|p| p.3.clone()).collect();
        let fitted: HashMap<Entity, usize> = plan
            .iter()
            .zip(fit(self.vertex_budget, &wants, &sizes))
            .map(|(p, level)| (p.1, level))
            .collect();
        let planned = started.elapsed().as_secs_f64() * 1000.0;
        for (e, settings, matrix) in wanted {
            let distance = matrix.w_axis.truncate().distance(camera);
            let ids = levels(&settings);
            // Culled (or not loaded): the level it had; it is not drawn.
            let level = fitted
                .get(&e)
                .copied()
                .unwrap_or_else(|| self.bodies.get(&e).map_or(0, |b| b.level))
                .min(ids.len() - 1);
            let id = ids[level];
            if !self.shared.contains_key(&id) {
                continue;
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
                body.model = vec![Mat4::IDENTITY; shared.asset.skeleton.len()];
            }
            let shared = self.shared.get_mut(&id).expect("loaded above");
            let body = self.bodies.entry(e).or_insert_with(|| {
                shared.users += 1;
                Body {
                    source: id,
                    level,
                    retry: None,
                    skins: Vec::new(),
                    clock: settings.time,
                    owed: 0.0,
                    palette: vec![Mat4::IDENTITY; shared.asset.skeleton.len()],
                    model: vec![Mat4::IDENTITY; shared.asset.skeleton.len()],
                    matrix: Mat4::IDENTITY,
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
                if body.retry.is_some_and(|at| Instant::now() < at) {
                    continue;
                }
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
                            // Once, not every frame: it asks again in a second.
                            if body.retry.is_none() {
                                warn!(%error, "skinned body could not be posed");
                            }
                            break;
                        }
                    }
                }
                if body.skins.len() != shared.meshes.len() {
                    unregister(r, body);
                    body.retry = Some(Instant::now() + std::time::Duration::from_secs(1));
                    continue;
                }
                body.retry = None;
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
                    && pose.to_model_space(&shared.asset.skeleton, &mut body.model)
                {
                    let skeleton = &shared.asset.skeleton;
                    for (i, joint) in body.model.iter().enumerate().take(skeleton.len()) {
                        body.palette[i] = *joint * skeleton.inverse_bind()[i];
                    }
                    for (skin, _) in &body.skins {
                        r.animated_geometry.update(*skin, &body.palette);
                    }
                    body.posed = true;
                    sampled += 1;
                }
            }
            body.matrix = matrix;
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
            posed += shared.vertices;
        }
        if drawn > 0 {
            r.invalidate_shadow_casters();
        }
        self.place_sockets(ctx.world);
        let s = &mut self.stats;
        s.bodies = self.bodies.len();
        s.drawn = drawn;
        s.posed = posed;
        s.sampled = sampled;
        s.frames += 1;
        let k = 1.0 / f64::from(s.frames.min(120));
        s.sample_ms += (started.elapsed().as_secs_f64() * 1000.0 - s.sample_ms) * k;
        s.plan_ms += (planned - s.plan_ms) * k;
        s.reported += dt;
        if s.bodies > 0 && s.reported >= 10.0 {
            s.reported = 0.0;
            info!(
                bodies = s.bodies,
                drawn = s.drawn,
                sampled = s.sampled,
                cpu_ms = s.sample_ms,
                plan_ms = s.plan_ms,
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
        assert_eq!(made.len(), 101, "a root, and a body and its ball each");
        assert_eq!(world.iter_with::<SkinnedSocket>().count(), 50);
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

    /// A socket and what hangs from it land on the joint as posed, the same
    /// frame (no GPU: the skeleton is read on the CPU).
    #[test]
    fn a_socket_follows_its_joint() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/models/animation_demo.gltf");
        let asset = AnimatedAsset::load(&path).expect("the demo rig");
        let tip = asset.joint("tip").expect("a tip joint");
        let joints = asset.skeleton.len();
        let pose = asset
            .clips
            .values()
            .next()
            .unwrap()
            .sample(&asset.skeleton, 0.5, Playback::LOOPING)
            .unwrap();
        let mut model = vec![Mat4::IDENTITY; joints];
        assert!(pose.to_model_space(&asset.skeleton, &mut model));
        let mut world = World::new();
        let body_at = Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0));
        let body = world.spawn((
            Name::new("Body"),
            Transform::default(),
            WorldTransform::identity(),
        ));
        let offset = Vec3::new(0.0, 0.1, 0.0);
        let socket = world.spawn((
            Name::new("Sword"),
            Transform::from_translation(offset),
            WorldTransform::identity(),
            Parent { entity: body },
            SkinnedSocket {
                joint: "tip".into(),
            },
        ));
        let pommel = world.spawn((
            Name::new("Pommel"),
            Transform::from_translation(Vec3::new(0.0, -0.2, 0.0)),
            WorldTransform::identity(),
            Parent { entity: socket },
        ));
        let source = AssetId::from_relative_path("models/animation_demo.gltf");
        let mut bodies = SkinnedBodies::default();
        bodies.shared.insert(
            source,
            Shared {
                asset,
                meshes: Vec::new(),
                users: 1,
                vertices: 0,
            },
        );
        bodies.bodies.insert(
            body,
            Body {
                source,
                level: 0,
                retry: None,
                skins: Vec::new(),
                clock: 0.5,
                owed: 0.0,
                palette: vec![Mat4::IDENTITY; joints],
                model: model.clone(),
                matrix: body_at,
                posed: true,
            },
        );
        bodies.place_sockets(&mut world);
        let at = |e: Entity| world.get::<WorldTransform>(e).unwrap().0;
        let expected = body_at * model[usize::from(tip)] * Mat4::from_translation(offset);
        assert!(at(socket).abs_diff_eq(expected, 1e-5));
        let below = expected * Mat4::from_translation(Vec3::new(0.0, -0.2, 0.0));
        assert!(at(pommel).abs_diff_eq(below, 1e-5), "the subtree follows");
    }

    /// The vertex budget coarsens from the back: the nearest keep what their
    /// distance wants while the rest still fit at their coarsest.
    #[test]
    fn the_vertex_budget_coarsens_from_the_back() {
        // WS3's crowd adult: 52k, 20.5k and 7.8k posed vertices.
        let adult = vec![52_000, 20_500, 7_800];
        let wants = vec![0; 160];
        let sizes = vec![adult.clone(); 160];
        let levels = fit(2_000_000, &wants, &sizes);
        let total: u32 = levels.iter().zip(&sizes).map(|(l, s)| s[*l]).sum();
        assert!(total <= 2_000_000, "{total}");
        assert!(levels.windows(2).all(|w| w[0] <= w[1]), "finer in front");
        assert_eq!(levels[0], 0, "the nearest keep full detail");
        assert_eq!(*levels.last().unwrap(), 2, "the back is coarsened");
        // Plenty: everyone as their distance wants.
        assert_eq!(
            fit(
                u32::MAX,
                &[0, 1, 2],
                &[adult.clone(), adult.clone(), adult.clone()]
            ),
            vec![0, 1, 2]
        );
        // Not even the coarsest fit: all coarsest.
        assert_eq!(
            fit(10, &[0, 0], &[adult.clone(), adult.clone()]),
            vec![2, 2]
        );
        // A body with one level never moves.
        assert_eq!(fit(0, &[0], &[vec![9_000]]), vec![0]);
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
