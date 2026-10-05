use somnium_asset::database::AssetId;
use somnium_core::{
    EditorCommand, Name, Parent, Transform, World,
    prefab::{self, PrefabEditCommand, PrefabLibrary, PrefabTemplate},
};

#[test]
fn capture_detaches_external_parent_and_rejects_unrelated_roots() {
    let mut world = World::new();
    let parent = world.spawn((Name::new("Outside"), Transform::default()));
    let root = world.spawn((
        Name::new("Root"),
        Transform::default(),
        Parent { entity: parent },
    ));
    let child = world.spawn((
        Name::new("Child"),
        Transform::default(),
        Parent { entity: root },
    ));
    let registry = somnium_core::reflect_registry::component_registry();
    let template = PrefabTemplate::capture(&mut world, &registry, &[child, root]).unwrap();
    let entry = template.scene["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["persistent_id"] == template.root)
        .unwrap();
    assert!(entry["components"].get("somnium.Parent").is_none());
    let id = AssetId::from_relative_path("parentless.somprefab");
    let library = PrefabLibrary::from([(id, template)]);
    let copied =
        prefab::instantiate(&mut world, &registry, &library, id, "parentless.somprefab").unwrap();
    assert!(world.get::<Parent>(copied).is_none());
    assert!(PrefabTemplate::capture(&mut world, &registry, &[root, copied]).is_err());
}

#[test]
fn nested_alias_cycle_preflight_is_atomic_and_mounts_under_owner() {
    let mut world = World::new();
    let registry = somnium_core::reflect_registry::component_registry();
    let a = world.spawn((Name::new("Owner"), Transform::default()));
    let b = world.spawn((Name::new("Child"), Transform::default()));
    let aid = AssetId::from_relative_path("owner.somprefab");
    let bid = AssetId::from_relative_path("child.somprefab");
    let mut library = PrefabLibrary::from([
        (
            aid,
            PrefabTemplate::capture(&mut world, &registry, &[a]).unwrap(),
        ),
        (
            bid,
            PrefabTemplate::capture(&mut world, &registry, &[b]).unwrap(),
        ),
    ]);
    assert!(prefab::add_nested_template(&mut library, aid, bid, "bad\\alias").is_err());
    assert!(library[&aid].nested.is_empty());
    prefab::add_nested_template(&mut library, aid, bid, "child").unwrap();
    assert!(prefab::add_nested_template(&mut library, bid, aid, "cycle").is_err());
    assert!(library[&bid].nested.is_empty());
    let instance =
        prefab::instantiate(&mut world, &registry, &library, aid, "owner.somprefab").unwrap();
    assert!(
        world
            .entities()
            .any(|e| world.get::<Parent>(e).is_some_and(|p| p.entity == instance))
    );
}

#[test]
fn source_aware_undo_restores_file_and_world_and_refuses_external_edits() {
    let mut world = World::new();
    let entity = world.spawn((Name::new("Before"), Transform::default()));
    let registry = somnium_core::reflect_registry::component_registry();
    let before = somnium_core::scene_schema::scene_to_json(&mut world, &registry);
    *world.get_mut::<Name>(entity).unwrap() = Name::new("After");
    let after = somnium_core::scene_schema::scene_to_json(&mut world, &registry);
    let path = std::env::temp_dir().join(format!(
        "somnium-prefab-undo-{}-{}.somprefab",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, b"after").unwrap();
    let mut command = PrefabEditCommand::with_sources(
        before,
        after,
        vec![(path.clone(), b"before".to_vec(), b"after".to_vec())],
    );
    command.undo(&mut world, &mut None);
    assert_eq!(std::fs::read(&path).unwrap(), b"before");
    assert_eq!(world.get::<Name>(entity).unwrap().as_str(), "Before");
    command.execute(&mut world, &mut None);
    assert_eq!(std::fs::read(&path).unwrap(), b"after");
    std::fs::write(&path, b"external edit").unwrap();
    command.undo(&mut world, &mut None);
    assert_eq!(std::fs::read(&path).unwrap(), b"external edit");
    assert_eq!(world.get::<Name>(entity).unwrap().as_str(), "After");
    std::fs::remove_file(path).unwrap();
}
