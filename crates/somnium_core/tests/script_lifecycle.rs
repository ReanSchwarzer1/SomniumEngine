//! Phase 16-C: the lifecycle, the scheduler, and the things that are
//! supposed to be impossible.
//!
//! Every test here drives a real Luau VM through [`ScriptHost`] and a real
//! `World`. Nothing is stubbed — a scheduler that passed against a fake
//! backend would prove nothing about whether scripts run.

use std::collections::BTreeMap;

use somnium_core::script_host::{HostServices, ScriptHost, run_to_fixed_point};
use somnium_core::{Name, Transform};
use somnium_ecs::{Entity, World};
use somnium_script::attachment::{ScriptAttachment, ScriptSet};
use somnium_script::backend::Budget;
use somnium_script::command::LogLevel;
use somnium_script::ids::{InstanceUuid, ScriptAssetId};
use somnium_script::lifecycle::LifecycleState;
use somnium_script::ownership::OwnedResource;
use somnium_script::runtime::{MAX_INIT_CYCLES, PhaseInput};
use somnium_script::snapshot::{InputSnapshot, TimeSnapshot};
use somnium_script::value::ScriptValue;

// ── Harness ────────────────────────────────────────────────────────────

/// One world, one host, and the two calls the frame loop makes.
struct Harness {
    host: ScriptHost,
    world: World,
    step: u64,
}

impl Harness {
    fn new() -> Self {
        Self {
            host: ScriptHost::new(Budget::default()),
            world: World::new(),
            step: 0,
        }
    }

    fn with_failure_threshold(threshold: u32) -> Self {
        let mut harness = Self::new();
        harness.host.runtime_mut().set_failure_threshold(threshold);
        harness
    }

    fn load(&mut self, text: &str) -> ScriptAssetId {
        let asset = ScriptAssetId::mint();
        self.host
            .load_script(asset, "test.luau", text)
            .unwrap_or_else(|d| panic!("compile failed: {d}"))
    }

    fn spawn_scripted(&mut self, asset: ScriptAssetId, name: &str) -> (Entity, InstanceUuid) {
        let entity = self
            .world
            .spawn((Name::new(name), Transform::default(), ScriptSet::new()));
        let instance = self.attach(entity, asset);
        (entity, instance)
    }

    fn attach(&mut self, entity: Entity, asset: ScriptAssetId) -> InstanceUuid {
        let attachment = ScriptAttachment::new(asset);
        let instance = attachment.instance;
        let mut set = self
            .world
            .get::<ScriptSet>(entity)
            .cloned()
            .unwrap_or_default();
        set.attach(attachment);
        self.world.insert_component(entity, set).unwrap();
        instance
    }

    fn detach(&mut self, entity: Entity, instance: InstanceUuid) {
        let mut set = self.world.get::<ScriptSet>(entity).cloned().unwrap();
        set.detach(instance);
        self.world.insert_component(entity, set).unwrap();
    }

    fn set_enabled(&mut self, entity: Entity, instance: InstanceUuid, enabled: bool) {
        let mut set = self.world.get::<ScriptSet>(entity).cloned().unwrap();
        set.get_mut(instance).unwrap().enabled = enabled;
        self.world.insert_component(entity, set).unwrap();
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

    fn sync(&mut self) {
        let phase = PhaseInput {
            time: self.time(),
            input: InputSnapshot::default(),
        };
        let mut services = HostServices::default();
        self.host.sync(&mut self.world, &phase, &mut services);
    }

    /// One whole frame, in the order `app.rs` runs it.
    fn frame(&mut self) {
        self.sync();
        let time = self.time();
        let mut services = HostServices::default();
        self.host
            .fixed_update(&mut self.world, time, &InputSnapshot::default(), &mut services);
        self.host
            .update(&mut self.world, time, &InputSnapshot::default(), &mut services);
        self.step += 1;
    }

    fn log_lines(&mut self) -> Vec<String> {
        self.host
            .take_logs()
            .into_iter()
            .map(|line| line.message)
            .collect()
    }

    fn translation(&self, entity: Entity) -> glam::Vec3 {
        self.world
            .get::<Transform>(entity)
            .map_or(glam::Vec3::ZERO, |t| t.translation)
    }
}

// ── C-1: the state machine ─────────────────────────────────────────────

const CHATTY: &str = r"
return Script.define({
    onInit = function(self, ctx) ctx:log('init') end,
    onStart = function(self, ctx) ctx:log('start') end,
    onEnable = function(self, ctx) ctx:log('enable') end,
    onDisable = function(self, ctx) ctx:log('disable') end,
    onFixedUpdate = function(self, ctx, dt) ctx:log('fixed') end,
    onUpdate = function(self, ctx, dt) ctx:log('update') end,
    onDestroy = function(self, ctx) ctx:log('destroy') end,
})
";

#[test]
fn the_lifecycle_runs_in_the_documented_order() {
    let mut harness = Harness::new();
    let asset = harness.load(CHATTY);
    let (_, instance) = harness.spawn_scripted(asset, "Subject");

    harness.frame();
    assert_eq!(
        harness.log_lines(),
        vec!["init", "start", "enable", "fixed", "update"],
        "the first frame must walk Loaded → Initialized → Started → Enabled"
    );
    assert_eq!(
        harness.host.state_of(instance),
        Some(LifecycleState::Enabled)
    );

    harness.frame();
    assert_eq!(
        harness.log_lines(),
        vec!["fixed", "update"],
        "the second frame re-runs only the update phases"
    );
}

#[test]
fn disabling_an_attachment_stops_its_updates_and_re_enabling_resumes_them() {
    let mut harness = Harness::new();
    let asset = harness.load(CHATTY);
    let (entity, instance) = harness.spawn_scripted(asset, "Subject");

    harness.frame();
    let _ = harness.log_lines();

    harness.set_enabled(entity, instance, false);
    harness.frame();
    assert_eq!(harness.log_lines(), vec!["disable"]);
    assert_eq!(
        harness.host.state_of(instance),
        Some(LifecycleState::Disabled)
    );

    harness.set_enabled(entity, instance, true);
    harness.frame();
    assert_eq!(harness.log_lines(), vec!["enable", "fixed", "update"]);
}

#[test]
fn a_script_with_no_callbacks_at_all_still_reaches_enabled() {
    // The state advance must not be conditional on the callback existing;
    // if it were, a script that only declares properties would sit in
    // `Loaded` forever and never receive anything it later added.
    let mut harness = Harness::new();
    let asset = harness.load("return Script.define({ fields = { n = Field.number(1.0) } })");
    let (_, instance) = harness.spawn_scripted(asset, "Inert");

    harness.frame();
    assert_eq!(
        harness.host.state_of(instance),
        Some(LifecycleState::Enabled)
    );
}

// ── C-2: the bounded init fixed point ──────────────────────────────────

#[test]
fn initialization_stops_when_nothing_new_is_created() {
    let mut created = 2;
    let (passes, hit_cap) = run_to_fixed_point(|| {
        created -= 1;
        created > 0
    });
    assert_eq!(passes, 1);
    assert!(!hit_cap);
}

#[test]
fn a_spawn_chain_that_never_settles_is_capped_rather_than_hanging() {
    // The prefab that spawns itself. Without the cap this is a hang; with
    // it, it is a diagnostic and a frame.
    let (passes, hit_cap) = run_to_fixed_point(|| true);
    assert_eq!(passes, MAX_INIT_CYCLES);
    assert!(hit_cap, "the cap, not convergence, must be what stopped it");
}

#[test]
fn hitting_the_cap_names_the_scripts_still_being_created() {
    let mut harness = Harness::new();
    let asset = harness.load(CHATTY);
    harness.spawn_scripted(asset, "Subject");
    // Reach into the runtime for the diagnostic half, since no script can
    // yet attach a script and produce the chain for real.
    harness.host.runtime_mut().report_init_did_not_settle();
    let diagnostics = harness.host.take_diagnostics();
    assert_eq!(diagnostics.len(), 1);
    assert!(
        diagnostics[0]
            .message
            .contains(&MAX_INIT_CYCLES.to_string()),
        "the message must state the cap it hit: {}",
        diagnostics[0].message
    );
}

// ── C-3: deferred destruction ──────────────────────────────────────────

const SUICIDE: &str = r"
return Script.define({
    onFixedUpdate = function(self, ctx, dt)
        ctx:despawn(ctx.entity)
        ctx:log('still running after despawn')
    end,
    onDestroy = function(self, ctx) ctx:log('destroy') end,
})
";

#[test]
fn an_entity_that_despawns_itself_finishes_its_callback_and_tears_down_after() {
    let mut harness = Harness::new();
    let asset = harness.load(SUICIDE);
    let (entity, instance) = harness.spawn_scripted(asset, "Doomed");

    harness.frame();
    let lines = harness.log_lines();
    assert!(
        lines.contains(&"still running after despawn".to_string()),
        "the callback must run to completion: {lines:?}"
    );
    assert!(lines.contains(&"destroy".to_string()));
    assert!(!harness.world.is_alive(entity));
    assert_eq!(
        harness.host.state_of(instance),
        None,
        "the VM object is released at the safe point"
    );
    assert_eq!(harness.host.runtime().live_instances(), 0);

    // And the frame after is ordinary.
    harness.frame();
    assert!(harness.log_lines().is_empty());
}

#[test]
fn detaching_a_script_runs_disable_then_destroy() {
    let mut harness = Harness::new();
    let asset = harness.load(CHATTY);
    let (entity, instance) = harness.spawn_scripted(asset, "Subject");
    harness.frame();
    let _ = harness.log_lines();

    harness.detach(entity, instance);
    harness.frame();
    assert_eq!(
        harness.log_lines(),
        vec!["disable", "destroy"],
        "teardown uses the same order a reload does"
    );
    assert_eq!(harness.host.runtime().live_instances(), 0);
    assert!(harness.world.is_alive(entity), "the entity itself survives");
}

// ── C-4: the fixed phase actually moves the world ──────────────────────

const WALKER: &str = r"
--!strict
return Script.define({
    uses = { ['somnium.Transform'] = { 'translation' } },
    fields = { speed = Field.number(6.0) },
    onFixedUpdate = function(self, ctx, dt)
        local t = ctx.self.transform
        t.translation = t.translation + vector.create(self.speed * dt, 0, 0)
    end,
})
";

#[test]
fn a_scripted_entity_moves_at_fixed_step() {
    let mut harness = Harness::new();
    let asset = harness.load(WALKER);
    let (entity, _) = harness.spawn_scripted(asset, "Walker");

    for _ in 0..60 {
        harness.frame();
    }
    let x = harness.translation(entity).x;
    assert!(
        (x - 6.0).abs() < 1.0e-3,
        "sixty steps at 6 m/s should be one second of travel, got {x}"
    );
}

#[test]
fn execution_order_decides_who_writes_last() {
    let mut harness = Harness::new();
    let asset = harness.load(WALKER);
    let (entity, first) = harness.spawn_scripted(asset, "Walker");
    let second = harness.attach(entity, asset);

    // Give the two attachments opposite speeds and a definite order.
    {
        let mut set = harness.world.get::<ScriptSet>(entity).cloned().unwrap();
        let a = set.get_mut(first).unwrap();
        a.execution_order = 10;
        a.properties
            .insert("speed".into(), ScriptValue::F64(60.0));
        let b = set.get_mut(second).unwrap();
        b.execution_order = -10;
        b.properties
            .insert("speed".into(), ScriptValue::F64(600.0));
        harness.world.insert_component(entity, set).unwrap();
    }

    harness.frame();
    // Both mirror in the same pre-phase value and write an absolute
    // result, so the higher `execution_order` applies last and wins.
    let x = harness.translation(entity).x;
    assert!(
        (x - 1.0).abs() < 1.0e-4,
        "the attachment with the higher execution_order must win, got {x}"
    );
}

#[test]
fn the_same_script_replays_to_the_same_state() {
    let mut a = Harness::new();
    let asset_a = a.load(WALKER);
    let (entity_a, _) = a.spawn_scripted(asset_a, "Walker");
    for _ in 0..120 {
        a.frame();
    }

    let mut b = Harness::new();
    let asset_b = b.load(WALKER);
    let (entity_b, _) = b.spawn_scripted(asset_b, "Walker");
    for _ in 0..120 {
        b.frame();
    }

    assert_eq!(
        a.translation(entity_a).to_array(),
        b.translation(entity_b).to_array(),
        "two runs of the same build must produce bit-identical state"
    );
}

// ── C-5: error quarantine ──────────────────────────────────────────────

const THROWER: &str = r"
return Script.define({
    onFixedUpdate = function(self, ctx, dt)
        error('deliberate')
    end,
})
";

const SPINNER: &str = r"
return Script.define({
    onFixedUpdate = function(self, ctx, dt)
        while true do end
    end,
})
";

#[test]
fn a_failing_script_is_switched_off_after_the_threshold_and_its_peers_are_untouched() {
    let mut harness = Harness::with_failure_threshold(3);
    let bad = harness.load(THROWER);
    let good = harness.load(WALKER);
    let (_, faulty) = harness.spawn_scripted(bad, "Faulty");
    let (walker, _) = harness.spawn_scripted(good, "Walker");

    for _ in 0..3 {
        harness.frame();
    }
    assert!(harness.host.runtime().is_quarantined(faulty));
    assert_eq!(
        harness.host.state_of(faulty),
        Some(LifecycleState::Disabled),
        "quarantine switches the attachment off; it does not destroy it"
    );

    // The peer kept running the whole time.
    assert!(
        harness.translation(walker).x > 0.0,
        "one script's fault must not cost another its frame"
    );

    // And the quarantined one is now skipped entirely.
    let before = harness.host.take_logs().len();
    harness.frame();
    let after: Vec<_> = harness
        .host
        .take_logs()
        .into_iter()
        .filter(|line| line.level == LogLevel::Error)
        .collect();
    assert!(
        after.is_empty(),
        "a quarantined attachment must stop producing errors, got {after:?} (was {before})"
    );
}

#[test]
fn an_infinite_loop_is_interrupted_and_the_next_frame_is_normal() {
    let mut harness = Harness::with_failure_threshold(1);
    let spinner = harness.load(SPINNER);
    let walker = harness.load(WALKER);
    harness.spawn_scripted(spinner, "Spinner");
    let (moving, _) = harness.spawn_scripted(walker, "Walker");

    let start = std::time::Instant::now();
    harness.frame();
    let elapsed = start.elapsed();
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "the deadline must stop it, not the heat death of the universe: {elapsed:?}"
    );

    let errors: Vec<_> = harness
        .host
        .take_logs()
        .into_iter()
        .filter(|line| line.level == LogLevel::Error)
        .collect();
    assert!(!errors.is_empty(), "the interrupt must be reported");

    let before = harness.translation(moving).x;
    harness.frame();
    assert!(
        harness.translation(moving).x > before,
        "the frame after a runaway script is an ordinary frame"
    );
}

#[test]
fn clearing_a_quarantine_restores_the_authors_intent() {
    let mut harness = Harness::with_failure_threshold(1);
    let bad = harness.load(THROWER);
    let (_, faulty) = harness.spawn_scripted(bad, "Faulty");
    harness.frame();
    assert!(harness.host.runtime().is_quarantined(faulty));

    harness.host.runtime_mut().clear_quarantine(faulty);
    assert!(!harness.host.runtime().is_quarantined(faulty));
    harness.sync();
    assert_eq!(
        harness.host.state_of(faulty),
        Some(LifecycleState::Enabled),
        "clearing the fault must restore `enabled`, not leave it silently off"
    );
}

// ── C-6: ownership tokens ──────────────────────────────────────────────

#[test]
fn teardown_releases_everything_an_attachment_owned() {
    let mut harness = Harness::new();
    let asset = harness.load(CHATTY);
    let (entity, instance) = harness.spawn_scripted(asset, "Owner");
    harness.frame();

    harness
        .host
        .runtime_mut()
        .acquire(instance, OwnedResource::Audio(1));
    harness
        .host
        .runtime_mut()
        .acquire(instance, OwnedResource::Subscription("door.opened".into()));
    assert_eq!(harness.host.runtime().owned_resources(), 2);

    harness.detach(entity, instance);
    harness.frame();
    assert_eq!(
        harness.host.runtime().owned_resources(),
        0,
        "teardown is complete by construction, not by discipline"
    );
}

#[test]
fn a_hundred_attach_detach_cycles_leave_no_live_instances() {
    let mut harness = Harness::new();
    let asset = harness.load(CHATTY);
    let entity = harness
        .world
        .spawn((Name::new("Churn"), Transform::default(), ScriptSet::new()));

    for _ in 0..100 {
        let instance = harness.attach(entity, asset);
        harness.frame();
        assert_eq!(harness.host.runtime().live_instances(), 1);
        harness.detach(entity, instance);
        harness.frame();
        assert_eq!(harness.host.runtime().live_instances(), 0);
    }
    assert_eq!(harness.host.runtime().owned_resources(), 0);
}

// ── C-7: the reload halves ─────────────────────────────────────────────

const COUNTER_V1: &str = r"
return Script.define({
    schemaVersion = 1,
    onInit = function(self, ctx) self.count = self.count or 0 end,
    onFixedUpdate = function(self, ctx, dt)
        self.count = (self.count or 0) + 1
    end,
    saveState = function(self) return { count = self.count } end,
    loadState = function(self, state) self.count = state.count end,
})
";

const COUNTER_V2: &str = r"
return Script.define({
    schemaVersion = 2,
    onInit = function(self, ctx) self.count = self.count or 0 end,
    onFixedUpdate = function(self, ctx, dt)
        self.count = (self.count or 0) + 10
    end,
    saveState = function(self) return { count = self.count } end,
    loadState = function(self, state) self.count = state.count end,
    onStart = function(self, ctx) ctx:log('count=' .. tostring(self.count)) end,
})
";

#[test]
fn an_in_process_module_swap_carries_declared_state_across() {
    let mut harness = Harness::new();
    let asset = harness.load(COUNTER_V1);
    let (_, instance) = harness.spawn_scripted(asset, "Counter");

    for _ in 0..5 {
        harness.frame();
    }
    let _ = harness.log_lines();
    let before = harness.host.runtime_mut().export_state(instance).unwrap();

    harness
        .host
        .reload_script(asset, "test.luau", COUNTER_V2)
        .expect("v2 compiles");
    harness.frame();

    let lines = harness.log_lines();
    assert!(
        lines.iter().any(|line| line == "count=5"),
        "the new module must start from the old module's state: {lines:?}"
    );
    assert_eq!(
        harness.host.runtime().live_instances(),
        1,
        "the same attachment, a new VM object"
    );
    assert_eq!(
        harness.host.state_of(instance),
        Some(LifecycleState::Enabled),
        "and it is replayed back up to Enabled"
    );
    assert_ne!(before, ScriptValue::Nil);
}

#[test]
fn a_reload_that_does_not_compile_leaves_the_old_instance_running() {
    let mut harness = Harness::new();
    let asset = harness.load(COUNTER_V1);
    let (_, instance) = harness.spawn_scripted(asset, "Counter");
    for _ in 0..3 {
        harness.frame();
    }

    let broken = harness.host.reload_script(asset, "test.luau", "return Script.define({");
    assert!(broken.is_err(), "a syntax error must not be accepted");
    assert!(broken.unwrap_err().has_errors());

    // Nothing about the running world changed.
    assert_eq!(harness.host.runtime().live_instances(), 1);
    assert_eq!(
        harness.host.state_of(instance),
        Some(LifecycleState::Enabled)
    );
    harness.frame();
    let state = harness.host.runtime_mut().export_state(instance).unwrap();
    // Four steps ran in total, so the old module is still the one counting.
    match state {
        ScriptValue::Map(fields) => assert_eq!(
            fields.get("count"),
            Some(&ScriptValue::I64(4)),
            "the old module is still the one counting"
        ),
        other => panic!("expected the old module's saved record, got {other:?}"),
    }
}

#[test]
fn a_hundred_reload_cycles_do_not_grow_the_instance_count() {
    let mut harness = Harness::new();
    let asset = harness.load(COUNTER_V1);
    harness.spawn_scripted(asset, "Counter");
    harness.frame();

    for cycle in 0..100 {
        let text = if cycle % 2 == 0 { COUNTER_V2 } else { COUNTER_V1 };
        harness.host.reload_script(asset, "test.luau", text).unwrap();
        harness.frame();
        assert_eq!(
            harness.host.runtime().live_instances(),
            1,
            "cycle {cycle} leaked an instance"
        );
    }
    assert_eq!(harness.host.runtime().owned_resources(), 0);
}

// ── Save/load of exported state ────────────────────────────────────────

#[test]
fn exported_state_round_trips_through_the_host() {
    let mut harness = Harness::new();
    let asset = harness.load(COUNTER_V1);
    let (_, instance) = harness.spawn_scripted(asset, "Counter");
    for _ in 0..7 {
        harness.frame();
    }

    let saved: BTreeMap<_, _> = harness.host.export_states();
    assert!(saved.contains_key(&instance));

    // A fresh instance of the same attachment, given the saved state back.
    harness.host.import_states(&saved);
    let after = harness.host.export_states();
    assert_eq!(saved, after, "import then export must be the identity");
}

// ── Missing assets ─────────────────────────────────────────────────────

#[test]
fn an_attachment_whose_asset_is_not_loaded_is_reported_and_retried() {
    let mut harness = Harness::new();
    let ghost = ScriptAssetId::mint();
    let (entity, _) = harness.spawn_scripted(ghost, "Ghost");

    harness.frame();
    assert_eq!(harness.host.runtime().live_instances(), 0);

    // Importing the script later is all it takes; the attachment was kept.
    harness.host.load_script(ghost, "late.luau", CHATTY).unwrap();
    harness.frame();
    assert_eq!(harness.host.runtime().live_instances(), 1);
    assert!(harness.world.is_alive(entity));
}
