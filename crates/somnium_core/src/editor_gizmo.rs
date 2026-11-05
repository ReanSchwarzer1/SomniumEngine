//! Hierarchy-aware transform-gizmo transactions.
//!
//! The window host decides when a gesture begins and ends. This module owns
//! the difficult invariant in between: the gesture is solved in world space,
//! while every selected entity must be written in its own parent's local
//! space. Capturing is atomic, so a degenerate parent cannot leave half a
//! multi-selection moved.

use crate::{Parent, Transform, WorldTransform};
use somnium_ecs::{Entity, World};

#[derive(Clone)]
pub(crate) struct GizmoFollower {
    pub entity_index: u32,
    pub start_transform: Transform,
    pub start_world_translation: glam::Vec3,
    pub parent_world_inverse: glam::Mat4,
    /// The parent's world rotation, so a world-space turn can be written into the local rotation.
    pub parent_rotation: glam::Quat,
}

pub(crate) fn capture_followers(
    world: &World,
    selection: &[Entity],
    primary_index: u32,
) -> Option<Vec<GizmoFollower>> {
    selection
        .iter()
        .filter(|entity| entity.index() != primary_index)
        .map(|entity| {
            let start_transform = *world.get::<Transform>(*entity)?;
            let parent_world = parent_world_matrix(world, *entity);
            let start_world_translation = entity_world_matrix(world, *entity)
                .map(|model| model.to_scale_rotation_translation().2)
                .unwrap_or_else(|| parent_world.transform_point3(start_transform.translation));
            Some(GizmoFollower {
                entity_index: entity.index(),
                start_transform,
                start_world_translation,
                parent_world_inverse: invert_affine(parent_world)?,
                parent_rotation: parent_world.to_scale_rotation_translation().1,
            })
        })
        .collect()
}

pub(crate) fn apply_followers(
    world: &mut World,
    followers: &[GizmoFollower],
    world_offset: glam::Vec3,
    spin: glam::Quat,
    growth: glam::Vec3,
    pivot: Option<glam::Vec3>,
) {
    for follower in followers {
        let Some(entity) = world.find_entity_by_index(follower.entity_index) else {
            continue;
        };
        let Some(transform) = world.get_mut::<Transform>(entity) else {
            continue;
        };
        let mut moved = follower.start_transform;
        moved.translation = follower
            .parent_world_inverse
            .transform_point3(follower.start_world_translation + world_offset);
        moved.rotation = spin_in_parent(follower.parent_rotation, spin) * follower.start_transform.rotation;
        moved.scale = follower.start_transform.scale * growth;
        if let Some(pivot) = pivot {
            let arm = follower.start_world_translation - pivot;
            let world_translation = pivot + spin * (arm * growth) + world_offset;
            moved.translation = follower
                .parent_world_inverse
                .transform_point3(world_translation);
        }
        *transform = moved;
    }
}

/// The model transform local authoring is relative to. A dangling or missing
/// parent is a root, and a parent whose propagation has not run yet falls back
/// to its authored transform.
pub(crate) fn parent_world_matrix(world: &World, entity: Entity) -> glam::Mat4 {
    let Some(parent) = world.get::<Parent>(entity).map(|parent| parent.entity) else {
        return glam::Mat4::IDENTITY;
    };
    if !world.is_alive(parent) {
        return glam::Mat4::IDENTITY;
    }
    entity_world_matrix(world, parent).unwrap_or(glam::Mat4::IDENTITY)
}

pub(crate) fn entity_world_matrix(world: &World, entity: Entity) -> Option<glam::Mat4> {
    world
        .get::<WorldTransform>(entity)
        .map(|world| world.0)
        .or_else(|| world.get::<Transform>(entity).map(Transform::to_matrix))
}

pub(crate) fn world_to_local_translation(
    parent_world_inverse: glam::Mat4,
    world: glam::Vec3,
) -> glam::Vec3 {
    parent_world_inverse.transform_point3(world)
}

pub(crate) fn invert_affine(model: glam::Mat4) -> Option<glam::Mat4> {
    let determinant = model.determinant();
    (determinant.is_finite() && determinant.abs() >= 1e-8).then(|| model.inverse())
}

/// A world-space turn written in the frame of a local rotation: its parent's. A part of a turned object turns about
/// the axis the gizmo shows, not about that axis read in the parent's frame.
pub(crate) fn spin_in_parent(parent_rotation: glam::Quat, world_spin: glam::Quat) -> glam::Quat {
    parent_rotation.inverse() * world_spin * parent_rotation
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A part of a turned object turns about the world axis the gizmo drew: its world rotation after the drag is the
    /// drag's turn times its world rotation before.
    #[test]
    fn a_part_of_a_turned_parent_turns_about_the_world_axis() {
        let parent = glam::Quat::from_rotation_z(0.7) * glam::Quat::from_rotation_x(0.4);
        let local = glam::Quat::from_rotation_y(0.3);
        let turn = glam::Quat::from_axis_angle(glam::Vec3::Y, 0.5);
        let after = parent * spin_in_parent(parent, turn) * local;
        assert!(after.abs_diff_eq(turn * parent * local, 1e-5));
        // The old rule, `turn * local`, turned it about the parent's Y instead.
        assert!(!(parent * turn * local).abs_diff_eq(turn * parent * local, 1e-3));
    }

    #[test]
    fn one_singular_parent_refuses_the_entire_capture() {
        let mut world = World::new();
        let good = world.spawn((Transform::default(),));
        let collapsed_parent = world.spawn((Transform {
            scale: glam::Vec3::ZERO,
            ..Transform::default()
        },));
        let bad = world.spawn((
            Transform::default(),
            Parent {
                entity: collapsed_parent,
            },
        ));

        assert!(capture_followers(&world, &[good, bad], u32::MAX).is_none());
    }
}
