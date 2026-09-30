//! Distance policy for imported plant hierarchies, shared by game render adapters.
use crate::{FoliageComponent, ImportedMesh, Parent, Transform, WorldTransform};
use glam::{Mat4, Vec3};
use somnium_ecs::{Entity, World};

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
