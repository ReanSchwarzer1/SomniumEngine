//! Phase 17B: the terrain heightfield collider, exercised end to end.
//!
//! These run against a real Jolt physics world rather than mocking it. The
//! whole point of the shape is that bodies land on the terrain instead of
//! falling through it, and only a stepped simulation can show that.
//!
//! Everything lives in one test because Jolt's global factory and type registry
//! are process-wide, and `cargo test` runs test functions on multiple threads.

use glam::Vec3;
use somnium_physics::body::{MotionType, RigidBodyDescriptor};
use somnium_physics::config::PhysicsConfig;
use somnium_physics::layer::{LAYER_MOVING, LAYER_NON_MOVING};
use somnium_physics::shape::ColliderShape;
use somnium_physics::world::PhysicsWorld;

/// Flat field at `height`, `n` samples per side.
fn flat_field(n: u32, height: f32) -> Vec<f32> {
    vec![height; (n * n) as usize]
}

/// A ramp climbing along +X, so a body should slide or rest at a known height.
fn ramp_field(n: u32, rise_per_sample: f32) -> Vec<f32> {
    let mut v = Vec::with_capacity((n * n) as usize);
    for _z in 0..n {
        for x in 0..n {
            v.push(x as f32 * rise_per_sample);
        }
    }
    v
}

fn drop_sphere(world: &mut PhysicsWorld, from: Vec3) -> somnium_physics::body::BodyId {
    world.create_body(RigidBodyDescriptor {
        shape: ColliderShape::Sphere { radius: 0.5 },
        position: from,
        motion_type: MotionType::Dynamic,
        object_layer: LAYER_MOVING,
        ..Default::default()
    })
}

#[test]
fn terrain_heightfield_collider() {
    let mut world = PhysicsWorld::new(PhysicsConfig::default());

    // ── A body dropped onto a flat field comes to rest on it ────────────────
    const N: u32 = 32;
    let terrain = world.create_body(RigidBodyDescriptor {
        shape: ColliderShape::HeightField {
            samples: flat_field(N, 0.0),
            sample_count: N,
            // 1 m spacing, so the field spans 31 m from the origin.
            scale: Vec3::new(1.0, 1.0, 1.0),
        },
        // Centre the field roughly on the origin.
        position: Vec3::new(-15.0, 0.0, -15.0),
        motion_type: MotionType::Static,
        object_layer: LAYER_NON_MOVING,
        ..Default::default()
    });
    assert!(terrain.is_valid(), "the heightfield shape failed to build");

    let ball = drop_sphere(&mut world, Vec3::new(0.0, 8.0, 0.0));
    for _ in 0..300 {
        world.step(1.0 / 60.0);
    }
    let resting = world.get_position(ball);
    assert!(
        resting.y > 0.2,
        "the body fell through the terrain, ending at y = {}",
        resting.y,
    );
    assert!(
        resting.y < 1.5,
        "the body never settled onto the surface, ending at y = {}",
        resting.y,
    );

    // ── The field follows its height data, not just a flat plane ────────────
    let mut sloped = PhysicsWorld::new(PhysicsConfig::default());
    sloped.create_body(RigidBodyDescriptor {
        shape: ColliderShape::HeightField {
            // Rises 0.5 m per sample, so x = +10 m sits 5 m up.
            samples: ramp_field(N, 0.5),
            sample_count: N,
            scale: Vec3::new(1.0, 1.0, 1.0),
        },
        position: Vec3::new(-15.0, 0.0, -15.0),
        motion_type: MotionType::Static,
        object_layer: LAYER_NON_MOVING,
        ..Default::default()
    });
    // Drop onto the high end. Ground there is ~12.5 m up (25 samples in).
    let high = drop_sphere(&mut sloped, Vec3::new(10.0, 25.0, 0.0));
    for _ in 0..240 {
        sloped.step(1.0 / 60.0);
    }
    let hit = sloped.get_position(high);
    assert!(
        hit.y > 5.0,
        "a body on the high end of the ramp ended at y = {}, so the height \
         samples are being ignored",
        hit.y,
    );

    // ── A malformed field is refused instead of corrupting the world ────────
    let mut bad = PhysicsWorld::new(PhysicsConfig::default());
    // Not a power of two: Jolt asserts on this, so the bridge rejects it first.
    let odd = bad.create_body(RigidBodyDescriptor {
        shape: ColliderShape::HeightField {
            samples: flat_field(30, 0.0),
            sample_count: 30,
            scale: Vec3::ONE,
        },
        motion_type: MotionType::Static,
        object_layer: LAYER_NON_MOVING,
        ..Default::default()
    });
    assert!(!odd.is_valid(), "a non-power-of-two field should be refused");

    // Too few samples for the declared side length: reading it would run off
    // the end of the slice.
    let short = bad.create_body(RigidBodyDescriptor {
        shape: ColliderShape::HeightField {
            samples: vec![0.0; 10],
            sample_count: 32,
            scale: Vec3::ONE,
        },
        motion_type: MotionType::Static,
        object_layer: LAYER_NON_MOVING,
        ..Default::default()
    });
    assert!(!short.is_valid(), "a short sample buffer should be refused");
}
