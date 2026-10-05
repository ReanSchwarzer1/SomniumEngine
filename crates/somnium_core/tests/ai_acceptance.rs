//! Cross-crate MORROWIND-X/Y acceptance: cells/jobs, native cook/build reload,
//! scene persistence, shared graph authoring and the production behavior tick.
use glam::Vec3;
use somnium_ai::{
    behavior::{Status, Value},
    navigation::{BakeInput, BakeSettings, Bounds},
};
use somnium_asset::cook::{
    AssetCooker, AssetLoadMode, AssetResolver, CookConfig, CookKind, CookRequest,
};
use somnium_core::{ai::*, world_partition::CellCoord};
use somnium_jobs::JobSystem;
fn floor(x: f32) -> BakeInput {
    BakeInput {
        cell: [0, 0, 0],
        bounds: Bounds {
            min: [x, -1.0, 0.0],
            max: [x + 8.0, 5.0, 8.0],
        },
        settings: BakeSettings {
            voxel_size: 1.0,
            agent_radius: 0.0,
            ..Default::default()
        },
        triangles: vec![
            [[x, 0.0, 0.0], [x + 8.0, 0.0, 0.0], [x + 8.0, 0.0, 8.0]],
            [[x, 0.0, 0.0], [x + 8.0, 0.0, 8.0], [x, 0.0, 8.0]],
        ],
        obstacles: vec![],
    }
}
#[test]
fn cell_jobs_partial_rebake_cook_and_source_free_reload() {
    let mut jobs = JobSystem::single_threaded();
    let mut cells = NavigationCells::default();
    let a = CellCoord { x: 0, y: 0, z: 0 };
    let b = CellCoord { x: 1, y: 0, z: 0 };
    cells.request(&mut jobs, a, floor(0.0)).unwrap();
    cells.request(&mut jobs, b, floor(8.0)).unwrap();
    assert!(cells.poll().iter().all(|(_, r)| r.is_ok()));
    let original = cells.world.tile([1, 0, 0]).unwrap().encode().unwrap();
    let changed = cells
        .set_obstacle(
            &mut jobs,
            1,
            Some(Bounds {
                min: [3.0, -0.5, 2.0],
                max: [5.0, 3.0, 6.0],
            }),
        )
        .unwrap();
    assert_eq!(changed, vec![a]);
    assert!(cells.poll().iter().all(|(_, r)| r.is_ok()));
    assert_eq!(
        cells.world.tile([1, 0, 0]).unwrap().encode().unwrap(),
        original
    );
    assert!(
        cells
            .world
            .path(Vec3::new(1.0, 0.0, 4.0), Vec3::new(15.0, 0.0, 4.0), 0.1)
            .unwrap()
            .points
            .len()
            > 2
    );
    let root = std::env::temp_dir().join(format!(
        "somnium_ai_acceptance_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let source = root.join("source");
    std::fs::create_dir_all(&source).unwrap();
    let bytes = cells.world.tile([0, 0, 0]).unwrap().encode().unwrap();
    std::fs::write(source.join("cell.somnav.json"), &bytes).unwrap();
    let request = CookRequest {
        source: "cell.somnav.json".into(),
        kind: CookKind::Navigation,
        dependencies: vec![],
    };
    let report = AssetCooker::new(CookConfig {
        source_root: source.clone(),
        output_root: root.join("build"),
        cache_root: root.join("cache"),
        cooker_version: 1,
    })
    .cook(std::slice::from_ref(&request))
    .unwrap();
    let asset_id = request.asset_id();
    std::fs::remove_file(source.join("cell.somnav.json")).unwrap();
    let resolver = AssetResolver::new(
        source,
        root.join("build"),
        report.manifest,
        AssetLoadMode::Build,
    );
    let native = resolver.load(asset_id).unwrap();
    cells.unload(a);
    assert!(cells.world.tile([0, 0, 0]).is_none());
    assert_eq!(cells.install_cooked(&native).unwrap(), a);
    assert_eq!(
        cells.world.tile([0, 0, 0]).unwrap().encode().unwrap(),
        bytes
    );
    // Only the uniquely created test directory is removed.
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn applied_graph_runs_and_survives_scene_roundtrip_without_transient_state() {
    use somnium_ui::graph::{Graph, PinRef, behavior, serial};
    let c = behavior::catalogue();
    let mut graph = Graph::new();
    let set = graph.add(&c, "behavior.set", glam::Vec2::ZERO).unwrap();
    let root = graph
        .add(&c, "behavior.root", glam::Vec2::new(300.0, 0.0))
        .unwrap();
    graph
        .connect(&c, PinRef::output(set, 0), PinRef::input(root, 0))
        .unwrap();
    let json = serial::to_json(&graph, &c).unwrap();
    let mut world = somnium_ecs::World::new();
    let entity = world.spawn((
        somnium_core::Name::new("AI fixture"),
        somnium_core::Transform::default(),
    ));
    attach_behavior(&mut world, entity, json.clone()).unwrap();
    assert_eq!(
        tick_behaviors(&mut world, 0.1),
        vec![(entity, Status::Success)]
    );
    assert_eq!(
        behavior_blackboard(&world, entity).unwrap().get("alert"),
        Some(&Value::Bool(true))
    );
    let registry = somnium_core::reflect_registry::component_registry();
    let scene = somnium_core::scene_schema::scene_to_json(&mut world, &registry);
    let mut restored = somnium_ecs::World::new();
    let loaded =
        somnium_core::scene_schema::scene_from_json(&mut restored, &registry, &scene).unwrap();
    assert_eq!(loaded.entities.len(), 1);
    let entity = loaded.entities[0];
    assert_eq!(
        restored
            .get::<BehaviorComponent>(entity)
            .unwrap()
            .graph_json,
        json
    );
    assert!(behavior_blackboard(&restored, entity).is_none());
    assert_eq!(
        tick_behaviors(&mut restored, 0.1),
        vec![(entity, Status::Success)]
    );
    reset_behaviors(&mut restored);
    assert!(behavior_blackboard(&restored, entity).is_none());
    assert!(attach_behavior(&mut restored, entity, "{}".into()).is_err());
    assert_eq!(
        restored
            .get::<BehaviorComponent>(entity)
            .unwrap()
            .graph_json,
        json
    );
}

#[test]
fn shipped_luau_patrol_uses_generic_schema_fields_and_the_budgeted_task_host() {
    use somnium_ai::{
        behavior::{BehaviorInstance, BehaviorTree, Node, Task},
        script::{ScriptTaskBinding, ScriptTaskHost},
    };
    use somnium_core::{
        Name, Transform,
        script_bridge::{EngineWorldView, apply_commands},
    };
    use somnium_script::{
        backend::{Budget, Callback, ScriptBackend, ScriptSource},
        command::CommandBuffer,
        ids::{InstanceUuid, LanguageTag, ScriptAssetId, ScriptInstanceId},
        order::OrderKey,
        snapshot::ScriptSnapshot,
    };
    use std::collections::BTreeMap;
    let mut world = somnium_ecs::World::new();
    let actor = world.spawn((
        Name::new("Scripted patrol"),
        Transform::default(),
        NavigationAgent::default(),
        PerceptionComponent {
            confidence: 0.8,
            ..Default::default()
        },
        BehaviorComponent::default(),
    ));
    let persistent = world.ensure_persistent_id(actor).unwrap();
    let registry = somnium_core::reflect_registry::component_registry();
    let mut backend = somnium_script_luau::LuauBackend::new(Budget::default()).unwrap();
    let module = backend
        .compile(&ScriptSource {
            id: ScriptAssetId::mint(),
            language: LanguageTag::LUAU,
            display_path: "assets/scripts/morrowind_ai_patrol.luau".into(),
            text: include_str!("../../../assets/scripts/morrowind_ai_patrol.luau").into(),
        })
        .unwrap();
    let instance = ScriptInstanceId::next();
    backend
        .instantiate(instance, module, &BTreeMap::new())
        .unwrap();
    let order = OrderKey::new(0, persistent, InstanceUuid::mint());
    let snapshot = ScriptSnapshot {
        time: Default::default(),
        input: Default::default(),
        self_entity: actor,
        self_persistent: persistent,
        self_components: BTreeMap::new(),
        spawn_results: vec![],
        events: vec![],
        rng_seed: 1,
    };
    let mut commands = CommandBuffer::new();
    backend
        .invoke(
            instance,
            order,
            Callback::FixedUpdate,
            &snapshot,
            &EngineWorldView::new(&world, &registry),
            &mut commands,
        )
        .unwrap();
    let _ = apply_commands(&mut world, &registry, commands.drain_sorted());
    assert_eq!(
        world.get::<NavigationAgent>(actor).unwrap().destination,
        Vec3::new(3.0, 0.0, 0.0)
    );
    assert_eq!(world.get::<NavigationAgent>(actor).unwrap().speed, 2.4);
    world.get_mut::<NavigationAgent>(actor).unwrap().arrived = true;
    backend
        .invoke(
            instance,
            order,
            Callback::FixedUpdate,
            &snapshot,
            &EngineWorldView::new(&world, &registry),
            &mut commands,
        )
        .unwrap();
    let _ = apply_commands(&mut world, &registry, commands.drain_sorted());
    assert_eq!(
        world.get::<NavigationAgent>(actor).unwrap().destination,
        Vec3::new(-3.0, 0.0, 0.0)
    );
    let bindings = BTreeMap::from([("patrol".into(), ScriptTaskBinding { instance, order })]);
    let view = EngineWorldView::new(&world, &registry);
    let mut host = ScriptTaskHost {
        backend: &mut backend,
        bindings: &bindings,
        snapshot: &snapshot,
        world: &view,
        commands: &mut commands,
        diagnostics: vec![],
    };
    let mut tree = BehaviorInstance::new(
        BehaviorTree::new(0, vec![Node::Task(Task::Script("patrol".into()))]).unwrap(),
    );
    assert_eq!(
        tree.tick(0.1, &mut host),
        Status::Success,
        "{:?}",
        host.diagnostics
    );
    assert_eq!(tree.blackboard.get("alert"), Some(&Value::Bool(true)));
    assert!(host.diagnostics.is_empty());
}

#[test]
fn latest_explicit_bake_owns_agents_and_clear_drops_transient_routes() {
    use somnium_core::{MeshKind, Name, Transform};
    let mut world = somnium_ecs::World::new();
    let profile = world.spawn((
        Name::new("Profile"),
        Transform::from_translation(Vec3::new(4.0, 1.0, 4.0)),
        NavigationProfile {
            bounds_size: Vec3::new(8.0, 6.0, 8.0),
            tile_size: 8.0,
            voxel_size: 1.0,
            agent_radius: 0.0,
            ..Default::default()
        },
    ));
    let actor = world.spawn((
        Name::new("Agent"),
        Transform::from_translation(Vec3::new(1.0, 0.0, 4.0)),
        MeshKind::Cube,
        NavigationAgent {
            destination: Vec3::new(7.0, 0.0, 4.0),
            ..Default::default()
        },
    ));
    let bounds = floor(0.0).bounds;
    assert!(
        collect_geometry(&world, None, bounds).unwrap().0.is_empty(),
        "moving actor geometry cannot become a permanent bake obstacle"
    );
    let mut jobs = JobSystem::single_threaded();
    let mut game = NavigationEditor::default();
    let mut editor = NavigationEditor::default();
    game.bake_geometry(&mut world, &mut jobs, profile, floor(0.0).triangles)
        .unwrap();
    game.update_runtime(&mut world, &mut jobs, 0.1, true);
    let moved = world.get::<Transform>(actor).unwrap().translation;
    let polygons = world
        .get::<NavigationProfile>(profile)
        .unwrap()
        .polygon_count;
    assert!(moved.x > 1.0 && polygons > 0);
    editor.update_runtime(&mut world, &mut jobs, 0.1, true);
    assert_eq!(world.get::<Transform>(actor).unwrap().translation, moved);
    assert_eq!(
        world
            .get::<NavigationProfile>(profile)
            .unwrap()
            .polygon_count,
        polygons
    );
    editor
        .bake_geometry(&mut world, &mut jobs, profile, floor(0.0).triangles)
        .unwrap();
    game.update_runtime(&mut world, &mut jobs, 0.1, true);
    assert_eq!(
        world.get::<Transform>(actor).unwrap().translation,
        moved,
        "old manager yields to explicit designer bake"
    );
    editor.update_runtime(&mut world, &mut jobs, 0.1, true);
    assert!(world.get::<Transform>(actor).unwrap().translation.x > moved.x);
    assert!(
        editor
            .preview_lines()
            .iter()
            .any(|line| line.color[2] > line.color[0])
    );
    editor.clear(&mut world);
    assert!(!world.get::<NavigationAgent>(actor).unwrap().arrived);
    assert!(editor.preview_lines().is_empty());
}

#[test]
fn designer_profiles_geometry_carving_agents_and_sensors_use_real_runtime() {
    use somnium_core::ai::{
        NavigationAgent, NavigationEditor, NavigationObstacle, NavigationProfile,
        PerceptionComponent, collect_geometry,
    };
    use somnium_core::{MeshKind, Name, Transform};
    let mut world = somnium_ecs::World::new();
    world.spawn((
        Name::new("Floor"),
        MeshKind::Plane,
        Transform {
            scale: Vec3::splat(8.0),
            ..Default::default()
        },
    ));
    let profile = world.spawn((
        Name::new("Navigation"),
        Transform::default(),
        NavigationProfile {
            bounds_size: Vec3::new(8.0, 4.0, 8.0),
            tile_size: 4.0,
            voxel_size: 0.5,
            agent_radius: 0.1,
            ..Default::default()
        },
    ));
    let (triangles, skipped) = collect_geometry(
        &world,
        None,
        Bounds {
            min: [-4.0, -2.0, -4.0],
            max: [4.0, 2.0, 4.0],
        },
    )
    .unwrap();
    assert_eq!(triangles.len(), 2);
    assert_eq!(skipped, 0);
    let mut editor = NavigationEditor::default();
    editor.inspect(&mut world, profile).unwrap();
    assert!(!editor.preview_lines().is_empty());
    let mut jobs = JobSystem::single_threaded();
    editor
        .bake_geometry(&mut world, &mut jobs, profile, triangles)
        .unwrap();
    editor.update_runtime(&mut world, &mut jobs, 0.1, false);
    assert!(
        world
            .get::<NavigationProfile>(profile)
            .unwrap()
            .status
            .starts_with("Ready")
    );
    assert!(
        world
            .get::<NavigationProfile>(profile)
            .unwrap()
            .polygon_count
            > 0
    );
    let obstacle = world.spawn((
        Name::new("Obstacle"),
        Transform::default(),
        NavigationObstacle {
            enabled: true,
            size: Vec3::new(2.0, 3.0, 4.0),
        },
    ));
    editor.update_runtime(&mut world, &mut jobs, 0.1, false);
    let start = Vec3::new(-3.0, 0.0, 0.0);
    let target = Vec3::new(3.0, 0.0, 0.0);
    assert!(editor.cells.world.raycast(start, target, 0.1).is_some());
    assert!(
        editor
            .cells
            .world
            .path(start, target, 0.1)
            .unwrap()
            .points
            .len()
            > 2
    );
    world
        .get_mut::<NavigationObstacle>(obstacle)
        .unwrap()
        .enabled = false;
    editor.update_runtime(&mut world, &mut jobs, 0.1, false);
    assert!(editor.cells.world.raycast(start, target, 0.1).is_none());
    let actor = world.spawn((
        Name::new("Agent"),
        Transform {
            translation: start,
            ..Default::default()
        },
        NavigationAgent {
            destination: target,
            radius: 0.1,
            ..Default::default()
        },
    ));
    for _ in 0..1800 {
        editor.update_runtime(&mut world, &mut jobs, 1.0 / 60.0, true);
        if world.get::<NavigationAgent>(actor).unwrap().arrived {
            break;
        }
    }
    assert!(
        world.get::<NavigationAgent>(actor).unwrap().arrived,
        "{} at {:?}",
        world.get::<NavigationAgent>(actor).unwrap().status,
        world.get::<Transform>(actor).unwrap().translation
    );
    let observer = world.spawn((
        Name::new("Observer"),
        Transform {
            translation: Vec3::new(3.0, 0.0, 3.0),
            ..Default::default()
        },
        PerceptionComponent {
            target: actor,
            ..Default::default()
        },
    ));
    editor.update_runtime(&mut world, &mut jobs, 0.1, false);
    assert_eq!(
        world.get::<PerceptionComponent>(observer).unwrap().status,
        "Target visible"
    );
    assert!(
        world
            .get::<PerceptionComponent>(observer)
            .unwrap()
            .confidence
            > 0.0
    );
    let registry = somnium_core::reflect_registry::component_registry();
    let scene = somnium_core::scene_schema::scene_to_json(&mut world, &registry);
    let text = serde_json::to_string(&scene).unwrap();
    assert!(text.contains("somnium.NavigationProfile"));
    assert!(text.contains("somnium.Perception"));
    assert!(!text.contains("Target visible"));
    assert!(!text.contains("Following path"));
    editor.clear(&mut world);
    assert_eq!(editor.cells.world.cells().count(), 0);
    assert_eq!(
        world
            .get::<NavigationProfile>(profile)
            .unwrap()
            .polygon_count,
        0
    );
}

fn attached_task_graph(name: &str) -> String {
    use somnium_ui::graph::{Graph, PinRef, behavior, serial};
    let catalogue = behavior::catalogue();
    let mut graph = Graph::new();
    let task = graph
        .add(&catalogue, "behavior.script", glam::Vec2::ZERO)
        .unwrap();
    graph
        .node_mut(task)
        .unwrap()
        .literals
        .insert(0, name.into());
    let root = graph
        .add(&catalogue, "behavior.root", glam::Vec2::new(300.0, 0.0))
        .unwrap();
    graph
        .connect(&catalogue, PinRef::output(task, 0), PinRef::input(root, 0))
        .unwrap();
    serial::to_json(&graph, &catalogue).unwrap()
}

#[test]
fn designer_attached_script_graph_repairs_binding_and_uses_normal_host_commands() {
    use somnium_core::{
        Name, Transform,
        script_host::{HostServices, ScriptHost},
    };
    use somnium_script::{
        attachment::{ScriptAttachment, ScriptSet},
        capability::Capabilities,
        ids::ScriptAssetId,
        runtime::PhaseInput,
    };
    let mut world = somnium_ecs::World::new();
    let actor = world.spawn((
        Name::new("Behavior actor"),
        Transform::default(),
        NavigationAgent {
            enabled: false,
            ..Default::default()
        },
        PerceptionComponent {
            confidence: 0.8,
            ..Default::default()
        },
    ));
    attach_behavior(
        &mut world,
        actor,
        attached_task_graph("morrowind_ai_patrol"),
    )
    .unwrap();
    let mut host = ScriptHost::default();
    let mut services = HostServices::default();
    let mut phase = PhaseInput::default();
    phase.time.delta = 0.1;
    assert_eq!(
        host.tick_behaviors(&mut world, phase.time, &phase.input, &mut services),
        vec![(actor, Status::Failure)]
    );
    assert!(
        world
            .get::<BehaviorComponent>(actor)
            .unwrap()
            .last_error
            .contains("No attached script")
    );
    let asset = ScriptAssetId::mint();
    host.load_script(
        asset,
        "assets/scripts/morrowind_ai_patrol.luau",
        include_str!("../../../assets/scripts/morrowind_ai_patrol.luau"),
    )
    .unwrap();
    world
        .insert_component(
            actor,
            ScriptSet {
                attachments: vec![ScriptAttachment::new(asset)],
            },
        )
        .unwrap();
    let other = world.spawn((
        Name::new("Other attachment"),
        Transform::default(),
        NavigationAgent {
            enabled: false,
            ..Default::default()
        },
        PerceptionComponent::default(),
        ScriptSet {
            attachments: vec![ScriptAttachment::new(asset)],
        },
    ));
    let report = host.sync(&mut world, &phase, &mut services);
    assert!(report.failures.is_empty());
    assert_eq!(
        host.tick_behaviors(&mut world, phase.time, &phase.input, &mut services),
        vec![(actor, Status::Running)]
    );
    assert!(world.get::<NavigationAgent>(actor).unwrap().enabled);
    assert!(
        !world.get::<NavigationAgent>(other).unwrap().enabled,
        "private AI events cannot invoke another attachment"
    );
    assert_eq!(
        behavior_blackboard(&world, actor).unwrap().get("alert"),
        Some(&Value::Bool(true))
    );
    assert!(
        world
            .get::<BehaviorComponent>(actor)
            .unwrap()
            .last_error
            .is_empty(),
        "attaching the missing script repairs the cached failed tree"
    );
    world.get_mut::<NavigationAgent>(actor).unwrap().arrived = true;
    assert_eq!(
        host.tick_behaviors(&mut world, phase.time, &phase.input, &mut services),
        vec![(actor, Status::Success)]
    );
    // The path-based alias uses the same attached module and capability gate.
    attach_behavior(
        &mut world,
        actor,
        attached_task_graph("assets/scripts/morrowind_ai_patrol.luau"),
    )
    .unwrap();
    world.get_mut::<NavigationAgent>(actor).unwrap().enabled = false;
    world.get_mut::<NavigationAgent>(actor).unwrap().arrived = false;
    host.runtime_mut()
        .set_capabilities(asset, Capabilities::NONE);
    host.tick_behaviors(&mut world, phase.time, &phase.input, &mut services);
    assert!(!world.get::<NavigationAgent>(actor).unwrap().enabled);
    assert!(
        host.take_rejections()
            .iter()
            .any(|message| message.contains("capability"))
    );
}

#[test]
fn attached_task_refuses_ambiguous_alias_and_invalid_state_without_partial_writes() {
    use somnium_core::{
        Name, Transform,
        script_host::{HostServices, ScriptHost},
    };
    use somnium_script::{
        attachment::{ScriptAttachment, ScriptSet},
        ids::ScriptAssetId,
        runtime::PhaseInput,
    };
    let mut world = somnium_ecs::World::new();
    let mut host = ScriptHost::default();
    let mut services = HostServices::default();
    let phase = PhaseInput::default();
    let broken = r#"return Script.define({
        loadState=function(self,state) self.board=state.blackboard end,
        onEvent=function(self,ctx,events) ctx:set(ctx.entity,'somnium.NavigationAgent','enabled',true) end,
        saveState=function(self) return {status='not a status',blackboard=self.board or {}} end
    })"#;
    let a = ScriptAssetId::mint();
    let b = ScriptAssetId::mint();
    host.load_script(a, "first/task.luau", broken).unwrap();
    host.load_script(b, "second/task.luau", broken).unwrap();
    let actor = world.spawn((
        Name::new("Actor"),
        Transform::default(),
        NavigationAgent {
            enabled: false,
            ..Default::default()
        },
        ScriptSet {
            attachments: vec![ScriptAttachment::new(a), ScriptAttachment::new(b)],
        },
    ));
    attach_behavior(&mut world, actor, attached_task_graph("task")).unwrap();
    host.sync(&mut world, &phase, &mut services);
    host.tick_behaviors(&mut world, phase.time, &phase.input, &mut services);
    assert!(
        world
            .get::<BehaviorComponent>(actor)
            .unwrap()
            .last_error
            .contains("Multiple attached scripts")
    );
    attach_behavior(&mut world, actor, attached_task_graph("first/task.luau")).unwrap();
    assert_eq!(
        host.tick_behaviors(&mut world, phase.time, &phase.input, &mut services),
        vec![(actor, Status::Failure)]
    );
    assert!(
        world
            .get::<BehaviorComponent>(actor)
            .unwrap()
            .last_error
            .contains("invalid status")
    );
    assert!(
        !world.get::<NavigationAgent>(actor).unwrap().enabled,
        "invalid protocol discards that attachment's command batch"
    );
}

#[test]
fn editor_agent_link_handoff_rejects_wrong_exit_and_resumes_through_details() {
    use somnium_core::{Children, Name, Parent, Transform, WorldTransform, propagate_transforms};
    use somnium_ecs::ReflectValue;
    let mut world = somnium_ecs::World::new();
    let profile = world.spawn((
        Name::new("Stacked floors"),
        Transform::from_translation(Vec3::new(4.0, 3.0, 4.0)),
        NavigationProfile {
            bounds_size: Vec3::new(8.0, 10.0, 8.0),
            tile_size: 8.0,
            voxel_size: 1.0,
            agent_radius: 0.0,
            ..Default::default()
        },
    ));
    let parent = world.spawn((
        Transform::from_translation(Vec3::new(1.0, 0.0, 4.0)),
        WorldTransform::identity(),
        Children::empty(),
    ));
    let actor = world.spawn((
        Name::new("Climber"),
        Transform::default(),
        WorldTransform::identity(),
        Parent { entity: parent },
        NavigationAgent {
            destination: Vec3::new(6.0, 4.0, 4.0),
            ..Default::default()
        },
    ));
    world.get_mut::<Children>(parent).unwrap().push(actor);
    let link = world.spawn((
        Name::new("Ladder"),
        Transform::from_translation(Vec3::new(1.0, 0.0, 4.0)),
        NavigationLink {
            destination: Vec3::new(1.0, 4.0, 4.0),
            bidirectional: false,
            ..Default::default()
        },
    ));
    let mut geometry = floor(0.0).triangles;
    geometry.extend(floor(0.0).triangles.into_iter().map(|triangle| {
        triangle.map(|mut vertex| {
            vertex[1] = 4.0;
            vertex
        })
    }));
    propagate_transforms(&mut world);
    let mut jobs = JobSystem::single_threaded();
    let mut navigation = NavigationEditor::default();
    navigation
        .bake_geometry(&mut world, &mut jobs, profile, geometry)
        .unwrap();
    navigation.update_runtime(&mut world, &mut jobs, 0.1, true);
    let actual_position = world
        .get::<WorldTransform>(actor)
        .unwrap()
        .0
        .transform_point3(Vec3::ZERO);
    assert_eq!(
        actual_position,
        Vec3::new(1.0, 0.0, 4.0),
        "fixture must register both directions of the hierarchy before propagation"
    );
    let pending = pending_navigation_link(&world, actor).unwrap_or_else(||panic!("agent did not reach link: position={actual_position:?}, agent={:?}, profile={:?}, path={:?}",world.get::<NavigationAgent>(actor),world.get::<NavigationProfile>(profile),navigation.cells.world.path(actual_position,Vec3::new(6.0,4.0,4.0),0.3)));
    assert_eq!(pending.link, link);
    assert_eq!(pending.exit, Vec3::new(1.0, 4.0, 4.0));
    assert_eq!(
        world.get::<NavigationAgent>(actor).unwrap().pending_link,
        link
    );
    assert!(acknowledge_navigation_link(&mut world, actor, profile).is_err());
    assert!(
        acknowledge_navigation_link(&mut world, actor, link)
            .unwrap_err()
            .contains("Move to Link Exit")
    );
    assert!(pending_navigation_link(&world, actor).is_some());
    // A link changed while the actor is traversing cannot acknowledge stale
    // endpoints. Even matching live handles are insufficient.
    world.get_mut::<NavigationLink>(link).unwrap().destination.y = 5.0;
    assert!(
        acknowledge_navigation_link(&mut world, actor, link)
            .unwrap_err()
            .contains("changed")
    );
    world.get_mut::<NavigationLink>(link).unwrap().destination = pending.exit;
    // Designer/game motion is separate. The one-shot field uses exactly the
    // same reflected path as Details and the generic Luau ctx:set API.
    world.get_mut::<Transform>(actor).unwrap().translation.y = 4.0;
    let registry = somnium_core::reflect_registry::component_registry();
    let schema = registry.by_name("somnium.NavigationAgent").unwrap();
    let field = schema.field_by_name("acknowledge_link").unwrap();
    assert!(
        field
            .flags
            .contains(somnium_ecs::reflect::FieldFlags::SCRIPT_WRITE)
    );
    assert!(
        !field
            .flags
            .contains(somnium_ecs::reflect::FieldFlags::SERIALIZE)
    );
    (schema.apply)(
        &mut world,
        actor,
        &std::collections::BTreeMap::from([(field.id, ReflectValue::Bool(true))]),
    )
    .unwrap();
    propagate_transforms(&mut world);
    navigation.update_runtime(&mut world, &mut jobs, 0.1, true);
    assert!(pending_navigation_link(&world, actor).is_none());
    assert!(
        !world
            .get::<NavigationAgent>(actor)
            .unwrap()
            .acknowledge_link
    );
    for _ in 0..500 {
        propagate_transforms(&mut world);
        navigation.update_runtime(&mut world, &mut jobs, 0.05, true);
        if world.get::<NavigationAgent>(actor).unwrap().arrived {
            break;
        }
    }
    assert!(
        world.get::<NavigationAgent>(actor).unwrap().arrived,
        "{:?}",
        world.get::<NavigationAgent>(actor).unwrap()
    );
    let scene = somnium_core::scene_schema::scene_to_json(&mut world, &registry).to_string();
    for transient in ["pending_link", "link_exit", "acknowledge_link"] {
        assert!(!scene.contains(transient));
    }
}
