//! Instanced prop sets: `somnium.PropSet` (open-world render scale, 19 Oct 2026).
//!
//! A town dressed with props as entities costs an entity per prop: a mesh loop row, a draw and its sort, instance,
//! motion and shadow work every frame, and a scene file entry each. A prop set is one entity holding a glTF source
//! and a compact list of copies (position and yaw, optional scale, rotation and tint), drawn through the renderer's
//! instance batches (`somnium_renderer::instance_batch`): set once, culled per copy on the GPU (frustum, occlusion,
//! the set's draw distance), shadowed and, near the eye, ray traced. A frame costs a walk over the sets, not over
//! their copies. In the spirit of Unreal's hierarchical instanced static meshes.
//!
//! Each copy places the source's root: every part draws at the copy's transform times its node's own place in the
//! file, so a cart's wheels stay on the cart. `parts` lists which of the source's mesh nodes (the ordinals an
//! `ImportedMesh` names) draw; empty draws every one, which is also how a node of several primitives (one per
//! material) is one prop. Copies are in the set entity's frame, so moving the entity moves the set.
//!
//! Colliders are opt-in: `collide_bounds` gives each copy one static box over its parts' bounds; `box_centres` and
//! `box_halves` list boxes in the copy's own frame instead. A tint is a small palette and an index per copy; each
//! tint draws through a tinted copy of the part's material, so it holds in every pass.
//!
//! What a set does not have: what instance batches lack (`instance_batch.rs`: own motion, vertex paint, highlight,
//! one-sided raster), per-copy picking in the editor (the set entity is selected through the Outliner), and LOD
//! meshes (one draw distance; a building with levels stays a `LodGroup`).

use glam::{Mat4, Quat, Vec3};
use somnium_ecs::component_schema;
use somnium_ecs::reflect::TypeRegistry;
use somnium_ecs::{Component, ComponentId, Entity, World};
use somnium_renderer::instance_batch::{BatchPart, InstanceBatch};

use crate::{Transform, WorldTransform};

/// Many copies of one glTF source, drawn as instance batches.
#[derive(Debug, Clone, PartialEq)]
pub struct PropSetComponent {
    /// Off: nothing draws or collides.
    pub enabled: bool,
    /// The glTF, as an `ImportedMesh` names it (project-relative path).
    pub source: String,
    /// The source's mesh nodes each copy draws (the ordinals an `ImportedMesh` uses); empty: every one.
    pub parts: Vec<u32>,
    /// One per copy, in the set's frame: position (x, y, z) and yaw (radians about +Y).
    pub copies: Vec<[f32; 4]>,
    /// Empty, or one uniform scale per copy.
    pub scales: Vec<f32>,
    /// Empty, or one rotation per copy (quaternion x, y, z, w) used instead of its yaw.
    pub rotations: Vec<[f32; 4]>,
    /// Tint palette (linear RGB multiplied into the base colour); empty: untinted.
    pub tints: Vec<[f32; 3]>,
    /// Empty, or one index into `tints` per copy (past the palette: untinted).
    pub tint_of: Vec<u32>,
    /// Horizontal metres from the camera a copy draws to; 0: no limit.
    pub draw_distance: f32,
    /// Copies cast shadows (within `shadow_distance`).
    pub casts_shadow: bool,
    /// Horizontal metres a copy casts to; 0: as far as it draws.
    pub shadow_distance: f32,
    /// One static box per copy over its parts' bounds.
    pub collide_bounds: bool,
    /// Static boxes per copy, in its frame: centres here, half extents in `box_halves` (paired by index).
    pub box_centres: Vec<[f32; 3]>,
    /// Half extents of the boxes in `box_centres`.
    pub box_halves: Vec<[f32; 3]>,
}

impl Default for PropSetComponent {
    fn default() -> Self {
        Self {
            enabled: true,
            source: String::new(),
            parts: Vec::new(),
            copies: Vec::new(),
            scales: Vec::new(),
            rotations: Vec::new(),
            tints: Vec::new(),
            tint_of: Vec::new(),
            draw_distance: 0.0,
            casts_shadow: true,
            shadow_distance: 0.0,
            collide_bounds: false,
            box_centres: Vec::new(),
            box_halves: Vec::new(),
        }
    }
}

impl Component for PropSetComponent {}

pub(crate) fn register(registry: &mut TypeRegistry) {
    registry.register(component_schema! {
        PropSetComponent as "somnium.PropSet", display "Prop Set", version 1,
        fields {
            enabled { group: "Prop Set", doc: "Off: nothing draws or collides." },
            source { group: "Prop Set", doc: "The glTF every copy draws, project-relative (as an Imported Mesh names it)." },
            parts { group: "Prop Set", advanced: true, doc: "Which of the source's mesh nodes draw (Imported Mesh ordinals); empty: all." },
            copies { group: "Copies", advanced: true, read_only: true, doc: "Per copy, in this entity's frame: x, y, z and yaw (radians)." },
            scales { group: "Copies", advanced: true, read_only: true, doc: "Empty, or a uniform scale per copy." },
            rotations { group: "Copies", advanced: true, read_only: true, doc: "Empty, or a rotation per copy (x, y, z, w), instead of its yaw." },
            tints { group: "Copies", advanced: true, doc: "Tint palette, linear RGB multiplied into the base colour." },
            tint_of { group: "Copies", advanced: true, read_only: true, doc: "Empty, or an index into the tints per copy." },
            draw_distance { group: "Drawing", min: 0.0, unit: "m", doc: "Horizontal metres a copy draws to; 0: no limit." },
            casts_shadow { group: "Drawing" },
            shadow_distance { group: "Drawing", min: 0.0, unit: "m", doc: "Horizontal metres a copy casts to; 0: as far as it draws." },
            collide_bounds { group: "Collision", doc: "One static box per copy over its parts' bounds." },
            box_centres { group: "Collision", advanced: true, doc: "Static boxes per copy, centres in its frame (paired with Box Halves)." },
            box_halves { group: "Collision", advanced: true, doc: "Half extents of the boxes in Box Centres." },
        }
    });
}

impl PropSetComponent {
    /// Copy `k`'s transform in the set's frame.
    #[must_use]
    pub fn copy_transform(&self, k: usize) -> Mat4 {
        let [x, y, z, yaw] = self.copies[k];
        let rotation = self
            .rotations
            .get(k)
            .map_or_else(|| Quat::from_rotation_y(yaw), |&[a, b, c, d]| Quat::from_xyzw(a, b, c, d).normalize());
        let scale = self.scales.get(k).copied().unwrap_or(1.0);
        Mat4::from_scale_rotation_translation(Vec3::splat(scale), rotation, Vec3::new(x, y, z))
    }

    /// Copy `k`'s tint, if it has one.
    #[must_use]
    pub fn tint(&self, k: usize) -> Option<[f32; 3]> {
        self.tint_of.get(k).and_then(|&t| self.tints.get(t as usize)).copied()
    }
}

/// One drawable part of a source: an uploaded mesh node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourcePart {
    /// Where its mesh sits in the renderer's geometry pool.
    pub vertex_offset: u32,
    /// Its first index in the pool.
    pub index_offset: u32,
    /// Its index count.
    pub index_count: u32,
    /// Its material's slot in the renderer's pool.
    pub material_id: u32,
    /// The node's place in the file.
    pub transform: Mat4,
}

/// Batch ids in this namespace are prop sets': the top byte.
const ID_SPACE: u64 = 0xF1 << 56;

/// The batch id of `entity`'s tint group `group` (255 groups at most: the rest share the last).
#[must_use]
pub(crate) fn batch_id(entity: Entity, group: usize) -> u64 {
    ID_SPACE | (u64::from(entity.index()) << 8) | group.min(255) as u64
}

/// `set`'s batches with its entity at `world`, one per tint group, over `parts` (the source's uploaded nodes).
/// `tinted(material, tint)` gives the material a tinted copy draws with.
pub(crate) fn batches(
    entity: Entity,
    set: &PropSetComponent,
    world: Mat4,
    parts: &[SourcePart],
    mut tinted: impl FnMut(u32, [f32; 3]) -> u32,
) -> Vec<(u64, InstanceBatch)> {
    let chosen: Vec<SourcePart> = if set.parts.is_empty() {
        parts.to_vec()
    } else {
        set.parts.iter().filter_map(|&p| parts.get(p as usize).copied()).collect()
    };
    if chosen.is_empty() || set.copies.is_empty() {
        return Vec::new();
    }
    // Groups by tint, in palette order, the untinted first.
    let mut groups: Vec<(Option<[f32; 3]>, Vec<Mat4>)> = Vec::new();
    for k in 0..set.copies.len() {
        let tint = set.tint(k);
        let placement = world * set.copy_transform(k);
        match groups.iter_mut().find(|(t, _)| *t == tint) {
            Some((_, list)) => list.push(placement),
            None => groups.push((tint, vec![placement])),
        }
    }
    let far = set.draw_distance.max(0.0);
    let shadow_far = if set.shadow_distance > 0.0 { set.shadow_distance } else { far };
    groups
        .into_iter()
        .enumerate()
        .map(|(g, (tint, placements))| {
            let parts = chosen
                .iter()
                .map(|p| BatchPart {
                    vertex_offset: p.vertex_offset,
                    index_offset: p.index_offset,
                    index_count: p.index_count,
                    material_id: tint.map_or(p.material_id, |t| tinted(p.material_id, t)),
                    local: p.transform,
                    near: 0.0,
                    far,
                    casts_shadow: set.casts_shadow,
                    shadow_far,
                })
                .collect();
            (batch_id(entity, g), InstanceBatch { parts, placements })
        })
        .collect()
}

/// A static box in the world: centre, rotation, half extents.
pub(crate) type StaticBox = (Vec3, Quat, Vec3);

/// `set`'s static boxes with its entity at `world`; `bounds` is the chosen parts' local bounds, from their meshes.
pub(crate) fn colliders(set: &PropSetComponent, world: Mat4, bounds: Option<(Vec3, Vec3)>) -> Vec<StaticBox> {
    let mut local: Vec<(Vec3, Vec3)> =
        set.box_centres.iter().zip(&set.box_halves).map(|(c, h)| (Vec3::from(*c), Vec3::from(*h).abs())).collect();
    if set.collide_bounds
        && let Some((min, max)) = bounds
    {
        local.push(((min + max) * 0.5, (max - min).abs() * 0.5));
    }
    if local.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(local.len() * set.copies.len());
    for k in 0..set.copies.len() {
        let copy = world * set.copy_transform(k);
        let (scale, rotation, _) = copy.to_scale_rotation_translation();
        for &(centre, half) in &local {
            out.push((copy.transform_point3(centre), rotation, half * scale.abs()));
        }
    }
    out
}

fn mix(h: &mut u64, word: u32) {
    *h = (*h ^ u64::from(word)).wrapping_mul(0x0100_0000_01b3);
}

/// Everything about `set` that changes what it draws or where it collides.
fn content_hash(set: &PropSetComponent) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    mix(&mut h, u32::from(set.enabled) | u32::from(set.casts_shadow) << 1 | u32::from(set.collide_bounds) << 2);
    for byte in set.source.bytes() {
        mix(&mut h, u32::from(byte));
    }
    let floats = set
        .copies
        .iter()
        .flatten()
        .chain(&set.scales)
        .chain(set.rotations.iter().flatten())
        .chain(set.tints.iter().flatten())
        .chain(set.box_centres.iter().flatten())
        .chain(set.box_halves.iter().flatten())
        .chain([&set.draw_distance, &set.shadow_distance]);
    for value in floats {
        mix(&mut h, value.to_bits());
    }
    for &word in set.parts.iter().chain(&set.tint_of) {
        mix(&mut h, word);
    }
    for len in [set.copies.len(), set.scales.len(), set.rotations.len(), set.tints.len(), set.tint_of.len()] {
        mix(&mut h, len as u32);
    }
    h
}

type EntityMap<V> = std::collections::HashMap<Entity, V, crate::foliage_visibility::EntityHash>;

/// What one frame must do: sets to (re)build and sets gone.
#[derive(Default, Debug)]
pub(crate) struct Plan {
    /// `(entity, signature, world transform)` of each set whose drawing must be (re)built.
    pub build: Vec<(Entity, u64, Mat4)>,
    /// Sets drawn before and no longer there (despawned, disabled, hidden).
    pub gone: Vec<Entity>,
}

/// What the planner knows of one set.
struct Known {
    /// Its content hash, as of the `PropSetComponent` change tick beside it.
    content: u64,
    content_tick: u64,
    /// The signature it was last built to, if it was.
    built: Option<u64>,
    /// The frame it was last seen.
    seen: u64,
}

/// The sets drawn now, and what each was built from.
#[derive(Default)]
pub(crate) struct PropSetPlanner {
    sets: EntityMap<Known>,
    frame: u64,
}

impl PropSetPlanner {
    /// This frame's plan: the sets' own archetype columns and one lookup a set, plus each set's copies hashed again
    /// when any `PropSetComponent` was written.
    pub(crate) fn plan(&mut self, world: &World) -> Plan {
        let tick = world.change_tick(ComponentId::of::<PropSetComponent>());
        self.frame += 1;
        let frame = self.frame;
        let mut plan = Plan::default();
        let required = somnium_ecs::ComponentSet::from_ids(vec![ComponentId::of::<PropSetComponent>()]);
        for archetype in world.query_archetypes(&required, &somnium_ecs::ComponentSet::empty()) {
            let column = |id: ComponentId| archetype.column_index(id);
            let set_col = column(ComponentId::of::<PropSetComponent>()).expect("queried for it");
            let (wt_col, t_col) = (column(ComponentId::of::<WorldTransform>()), column(ComponentId::of::<Transform>()));
            let flags_col = column(ComponentId::of::<crate::EditorFlags>());
            for row in 0..archetype.len() {
                // SAFETY: each column is the one `column_index` found for that component's id; `row` is this
                // archetype's.
                let set = unsafe { archetype.column(set_col).get::<PropSetComponent>(row) };
                let hidden = flags_col.is_some_and(|c| unsafe { archetype.column(c).get::<crate::EditorFlags>(row) }.hidden);
                if !set.enabled || set.copies.is_empty() || hidden {
                    continue;
                }
                let at = wt_col
                    .map(|c| unsafe { archetype.column(c).get::<WorldTransform>(row) }.0)
                    .or_else(|| t_col.map(|c| unsafe { archetype.column(c).get::<Transform>(row) }.to_matrix()))
                    .unwrap_or(Mat4::IDENTITY);
                let entity = archetype.entities()[row];
                let known = self.sets.entry(entity).or_insert(Known { content: 0, content_tick: u64::MAX, built: None, seen: 0 });
                if known.content_tick != tick {
                    (known.content, known.content_tick) = (content_hash(set), tick);
                }
                known.seen = frame;
                let mut signature = known.content;
                for value in at.to_cols_array() {
                    mix(&mut signature, value.to_bits());
                }
                if known.built != Some(signature) {
                    plan.build.push((entity, signature, at));
                }
            }
        }
        // Not seen this frame: gone if it was drawn, forgotten either way.
        self.sets.retain(|&entity, known| {
            let here = known.seen == frame;
            if !here && known.built.is_some() {
                plan.gone.push(entity);
            }
            here
        });
        plan
    }

    /// `entity` is drawn as of `signature`.
    pub(crate) fn built(&mut self, entity: Entity, signature: u64) {
        if let Some(known) = self.sets.get_mut(&entity) {
            known.built = Some(signature);
        }
    }

    /// Nothing is drawn any more: every set is built again.
    pub(crate) fn reset(&mut self) {
        for known in self.sets.values_mut() {
            known.built = None;
        }
    }
}

/// A set's source on its way to the GPU: decoded on a thread, then uploaded.
pub(crate) enum PropSource {
    Loading(std::sync::mpsc::Receiver<Result<somnium_asset::LoadedScene, String>>),
    Ready(Vec<SourcePart>),
    Failed,
}

/// The engine's prop sets: what each live set holds on the GPU and in physics.
#[derive(Default)]
pub(crate) struct PropSets {
    pub planner: PropSetPlanner,
    /// By source path. Kept apart from the imported-mesh uploads, which the cell streamer releases with the cells
    /// that used them: a source a set draws stays uploaded.
    pub sources: std::collections::HashMap<String, PropSource>,
    /// Each live set's batch ids and static bodies.
    pub live: EntityMap<(Vec<u64>, Vec<somnium_physics::body::BodyId>)>,
    /// Tinted copies of materials, by (material, tint bits).
    pub tinted: std::collections::HashMap<(u32, [u32; 3]), u32>,
}

impl PropSets {
    /// A new renderer holds none of this: every set is uploaded and batched again. Static bodies are kept, and
    /// replaced when their set is rebuilt.
    pub(crate) fn reset_gpu(&mut self) {
        self.planner.reset();
        self.sources.clear();
        self.tinted.clear();
        for (ids, _) in self.live.values_mut() {
            ids.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(vertex_offset: u32, material_id: u32, x: f32) -> SourcePart {
        SourcePart {
            vertex_offset,
            index_offset: 0,
            index_count: 36,
            material_id,
            transform: Mat4::from_translation(Vec3::new(x, 0.0, 0.0)),
        }
    }

    fn barrels(n: usize) -> PropSetComponent {
        PropSetComponent {
            source: "props/barrel.glb".into(),
            copies: (0..n).map(|k| [k as f32 * 2.0, 0.0, 0.0, 0.0]).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_copy_places_every_part_where_it_sits_in_the_file() {
        let mut set = barrels(2);
        set.copies[1] = [10.0, 0.0, 5.0, std::f32::consts::FRAC_PI_2];
        set.scales = vec![1.0, 2.0];
        let world = Mat4::from_translation(Vec3::new(100.0, 0.0, 0.0));
        let out = batches(Entity::DANGLING, &set, world, &[part(1, 7, 0.0), part(2, 8, 0.5)], |m, _| m);
        assert_eq!(out.len(), 1);
        let batch = &out[0].1;
        assert_eq!(batch.parts.len(), 2);
        assert_eq!(batch.parts[1].local, Mat4::from_translation(Vec3::new(0.5, 0.0, 0.0)));
        // The second copy: in the set's frame at (10, 0, 5), turned a quarter about +Y, twice the size.
        let wheel = batch.placements[1] * batch.parts[1].local;
        let at = wheel.transform_point3(Vec3::ZERO);
        assert!((at - Vec3::new(110.0, 0.0, 4.0)).length() < 1e-4, "{at}");
    }

    #[test]
    fn parts_choose_nodes_and_tints_split_into_their_own_materials() {
        let mut set = barrels(3);
        set.parts = vec![1];
        set.tints = vec![[1.0, 0.5, 0.5]];
        set.tint_of = vec![9, 0, 0];
        set.draw_distance = 80.0;
        let mut made = Vec::new();
        let out = batches(Entity::DANGLING, &set, Mat4::IDENTITY, &[part(1, 7, 0.0), part(2, 8, 0.0)], |m, t| {
            made.push((m, t));
            100 + m
        });
        assert_eq!(out.len(), 2, "untinted (palette index 9 is past it) and the one tint");
        assert_eq!((out[0].1.placements.len(), out[1].1.placements.len()), (1, 2));
        assert_eq!(out[0].1.parts[0].vertex_offset, 2, "only the chosen node");
        assert_eq!(out[1].1.parts[0].material_id, 108);
        assert_eq!((out[0].1.parts[0].far, out[0].1.parts[0].shadow_far), (80.0, 80.0));
        assert_eq!(made, [(8, [1.0, 0.5, 0.5])]);
        assert_ne!(out[0].0, out[1].0);
    }

    #[test]
    fn colliders_are_opt_in_and_follow_each_copy() {
        let mut set = barrels(2);
        assert!(colliders(&set, Mat4::IDENTITY, Some((Vec3::splat(-1.0), Vec3::splat(1.0)))).is_empty());
        set.collide_bounds = true;
        set.box_centres = vec![[0.0, 2.0, 0.0]];
        set.box_halves = vec![[0.5, 0.5, 0.5]];
        set.scales = vec![1.0, 3.0];
        let boxes = colliders(&set, Mat4::IDENTITY, Some((Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 2.0, 1.0))));
        assert_eq!(boxes.len(), 4, "two boxes for each of two copies");
        // The second copy, at x = 2 and three times the size: its listed box sits 6 m up and is 1.5 m half wide.
        let (centre, _, half) = boxes[2];
        assert!((centre - Vec3::new(2.0, 6.0, 0.0)).length() < 1e-4 && (half - Vec3::splat(1.5)).length() < 1e-4);
        let (centre, _, half) = boxes[3];
        assert!((centre - Vec3::new(2.0, 3.0, 0.0)).length() < 1e-4 && (half - Vec3::new(3.0, 3.0, 3.0)).length() < 1e-4);
    }

    /// A dense town both ways: every prop an entity, or the same props as sets (one per house and kind). Ignored;
    /// run in release: `cargo test --release -p somnium_core --lib prop_set::tests::a_dense_town -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore = "a measurement: run in release with --nocapture"]
    fn a_dense_town_as_entities_and_as_prop_sets() {
        use crate::foliage_visibility::ImportedDraws;
        use crate::lod_group::{LodGroupComponent, Lods};
        use crate::{ImportedMesh, MaterialComponent, MeshComponent, Parent};
        use somnium_ecs::ComponentSet;
        use std::time::Instant;
        const KINDS: usize = 8;
        fn part(world: &mut World, parent: Entity, at: Mat4, node: u32) {
            let e = world.spawn((
                Transform::default(),
                Parent { entity: parent },
                WorldTransform(at),
                MeshComponent { vertex_offset: node, index_offset: 0, index_count: 300 },
            ));
            let _ = world.insert_component(e, MaterialComponent { runtime_id: 1, ..Default::default() });
            let _ = world.insert_component(e, ImportedMesh { source: "kit.glb".into(), node });
        }
        // `houses` houses of 45 LOD0 parts (15 at LOD1, 2 at LOD2), each with `props` props of eight kinds round it.
        fn town(houses: usize, props: usize, as_sets: bool) -> World {
            let mut world = World::new();
            for h in 0..houses {
                let at = Vec3::new((h % 40) as f32 * 25.0, 0.0, (h / 40) as f32 * 25.0);
                let root = world.spawn((Transform::from_translation(at), LodGroupComponent::default()));
                let _ = root;
                for (level, count) in [45usize, 15, 2].into_iter().enumerate() {
                    let lod = world.spawn((Transform::default(), Parent { entity: root }, crate::Name::new(&format!("LOD{level}"))));
                    for p in 0..count {
                        part(&mut world, lod, Mat4::from_translation(at), p as u32);
                    }
                }
                let place = |k: usize| Vec3::new((k % 10) as f32 * 1.7 - 8.0, 0.0, (k / 10) as f32 * 1.7 - 8.0);
                // The house's yard: its props, beside the levels rather than among them.
                let yard = world.spawn((Transform::from_translation(at),));
                if as_sets {
                    for kind in 0..KINDS {
                        let copies = (0..props).filter(|k| k % KINDS == kind).map(|k| {
                            let p = place(k);
                            [p.x, p.y, p.z, k as f32]
                        });
                        let set = PropSetComponent { source: format!("props/{kind}.glb"), copies: copies.collect(), ..Default::default() };
                        world.spawn((Transform::default(), Parent { entity: yard }, set));
                    }
                } else {
                    for k in 0..props {
                        part(&mut world, yard, Mat4::from_translation(at + place(k)), 2000 + (k % KINDS) as u32);
                    }
                }
            }
            world
        }
        // hello_engine's mesh loop with the level test first and the frame's verdict cache.
        fn mesh_loop(
            world: &World,
            lods: &Lods,
            draws: &mut ImportedDraws,
            camera: Vec3,
            out: &mut Vec<somnium_renderer::command::DrawCommand>,
        ) -> usize {
            draws.begin_frame();
            out.clear();
            let required = ComponentSet::from_ids(vec![
                ComponentId::of::<WorldTransform>(),
                ComponentId::of::<MeshComponent>(),
                ComponentId::of::<MaterialComponent>(),
            ]);
            let mut drawn = 0;
            for archetype in world.query_archetypes(&required, &ComponentSet::empty()) {
                let wt_col = archetype.column_index(ComponentId::of::<WorldTransform>()).unwrap();
                let m_col = archetype.column_index(ComponentId::of::<MeshComponent>()).unwrap();
                for row in 0..archetype.len() {
                    let entity = archetype.entities()[row];
                    if lods.is_hidden(entity) {
                        continue;
                    }
                    let Some(casts_shadow) = draws.draw_row(world, archetype, row, camera) else { continue };
                    // The draw hello_engine submits: what the renderer is then given to sort, instance and cull.
                    let (wt, mesh) = unsafe {
                        (archetype.column(wt_col).get::<WorldTransform>(row), archetype.column(m_col).get::<MeshComponent>(row))
                    };
                    out.push(somnium_renderer::command::DrawCommand {
                        paint: 0,
                        casts_shadow,
                        sort_key: somnium_renderer::command::SortKey::new(0, 1, entity.index()),
                        vertex_offset: mesh.vertex_offset,
                        index_offset: mesh.index_offset,
                        index_count: mesh.index_count,
                        material_id: 1,
                        transform: wt.0,
                    });
                    drawn += 1;
                }
            }
            drawn
        }
        let camera = Vec3::new(500.0, 70.0, 300.0);
        for (houses, props) in [(400, 60), (1000, 120)] {
            let mut row = Vec::new();
            for as_sets in [false, true] {
                let world = town(houses, props, as_sets);
                let mut lods = Lods::default();
                lods.update(&world, camera);
                let mut draws = ImportedDraws::default();
                let mut submitted = Vec::new();
                let mut planner = PropSetPlanner::default();
                let first = planner.plan(&world);
                for (entity, signature, _) in &first.build {
                    planner.built(*entity, *signature);
                }
                let frames = 30;
                let started = Instant::now();
                let mut drawn = 0;
                for _ in 0..frames {
                    lods.update(&world, camera);
                    drawn = mesh_loop(&world, &lods, &mut draws, camera, &mut submitted);
                    assert!(planner.plan(&world).build.is_empty());
                }
                let ms = started.elapsed().as_secs_f64() * 1000.0 / frames as f64;
                row.push((world.entity_count(), drawn, first.build.len(), ms));
            }
            let [(e0, d0, _, ms0), (e1, d1, sets, ms1)] = [row[0], row[1]];
            println!(
                "{houses} houses x {props} props: as entities {e0} entities, {d0} mesh draws a frame, {ms0:.2} ms; \
                 as prop sets {e1} entities ({sets} sets), {d1} mesh draws + {} batched copies, {ms1:.2} ms",
                houses * props
            );
        }
    }

    #[test]
    fn a_frame_rebuilds_only_sets_that_changed_moved_or_went() {
        let mut world = World::new();
        let a = world.spawn((Transform::default(), barrels(10)));
        let b = world.spawn((Transform::from_translation(Vec3::X), barrels(5)));
        let mut planner = PropSetPlanner::default();
        let plan = planner.plan(&world);
        assert_eq!(plan.build.len(), 2);
        for (entity, signature, _) in plan.build {
            planner.built(entity, signature);
        }
        assert!(planner.plan(&world).build.is_empty(), "nothing changed: nothing to do");
        world.get_mut::<PropSetComponent>(a).unwrap().copies.push([0.0, 0.0, 9.0, 0.0]);
        world.get_mut::<Transform>(b).unwrap().translation = Vec3::Y;
        let plan = planner.plan(&world);
        assert_eq!(plan.build.iter().map(|p| p.0).collect::<Vec<_>>().len(), 2, "one edited, one moved");
        for (entity, signature, _) in plan.build {
            planner.built(entity, signature);
        }
        world.get_mut::<PropSetComponent>(b).unwrap().enabled = false;
        world.despawn(a);
        let plan = planner.plan(&world);
        assert!(plan.build.is_empty());
        let mut gone = plan.gone.clone();
        gone.sort_unstable_by_key(|e| e.index());
        assert_eq!(gone, [a, b]);
    }
}
