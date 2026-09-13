//! Typed, revision-checked authoring shared by the editor and local adapters.
mod codec;
pub mod documents;
pub mod feedback;
pub mod project;
pub mod registration;
mod transaction;
pub mod transport;
pub use registration::{GameDocument, GamePreset, GameRegistration, game_registration};
pub use transaction::{AuthoringError, AuthoringSession, Operation, PlanRequest};

/// Main-thread publication after a game changes authored state.
pub fn rebuild_game(world: &mut somnium_ecs::World) {
    registration::rebuild(world);
}

#[cfg(test)]
mod persistence_tests {
    #[test]
    fn authoring_scene_restores_import_origin_and_child_world_transform() {
        use crate::{ImportedMesh, Name, Parent, Transform, WorldTransform};
        let registry = crate::reflect_registry::component_registry();
        let mut world = somnium_ecs::World::new();
        let parent = world.spawn((
            Name::new("Parent"),
            Transform::from_translation(glam::Vec3::new(10.0, 0.0, 0.0)),
        ));
        let child = world.spawn((
            Name::new("Imported"),
            Transform::from_translation(glam::Vec3::new(0.0, 2.0, 0.0)),
            Parent { entity: parent },
            ImportedMesh {
                source: "assets/props/desk.glb".into(),
                node: 2,
            },
        ));
        let id = world.ensure_persistent_id(child).unwrap();
        let document = crate::scene_schema::scene_to_json(&mut world, &registry);
        let mut restored = somnium_ecs::World::new();
        crate::scene_schema::scene_from_json(&mut restored, &registry, &document).unwrap();
        crate::propagate_transforms(&mut restored);
        let child = restored.entity_by_persistent_id(id).unwrap();
        let origin = restored.get::<ImportedMesh>(child).unwrap();
        assert_eq!(origin.source, "assets/props/desk.glb");
        assert_eq!(origin.node, 2);
        assert_eq!(
            restored
                .get::<WorldTransform>(child)
                .unwrap()
                .0
                .w_axis
                .truncate(),
            glam::Vec3::new(10.0, 2.0, 0.0)
        );
    }
}
