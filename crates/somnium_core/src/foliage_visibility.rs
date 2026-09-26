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
    let mut owner = entity;
    // Bound corrupt parent cycles without allocating per mesh part.
    for _ in 0..64 {
        if let Some(foliage) = world.get::<FoliageComponent>(owner)
            && foliage.enabled
        {
            let Some(position) = world_position(world, owner) else {
                return Some(true);
            };
            let delta = position - camera;
            let distance_sq = delta.x * delta.x + delta.z * delta.z;
            if foliage.cull_distance > 0.0
                && distance_sq > foliage.cull_distance * foliage.cull_distance
            {
                return None;
            }
            if distance_sq < foliage.near_distance * foliage.near_distance {
                return None;
            }
            return Some(
                foliage.foliage_shadow_distance <= 0.0
                    || distance_sq
                        <= foliage.foliage_shadow_distance * foliage.foliage_shadow_distance,
            );
        }
        let Some(parent) = world.get::<Parent>(owner) else {
            break;
        };
        owner = parent.entity;
    }
    Some(true)
}

fn world_position(world: &World, mut entity: Entity) -> Option<Vec3> {
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
        world.spawn((Transform::default(), Parent { entity: root }, ImportedMesh::default()))
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
