//! Phase 16-E: transactional hot reload, and the module graph it needs.
//!
//! The failure paths are the point. A reload that goes wrong must leave
//! the running world exactly as it was — that is the property that makes
//! it safe to bind to a key and to a file watcher, and it is what most of
//! this file asserts.

use std::time::Duration;

use somnium_core::script_host::{HostServices, ScriptHost, display_path};
use somnium_core::{Name, Transform};
use somnium_ecs::{Entity, World};
use somnium_script::attachment::{ScriptAttachment, ScriptSet};
use somnium_script::backend::Budget;
use somnium_script::ids::{InstanceUuid, ScriptAssetId};
use somnium_script::lifecycle::LifecycleState;
use somnium_script::runtime::PhaseInput;
use somnium_script::snapshot::{InputSnapshot, TimeSnapshot};
use somnium_script::value::ScriptValue;

// ── Harness ────────────────────────────────────────────────────────────

/// A scratch directory of this test's own, so the cases can run in
/// parallel without treading on each other.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "somnium_reload_{}_{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, name: &str, text: &str) -> std::path::PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Sim {
    host: ScriptHost,
    world: World,
    entity: Entity,
    instance: InstanceUuid,
    step: u64,
}

impl Sim {
    fn new(asset: ScriptAssetId, host: ScriptHost) -> Self {
        let mut world = World::new();
        let entity = world.spawn((Name::new("Subject"), Transform::default(), ScriptSet::new()));
        let attachment = ScriptAttachment::new(asset);
        let instance = attachment.instance;
        world.get_mut::<ScriptSet>(entity).unwrap().attach(attachment);
        Self {
            host,
            world,
            entity,
            instance,
            step: 0,
        }
    }

    fn frame(&mut self) {
        let time = TimeSnapshot {
            fixed_delta: 1.0 / 60.0,
            delta: 1.0 / 60.0,
            #[allow(clippy::cast_precision_loss)]
            simulation_time: self.step as f64 / 60.0,
            step: self.step,
        };
        let phase = PhaseInput {
            time,
            input: InputSnapshot::default(),
        };
        let mut services = HostServices::default();
        self.host.sync(&mut self.world, &phase, &mut services);
        self.host.fixed_update(
            &mut self.world,
            time,
            &InputSnapshot::default(),
            &mut services,
        );
        self.step += 1;
    }

    fn logs(&mut self) -> Vec<String> {
        self.host
            .take_logs()
            .into_iter()
            .map(|line| line.message)
            .collect()
    }

    fn property(&self, name: &str) -> Option<ScriptValue> {
        self.world
            .get::<ScriptSet>(self.entity)?
            .get(self.instance)?
            .properties
            .get(name)
            .cloned()
    }

    fn set_property(&mut self, name: &str, value: ScriptValue) {
        let mut set = self.world.get::<ScriptSet>(self.entity).cloned().unwrap();
        set.get_mut(self.instance)
            .unwrap()
            .properties
            .insert(name.into(), value);
        self.world.insert_component(self.entity, set).unwrap();
    }
}

const COUNTER: &str = r"
return Script.define({
    schemaVersion = 1,
    fields = { speed = Field.number(3.0) },
    onFixedUpdate = function(self, ctx, dt) self.n = (self.n or 0) + 1 end,
    saveState = function(self) return { n = self.n } end,
    loadState = function(self, s) self.n = s.n end,
})
";

// ── The module graph (B-3, which 16-E depends on) ──────────────────────

#[test]
fn a_script_can_require_a_shared_module() {
    let dir = Scratch::new("require");
    dir.write("util.luau", "return { greeting = 'hello' }");
    let main = dir.write(
        "main.luau",
        r"
        local util = require('util')
        return Script.define({
            onStart = function(self, ctx) ctx:log(util.greeting) end,
        })
        ",
    );

    let mut host = ScriptHost::new(Budget::default());
    // Only `main` is imported; `util` is pulled in because `main` requires
    // it. Making the author import dependencies by hand, in linker order,
    // would be an absurd thing to ask.
    let asset = host.import_script_file(&main).unwrap();
    let mut sim = Sim::new(asset, host);
    sim.frame();
    assert!(sim.logs().contains(&"hello".to_string()));
}

#[test]
fn a_shared_module_is_evaluated_once_and_frozen() {
    let dir = Scratch::new("frozen");
    dir.write("shared.luau", "return { count = 0 }");
    let main = dir.write(
        "main.luau",
        r"
        local shared = require('shared')
        return Script.define({
            onStart = function(self, ctx)
                local ok = pcall(function() shared.count = 1 end)
                ctx:log(if ok then 'mutable' else 'frozen')
            end,
        })
        ",
    );

    let mut host = ScriptHost::new(Budget::default());
    let asset = host.import_script_file(&main).unwrap();
    let mut sim = Sim::new(asset, host);
    sim.frame();
    assert!(
        sim.logs().contains(&"frozen".to_string()),
        "one attachment must not be able to rewrite a helper for every other one"
    );
}

#[test]
fn a_require_cycle_is_rejected_at_link_time_with_both_names() {
    let dir = Scratch::new("cycle");
    dir.write("a.luau", "local b = require('b')\nreturn {}");
    let b = dir.write("b.luau", "local a = require('a')\nreturn Script.define({})");

    let mut host = ScriptHost::new(Budget::default());
    let error = host.import_script_file(&b).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("cycle"), "got: {text}");
    assert!(text.contains("a.luau") && text.contains("b.luau"), "got: {text}");
}

#[test]
fn a_computed_require_is_a_compile_error_not_a_runtime_surprise() {
    let dir = Scratch::new("dynamic");
    let path = dir.write(
        "main.luau",
        "local name = 'util'\nlocal m = require(name)\nreturn Script.define({})",
    );
    let mut host = ScriptHost::new(Budget::default());
    let error = host.import_script_file(&path).unwrap_err();
    assert!(
        error.to_string().contains("dependency graph"),
        "got: {error}"
    );
    assert_eq!(
        error.messages[0].line, 2,
        "and it points at the line that did it"
    );
}

#[test]
fn the_blast_radius_of_an_edit_is_computed_from_the_graph() {
    let dir = Scratch::new("radius");
    dir.write("base.luau", "return { v = 1 }");
    dir.write("mid.luau", "local b = require('base')\nreturn { v = b.v }");
    let top = dir.write(
        "top.luau",
        "local m = require('mid')\nreturn Script.define({ onStart = function(self, ctx) ctx:log(tostring(m.v)) end })",
    );

    let mut host = ScriptHost::new(Budget::default());
    let top_asset = host.import_script_file(&top).unwrap();
    let base = ScriptAssetId::from_path(&display_path(&dir.0.join("base.luau")));

    let radius = host.runtime().blast_radius(base);
    assert_eq!(
        radius.len(),
        3,
        "editing `base` must reach `mid` and `top` as well: {radius:?}"
    );
    assert!(radius.contains(&top_asset));
    assert_eq!(
        host.runtime().blast_radius(top_asset),
        vec![top_asset],
        "and editing the leaf reaches only itself"
    );
}

#[test]
fn reloading_a_shared_module_reaches_the_scripts_that_require_it() {
    let dir = Scratch::new("transitive");
    dir.write("base.luau", "return { v = 1 }");
    let top = dir.write(
        "top.luau",
        "local b = require('base')\nreturn Script.define({ onStart = function(self, ctx) ctx:log('v=' .. tostring(b.v)) end })",
    );

    let mut host = ScriptHost::new(Budget::default());
    let asset = host.import_script_file(&top).unwrap();
    let mut sim = Sim::new(asset, host);
    sim.frame();
    assert!(sim.logs().contains(&"v=1".to_string()));

    // Edit only the shared module.
    let base_path = dir.write("base.luau", "return { v = 42 }");
    sim.host.import_script_file(&base_path).unwrap();
    sim.frame();
    assert!(
        sim.logs().contains(&"v=42".to_string()),
        "the dependent's instance must be rebuilt against the new module"
    );
}

// ── The reload transaction ─────────────────────────────────────────────

#[test]
fn a_reload_that_does_not_compile_changes_nothing_about_the_running_world() {
    let dir = Scratch::new("broken");
    let path = dir.write("main.luau", COUNTER);
    let mut host = ScriptHost::new(Budget::default());
    let asset = host.import_script_file(&path).unwrap();
    let mut sim = Sim::new(asset, host);
    for _ in 0..5 {
        sim.frame();
    }
    let before = sim.host.export_states();

    dir.write("main.luau", "return Script.define({");
    assert!(sim.host.import_script_file(&path).is_err());

    assert_eq!(sim.host.runtime().live_instances(), 1);
    assert_eq!(
        sim.host.state_of(sim.instance),
        Some(LifecycleState::Enabled),
        "the old instance is still running, not merely still allocated"
    );
    sim.frame();
    let after = sim.host.export_states();
    assert_ne!(
        before, after,
        "and it is still counting, so it really is the old module"
    );
}

#[test]
fn a_reload_commits_at_a_frame_boundary_and_replays_the_lifecycle() {
    let dir = Scratch::new("commit");
    let path = dir.write(
        "main.luau",
        "return Script.define({ onStart = function(self, ctx) ctx:log('v1') end })",
    );
    let mut host = ScriptHost::new(Budget::default());
    let asset = host.import_script_file(&path).unwrap();
    let mut sim = Sim::new(asset, host);
    sim.frame();
    assert_eq!(sim.logs(), vec!["v1"]);

    dir.write(
        "main.luau",
        "return Script.define({ onStart = function(self, ctx) ctx:log('v2') end })",
    );
    sim.host.import_script_file(&path).unwrap();
    assert_eq!(
        sim.host.state_of(sim.instance),
        Some(LifecycleState::Loaded),
        "the swap leaves the instance waiting for the next sync"
    );
    sim.frame();
    assert_eq!(sim.logs(), vec!["v2"]);
    assert_eq!(
        sim.host.state_of(sim.instance),
        Some(LifecycleState::Enabled)
    );
}

#[test]
fn one_generation_of_rollback_is_retained() {
    let dir = Scratch::new("rollback");
    let path = dir.write(
        "main.luau",
        "return Script.define({ onStart = function(self, ctx) ctx:log('good') end })",
    );
    let mut host = ScriptHost::new(Budget::default());
    let asset = host.import_script_file(&path).unwrap();
    assert!(
        !host.runtime().can_rollback(asset),
        "nothing to return to before the first edit"
    );

    let mut sim = Sim::new(asset, host);
    sim.frame();
    let _ = sim.logs();

    // It compiles. It is also wrong.
    dir.write(
        "main.luau",
        "return Script.define({ onStart = function(self, ctx) ctx:log('regrettable') end })",
    );
    sim.host.import_script_file(&path).unwrap();
    sim.frame();
    assert_eq!(sim.logs(), vec!["regrettable"]);

    assert!(sim.host.runtime().can_rollback(asset));
    sim.host.runtime_mut().rollback_asset(asset).unwrap();
    sim.frame();
    assert_eq!(
        sim.logs(),
        vec!["good"],
        "a rollback is not a syntax-error path — it is for the edit that \
         compiled and was wrong anyway"
    );
}

// ── Migration ──────────────────────────────────────────────────────────

const V1: &str = r"
return Script.define({
    schemaVersion = 1,
    fields = { speed = Field.number(1.0) },
    onStart = function(self, ctx) ctx:log('speed=' .. tostring(self.speed)) end,
})
";

/// Same script, one schema version later: `speed` became `velocity`, and
/// the author said so.
const V2_RENAMED: &str = r"
return Script.define({
    schemaVersion = 2,
    fields = { velocity = Field.number(1.0) },
    migrateProperties = function(self, props, fromVersion)
        if fromVersion < 2 and props.speed ~= nil then
            props.velocity = props.speed
            props.speed = nil
        end
        return props
    end,
    onStart = function(self, ctx) ctx:log('velocity=' .. tostring(self.velocity)) end,
})
";

/// And the same rename without the migration declared.
const V2_NO_MIGRATION: &str = r"
return Script.define({
    schemaVersion = 2,
    fields = { velocity = Field.number(1.0) },
    onStart = function(self, ctx) ctx:log('velocity=' .. tostring(self.velocity)) end,
})
";

#[test]
fn a_renamed_field_migrates_when_the_author_says_how() {
    let dir = Scratch::new("rename");
    let path = dir.write("main.luau", V1);
    let mut host = ScriptHost::new(Budget::default());
    let asset = host.import_script_file(&path).unwrap();
    let mut sim = Sim::new(asset, host);
    sim.set_property("speed", ScriptValue::F64(9.0));
    sim.frame();
    assert_eq!(sim.logs(), vec!["speed=9"]);

    dir.write("main.luau", V2_RENAMED);
    sim.host.import_script_file(&path).unwrap();
    sim.frame();

    assert_eq!(
        sim.logs(),
        vec!["velocity=9"],
        "the value the author set must survive the rename"
    );
    assert_eq!(sim.property("velocity"), Some(ScriptValue::F64(9.0)));
    assert_eq!(
        sim.property("speed"),
        None,
        "and the old key is gone from the scene, not left as litter"
    );
}

#[test]
fn a_removed_field_warns_and_drops_rather_than_failing_the_load() {
    let dir = Scratch::new("removed");
    let path = dir.write("main.luau", V1);
    let mut host = ScriptHost::new(Budget::default());
    let asset = host.import_script_file(&path).unwrap();
    let mut sim = Sim::new(asset, host);
    sim.set_property("speed", ScriptValue::F64(9.0));
    sim.frame();
    let _ = sim.logs();
    let _ = sim.host.take_diagnostics();

    dir.write("main.luau", V2_NO_MIGRATION);
    sim.host.import_script_file(&path).unwrap();
    sim.frame();

    let warnings: Vec<String> = sim
        .host
        .take_diagnostics()
        .into_iter()
        .map(|d| d.message)
        .collect();
    assert!(
        warnings.iter().any(|m| m.contains("speed")),
        "the author has to be told a value was dropped: {warnings:?}"
    );
    assert_eq!(
        sim.logs(),
        vec!["velocity=1"],
        "and the script still runs, on its declared default"
    );
}

#[test]
fn a_callback_that_is_not_a_function_is_rejected_with_a_diagnostic() {
    // Luau is dynamically typed, so a genuinely changed *signature*
    // cannot be caught before the call. What can — and what this asserts —
    // is a callback name bound to something that is not callable at all,
    // which would otherwise be silently absent until frame four hundred.
    let dir = Scratch::new("signature");
    let path = dir.write("main.luau", "return Script.define({ onFixedUpdate = 5 })");
    let mut host = ScriptHost::new(Budget::default());
    let error = host.import_script_file(&path).unwrap_err();
    assert!(
        error.to_string().contains("onFixedUpdate"),
        "got: {error}"
    );
}

// ── The watcher ────────────────────────────────────────────────────────

#[test]
fn a_file_change_is_not_acted_on_until_it_settles() {
    let dir = Scratch::new("debounce");
    let path = dir.write("main.luau", COUNTER);
    let mut host = ScriptHost::new(Budget::default());
    host.import_script_file(&path).unwrap();
    assert!(
        host.poll_file_changes(Duration::ZERO).is_empty(),
        "an untouched file is not a change"
    );

    // An editor writing the file. The first poll sees a difference and
    // starts the clock; it must not report yet, however long the settle.
    dir.write("main.luau", "return Script.define({ schemaVersion = 2 })");
    assert!(
        host.poll_file_changes(Duration::from_secs(60)).is_empty(),
        "a change must not be reported the instant it is seen — half a \
         file on disk compiles to a syntax error the author never made"
    );
    // Second poll, same content, settle satisfied.
    assert_eq!(host.poll_file_changes(Duration::ZERO).len(), 1);
    assert!(
        host.poll_file_changes(Duration::ZERO).is_empty(),
        "and it is reported once, not every frame after"
    );
}

#[test]
fn a_change_still_in_flight_restarts_the_clock() {
    let dir = Scratch::new("inflight");
    let path = dir.write("main.luau", COUNTER);
    let mut host = ScriptHost::new(Budget::default());
    host.import_script_file(&path).unwrap();

    dir.write("main.luau", "-- half");
    assert!(host.poll_file_changes(Duration::ZERO).is_empty());
    // The writer is still going: a different signature resets the wait.
    dir.write("main.luau", "-- half, and the other half");
    assert!(
        host.poll_file_changes(Duration::ZERO).is_empty(),
        "the file moved again, so the settle window starts over"
    );
    assert_eq!(host.poll_file_changes(Duration::ZERO).len(), 1);
}

#[test]
fn the_watcher_recompiles_and_a_broken_edit_costs_only_a_diagnostic() {
    let dir = Scratch::new("watch");
    let path = dir.write("main.luau", COUNTER);
    let mut host = ScriptHost::new(Budget::default());
    let asset = host.import_script_file(&path).unwrap();
    let mut sim = Sim::new(asset, host);
    sim.frame();

    dir.write(
        "main.luau",
        "return Script.define({ schemaVersion = 5, onFixedUpdate = function(self, ctx, dt) end })",
    );
    let _ = sim.host.poll_file_changes(Duration::ZERO);
    assert_eq!(sim.host.reload_changed(Duration::ZERO), (1, 0));
    assert_eq!(
        sim.host.runtime().asset_schema(asset).unwrap().schema_version,
        5
    );

    dir.write("main.luau", "return Script.define({");
    let _ = sim.host.poll_file_changes(Duration::ZERO);
    assert_eq!(sim.host.reload_changed(Duration::ZERO), (0, 1));
    assert_eq!(
        sim.host.runtime().asset_schema(asset).unwrap().schema_version,
        5,
        "the last good module is still the one loaded"
    );
    assert_eq!(sim.host.runtime().live_instances(), 1);
}

#[test]
fn a_hundred_watcher_reloads_leak_nothing() {
    let dir = Scratch::new("churn");
    let path = dir.write("main.luau", COUNTER);
    let mut host = ScriptHost::new(Budget::default());
    let asset = host.import_script_file(&path).unwrap();
    let mut sim = Sim::new(asset, host);
    sim.frame();

    for cycle in 0..100 {
        dir.write(
            "main.luau",
            &format!("return Script.define({{ schemaVersion = {}, onFixedUpdate = function(self, ctx, dt) end }})", cycle + 1),
        );
        sim.host.import_script_file(&path).unwrap();
        sim.frame();
        assert_eq!(
            sim.host.runtime().live_instances(),
            1,
            "cycle {cycle} leaked an instance"
        );
    }
    assert_eq!(sim.host.runtime().owned_resources(), 0);
    assert!(sim.host.runtime().can_rollback(asset));
}
