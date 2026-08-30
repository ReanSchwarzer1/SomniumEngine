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
use somnium_script::snapshot::{InputActionSnapshot, InputSnapshot, TimeSnapshot};
use somnium_script::value::ScriptValue;

const CONTROLLER: &str = include_str!("../../../assets/scripts/first_person_controller.luau");
const CAMERA: &str = include_str!("../../../assets/scripts/first_person_camera.luau");

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
        Self::on_slope(0.0)
    }

    /// The same rig with the floor tilted `slope_deg` about X.
    ///
    /// A hill is not a decoration here: a body walking one has a
    /// legitimate vertical speed, which is the input that the original
    /// `grounded` heuristic could not tell from falling. Every character
    /// test that only ever ran on flat ground agreed with it.
    fn on_slope(slope_deg: f32) -> Self {
        let jolt = JOLT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut host = ScriptHost::new(Budget::default());
        // The cache is keyed on a path that does not exist for these
        // in-memory sources; keeping it off makes the test independent of
        // whatever is in `target/`.
        host.set_bytecode_cache(false);

        let controller_asset =
            ScriptAssetId::from_path("assets/scripts/first_person_controller.luau");
        host.load_script(controller_asset, "first_person_controller.luau", CONTROLLER)
            .unwrap_or_else(|d| panic!("the controller must compile:\n{d}"));
        let camera_asset = ScriptAssetId::from_path("assets/scripts/first_person_camera.luau");
        host.load_script(camera_asset, "first_person_camera.luau", CAMERA)
            .unwrap_or_else(|d| panic!("the camera must compile:\n{d}"));

        let mut physics = PhysicsWorld::new(PhysicsConfig::default());
        // A wide floor at y = 0, so "grounded" and "jump" mean something.
        physics.create_body(RigidBodyDescriptor {
            shape: ColliderShape::Box {
                half_extents: glam::Vec3::new(200.0, 0.5, 200.0),
            },
            position: glam::Vec3::new(0.0, -0.5, 0.0),
            rotation: glam::Quat::from_rotation_x(slope_deg.to_radians()),
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

        read_physics_into_world(&mut self.world, &self.physics, 1.0 / 60.0);
        {
            let mut services = HostServices {
                physics: Some(&mut self.physics),
                audio: None,
            };
            self.host
                .fixed_update(&mut self.world, time, input, &mut services);
        }
        write_world_into_physics(&mut self.world, &mut self.physics);
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
        self.world
            .get::<Transform>(self.player)
            .unwrap()
            .translation
    }

    fn grounded(&self) -> bool {
        self.world
            .get::<RigidBodyComponent>(self.player)
            .unwrap()
            .grounded
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

fn action(name: &str, value: [f32; 2], pressed: bool) -> InputSnapshot {
    let active = value[0].abs().max(value[1].abs()) > 0.5;
    InputSnapshot {
        actions: [(name.to_string(), InputActionSnapshot { value, active, pressed })]
            .into_iter()
            .collect(),
    }
}

fn movement(x: f32, y: f32, sprint: bool) -> InputSnapshot {
    let mut input = action("Move", [x, y], false);
    if sprint {
        input.actions.insert(
            "Sprint".to_string(),
            InputActionSnapshot { value: [1.0, 0.0], active: true, pressed: false },
        );
    }
    input
}

fn jump(pressed: bool) -> InputSnapshot {
    action("Jump", [1.0, 0.0], pressed)
}

fn look(x: f32, y: f32) -> InputSnapshot {
    action("Look", [x, y], false)
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
            "lookSensitivity",
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
        vec![
            "eyeHeight",
            "invertMouseY",
            "lookSensitivity",
            "pitchLimit"
        ]
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
    host.load_script(asset, "controller.luau", CONTROLLER)
        .unwrap();
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

    rig.run(60, &movement(0.0, -1.0, false));
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
    rig.run(30, &movement(0.0, -1.0, false));
    let walked = (rig.position() - before).length();

    let before = rig.position();
    rig.run(30, &movement(0.0, -1.0, true));
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
    rig.run(30, &movement(0.0, -1.0, false));
    let straight = (rig.position() - before).length();

    let before = rig.position();
    let diagonal = -1.0 / 2.0_f32.sqrt();
    rig.run(30, &movement(diagonal, diagonal, false));
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

    // The default mouse binding scales 750 pixels by 0.1 into 75 Look units.
    rig.step(&look(75.0, 0.0));
    let yaw = rig.state(rig.controller, "yaw");
    assert!(
        (yaw + 90.0).abs() < 1.0,
        "75 Look units at 1.2°/unit should be about -90°, got {yaw}"
    );

    let before = rig.position();
    rig.run(30, &movement(0.0, -1.0, false));
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
    rig.run(40, &look(0.0, -20.0));
    let pitch = rig.state(rig.camera_instance, "pitch");
    assert!(
        (pitch - 89.0).abs() < 0.001,
        "looking up must stop at the limit, got {pitch}"
    );

    rig.run(80, &look(0.0, 20.0));
    let pitch = rig.state(rig.camera_instance, "pitch");
    assert!(
        (pitch + 89.0).abs() < 0.001,
        "and looking down must stop too, got {pitch}"
    );
}

#[test]
fn the_camera_rides_at_eye_height_on_the_player() {
    let mut rig = Rig::new();
    rig.run(20, &movement(0.0, -1.0, false));

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
    rig.step(&jump(true));
    let mut peak = ground;
    for _ in 0..40 {
        rig.step(&jump(false));
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
        rig.step(&jump(true));
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
    rig.step(&look(30.0, -10.0));

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

// ── Footing ────────────────────────────────────────────────────────────

/// Walk each slope for ten seconds and report `(grounded steps, footfalls)`.
///
/// A footfall is counted by watching `footstepIndex`, which the controller
/// advances once per `ctx:playAudio`. That makes the cadence observable
/// without an audio device, which is the only reason this can run in CI.
fn walk_a_slope(slope_deg: f32) -> (u32, u32) {
    let mut rig = Rig::on_slope(slope_deg);
    rig.run(30, &InputSnapshot::default());

    let walk = movement(0.0, -1.0, false);
    let mut previous = rig.state(rig.controller, "footstepIndex");
    let (mut grounded, mut footfalls) = (0, 0);
    for _ in 0..600 {
        rig.step(&walk);
        if rig.grounded() {
            grounded += 1;
        }
        let index = rig.state(rig.controller, "footstepIndex");
        if (index - previous).abs() > 0.001 {
            footfalls += 1;
            previous = index;
        }
    }
    (grounded, footfalls)
}

#[test]
fn a_slope_is_not_the_same_thing_as_falling() {
    // The bug this test exists for: `grounded` was `velocity.y.abs() <
    // 0.35`, and a character walking at 4.5 m/s up a five-degree rise has
    // a vertical speed of 0.39 — over the line. Ten degrees read as
    // airborne on 599 of 600 steps. Nothing on flat ground could see it,
    // which is why every hill in the engine had a character that could not
    // jump and did not make a sound.
    for slope in [0.0_f32, 5.0, 10.0, 20.0, 30.0] {
        let (grounded, _) = walk_a_slope(slope);
        assert!(
            grounded >= 590,
            "walking a {slope}-degree slope must not read as falling:              grounded on only {grounded} of 600 steps"
        );
    }
}

#[test]
fn footsteps_keep_their_cadence_on_a_hill() {
    // Distance-driven cadence is slope-independent by construction, so the
    // count is allowed to differ by one footfall and no more. Before the
    // `grounded` fix this was 25 footfalls on the flat and none at all at
    // ten degrees.
    let (_, flat) = walk_a_slope(0.0);
    assert!(
        flat >= 20,
        "ten seconds of walking is more than {flat} footsteps"
    );
    for slope in [5.0_f32, 10.0, 20.0, 30.0] {
        let (_, hill) = walk_a_slope(slope);
        assert!(
            hill.abs_diff(flat) <= 1,
            "a {slope}-degree slope changed the cadence: {hill} footfalls against              {flat} on the flat"
        );
    }
}

#[test]
fn the_first_footstep_lands_on_the_first_step_taken() {
    // Waiting a full stride before the first sound puts the audio a fifth
    // of a second behind the key, every time the player starts walking —
    // which reads as broken rather than as latency.
    let mut rig = Rig::new();
    rig.run(30, &InputSnapshot::default());
    let before = rig.state(rig.controller, "footstepIndex");

    rig.step(&movement(0.0, -1.0, false));
    assert!(
        (rig.state(rig.controller, "footstepIndex") - before).abs() > 0.001,
        "the first fixed step of walking should already have asked for a footstep"
    );
}

#[test]
fn a_jump_still_leaves_the_ground() {
    // The counterweight to the coyote window: grace measured in a handful
    // of steps must not turn into a character who is never airborne, or
    // the flag stops meaning anything and the jump gate opens in mid-air.
    let mut rig = Rig::new();
    rig.run(15, &InputSnapshot::default());
    assert!(rig.grounded(), "standing on the floor");

    rig.step(&jump(true));
    let mut airborne = 0;
    for _ in 0..60 {
        rig.step(&InputSnapshot::default());
        if !rig.grounded() {
            airborne += 1;
        }
    }
    assert!(
        airborne > 40,
        "most of a one-second flight should read as airborne, not {airborne} steps"
    );
}
