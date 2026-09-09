use glam::{Mat4, Quat, Vec3};
use somnium_physics::{
    animation::{AnimationPhysicsError, CapsuleSweep, RagdollPart},
    body::RigidBodyDescriptor,
    config::PhysicsConfig,
    shape::ColliderShape,
    world::PhysicsWorld,
};

// One native world in this test: the existing bridge's process-global Jolt
// initialization does not support concurrently constructing multiple worlds.
#[test]
fn capsule_sweeps_and_constrained_ragdoll_lifecycle_use_real_jolt() {
    let mut physics = PhysicsWorld::new(PhysicsConfig::default());
    physics.create_body(RigidBodyDescriptor {
        shape: ColliderShape::Box {
            half_extents: Vec3::new(0.1, 3.0, 3.0),
        },
        position: Vec3::new(2.0, 0.0, 0.0),
        ..Default::default()
    });
    let hit = physics
        .cast_capsule(CapsuleSweep {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            displacement: Vec3::X * 4.0,
            half_height: 0.5,
            radius: 0.25,
            ignore_body: None,
        })
        .unwrap()
        .unwrap();
    assert!(hit.fraction > 0.39 && hit.fraction < 0.45, "{hit:?}");
    assert!(hit.normal.dot(-Vec3::X) > 0.99);
    let count = physics.num_bodies();
    let parent = RagdollPart {
        parent: None,
        position: Vec3::new(0.0, 5.0, 0.0),
        rotation: Quat::IDENTITY,
        half_height: 0.3,
        radius: 0.15,
        anchor: Vec3::new(0.0, 4.5, 0.0),
        swing_limit: 0.5,
        twist_limit: 0.3,
    };
    let child = RagdollPart {
        parent: Some(0),
        position: Vec3::new(0.0, 4.0, 0.0),
        ..parent
    };
    let ragdoll = physics.create_ragdoll(&[parent, child]).unwrap();
    assert_eq!(physics.num_bodies(), count + 2);
    let start = physics.ragdoll_pose(ragdoll).unwrap();
    let mut disturbed = start.clone();
    disturbed[1].w_axis.x += 0.3;
    physics.set_ragdoll_pose(ragdoll, &disturbed).unwrap();
    for _ in 0..60 {
        physics.step(1.0 / 60.0);
    }
    let fallen = physics.ragdoll_pose(ragdoll).unwrap();
    assert!(fallen[0].w_axis.y < start[0].w_axis.y - 1.0);
    let a = fallen[0].transform_point3(-Vec3::Y * 0.5);
    let b = fallen[1].transform_point3(Vec3::Y * 0.5);
    assert!(
        a.distance(b) < 0.1,
        "parent-child constraint separated by {}",
        a.distance(b)
    );
    assert!(physics.set_ragdoll_pose(ragdoll, &[Mat4::ZERO; 2]).is_err());
    physics.set_ragdoll_pose(ragdoll, &start).unwrap();
    for (a, b) in physics.ragdoll_pose(ragdoll).unwrap().iter().zip(start) {
        assert!(a.abs_diff_eq(b, 1e-4));
    }
    physics.destroy_ragdoll(ragdoll).unwrap();
    assert_eq!(physics.num_bodies(), count);
    assert_eq!(
        physics.ragdoll_pose(ragdoll),
        Err(AnimationPhysicsError::UnknownRagdoll)
    );
}
