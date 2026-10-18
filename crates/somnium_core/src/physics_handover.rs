//! Physics bodies that hand over with streamed cells.
//!
//! A game makes Jolt bodies for its actors (a **Rigid Body**'s `body`), and
//! world partition despawns actors when their cell streams out. Nothing else
//! connects the two, so a streamed-out actor's body would stay in Jolt,
//! simulating out of sight with nobody to read it. [`BodyHandover`] closes
//! that: each update it adopts the bodies of streamed actors (those with a
//! [`crate::world_partition::StreamedActor`] and a Rigid Body) and frees the
//! body of any actor that is gone. With
//! [`crate::world_partition::WorldPartition::rehome`] first, an actor that
//! walked or was carried into another cell is saved there, and streams back
//! where it was left, ready for the game to give it a body again.
//!
//! Opt-in: a game calls it. Bodies of actors that are not streamed, and bodies
//! a game tracks itself, are never touched. Resting bodies already sleep in
//! Jolt (`RigidBodyDescriptor::allow_sleeping`, on by default).

use crate::character::RigidBodyComponent;
use crate::world_partition::StreamedActor;
use somnium_ecs::{Entity, World};
use somnium_physics::body::BodyId;
use somnium_physics::world::PhysicsWorld;
use std::collections::HashMap;

/// Bodies owned by streamed actors.
#[derive(Debug, Default)]
pub struct BodyHandover {
    bodies: HashMap<Entity, BodyId>,
}

impl BodyHandover {
    /// Hand over `body` with `entity` (an actor that is not streamed but
    /// should still lose its body when despawned).
    pub fn track(&mut self, entity: Entity, body: BodyId) {
        if body.is_valid() {
            self.bodies.insert(entity, body);
        }
    }

    /// Stop handing over `entity`'s body (the game frees it itself).
    pub fn untrack(&mut self, entity: Entity) -> Option<BodyId> {
        self.bodies.remove(&entity)
    }

    /// Bodies being handed over.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bodies.len()
    }

    /// Whether none are.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bodies.is_empty()
    }

    /// Adopt streamed actors' bodies and free those whose actor is gone.
    /// Returns how many bodies were freed.
    pub fn update(&mut self, world: &World, physics: &mut PhysicsWorld) -> usize {
        for entity in world.entities_with::<StreamedActor>() {
            if let Some(body) = world
                .get::<RigidBodyComponent>(entity)
                .and_then(RigidBodyComponent::body_id)
            {
                self.bodies.insert(entity, body);
            }
        }
        let gone: Vec<(Entity, BodyId)> = self
            .bodies
            .iter()
            .filter(|(e, _)| !world.is_alive(**e))
            .map(|(e, b)| (*e, *b))
            .collect();
        for (entity, body) in &gone {
            physics.destroy_body(*body);
            self.bodies.remove(entity);
        }
        gone.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_partition::CellCoord;
    use glam::Vec3;
    use somnium_physics::body::{MotionType, RigidBodyDescriptor};
    use somnium_physics::config::PhysicsConfig;
    use somnium_physics::shape::ColliderShape;

    fn body(physics: &mut PhysicsWorld) -> BodyId {
        physics.create_body(RigidBodyDescriptor {
            shape: ColliderShape::Box {
                half_extents: Vec3::splat(0.5),
            },
            motion_type: MotionType::Dynamic,
            ..RigidBodyDescriptor::default()
        })
    }

    #[test]
    fn a_streamed_actors_body_goes_with_it_and_others_stay() {
        let mut physics = PhysicsWorld::new(PhysicsConfig::default());
        let mut world = World::new();
        let streamed = world.spawn((
            RigidBodyComponent::driven(body(&mut physics)),
            StreamedActor {
                cell: CellCoord::default(),
                position: [0.0; 3],
            },
        ));
        let plain = world.spawn((RigidBodyComponent::driven(body(&mut physics)),));
        let tracked = world.spawn((crate::Transform::default(),));
        let mut handover = BodyHandover::default();
        handover.track(tracked, body(&mut physics));
        assert_eq!(physics.num_bodies(), 3);
        assert_eq!(handover.update(&world, &mut physics), 0);
        assert_eq!(handover.len(), 2, "the streamed actor's and the tracked");
        // The cell streams out: its actor is despawned. So is the tracked one;
        // the plain body's entity is despawned too but it was never handed over.
        world.despawn(streamed);
        world.despawn(tracked);
        world.despawn(plain);
        assert_eq!(handover.update(&world, &mut physics), 2);
        assert_eq!(physics.num_bodies(), 1, "the game still owns the plain one");
        assert!(handover.is_empty());
    }
}
