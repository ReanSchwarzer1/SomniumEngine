//! Distance policy for imported plant hierarchies, shared by game render adapters.
use crate::{FoliageComponent, ImportedMesh, Parent, Transform, WorldTransform};
use glam::{Mat4, Vec3};
use somnium_ecs::{Archetype, ComponentId, Entity, World};

/// `None` omits an imported foliage draw; `Some` carries its shadow eligibility.
/// Untagged meshes are unchanged. This only controls graphics submission and
/// never changes lights, collision, scripts or entity visibility/ownership.
/// Use the camera currently submitted to the renderer, in editor and play alike.
pub fn imported_draw(world: &World, entity: Entity, camera: Vec3) -> Option<bool> {
    if world.get::<ImportedMesh>(entity).is_none() {
        return Some(true);
    }
    let Some(owner) = foliage_owner(world, entity) else {
        return Some(true);
    };
    let Some(position) = world_position(world, owner) else {
        return Some(true);
    };
    let foliage = world
        .get::<FoliageComponent>(owner)
        .expect("owner carries foliage");
    distance_policy(foliage, position, camera, 1.0, 1.0)
}

/// The entity whose enabled `FoliageComponent` governs `entity`'s drawing:
/// itself or its nearest such ancestor. Split out so a renderer can resolve
/// it once and keep it, rather than walk the hierarchy for every part.
pub fn foliage_owner(world: &World, entity: Entity) -> Option<Entity> {
    let mut owner = entity;
    // Bound corrupt parent cycles without allocating per mesh part.
    for _ in 0..64 {
        if world
            .get::<FoliageComponent>(owner)
            .is_some_and(|f| f.enabled)
        {
            return Some(owner);
        }
        owner = world.get::<Parent>(owner)?.entity;
    }
    None
}

/// How an imported foliage part is submitted this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportedDraw {
    Hidden,
    /// Drawn into the shadow maps only: a `shadow_proxy` far half inside its
    /// `near_distance`.
    ShadowOnly,
    Visible { casts_shadow: bool },
}

/// [`imported_draw`]'s verdict for a part governed by `foliage` whose owner
/// stands at `position` (horizontal distances only). `reach` scales the draw
/// distances -- the cull and the near hand-over together, so a tree's near
/// and far halves still meet -- and `shadow_reach` the shadow distance; both
/// are 1.0 except under a lower graphics tier.
pub fn distance_policy(
    foliage: &FoliageComponent,
    position: Vec3,
    camera: Vec3,
    reach: f32,
    shadow_reach: f32,
) -> Option<bool> {
    match draw_policy(foliage, position, camera, reach, shadow_reach) {
        ImportedDraw::Visible { casts_shadow } => Some(casts_shadow),
        _ => None,
    }
}

/// [`distance_policy`] with the shadow-only case a `shadow_proxy` asks for.
pub fn draw_policy(
    foliage: &FoliageComponent,
    position: Vec3,
    camera: Vec3,
    reach: f32,
    shadow_reach: f32,
) -> ImportedDraw {
    let delta = position - camera;
    let distance_sq = delta.x * delta.x + delta.z * delta.z;
    let cull = foliage.cull_distance * reach;
    if foliage.cull_distance > 0.0 && distance_sq > cull * cull {
        return ImportedDraw::Hidden;
    }
    let shadow = foliage.foliage_shadow_distance * shadow_reach;
    let casts_shadow = foliage.foliage_shadow_distance <= 0.0 || distance_sq <= shadow * shadow;
    let near = foliage.near_distance * reach;
    if distance_sq < near * near {
        return if foliage.shadow_proxy && casts_shadow {
            ImportedDraw::ShadowOnly
        } else {
            ImportedDraw::Hidden
        };
    }
    ImportedDraw::Visible { casts_shadow }
}

/// [`imported_draw`] for a whole draw loop (open-world render scale, 19 Oct 2026): the same verdicts, with each
/// parent's foliage owner and each owner's verdict found once a frame instead of once per mesh. A building's forty
/// parts share one ancestry, which `imported_draw` walks forty times. In a region's mesh loop that walk was most of the
/// frame: 18 of 23 ms at 73,600 mesh entities in release (`tests/draw_loop_scale.rs`). Call [`Self::begin_frame`]
/// before each loop; within a frame the world and the camera must not change.
///
/// Differs from `imported_draw` only past 63 ancestors, where both give up (the parent's walk starts one level up).
#[derive(Default)]
pub struct ImportedDraws {
    owners: std::collections::HashMap<Entity, Option<Entity>, EntityHash>,
    verdicts: std::collections::HashMap<Entity, Option<bool>, EntityHash>,
}

impl ImportedDraws {
    /// Forget last frame's owners and verdicts.
    pub fn begin_frame(&mut self) {
        self.owners.clear();
        self.verdicts.clear();
    }

    /// [`imported_draw`]'s verdict for `entity` this frame.
    pub fn draw(&mut self, world: &World, entity: Entity, camera: Vec3) -> Option<bool> {
        if world.get::<ImportedMesh>(entity).is_none() {
            return Some(true);
        }
        let own = world.get::<FoliageComponent>(entity).is_some_and(|f| f.enabled);
        let parent = world.get::<Parent>(entity).map(|p| p.entity);
        self.verdict(world, entity, own, parent, camera)
    }

    /// [`Self::draw`] for row `row` of `archetype`, the draw loop's own iteration: the row's components are read from
    /// its columns, in order, instead of looked up through the world per entity.
    pub fn draw_row(&mut self, world: &World, archetype: &Archetype, row: usize, camera: Vec3) -> Option<bool> {
        if archetype.column_index(ComponentId::of::<ImportedMesh>()).is_none() {
            return Some(true);
        }
        // SAFETY: each column is the one `column_index` found for that component's id, and `row` is the caller's row
        // of this archetype.
        let own = archetype
            .column_index(ComponentId::of::<FoliageComponent>())
            .is_some_and(|c| unsafe { archetype.column(c).get::<FoliageComponent>(row) }.enabled);
        let parent = archetype
            .column_index(ComponentId::of::<Parent>())
            .map(|c| unsafe { archetype.column(c).get::<Parent>(row) }.entity);
        self.verdict(world, archetype.entities()[row], own, parent, camera)
    }

    fn verdict(&mut self, world: &World, entity: Entity, own: bool, parent: Option<Entity>, camera: Vec3) -> Option<bool> {
        let owner = if own {
            Some(entity)
        } else {
            let Some(parent) = parent else {
                return Some(true);
            };
            *self.owners.entry(parent).or_insert_with(|| foliage_owner(world, parent))
        };
        let Some(owner) = owner else {
            return Some(true);
        };
        *self.verdicts.entry(owner).or_insert_with(|| {
            let Some(position) = world_position(world, owner) else {
                return Some(true);
            };
            let foliage = world.get::<FoliageComponent>(owner).expect("owner carries foliage");
            distance_policy(foliage, position, camera, 1.0, 1.0)
        })
    }
}

/// A multiply-rotate hash for entity keys: `ImportedDraws` looks one up per mesh per frame.
#[derive(Default, Clone, Copy)]
pub struct EntityHash;

impl std::hash::BuildHasher for EntityHash {
    type Hasher = EntityHasher;
    fn build_hasher(&self) -> EntityHasher {
        EntityHasher(0)
    }
}

#[doc(hidden)]
#[derive(Default)]
pub struct EntityHasher(u64);

impl std::hash::Hasher for EntityHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_u64(u64::from(byte));
        }
    }
    fn write_u32(&mut self, value: u32) {
        self.write_u64(u64::from(value));
    }
    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

/// World position of `entity`: its `WorldTransform` if it has one, else its
/// local transforms composed up to the nearest ancestor that has one.
pub fn world_position(world: &World, mut entity: Entity) -> Option<Vec3> {
    let mut local = Mat4::IDENTITY;
    for _ in 0..64 {
        if let Some(transform) = world.get::<WorldTransform>(entity) {
            return Some((transform.0 * local).transform_point3(Vec3::ZERO));
        }
        local = world.get::<Transform>(entity)?.to_matrix() * local;
        let Some(parent) = world.get::<Parent>(entity) else {
            return Some(local.transform_point3(Vec3::ZERO));
        };
        entity = parent.entity;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FoliageComponent;

    /// `ImportedDraws` against `imported_draw` for every entity of a mixed world, from several cameras.
    #[test]
    fn the_frame_cache_gives_the_same_verdicts() {
        let mut world = World::new();
        let near_far = plant(&mut world, 10.0, 60.0);
        let near_far_root = world.get::<Parent>(near_far).unwrap().entity;
        // Siblings under the same root, a mesh that is not imported, and a disabled foliage root.
        let sibling = world.spawn((Transform::default(), Parent { entity: near_far_root }, ImportedMesh::default()));
        let plain = world.spawn((Transform::default(), Parent { entity: near_far_root }));
        let off_root = world.spawn((
            Transform::from_translation(Vec3::new(-30.0, 0.0, 5.0)),
            FoliageComponent { enabled: false, cull_distance: 1.0, ..Default::default() },
        ));
        let under_off = world.spawn((Transform::default(), Parent { entity: off_root }, ImportedMesh::default()));
        // A house: root, a level, parts; no foliage above them.
        let house = world.spawn((Transform::from_translation(Vec3::new(5.0, 0.0, 5.0)),));
        let level = world.spawn((Transform::default(), Parent { entity: house }));
        let part = world.spawn((Transform::default(), Parent { entity: level }, ImportedMesh::default()));
        // A mesh that is itself the foliage, and one whose parent was despawned.
        let own = world.spawn((
            Transform::from_translation(Vec3::new(0.0, 0.0, 200.0)),
            ImportedMesh::default(),
            FoliageComponent { enabled: true, cull_distance: 100.0, ..Default::default() },
        ));
        let gone = world.spawn((Transform::default(),));
        let orphan = world.spawn((Transform::default(), Parent { entity: gone }, ImportedMesh::default()));
        world.despawn(gone);
        let dangling = world.spawn((Transform::default(), Parent { entity: Entity::DANGLING }, ImportedMesh::default()));
        let all = [near_far, sibling, plain, under_off, part, own, orphan, dangling];
        let mut cache = ImportedDraws::default();
        for camera in [Vec3::ZERO, Vec3::new(50.0, 3.0, 0.0), Vec3::new(45.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 400.0)] {
            cache.begin_frame();
            for entity in all {
                assert_eq!(cache.draw(&world, entity, camera), imported_draw(&world, entity, camera), "{camera}");
            }
            // The same through the loop's own archetype rows.
            cache.begin_frame();
            let required = somnium_ecs::ComponentSet::from_ids(vec![ComponentId::of::<Transform>()]);
            for archetype in world.query_archetypes(&required, &somnium_ecs::ComponentSet::empty()) {
                for row in 0..archetype.len() {
                    let entity = archetype.entities()[row];
                    let expected = imported_draw(&world, entity, camera);
                    assert_eq!(cache.draw_row(&world, archetype, row, camera), expected, "{camera} row");
                }
            }
        }
    }

    fn plant(world: &mut World, near: f32, cull: f32) -> Entity {
        let root = world.spawn((
            Transform::from_translation(Vec3::new(50.0, 0.0, 0.0)),
            FoliageComponent {
                enabled: true,
                cull_distance: cull,
                near_distance: near,
                foliage_shadow_distance: 0.0,
                ..Default::default()
            },
        ));
        world.spawn((
            Transform::default(),
            Parent { entity: root },
            ImportedMesh::default(),
        ))
    }

    /// A proxy far half casts the pair's shadow up close without being drawn,
    /// and only inside its shadow distance; an ordinary far half does neither.
    #[test]
    fn a_shadow_proxy_far_half_casts_shadow_only_up_close() {
        let mut far = FoliageComponent {
            enabled: true,
            cull_distance: 160.0,
            near_distance: 38.0,
            foliage_shadow_distance: 60.0,
            shadow_proxy: true,
            ..Default::default()
        };
        let tree = Vec3::new(50.0, 0.0, 0.0);
        assert_eq!(draw_policy(&far, tree, Vec3::new(30.0, 1.7, 0.0), 1.0, 1.0), ImportedDraw::ShadowOnly);
        assert_eq!(draw_policy(&far, tree, Vec3::new(0.0, 1.7, 0.0), 1.0, 1.0),
                   ImportedDraw::Visible { casts_shadow: true });
        assert_eq!(draw_policy(&far, tree, Vec3::new(-20.0, 1.7, 0.0), 1.0, 1.0),
                   ImportedDraw::Visible { casts_shadow: false });
        far.shadow_proxy = false;
        assert_eq!(draw_policy(&far, tree, Vec3::new(30.0, 1.7, 0.0), 1.0, 1.0), ImportedDraw::Hidden);
    }

    #[test]
    fn near_and_far_lod_halves_hand_over_at_one_distance() {
        let mut world = World::new();
        let near = plant(&mut world, 0.0, 38.0);
        let far = plant(&mut world, 38.0, 160.0);
        let close = Vec3::new(20.0, 1.7, 0.0); // 30 m from the tree
        let away = Vec3::new(-10.0, 1.7, 0.0); // 60 m from the tree
        assert!(imported_draw(&world, near, close).is_some());
        assert!(imported_draw(&world, far, close).is_none());
        assert!(imported_draw(&world, near, away).is_none());
        assert!(imported_draw(&world, far, away).is_some());
    }
}
