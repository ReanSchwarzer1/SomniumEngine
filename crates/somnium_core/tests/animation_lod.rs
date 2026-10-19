//! Animation LOD on real rigs: a near rig every frame, a far one about ten
//! times a second (with no time lost), a culled one held after its first pose,
//! and a rig without the component untouched.
use glam::Vec3;
use somnium_core::World;
use somnium_core::animation_authoring::{AnimationAuthoring, AnimationAuthoringSystem, create_preview_rig};
use somnium_core::animation_lod::AnimationLod;

fn rig(world: &mut World, at: Vec3, lod: bool) -> somnium_core::Entity {
    let rig = create_preview_rig(world, at);
    world.get_mut::<AnimationAuthoring>(rig).unwrap().pose_jobs = false;
    if lod {
        world.insert_component(rig, AnimationLod::default()).unwrap();
    }
    rig
}

#[test]
fn far_rigs_are_evaluated_less_often_and_keep_time() {
    let mut world = World::new();
    let near = rig(&mut world, Vec3::new(5.0, 0.0, 0.0), true);
    let far = rig(&mut world, Vec3::new(100.0, 0.0, 0.0), true);
    let culled = rig(&mut world, Vec3::new(400.0, 0.0, 0.0), true);
    let plain = rig(&mut world, Vec3::new(400.0, 0.0, 10.0), false);
    let mut system = AnimationAuthoringSystem::default();
    system.set_viewer(Some(Vec3::ZERO));
    let mut jobs = somnium_core::jobs::JobSystem::single_threaded();
    let time = |w: &World, e| w.get::<AnimationAuthoring>(e).unwrap().preview_time;
    let mut changes = [0usize; 4];
    let mut last = [0.0f32; 4];
    // A first frame builds every rig, whatever its distance.
    system.update(&mut world, None, &mut jobs, 1.0 / 60.0);
    let culled_first = time(&world, culled);
    for _ in 0..120 {
        system.update(&mut world, None, &mut jobs, 1.0 / 60.0);
        for (i, e) in [near, far, culled, plain].into_iter().enumerate() {
            let t = time(&world, e);
            if t != last[i] {
                changes[i] += 1;
                last[i] = t;
            }
        }
    }
    assert_eq!(changes[0], 120, "near: every frame");
    assert!((18..=22).contains(&changes[1]), "far: ten a second, {}", changes[1]);
    assert_eq!(time(&world, culled), culled_first, "culled: held");
    assert_eq!(changes[3], 120, "without the component: every frame, however far");
    let (near_t, far_t) = (time(&world, near), time(&world, far));
    assert!((near_t - far_t).abs() < 0.11, "no time lost: {near_t} vs {far_t}");
}

#[test]
fn without_a_viewer_every_rig_runs_every_frame() {
    let mut world = World::new();
    let far = rig(&mut world, Vec3::new(400.0, 0.0, 0.0), true);
    let mut system = AnimationAuthoringSystem::default();
    let mut jobs = somnium_core::jobs::JobSystem::single_threaded();
    let mut seen = Vec::new();
    for _ in 0..10 {
        system.update(&mut world, None, &mut jobs, 0.1);
        seen.push(world.get::<AnimationAuthoring>(far).unwrap().preview_time);
    }
    seen.dedup();
    assert_eq!(seen.len(), 10);
}
