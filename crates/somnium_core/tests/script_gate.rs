//! Phase 16-C gate, asserted rather than eyeballed.
//!
//! The sub-phase's acceptance criterion is a `.luau` file attached to an
//! entity that "rotates it at fixed step, reads input, applies a force,
//! spawns and despawns, emits and receives an event, and persists its
//! exported fields through a save/load cycle". This runs **the same file
//! `hello_engine` runs** — `assets/scripts/demo_rotator.luau` — through
//! the same [`ScriptHost`], and checks each clause.
//!
//! Running it in `hello_engine` proves it renders. Running it here proves
//! it is correct, on every commit, without a window.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use somnium_core::script_host::{HostServices, ScriptHost};
use somnium_core::{Name, Transform};
use somnium_ecs::{Entity, World};
use somnium_physics::config::PhysicsConfig;
use somnium_physics::world::PhysicsWorld;
use somnium_script::attachment::{ScriptAttachment, ScriptSet};
use somnium_script::backend::Budget;
use somnium_script::ids::{InstanceUuid, ScriptAssetId};
use somnium_script::runtime::PhaseInput;
use somnium_script::snapshot::{InputActionSnapshot, InputSnapshot, TimeSnapshot};
use somnium_script::value::ScriptValue;

/// The gate script itself, compiled in so the test does not depend on the
/// working directory the harness happens to run from.
const DEMO: &str = include_str!("../../../assets/scripts/demo_rotator.luau");

struct Gate {
    host: ScriptHost,
    world: World,
    /// Only the force test needs one.
    ///
    /// Jolt's global initialiser is not safe to run from several threads
    /// at once, and `cargo test` runs the cases in this file in parallel;
    /// building a world per test crashed the binary before a single
    /// assertion ran. One test constructs one, and the rest pass `None` —
    /// which is also the headless-server configuration, so it is worth
    /// exercising.
    physics: Option<PhysicsWorld>,
    entity: Entity,
    instance: InstanceUuid,
    step: u64,
    forces: Arc<AtomicU32>,
}

impl Gate {
    fn new() -> Self {
        let mut host = ScriptHost::new(Budget::default());
        let asset = ScriptAssetId::mint();
        host.load_script(asset, "demo_rotator.luau", DEMO)
            .unwrap_or_else(|d| panic!("the gate script must compile:\n{d}"));

        // The router the engine cannot write for itself: only game code
        // knows how an entity maps to a rigid body. Counting the calls is
        // enough to prove the command reached physics.
        let forces = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&forces);
        host.set_force_router(Box::new(move |_world, _physics, _entity, force, _mode| {
            assert!(
                force.is_finite(),
                "a non-finite force must never get this far"
            );
            counter.fetch_add(1, Ordering::Relaxed);
        }));

        let mut world = World::new();
        let entity = world.spawn((
            Name::new("Scripted Rotator"),
            Transform::from_translation(glam::Vec3::new(0.0, 2.0, 0.0)),
            ScriptSet::new(),
        ));
        let attachment = ScriptAttachment::new(asset);
        let instance = attachment.instance;
        world
            .get_mut::<ScriptSet>(entity)
            .unwrap()
            .attach(attachment);

        Self {
            host,
            world,
            physics: None,
            entity,
            instance,
            step: 0,
            forces,
        }
    }

    /// Give this gate a physics world, so `applyForce` has somewhere to go.
    fn with_physics(mut self) -> Self {
        self.physics = Some(PhysicsWorld::new(PhysicsConfig::default()));
        self
    }

    fn time(&self) -> TimeSnapshot {
        TimeSnapshot {
            fixed_delta: 1.0 / 60.0,
            delta: 1.0 / 60.0,
            #[allow(clippy::cast_precision_loss)]
            simulation_time: self.step as f64 / 60.0,
            step: self.step,
        }
    }

    /// One frame, in the order `app.rs` runs it.
    fn frame(&mut self, input: &InputSnapshot) {
        let time = self.time();
        let phase = PhaseInput {
            time,
            input: input.clone(),
        };
        let mut services = HostServices {
            physics: self.physics.as_mut(),
            audio: None,
        };
        self.host.sync(&mut self.world, &phase, &mut services);
        self.host
            .fixed_update(&mut self.world, time, input, &mut services);
        self.host
            .update(&mut self.world, time, input, &mut services);
        self.step += 1;
    }

    fn transform(&self) -> Transform {
        *self.world.get::<Transform>(self.entity).unwrap()
    }

    fn log(&mut self) -> Vec<String> {
        self.host
            .take_logs()
            .into_iter()
            .map(|line| line.message)
            .collect()
    }
}

#[test]
fn the_gate_script_rotates_its_entity_at_fixed_step() {
    let mut gate = Gate::new();
    let start = gate.transform().rotation;

    for _ in 0..30 {
        gate.frame(&InputSnapshot::default());
    }
    let after = gate.transform().rotation;

    assert!(
        start.angle_between(after) > 0.1,
        "half a second at 1.5 rad/s should be a visible rotation; \
         got {start:?} → {after:?}"
    );
    assert!(
        (after.length() - 1.0).abs() < 1.0e-4,
        "the quaternion must arrive normalised, not as four loose numbers"
    );
}

#[test]
fn the_gate_script_bobs_by_reading_its_own_writes() {
    // The vertical motion is a read-modify-write through the mirror. Doing
    // it through `ctx:get`/`ctx:set` would re-read the pre-phase value
    // every step and only the last write would land.
    let mut gate = Gate::new();
    let base = gate.transform().translation.y;

    let mut seen_above = false;
    let mut seen_below = false;
    for _ in 0..240 {
        gate.frame(&InputSnapshot::default());
        let y = gate.transform().translation.y;
        seen_above |= y > base + 0.2;
        seen_below |= y < base - 0.2;
    }
    assert!(seen_above && seen_below, "the bob must go both ways");
    assert!(
        (gate.transform().translation.x).abs() < 1.0e-5,
        "and it must not drift on the axes it does not touch"
    );
}

#[test]
fn the_gate_script_applies_a_force_only_while_the_key_is_held() {
    let mut gate = Gate::new().with_physics();

    for _ in 0..5 {
        gate.frame(&InputSnapshot::default());
    }
    assert_eq!(gate.forces.load(Ordering::Relaxed), 0, "no key, no force");

    let held = InputSnapshot {
        actions: [(
            "Move".to_string(),
            InputActionSnapshot {
                value: [0.0, -1.0],
                active: true,
                pressed: false,
            },
        )]
        .into_iter()
        .collect(),
    };
    for _ in 0..5 {
        gate.frame(&held);
    }
    assert_eq!(
        gate.forces.load(Ordering::Relaxed),
        5,
        "one force per fixed step the key was held"
    );
}

#[test]
fn the_gate_script_spawns_an_entity_and_then_despawns_what_it_spawned() {
    let mut gate = Gate::new();
    let before = gate.world.entities().count();

    // The script spawns on its sixtieth step and despawns the entity it
    // got back on the step after — which is the only way it *can* work,
    // since a spawn returns a token and the entity arrives next phase.
    for _ in 0..59 {
        gate.frame(&InputSnapshot::default());
    }
    assert_eq!(gate.world.entities().count(), before, "not yet");

    gate.frame(&InputSnapshot::default());
    assert_eq!(
        gate.world.entities().count(),
        before + 1,
        "the spawn commits at the phase boundary"
    );
    let lines = gate.log();
    assert!(lines.iter().any(|l| l == "spawned a marker"), "{lines:?}");

    gate.frame(&InputSnapshot::default());
    assert_eq!(
        gate.world.entities().count(),
        before,
        "and the script found its own spawn in ctx.spawns and destroyed it"
    );
    let lines = gate.log();
    assert!(
        lines.iter().any(|l| l == "despawned the marker"),
        "{lines:?}"
    );
}

#[test]
fn the_gate_script_emits_an_event_and_hears_it() {
    let mut gate = Gate::new();
    gate.frame(&InputSnapshot::default());
    gate.frame(&InputSnapshot::default());

    let lines = gate.log();
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("heard rotator.started #")),
        "the event a script emits must come back to it with a sequence number: {lines:?}"
    );
}

#[test]
fn the_gate_scripts_exported_state_survives_a_save_and_load() {
    let mut gate = Gate::new();
    for _ in 0..25 {
        gate.frame(&InputSnapshot::default());
    }

    let saved = gate.host.export_states();
    let state = saved
        .get(&gate.instance)
        .expect("the attachment declares saveState");
    let ScriptValue::Map(fields) = state else {
        panic!("saveState must return pure data, got {state:?}");
    };
    assert_eq!(fields.get("ticks"), Some(&ScriptValue::I64(25)));
    assert!(matches!(fields.get("angle"), Some(ScriptValue::F64(_))));

    // A reload rebuilds the VM object and hands the state back; the step
    // count carries across rather than starting from zero.
    let asset = gate
        .host
        .runtime()
        .assets()
        .next()
        .map(|(id, _)| id)
        .unwrap();
    gate.host
        .reload_script(asset, "demo_rotator.luau", DEMO)
        .expect("the same source recompiles");
    gate.frame(&InputSnapshot::default());

    let after = gate.host.export_states();
    let ScriptValue::Map(fields) = &after[&gate.instance] else {
        panic!("state must still be pure data");
    };
    assert_eq!(
        fields.get("ticks"),
        Some(&ScriptValue::I64(26)),
        "the reloaded instance counts on from where the old one stopped"
    );
}

#[test]
fn the_gate_scripts_declared_properties_are_readable_without_running_it() {
    let mut host = ScriptHost::new(Budget::default());
    let asset = ScriptAssetId::mint();
    host.load_script(asset, "demo_rotator.luau", DEMO).unwrap();

    let schema = host.runtime().asset_schema(asset).expect("described");
    let names: Vec<&str> = schema.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["riseHeight", "spinSpeed", "thrust"]);

    let spin = schema.field("spinSpeed").unwrap();
    assert_eq!(spin.default, ScriptValue::F64(1.5));
    assert_eq!(spin.min, Some(0.0));
    assert_eq!(spin.max, Some(20.0));
    assert!(
        spin.description.is_some(),
        "the editor draws this as a tooltip"
    );
}

#[test]
fn an_authored_property_overrides_the_scripts_own_default() {
    let mut gate = Gate::new();
    {
        let mut set = gate.world.get::<ScriptSet>(gate.entity).cloned().unwrap();
        set.get_mut(gate.instance)
            .unwrap()
            .properties
            .insert("spinSpeed".into(), ScriptValue::F64(0.0));
        gate.world.insert_component(gate.entity, set).unwrap();
    }

    let start = gate.transform().rotation;
    for _ in 0..30 {
        gate.frame(&InputSnapshot::default());
    }
    assert!(
        start.angle_between(gate.transform().rotation) < 1.0e-4,
        "a spin speed of zero authored in the editor must actually stop it"
    );
}
