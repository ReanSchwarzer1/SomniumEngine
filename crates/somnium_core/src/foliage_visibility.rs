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
