//! A second game consuming O/P/P2/W/W2/X/Y/AF through public interfaces only.
//! The engine owns rendering, navigation and its scheduler. Save/rebase
//! evidence uses a separate World and a fresh temporary slot.

use glam::{Mat4, Quat, Vec2, Vec3};
use somnium_core::ai::{
    NavigationAgent, NavigationEditor, NavigationObstacle, NavigationProfile, PerceptionComponent,
};
use somnium_core::blockout::BlockoutComponent;
use somnium_core::prefab::{self, PrefabLibrary, PrefabTemplate};
use somnium_core::{Entity, Name, PersistentId, Transform, World, WorldTransform};
use somnium_jobs::JobSystem;
use std::collections::BTreeMap;

pub struct Slice {
    pub profile: Entity,
    pub actor: Entity,
    pub rig: Entity,
    pub scattered: usize,
}

/// Create actual scene intent, compile an authored scatter graph, enqueue a
/// bounded nav bake on the supplied scheduler, and exercise save rebasing.
pub fn exercise(
    world: &mut World,
    jobs: &mut JobSystem,
    navigation: &mut NavigationEditor,
) -> Result<Slice, String> {
    let registry = somnium_core::reflect_registry::component_registry();
    world.spawn((
        Name::new("Morrowind Courtyard"),
        Transform::from_translation(Vec3::new(0.0, -0.1, 0.0)),
        WorldTransform(Mat4::from_translation(Vec3::new(0.0, -0.1, 0.0))),
        BlockoutComponent {
            size: Vec3::new(12.0, 0.2, 12.0),
            ..Default::default()
        },
    ));
    let source = world.spawn((
        Name::new("Blockout Prefab Source"),
        Transform::from_translation(Vec3::new(0.0, 0.5, 4.5)),
        BlockoutComponent {
            shape: 1,
            size: Vec3::ONE,
            ..Default::default()
        },
    ));
    let template = PrefabTemplate::capture(world, &registry, &[source])?;
    let asset =
        somnium_asset::database::AssetId::from_relative_path("vvardenfell/courtyard.somprefab");
    let library = PrefabLibrary::from([(asset, template)]);
    for x in [-3.0, 0.0, 3.0] {
        let instance = prefab::instantiate(
            world,
            &registry,
            &library,
            asset,
            "vvardenfell/courtyard.somprefab",
        )?;
        world
            .get_mut::<Transform>(instance)
            .ok_or("Prefab has no transform")?
            .translation = Vec3::new(x, 0.5, 4.5);
    }
    let spline = somnium_core::SplineComponent::straight(4, 2.0);
    if (spline.arc_length(Mat4::IDENTITY).length() - 6.0).abs() > 1e-4 {
        return Err("Spline lost metre parameterisation".into());
    }
    world.spawn((
        Name::new("Patrol Guide"),
        Transform::from_translation(Vec3::new(0.0, 0.1, -4.0)),
        spline,
    ));

    let surface = somnium_ui::graph::scatter::default_surface();
    let rule = somnium_ui::graph::scatter::compile(&surface.graph)?;
    let instances = rule.scatter(
        Vec2::new(-5.0, 7.0),
        Vec2::new(5.0, 11.0),
        &BTreeMap::new(),
        |p| {
            Some(somnium_asset::scatter::SurfacePoint {
                position: Vec3::new(p.x, 0.25, p.y),
                normal: Vec3::Y,
                tags: BTreeMap::from([("ground".into(), 1.0)]),
            })
        },
    )?;
    let scattered = instances.len();
    for instance in instances {
        let transform = Transform {
            translation: instance.position,
            rotation: Quat::from_rotation_y(instance.yaw),
            scale: Vec3::splat(instance.scale),
        };
        world.spawn((
            Name::new("Scattered Stone"),
            transform,
            WorldTransform(transform.to_matrix()),
            BlockoutComponent {
                shape: 3,
                size: Vec3::splat(0.5),
                segments: 8,
            },
        ));
    }
    world.spawn((
        Name::new("Scatter Settings"),
        somnium_core::scatter_scene::ScatterSettings {
            width: 10.0,
            depth: 6.0,
            offset: Vec2::ZERO,
        },
    ));
    let rig =
        somnium_core::animation_authoring::create_preview_rig(world, Vec3::new(4.5, 0.0, -4.5));
    if let Some(settings) =
        world.get_mut::<somnium_core::animation_authoring::AnimationAuthoring>(rig)
    {
        settings.ik_weight = 0.6;
        settings.look_weight = 0.4;
        settings.compression = true;
        settings.pose_jobs = true;
    }
    exercise_animation_math()?;

    let profile = world.spawn((
        Name::new("Courtyard Navigation"),
        Transform::from_translation(Vec3::Y),
        NavigationProfile {
            bounds_size: Vec3::new(8.0, 6.0, 8.0),
            tile_size: 8.0,
            voxel_size: 0.5,
            ..Default::default()
        },
    ));
    let actor_transform = Transform::from_translation(Vec3::new(-3.0, 0.1, -3.0));
    let actor = world.spawn((
        Name::new("Courtyard Patrol"),
        actor_transform,
        WorldTransform(actor_transform.to_matrix()),
        BlockoutComponent {
            shape: 3,
            size: Vec3::new(0.4, 1.0, 0.4),
            segments: 8,
        },
        NavigationAgent {
            destination: Vec3::new(3.0, 0.0, 3.0),
            speed: 1.5,
            ..Default::default()
        },
        PerceptionComponent {
            target: source,
            target_loudness: 1.0,
            ..Default::default()
        },
    ));
    world.spawn((
        Name::new("Navigation Crate"),
        Transform::from_translation(Vec3::new(0.0, 0.5, 0.0)),
        BlockoutComponent {
            size: Vec3::ONE,
            ..Default::default()
        },
        NavigationObstacle {
            size: Vec3::ONE,
            ..Default::default()
        },
    ));
    let behavior = somnium_ui::graph::behavior::default_surface();
    let document = somnium_ui::graph::serial::to_json(&behavior.graph, &behavior.catalogue)
        .map_err(|e| e.to_string())?;
    somnium_core::ai::attach_behavior(world, actor, document)?;
    somnium_core::propagate_transforms(world);
    let (geometry, _) = somnium_core::ai::collect_geometry(
        world,
        None,
        somnium_ai::navigation::Bounds {
            min: [-4.0, -2.0, -4.0],
            max: [4.0, 4.0, 4.0],
        },
    )?;
    navigation.bake_geometry(world, jobs, profile, geometry)?;
    exercise_save_rebase()?;
    println!(
        "  MORROWIND public slice -> 3 prefabs, arc-length spline, {scattered} scatter instances, procedural rig, nav bake + sensing + behavior, save rebase"
    );
    Ok(Slice {
        profile,
        actor,
        rig,
        scattered,
    })
}

fn exercise_animation_math() -> Result<(), String> {
    use somnium_anim::{
        AnimationClip, ClipId, CompressionBudget, Keyframe, NO_PARENT, Playback, Skeleton,
        SkeletonId, TransformTrack,
    };
    let skeleton = Skeleton::new(
        SkeletonId(404),
        vec!["root".into()],
        vec![NO_PARENT],
        vec![Mat4::IDENTITY],
        vec![somnium_anim::Transform::IDENTITY],
    )
    .ok_or("Invalid acceptance skeleton")?
    .0;
    let clip = AnimationClip::new(
        ClipId(404),
        &skeleton,
        1.0,
        vec![TransformTrack {
            joint: 0,
            translation: vec![
                Keyframe::new(0.0, Vec3::ZERO),
                Keyframe::new(0.5, Vec3::X * 0.5),
                Keyframe::new(1.0, Vec3::X),
            ],
            rotation: vec![],
            scale: vec![],
        }],
        vec![],
    )
    .map_err(|e| format!("{e:?}"))?;
    let (clip, report) = clip
        .compress(
            &skeleton,
            CompressionBudget {
                translation: 0.001,
                rotation: 0.001,
                scale: 0.001,
            },
        )
        .map_err(|e| format!("{e:?}"))?;
    if report.retained_keys >= report.source_keys {
        return Err("Linear clip did not reduce redundant keys".into());
    }
    let (_, motion) = clip
        .sample_root_motion(&skeleton, 0, 0.0, 0.5, Playback::LOOPING)
        .map_err(|e| format!("{e:?}"))?;
    let mut transform = Transform::default();
    somnium_core::animation_motion::apply_root_motion(&mut transform, motion, 0.01, |_, _| None)
        .map_err(|e| format!("{e:?}"))?;
    if transform.translation.distance(Vec3::X * 0.5) > 1e-5 {
        return Err("Root displacement changed during compression/application".into());
    }
    Ok(())
}

fn exercise_save_rebase() -> Result<(), String> {
    use somnium_core::{
        save_game::{SaveGame, SaveSlots, SceneDelta},
        scene_schema::{scene_from_json, scene_to_json},
    };
    let registry = somnium_core::reflect_registry::component_registry();
    let mut isolated = World::new();
    let entity = isolated.spawn((
        Name::new("Closed chest"),
        Transform::default(),
        BlockoutComponent::default(),
    ));
    let id = isolated
        .ensure_persistent_id(entity)
        .map_err(|e| e.to_string())?;
    let baseline = scene_to_json(&mut isolated, &registry);
    *isolated
        .get_mut::<Name>(entity)
        .ok_or("Missing chest name")? = Name::new("Opened chest");
    let played = scene_to_json(&mut isolated, &registry);
    let mut save = SaveGame::new(1);
    save.global = SceneDelta::between(&baseline, &played)?;
    *isolated
        .get_mut::<Name>(entity)
        .ok_or("Missing chest name")? = Name::new("Closed chest");
    isolated
        .get_mut::<Transform>(entity)
        .ok_or("Missing chest transform")?
        .translation
        .x = 9.0;
    let authored_patch = scene_to_json(&mut isolated, &registry);
    let directory =
        std::env::temp_dir().join(format!("somnium-vvardenfell-{}", PersistentId::mint()));
    let slots = SaveSlots::new(&directory);
    slots.write("acceptance", &save)?;
    let loaded = slots.read("acceptance")?;
    std::fs::remove_file(directory.join("acceptance.somsave")).map_err(|e| e.to_string())?;
    std::fs::remove_dir(&directory).map_err(|e| e.to_string())?;
    let rebased = loaded.global.rebase(&authored_patch)?;
    let mut restored = World::new();
    scene_from_json(&mut restored, &registry, &rebased.scene).map_err(|e| e.to_string())?;
    let entity = restored
        .entity_by_persistent_id(id)
        .ok_or("Save lost durable identity")?;
    if restored.get::<Name>(entity).unwrap().as_str() != "Opened chest"
        || restored.get::<Transform>(entity).unwrap().translation.x != 9.0
    {
        return Err("Save rebase did not retain both player and author changes".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_eight_subphases_are_consumed_through_public_game_interfaces() {
        let mut world = World::new();
        let mut jobs = JobSystem::single_threaded();
        let mut navigation = NavigationEditor::default();
        let slice = exercise(&mut world, &mut jobs, &mut navigation).unwrap();
        navigation.update_runtime(&mut world, &mut jobs, 1.0 / 60.0, true);
        assert!(slice.scattered > 0);
        assert!(
            world
                .get::<NavigationProfile>(slice.profile)
                .unwrap()
                .polygon_count
                > 0
        );
        assert!(
            world
                .get::<somnium_core::ai::BehaviorComponent>(slice.actor)
                .is_some()
        );
        let mut animation = somnium_core::animation_authoring::AnimationAuthoringSystem::default();
        animation.update(&mut world, None, &mut jobs, 1.0 / 60.0);
        assert!(animation.pose(slice.rig).is_some());
        assert!(
            !world
                .get::<somnium_core::animation_authoring::AnimationAuthoring>(slice.rig)
                .unwrap()
                .compression_report
                .is_empty()
        );
    }
}
