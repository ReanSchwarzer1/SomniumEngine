use glam::{Mat4, Quat, Vec3};
use somnium_core::{
    Parent, Transform, World,
    animation_authoring::{
        AnimationAuthoring, AnimationAuthoringSystem, AnimationPreviewJoint, create_preview_rig,
    },
};

#[test]
fn parented_rig_uses_current_hierarchy_and_clear_preserves_authored_transforms() {
    let mut world = World::new();
    let rig = create_preview_rig(&mut world, Vec3::new(2.0, 0.0, 1.0));
    let parent = world.spawn((Transform {
        translation: Vec3::new(12.0, 3.0, -8.0),
        rotation: Quat::from_rotation_y(0.7),
        scale: Vec3::ONE,
    },));
    world
        .insert_component(rig, Parent { entity: parent })
        .unwrap();
    let initial = *world.get::<Transform>(rig).unwrap();
    let target = world.get::<AnimationAuthoring>(rig).unwrap().reach_target;
    // These edits deliberately have not propagated their WorldTransform caches.
    world.get_mut::<Transform>(target).unwrap().translation = Vec3::new(0.8, 1.3, 0.4);
    let settings = world.get_mut::<AnimationAuthoring>(rig).unwrap();
    settings.pose_jobs = false;
    settings.ik_weight = 1.0;
    let mut system = AnimationAuthoringSystem::default();
    let mut jobs = somnium_core::jobs::JobSystem::single_threaded();
    system.update(&mut world, None, &mut jobs, 1.0);
    assert_eq!(
        world.get::<AnimationAuthoring>(rig).unwrap().status,
        "Playing"
    );
    let end = system
        .pose(rig)
        .unwrap()
        .local
        .iter()
        .fold(Mat4::IDENTITY, |m, t| m * t.to_matrix());
    assert!(
        end.w_axis
            .truncate()
            .abs_diff_eq(Vec3::new(0.8, 1.3, 0.4), 1e-4)
    );
    assert!(
        world
            .get::<Transform>(rig)
            .unwrap()
            .to_matrix()
            .abs_diff_eq(initial.to_matrix(), 1e-5)
    );
    // Scene boundaries can already have loaded a different authored transform.
    world.get_mut::<Transform>(rig).unwrap().translation = Vec3::new(-3.0, 4.0, 2.0);
    system.clear(&mut world, None);
    assert!(system.pose(rig).is_none());
    assert_eq!(
        world.get::<Transform>(rig).unwrap().translation,
        Vec3::new(-3.0, 4.0, 2.0)
    );
    let settings = world.get::<AnimationAuthoring>(rig).unwrap();
    assert_eq!(settings.preview_time, 0.0);
    assert_eq!(settings.event_count, 0);
    assert!(settings.last_event.is_empty());
    assert_eq!(settings.status, "Ready");
}

#[test]
fn designer_preview_uses_details_controls_pose_jobs_and_draggable_targets() {
    let mut world = World::new();
    let rig = create_preview_rig(&mut world, Vec3::ZERO);
    assert_eq!(
        world
            .entities()
            .filter(|e| world.get::<AnimationPreviewJoint>(*e).is_some())
            .count(),
        3
    );
    assert_eq!(
        world
            .entities()
            .filter(|e| {
                world.get::<somnium_core::MeshKind>(*e).is_some()
                    && world.get::<somnium_core::MaterialComponent>(*e).is_some()
                    && world.get::<somnium_core::WorldTransform>(*e).is_some()
            })
            .count(),
        5,
        "Every preview mesh needs a material and propagated transform for the render query"
    );
    let reach = world.get::<AnimationAuthoring>(rig).unwrap().reach_target;
    assert!(
        world
            .get::<somnium_core::WorldTransform>(reach)
            .unwrap()
            .0
            .w_axis
            .truncate()
            .abs_diff_eq(Vec3::new(1.2, 1.2, 0.0), 1e-5),
        "The hierarchy child index must participate in transform propagation"
    );
    let mut system = AnimationAuthoringSystem::default();
    let mut jobs = somnium_core::jobs::JobSystem::single_threaded();
    let events = system.update(&mut world, None, &mut jobs, 1.0);
    assert_eq!(events.len(), 1);
    let settings = world.get::<AnimationAuthoring>(rig).unwrap();
    assert_eq!(settings.preview_time, 1.0);
    assert_eq!(settings.last_event, "footstep.left");
    assert!(settings.compression_report.contains("keys"));
    let previous = system.pose(rig).unwrap().clone();
    world.get_mut::<AnimationAuthoring>(rig).unwrap().pose_jobs = false;
    system.update(&mut world, None, &mut jobs, 0.0);
    assert_eq!(system.pose(rig).unwrap(), &previous);
    world.get_mut::<AnimationAuthoring>(rig).unwrap().ik_weight = 1.0;
    system.update(&mut world, None, &mut jobs, 0.0);
    let posed = system.pose(rig).unwrap();
    let matrices = posed
        .local
        .iter()
        .fold(Vec::new(), |mut out: Vec<Mat4>, t| {
            out.push(out.last().copied().unwrap_or(Mat4::IDENTITY) * t.to_matrix());
            out
        });
    assert!(
        matrices[2]
            .w_axis
            .truncate()
            .abs_diff_eq(Vec3::new(1.2, 1.2, 0.0), 1e-4)
    );
    assert_eq!(
        world.get::<AnimationAuthoring>(rig).unwrap().preview_time,
        1.0,
        "IK edits do not reset playback"
    );
    system.reset(&mut world, None, rig);
    assert_eq!(world.get::<AnimationAuthoring>(rig).unwrap().event_count, 0);
    assert_eq!(
        world.get::<AnimationAuthoring>(rig).unwrap().preview_time,
        0.0
    );
}

#[test]
fn animation_details_fields_validate_and_survive_scene_roundtrip() {
    let mut world = World::new();
    let rig = create_preview_rig(&mut world, Vec3::Y);
    world
        .get_mut::<AnimationAuthoring>(rig)
        .unwrap()
        .rotation_error = 0.02;
    let registry = somnium_core::reflect_registry::component_registry();
    let schema = registry.by_name("somnium.AnimationAuthoring").unwrap();
    assert!(
        schema
            .field_by_name("rotation_error")
            .unwrap()
            .validate(&somnium_ecs::ReflectValue::F64(-1.0))
            .is_err()
    );
    let source = somnium_core::scene_schema::scene_to_json(&mut world, &registry);
    let mut loaded = World::new();
    somnium_core::scene_schema::scene_from_json(&mut loaded, &registry, &source).unwrap();
    assert_eq!(
        loaded
            .entities()
            .filter(|e| {
                loaded.get::<somnium_core::MeshKind>(*e).is_some()
                    && loaded.get::<somnium_core::MaterialComponent>(*e).is_some()
            })
            .count(),
        5,
        "Scene reconstruction must retain the preview's material bindings"
    );
    let settings = loaded
        .entities()
        .find_map(|e| loaded.get::<AnimationAuthoring>(e))
        .unwrap();
    assert_eq!(settings.rotation_error, 0.02);
    assert!(loaded.is_alive(settings.reach_target));
    assert!(loaded.is_alive(settings.look_target));
    let mut system = AnimationAuthoringSystem::default();
    let mut jobs = somnium_core::jobs::JobSystem::single_threaded();
    system.update(&mut loaded, None, &mut jobs, 0.1);
    assert!(
        loaded
            .entities()
            .find_map(|e| loaded.get::<AnimationAuthoring>(e))
            .unwrap()
            .status
            .contains("Playing")
    );
}

#[test]
fn repairing_invalid_preview_resumes_at_last_accepted_time() {
    let mut world = World::new();
    let rig = create_preview_rig(&mut world, Vec3::ZERO);
    let mut system = AnimationAuthoringSystem::default();
    let mut jobs = somnium_core::jobs::JobSystem::single_threaded();
    world.get_mut::<AnimationAuthoring>(rig).unwrap().pose_jobs = false;
    system.update(&mut world, None, &mut jobs, 1.0);
    let settings = world.get_mut::<AnimationAuthoring>(rig).unwrap();
    let target = settings.reach_target;
    settings.ik_weight = 1.0;
    settings.reach_target = somnium_ecs::Entity::DANGLING;
    system.update(&mut world, None, &mut jobs, 3_600.0);
    let settings = world.get_mut::<AnimationAuthoring>(rig).unwrap();
    assert_eq!(settings.preview_time, 1.0);
    assert_eq!(settings.status, "Choose a live Reach Target");
    settings.reach_target = target;
    system.update(&mut world, None, &mut jobs, 0.1);
    let settings = world.get::<AnimationAuthoring>(rig).unwrap();
    assert_eq!(settings.status, "Playing");
    assert!((settings.preview_time - 1.1).abs() < 1e-5);
    assert_eq!(
        settings.event_count, 1,
        "Invalid time does not replay events"
    );
}
