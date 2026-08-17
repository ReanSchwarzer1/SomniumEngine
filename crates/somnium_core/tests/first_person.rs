//! The scripted first-person character, driven without a window.
//!
//! These run **the same two `.luau` files `hello_engine` attaches**, on a
//! real Jolt world, through the real `ScriptHost`. Pressing Play and
//! walking around proves it renders; this proves it is correct, on every
//! commit, and it is what catches a change to the mirror or to the physics
//! sync that would otherwise only show up as "the character feels wrong".

use somnium_core::character::{read_physics_into_world, write_world_into_physics};
use somnium_core::script_host::{HostServices, ScriptHost};
use somnium_core::{Name, RigidBodyComponent, Transform, WorldTransform, propagate_transforms};
use somnium_ecs::{Entity, World};
use somnium_physics::body::{MotionType, RigidBodyDescriptor};
use somnium_physics::config::PhysicsConfig;
use somnium_physics::layer::{LAYER_MOVING, LAYER_NON_MOVING};
use somnium_physics::shape::ColliderShape;
use somnium_physics::world::PhysicsWorld;
use somnium_script::attachment::{ScriptAttachment, ScriptSet};
use somnium_script::backend::Budget;
use somnium_script::ids::{InstanceUuid, ScriptAssetId};
use somnium_script::runtime::PhaseInput;
use somnium_script::snapshot::{InputSnapshot, TimeSnapshot};
use somnium_script::value::ScriptValue;

const CONTROLLER: &str = include_str!("../../../assets/scripts/first_person_controller.luau");
const CAMERA: &str = include_str!("../../../assets/scripts/first_person_camera.luau");

/// Engine key numbers, as `script_input` defines them.
const KEY_W: u32 = b'W' as u32;
const KEY_A: u32 = b'A' as u32;
const KEY_SPACE: u32 = 32;
const KEY_SHIFT: u32 = 264;

/// Serialises Jolt world creation across the cases in this binary.
///
/// `cargo test` runs the cases in one binary on several threads, and
/// Jolt's global initialiser is not safe to call concurrently — the
/// symptom is the whole binary dying with no failing test named, which is
/// a miserable thing to debug. Held for the rig's whole life rather than
/// just the constructor, because the body interface is shared too.
///
/// A guard rather than `--test-threads=1`: a flag only helps whoever
/// remembers to pass it, and CI does not.
static JOLT: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A player, its camera child, a floor, and the loop that drives them.
struct Rig {
    /// Poisoning is not meaningful here — the mutex guards a C++ global,
    /// not data — so a panicking case must not take every later one with
    /// it.
    _jolt: std::sync::MutexGuard<'static, ()>,
    host: ScriptHost,
    world: World,
    physics: PhysicsWorld,
    player: Entity,
    camera: Entity,
    controller: InstanceUuid,
    camera_instance: InstanceUuid,
    step: u64,
}

impl Rig {
    fn new() -> Self {
        let jolt = JOLT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut host = ScriptHost::new(Budget::default());
        // The cache is keyed on a path that does not exist for these
        // in-memory sources; keeping it off makes the test independent of
        // whatever is in `target/`.
        host.set_bytecode_cache(false);

        let controller_asset = ScriptAssetId::from_path("assets/scripts/first_person_controller.luau");
        host.load_script(controller_asset, "first_person_controller.luau", CONTROLLER)
            .unwrap_or_else(|d| panic!("the controller must compile:\n{d}"));
        let camera_asset = ScriptAssetId::from_path("assets/scripts/first_person_camera.luau");
        host.load_script(camera_asset, "first_person_camera.luau", CAMERA)
            .unwrap_or_else(|d| panic!("the camera must compile:\n{d}"));

        let mut physics = PhysicsWorld::new(PhysicsConfig::default());
        // A wide floor at y = 0, so "grounded" and "jump" mean something.
        physics.create_body(RigidBodyDescriptor {
            shape: ColliderShape::Box {
                half_extents: glam::Vec3::new(60.0, 0.5, 60.0),
            },
            position: glam::Vec3::new(0.0, -0.5, 0.0),
            motion_type: MotionType::Static,
            object_layer: LAYER_NON_MOVING,
            ..Default::default()
        });

        // Capsule centre at 0.9 puts the feet exactly on the floor.
        let centre = glam::Vec3::new(0.0, 0.9, 0.0);
        let body = physics.create_body(RigidBodyDescriptor {
            shape: ColliderShape::Capsule {
                half_height: 0.6,
                radius: 0.3,
            },
            position: centre,
            motion_type: MotionType::Dynamic,
            object_layer: LAYER_MOVING,
            friction: 0.0,
            restitution: 0.0,
            linear_damping: 0.0,
            angular_damping: 1.0,
            allow_sleeping: false,
            ..Default::default()
        });

        let mut world = World::new();
        let mut player_scripts = ScriptSet::new();
        let controller = ScriptAttachment::new(controller_asset);
        let controller_id = controller.instance;
        player_scripts.attach(controller);
        let player = world.spawn((
            Transform::from_translation(centre),
            WorldTransform::identity(),
            Name::new("Player"),
            RigidBodyComponent::driven(body),
            player_scripts,
            somnium_core::Children::empty(),
        ));

        let mut camera_scripts = ScriptSet::new();
        let camera_attachment = ScriptAttachment::new(camera_asset);
        let camera_instance = camera_attachment.instance;
        camera_scripts.attach(camera_attachment);
        let camera = world.spawn((
            Transform::from_translation(glam::Vec3::Y * 0.72),
            WorldTransform::identity(),
            Name::new("PlayerCamera"),
            somnium_core::Parent { entity: player },
            camera_scripts,
        ));
        world
            .get_mut::<somnium_core::Children>(player)
            .unwrap()
            .push(camera);

        Self {
            _jolt: jolt,
            host,
            world,
            physics,
            player,
            camera,
            controller: controller_id,
            camera_instance,
            step: 0,
        }
    }

    /// One fixed step, in exactly the order `app.rs` runs it.
    fn step(&mut self, input: &InputSnapshot) {
        let time = TimeSnapshot {
            fixed_delta: 1.0 / 60.0,
            delta: 1.0 / 60.0,
            #[allow(clippy::cast_precision_loss)]
            simulation_time: self.step as f64 / 60.0,
            step: self.step,
        };
        let phase = PhaseInput {
            time,
            input: input.clone(),
        };
        // Each borrow of `physics` is scoped, because the sync either side
        // of the script phase needs it back.
        {
            let mut services = HostServices {
                physics: Some(&mut self.physics),
                audio: None,
            };
            self.host.sync(&mut self.world, &phase, &mut services);
        }

        read_physics_into_world(&mut self.world, &self.physics);
        {
            let mut services = HostServices {
                physics: Some(&mut self.physics),
                audio: None,
            };
            self.host
                .fixed_update(&mut self.world, time, input, &mut services);
        }
        write_world_into_physics(&self.world, &mut self.physics);
        self.physics.step(1.0 / 60.0);
        propagate_transforms(&mut self.world);
        self.step += 1;
    }

    fn run(&mut self, steps: u32, input: &InputSnapshot) {
        for _ in 0..steps {
            self.step(input);
        }
    }

    fn position(&self) -> glam::Vec3 {
        self.world.get::<Transform>(self.player).unwrap().translation
    }

    fn eye(&self) -> glam::Vec3 {
        self.world
            .get::<WorldTransform>(self.camera)
            .map_or(glam::Vec3::ZERO, |wt| wt.0.w_axis.truncate())
    }

    fn state(&mut self, instance: InstanceUuid, key: &str) -> f64 {
        let Ok(ScriptValue::Map(fields)) = self.host.runtime_mut().export_state(instance) else {
            panic!("expected the script's saved state");
        };
        match fields.get(key) {
            Some(ScriptValue::F64(v)) => *v,
            #[allow(clippy::cast_precision_loss)]
            Some(ScriptValue::I64(v)) => *v as f64,
            other => panic!("`{key}` is {other:?}"),
        }
    }
}

fn keys(down: &[u32]) -> InputSnapshot {
    let mut sorted = down.to_vec();
    sorted.sort_unstable();
    InputSnapshot {
        keys_down: sorted,
        ..InputSnapshot::default()
    }
}

/// Keys held *and* reported as newly pressed this step — what the tracker
/// produces on the frame a key goes down.
fn press(down: &[u32]) -> InputSnapshot {
    let mut sorted = down.to_vec();
    sorted.sort_unstable();
    InputSnapshot {
        keys_down: sorted.clone(),
        keys_pressed: sorted,
        ..InputSnapshot::default()
    }
}

fn mouse(dx: f32, dy: f32) -> InputSnapshot {
    InputSnapshot {
        mouse_delta: [dx, dy],
        ..InputSnapshot::default()
    }
}

// ── The scripts themselves ─────────────────────────────────────────────

#[test]
fn both_character_scripts_compile_and_declare_their_fields() {
    let mut host = ScriptHost::new(Budget::default());
    host.set_bytecode_cache(false);

    let controller = ScriptAssetId::mint();
    host.load_script(controller, "controller.luau", CONTROLLER)
        .unwrap_or_else(|d| panic!("{d}"));
    let schema = host.runtime().asset_schema(controller).unwrap();
    let names: Vec<&str> = schema.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "airControl",
            "invertMouseX",
            "jumpSpeed",
            "mouseSensitivity",
            "runSpeed",
            "walkSpeed"
        ],
        "these are the rows the Details panel draws"
    );
    // The panel needs the bounds to clamp the drag, and the tooltip.
    let walk = schema.field("walkSpeed").unwrap();
    assert_eq!(walk.default, ScriptValue::F64(4.5));
    assert_eq!(walk.min, Some(0.0));
    assert!(walk.description.is_some());

    let camera = ScriptAssetId::mint();
    host.load_script(camera, "camera.luau", CAMERA)
        .unwrap_or_else(|d| panic!("{d}"));
    let schema = host.runtime().asset_schema(camera).unwrap();
    let names: Vec<&str> = schema.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["eyeHeight", "invertMouseY", "mouseSensitivity", "pitchLimit"]
    );
}

#[test]
fn the_controller_declares_only_the_fields_it_touches() {
    // Mirroring a whole component marshals every field in both directions
    // every frame. `dev`-time convenience, measured cost — the budget
    // record puts whole-Transform mirroring at 2.3 → 5.5 ms per thousand
    // entities.
    let mut host = ScriptHost::new(Budget::default());
    host.set_bytecode_cache(false);
    let asset = ScriptAssetId::mint();
    host.load_script(asset, "controller.luau", CONTROLLER).unwrap();
    let uses = &host.runtime().asset_schema(asset).unwrap().uses;

    for declared in uses {
        assert!(
            !declared.fields.is_empty(),
            "`{}` is mirrored whole; name the fields it actually uses",
            declared.component
        );
    }
    let names: Vec<&str> = uses.iter().map(|u| u.component.as_str()).collect();
    assert_eq!(names, vec!["somnium.RigidBody", "somnium.Transform"]);
}

// ── Walking ────────────────────────────────────────────────────────────

#[test]
fn w_walks_forward_and_releasing_it_stops_dead() {
    let mut rig = Rig::new();
    // Settle onto the floor first, so the first steps are not a fall.
    rig.run(10, &InputSnapshot::default());
    let start = rig.position();

    rig.run(60, &keys(&[KEY_W]));
    let walked = rig.position();
    let travelled = (walked - start).length();
    assert!(
        travelled > 3.5 && travelled < 5.5,
        "a second at 4.5 m/s should be about 4.5 m, got {travelled}"
    );
    // Default yaw is zero, and the script's forward at zero yaw is -Z.
    assert!(
        walked.z < start.z - 3.0,
        "forward is -Z at zero yaw: {start:?} → {walked:?}"
    );

    // Let go. A character sets its velocity outright rather than pushing,
    // which is what makes this stop instead of skate.
    rig.run(20, &InputSnapshot::default());
    let stopped = rig.position();
    assert!(
        (stopped - walked).length() < 0.15,
        "releasing the key must stop the character, not coast: moved {}",
        (stopped - walked).length()
    );
}

#[test]
fn shift_runs_and_it_is_measurably_faster_than_walking() {
    let mut rig = Rig::new();
    rig.run(10, &InputSnapshot::default());

    let before = rig.position();
    rig.run(30, &keys(&[KEY_W]));
    let walked = (rig.position() - before).length();

    let before = rig.position();
    rig.run(30, &keys(&[KEY_W, KEY_SHIFT]));
    let ran = (rig.position() - before).length();

    assert!(
        ran > walked * 1.5,
        "8 m/s against 4.5 should be clearly faster: walked {walked}, ran {ran}"
    );
}

#[test]
fn strafing_diagonally_is_not_faster_than_walking_straight() {
    // The classic bug: summing two unit vectors without normalising makes
    // diagonal movement 1.41× faster.
    let mut rig = Rig::new();
    rig.run(10, &InputSnapshot::default());

    let before = rig.position();
    rig.run(30, &keys(&[KEY_W]));
    let straight = (rig.position() - before).length();

    let before = rig.position();
    rig.run(30, &keys(&[KEY_W, KEY_A]));
    let diagonal = (rig.position() - before).length();

    assert!(
        (diagonal - straight).abs() < straight * 0.1,
        "diagonal {diagonal} should match straight {straight}"
    );
}

// ── Looking ────────────────────────────────────────────────────────────

#[test]
fn the_mouse_turns_the_player_and_walking_follows_the_new_facing() {
    let mut rig = Rig::new();
    rig.run(10, &InputSnapshot::default());

    // A quarter turn: 90° at 0.12°/pixel is 750 pixels.
    rig.step(&mouse(750.0, 0.0));
    let yaw = rig.state(rig.controller, "yaw");
    assert!(
        (yaw + 90.0).abs() < 1.0,
        "750 px at 0.12°/px should be about -90°, got {yaw}"
    );

    let before = rig.position();
    rig.run(30, &keys(&[KEY_W]));
    let moved = rig.position() - before;
    assert!(
        moved.x.abs() > moved.z.abs() * 3.0,
        "after a quarter turn, forward should be along X: {moved:?}"
    );
}

#[test]
fn pitch_is_clamped_so_the_view_never_flips() {
    let mut rig = Rig::new();
    rig.run(5, &InputSnapshot::default());

    // Far more than enough to pass vertical.
    rig.run(40, &mouse(0.0, -200.0));
    let pitch = rig.state(rig.camera_instance, "pitch");
    assert!(
        (pitch - 89.0).abs() < 0.001,
        "looking up must stop at the limit, got {pitch}"
    );

    rig.run(80, &mouse(0.0, 200.0));
    let pitch = rig.state(rig.camera_instance, "pitch");
    assert!(
        (pitch + 89.0).abs() < 0.001,
        "and looking down must stop too, got {pitch}"
    );
}

#[test]
fn the_camera_rides_at_eye_height_on_the_player() {
    let mut rig = Rig::new();
    rig.run(20, &keys(&[KEY_W]));

    let eye = rig.eye();
    let feet = rig.position();
    assert!(
        (eye.y - (feet.y + 0.72)).abs() < 0.02,
        "the eye should sit 0.72 m above the capsule centre: eye {eye:?}, body {feet:?}"
    );
    assert!(
        (eye.x - feet.x).abs() < 0.02 && (eye.z - feet.z).abs() < 0.02,
        "and it should track the player horizontally"
    );
}

// ── Jumping ────────────────────────────────────────────────────────────

#[test]
fn space_jumps_and_the_character_comes_back_down() {
    let mut rig = Rig::new();
    rig.run(15, &InputSnapshot::default());
    let ground = rig.position().y;

    // One press, then the key merely held — which is what the input
    // tracker reports for a key someone is resting a finger on.
    rig.step(&press(&[KEY_SPACE]));
    let mut peak = ground;
    for _ in 0..40 {
        rig.step(&keys(&[KEY_SPACE]));
        peak = peak.max(rig.position().y);
    }
    assert!(
        peak > ground + 0.6,
        "5.2 m/s upward should clear well over half a metre: peak {peak}, ground {ground}"
    );

    rig.run(90, &InputSnapshot::default());
    assert!(
        (rig.position().y - ground).abs() < 0.1,
        "and gravity should bring it back: {} vs {ground}",
        rig.position().y
    );
}

#[test]
fn the_cooldown_stops_a_free_second_jump_at_the_apex() {
    // `grounded` is a vertical-speed heuristic, so it reads true for a
    // few frames at the top of a jump, where vertical speed also passes
    // through zero. This is the adversarial input for that: Space
    // reported as *newly pressed* on every single step, which defeats the
    // edge trigger and leaves only the cooldown standing between the
    // player and an infinite staircase.
    let mut rig = Rig::new();
    rig.run(15, &InputSnapshot::default());
    let ground = rig.position().y;

    let mut peak = ground;
    for _ in 0..120 {
        rig.step(&press(&[KEY_SPACE]));
        peak = peak.max(rig.position().y);
    }
    assert!(
        peak < ground + 2.5,
        "holding jump must not climb: a single 5.2 m/s jump peaks near 1.4 m, \
         got {peak} above {ground}"
    );
}

// ── State ──────────────────────────────────────────────────────────────

#[test]
fn look_direction_survives_a_reload() {
    let mut rig = Rig::new();
    rig.run(5, &InputSnapshot::default());
    rig.step(&mouse(300.0, -100.0));

    let yaw = rig.state(rig.controller, "yaw");
    let pitch = rig.state(rig.camera_instance, "pitch");
    assert!(yaw.abs() > 1.0 && pitch.abs() > 1.0, "something to carry");

    let controller_asset = ScriptAssetId::from_path("assets/scripts/first_person_controller.luau");
    rig.host
        .reload_script(controller_asset, "first_person_controller.luau", CONTROLLER)
        .expect("the same source recompiles");
    // The reload leaves the instance in `Loaded`; the next sync replays
    // init/start/enable and `loadState` has already put the yaw back.
    rig.step(&InputSnapshot::default());

    assert!(
        (rig.state(rig.controller, "yaw") - yaw).abs() < 0.001,
        "an author editing the walk speed must not spin the player round"
    );
}
