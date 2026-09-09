//! MORROWIND-O/AF acceptance through public crate interfaces, without a window.
use glam::Vec3;
use somnium_asset::database::AssetId;
use somnium_core::prefab::{self, NestedPrefab, PrefabLibrary, PrefabMember, PrefabTemplate};
use somnium_core::save_game::{
    GameState, GameStateStack, SaveGame, SaveSlots, SceneDelta, StateEvent,
};
use somnium_core::scene_schema::{scene_from_json, scene_to_json};
use somnium_core::world_partition::CellCoord;
use somnium_core::{Name, Transform, WorldTransform};
use somnium_ecs::{PersistentId, World};

fn authored() -> (World, somnium_ecs::Entity) {
    let mut world = World::new();
    let entity = world.spawn((
        PersistentId::from_raw(123),
        Name::new("Crate"),
        Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
        WorldTransform::identity(),
    ));
    (world, entity)
}
fn library(world: &mut World, entity: somnium_ecs::Entity) -> (AssetId, PrefabLibrary) {
    let registry = somnium_core::reflect_registry::component_registry();
    let template = PrefabTemplate::capture(world, &registry, &[entity]).unwrap();
    let id = AssetId::from_relative_path("crate.somprefab");
    (id, PrefabLibrary::from([(id, template)]))
}

#[test]
fn prefab_instances_rebase_independently_and_roundtrip_v4() {
    let registry = somnium_core::reflect_registry::component_registry();
    let (mut world, entity) = authored();
    let (id, mut templates) = library(&mut world, entity);
    let one =
        prefab::instantiate(&mut world, &registry, &templates, id, "crate.somprefab").unwrap();
    let two =
        prefab::instantiate(&mut world, &registry, &templates, id, "crate.somprefab").unwrap();
    assert_ne!(world.persistent_id(one), world.persistent_id(two));
    world.get_mut::<Transform>(one).unwrap().translation.x = 9.0;
    assert_eq!(
        prefab::field_overrides(&mut world, &registry, one)
            .unwrap()
            .len(),
        1
    );
    templates.get_mut(&id).unwrap().scene["entities"][0]["components"]["somnium.Name"]["fields"]
        ["value"] = "Patched crate".into();
    prefab::refresh(&mut world, &registry, &templates, one, true).unwrap();
    prefab::refresh(&mut world, &registry, &templates, two, true).unwrap();
    assert_eq!(world.get::<Transform>(one).unwrap().translation.x, 9.0);
    assert_eq!(world.get::<Transform>(two).unwrap().translation.x, 1.0);
    assert_eq!(world.get::<Name>(one).unwrap().as_str(), "Patched crate");
    let scene = scene_to_json(&mut world, &registry);
    assert_eq!(scene["version"], 4);
    let mut loaded = World::new();
    scene_from_json(&mut loaded, &registry, &scene).unwrap();
    assert_eq!(scene_to_json(&mut loaded, &registry), scene);
    let loaded_one = loaded
        .entity_by_persistent_id(world.persistent_id(one).unwrap())
        .unwrap();
    prefab::refresh(&mut loaded, &registry, &templates, loaded_one, false).unwrap();
    assert_eq!(
        loaded.get::<Transform>(loaded_one).unwrap().translation.x,
        1.0
    );
    prefab::break_link(&mut loaded, loaded_one).unwrap();
    assert!(loaded.get::<PrefabMember>(loaded_one).is_none());
    assert!(loaded.is_alive(loaded_one));
}

#[test]
fn nested_prefabs_have_stable_paths_and_cycles_fail_before_mutation() {
    let registry = somnium_core::reflect_registry::component_registry();
    let (mut world, entity) = authored();
    let (id, mut templates) = library(&mut world, entity);
    let parent_id = AssetId::from_relative_path("house.somprefab");
    let mut parent = templates[&id].clone();
    parent.nested = vec![
        NestedPrefab {
            alias: "left".into(),
            template: id,
        },
        NestedPrefab {
            alias: "right".into(),
            template: id,
        },
    ];
    templates.insert(parent_id, parent);
    let root = prefab::instantiate(
        &mut world,
        &registry,
        &templates,
        parent_id,
        "house.somprefab",
    )
    .unwrap();
    let root_id = world.persistent_id(root).unwrap().to_string();
    let paths: Vec<_> = world
        .entities()
        .filter_map(|e| {
            world
                .get::<PrefabMember>(e)
                .filter(|m| m.root == root_id)
                .map(|m| m.path.clone())
        })
        .collect();
    assert_eq!(paths.len(), 3);
    assert!(paths.iter().any(|p| p[0] == "left"));
    templates.get_mut(&id).unwrap().nested.push(NestedPrefab {
        alias: "cycle".into(),
        template: parent_id,
    });
    let before = world.entities().count();
    assert!(
        prefab::instantiate(
            &mut world,
            &registry,
            &templates,
            parent_id,
            "house.somprefab"
        )
        .is_err()
    );
    assert_eq!(world.entities().count(), before);
}

#[test]
fn linked_selection_refresh_preserves_child_references_and_undo_scope() {
    use somnium_core::editor_commands::{EditorCommand, SetFieldCmd};
    use somnium_ecs::{FieldId, ReflectValue, StableId};
    let registry = somnium_core::reflect_registry::component_registry();
    let (mut world, entity) = authored();
    let child = world.spawn((
        PersistentId::from_raw(124),
        Name::new("Child"),
        Transform::default(),
        somnium_core::Parent { entity },
    ));
    let (id, templates) = library(&mut world, entity);
    prefab::link_selection(&mut world, &templates[&id], id, "crate.somprefab").unwrap();
    let mut edit = SetFieldCmd::new(
        &world,
        entity,
        StableId::new("somnium.Transform"),
        FieldId(0),
        ReflectValue::Vec3([8.0, 0.0, 0.0]),
        somnium_ui::GestureId(0),
        None,
    )
    .unwrap();
    edit.execute(&mut world, &mut Some(entity));
    assert_eq!(
        prefab::field_overrides(&mut world, &registry, entity)
            .unwrap()
            .len(),
        1
    );
    edit.undo(&mut world, &mut Some(entity));
    assert!(
        prefab::field_overrides(&mut world, &registry, entity)
            .unwrap()
            .is_empty()
    );
    prefab::refresh(&mut world, &registry, &templates, entity, true).unwrap();
    assert_eq!(
        world.get::<somnium_core::Parent>(child).unwrap().entity,
        entity
    );
}

#[test]
fn player_save_survives_an_author_patch_and_retains_cell_deltas() {
    let registry = somnium_core::reflect_registry::component_registry();
    let (mut world, entity) = authored();
    let authored = scene_to_json(&mut world, &registry);
    *world.get_mut::<Name>(entity).unwrap() = Name::new("Opened crate");
    let played = scene_to_json(&mut world, &registry);
    let mut save = SaveGame::new(1);
    save.global = SceneDelta::between(&authored, &played).unwrap();
    save.save_cell(CellCoord { x: 0, y: 0, z: 0 }, &authored, &played)
        .unwrap();
    save.save_cell(CellCoord { x: 1, y: 0, z: 0 }, &authored, &played)
        .unwrap();
    let retained = save.cells[1].clone();
    save.save_cell(CellCoord { x: 0, y: 0, z: 0 }, &authored, &authored)
        .unwrap();
    assert_eq!(save.cells[1], retained);
    let mut patch = authored.clone();
    patch["entities"][0]["components"]["somnium.Transform"]["fields"]["translation"] =
        serde_json::json!([50.0, 0.0, 0.0]);
    let result = save.global.rebase(&patch).unwrap();
    assert!(result.unresolved.is_empty());
    let mut loaded = World::new();
    scene_from_json(&mut loaded, &registry, &result.scene).unwrap();
    let entity = loaded
        .entity_by_persistent_id(PersistentId::from_raw(123))
        .unwrap();
    assert_eq!(loaded.get::<Transform>(entity).unwrap().translation.x, 50.0);
    assert_eq!(loaded.get::<Name>(entity).unwrap().as_str(), "Opened crate");
    let old = save.clone();
    assert!(
        save.migrate_content(2, |_, _, _| Err("missing migration".into()))
            .is_err()
    );
    assert_eq!(save, old);
    save.migrate_content(2, |next, from, to| {
        assert_eq!((from, to), (1, 2));
        next.state = serde_json::json!({"quest":2});
        Ok(())
    })
    .unwrap();
    assert_eq!(save.content_version, 2);
}

#[test]
fn slots_overwrite_atomically_reject_escape_and_migrate_old_format() {
    let directory = std::env::temp_dir().join(format!("somnium-save-{}", PersistentId::mint()));
    let slots = SaveSlots::new(&directory);
    let mut save = SaveGame::new(12);
    save.metadata.title = "First".into();
    slots.write("slot_1", &save).unwrap();
    save.metadata.title = "Second".into();
    slots.write("slot_1", &save).unwrap();
    assert_eq!(slots.read("slot_1").unwrap(), save);
    assert_eq!(slots.list().unwrap().len(), 1);
    assert!(slots.write("../escape", &save).is_err());
    assert!(slots.write("C:\\escape", &save).is_err());
    let mut version1 = serde_json::to_value(&save).unwrap();
    version1["version"] = 1.into();
    version1.as_object_mut().unwrap().remove("cells");
    let migrated = SaveGame::from_bytes(&serde_json::to_vec(&version1).unwrap()).unwrap();
    assert_eq!(migrated.version, 2);
    assert!(migrated.cells.is_empty());
    assert!(SaveGame::from_bytes(b"{partial").is_err());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn removed_content_is_reported_without_losing_the_players_override() {
    let registry = somnium_core::reflect_registry::component_registry();
    let (mut world, entity) = authored();
    let baseline = scene_to_json(&mut world, &registry);
    *world.get_mut::<Name>(entity).unwrap() = Name::new("Player name");
    let delta = SceneDelta::between(&baseline, &scene_to_json(&mut world, &registry)).unwrap();
    let mut deleted = baseline.clone();
    deleted["entities"] = serde_json::json!([]);
    assert_eq!(delta.rebase(&deleted).unwrap().unresolved.len(), 1);
    assert_eq!(
        delta.rebase(&baseline).unwrap().scene["entities"][0]["components"]["somnium.Name"]["fields"]
            ["value"],
        "Player name"
    );
}

#[test]
fn state_stack_dispatches_lifecycle_and_pauses_gameplay() {
    let mut stack = GameStateStack::default();
    stack.replace(GameState::Loading);
    stack.replace(GameState::Playing);
    assert!(stack.simulating());
    assert_eq!(
        stack.push(GameState::Paused),
        vec![
            StateEvent::Suspend(GameState::Playing),
            StateEvent::Enter(GameState::Paused)
        ]
    );
    assert!(!stack.simulating());
    assert_eq!(
        stack.pop().unwrap(),
        vec![
            StateEvent::Exit(GameState::Paused),
            StateEvent::Resume(GameState::Playing)
        ]
    );
    assert!(stack.simulating());
    assert!(stack.pop().is_err());
}

#[test]
fn propagated_nested_references_resolve_inside_the_new_instance() {
    let registry = somnium_core::reflect_registry::component_registry();
    let (mut world, entity) = authored();
    let (leaf, mut templates) = library(&mut world, entity);
    let parent_id = AssetId::from_relative_path("linked.somprefab");
    let mut parent = templates[&leaf].clone();
    parent.nested.push(NestedPrefab {
        alias: "nested".into(),
        template: leaf,
    });
    templates.insert(parent_id, parent);
    let root = prefab::instantiate(
        &mut world,
        &registry,
        &templates,
        parent_id,
        "linked.somprefab",
    )
    .unwrap();
    let root_id = world.persistent_id(root).unwrap().to_string();
    let nested = world
        .entities()
        .find(|e| {
            world
                .get::<PrefabMember>(*e)
                .is_some_and(|m| m.root == root_id && m.path[0] == "nested")
        })
        .unwrap();
    let mut unknown = somnium_core::RetainedUnknowns::default();
    unknown.keep_component("game.Reference",serde_json::json!({"version":1,"fields":{"target":{"$entity":world.persistent_id(nested).unwrap().to_string()}}}));
    world.insert_component(root, unknown).unwrap();
    prefab::propagate(&mut world, &registry, root, &mut templates).unwrap();
    let fresh = prefab::instantiate(
        &mut world,
        &registry,
        &templates,
        parent_id,
        "linked.somprefab",
    )
    .unwrap();
    let fresh_id = world.persistent_id(fresh).unwrap().to_string();
    let fresh_nested = world
        .entities()
        .find(|e| {
            world
                .get::<PrefabMember>(*e)
                .is_some_and(|m| m.root == fresh_id && m.path[0] == "nested")
        })
        .unwrap();
    let scene =
        somnium_core::scene_schema::entities_to_json(&mut world, &registry, &[fresh]).unwrap();
    assert_eq!(
        scene["entities"][0]["components"]["game.Reference"]["fields"]["target"]["$entity"],
        world.persistent_id(fresh_nested).unwrap().to_string()
    );
}

#[test]
fn prefab_transaction_undo_keeps_unrelated_handles_and_redoes_instance() {
    use somnium_core::EditorCommand;
    let registry = somnium_core::reflect_registry::component_registry();
    let (mut world, entity) = authored();
    let (id, templates) = library(&mut world, entity);
    let before = scene_to_json(&mut world, &registry);
    let root =
        prefab::instantiate(&mut world, &registry, &templates, id, "crate.somprefab").unwrap();
    let root_id = world.persistent_id(root).unwrap();
    let after = scene_to_json(&mut world, &registry);
    let mut command = prefab::PrefabEditCommand::new(before, after);
    command.undo(&mut world, &mut None);
    assert!(world.is_alive(entity));
    assert!(world.entity_by_persistent_id(root_id).is_none());
    command.execute(&mut world, &mut None);
    assert!(world.is_alive(entity));
    assert!(
        world
            .get::<PrefabMember>(world.entity_by_persistent_id(root_id).unwrap())
            .is_some()
    );
}

#[test]
fn null_and_removal_are_distinct_after_a_slot_roundtrip() {
    let baseline = serde_json::json!({"version":4,"entities":[{"persistent_id":format!("{:032x}",1),"components":{"game.Nullable":{"version":1,"fields":{"clear":7,"remove":8}}},"scripts":[]}]});
    let mut played = baseline.clone();
    let fields = played["entities"][0]["components"]["game.Nullable"]["fields"]
        .as_object_mut()
        .unwrap();
    fields.insert("clear".into(), serde_json::Value::Null);
    fields.remove("remove");
    let mut save = SaveGame::new(1);
    save.global = SceneDelta::between(&baseline, &played).unwrap();
    let restored = SaveGame::from_bytes(&serde_json::to_vec(&save).unwrap()).unwrap();
    let result = restored.global.rebase(&baseline).unwrap();
    assert_eq!(result.scene, played);
}

#[test]
fn malformed_save_cannot_rewrite_entity_identity() {
    let mut save = serde_json::to_value(SaveGame::new(1)).unwrap();
    save["global"]["changes"] = serde_json::json!({format!("{:032x}",123):[{"path":["persistent_id"],"value":format!("{:032x}",456)}]});
    assert!(SaveGame::from_bytes(&serde_json::to_vec(&save).unwrap()).is_err());
}
