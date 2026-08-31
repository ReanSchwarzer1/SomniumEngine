//! Phase 16-F: the threat model, as tests.
//!
//! `dev records/phase_16.md` §4.6 is a table of fourteen things a script
//! must not be able to do and the control that stops each one. It was
//! written as an acceptance list, not as background reading, so this file
//! has one test per row and says which row it is.
//!
//! Everything here drives a real VM through the real host. A test that
//! asserted a sandbox property against a mock would be asserting nothing.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use somnium_core::script_host::{HostServices, ScriptHost};
use somnium_core::{Name, Transform};
use somnium_ecs::{Entity, World};
use somnium_script::attachment::{ScriptAttachment, ScriptSet};
use somnium_script::backend::Budget;
use somnium_script::capability::Capabilities;
use somnium_script::command::LogLevel;
use somnium_script::ids::{InstanceUuid, ScriptAssetId};
use somnium_script::lifecycle::LifecycleState;
use somnium_script::runtime::PhaseInput;
use somnium_script::snapshot::{InputSnapshot, TimeSnapshot};

// ── Harness ────────────────────────────────────────────────────────────

struct Adversary {
    host: ScriptHost,
    world: World,
    entity: Entity,
    instance: InstanceUuid,
    asset: ScriptAssetId,
    step: u64,
}

impl Adversary {
    /// Attach `text` to an entity. The script is expected to *compile* —
    /// what it does at run time is what each test is about.
    fn new(text: &str) -> Self {
        Self::with_budget(text, Budget::default())
    }

    fn with_budget(text: &str, budget: Budget) -> Self {
        let mut host = ScriptHost::new(budget);
        host.runtime_mut().set_failure_threshold(1);
        let asset = ScriptAssetId::mint();
        host.load_script(asset, "adversary.luau", text)
            .unwrap_or_else(|d| panic!("the case must compile to be worth running:\n{d}"));

        let mut world = World::new();
        let entity = world.spawn((Name::new("Target"), Transform::default(), ScriptSet::new()));
        let attachment = ScriptAttachment::new(asset);
        let instance = attachment.instance;
        world
            .get_mut::<ScriptSet>(entity)
            .unwrap()
            .attach(attachment);

        Self {
            host,
            world,
            entity,
            instance,
            asset,
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
            stepping: false,
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
        self.host.update(
            &mut self.world,
            time,
            &InputSnapshot::default(),
            &mut services,
        );
        self.step += 1;
    }

    fn errors(&mut self) -> Vec<String> {
        self.host
            .take_logs()
            .into_iter()
            .filter(|line| line.level == LogLevel::Error)
            .map(|line| line.message)
            .collect()
    }

    fn logs(&mut self) -> Vec<String> {
        self.host
            .take_logs()
            .into_iter()
            .map(|line| line.message)
            .collect()
    }
}

/// A script body that runs `body` once per fixed step.
fn fixed(body: &str) -> String {
    format!("return Script.define({{ onFixedUpdate = function(self, ctx, dt)\n{body}\nend }})")
}

// ── §4.6 row 1: infinite loop / runaway recursion ──────────────────────

#[test]
fn row_1_an_infinite_loop_is_interrupted_and_the_instance_is_disabled() {
    let mut adversary = Adversary::new(&fixed("while true do end"));
    let started = std::time::Instant::now();
    adversary.frame();
    assert!(
        started.elapsed() < std::time::Duration::from_millis(500),
        "the deadline, not the heat death of the universe, has to stop it"
    );
    assert!(!adversary.errors().is_empty());
    assert!(adversary.host.runtime().is_quarantined(adversary.instance));
    assert_eq!(
        adversary.host.state_of(adversary.instance),
        Some(LifecycleState::Disabled)
    );
}

#[test]
fn row_1_runaway_recursion_is_an_error_not_a_process_crash() {
    let mut adversary = Adversary::new(
        "local function f(n) return f(n + 1) end
         return Script.define({ onFixedUpdate = function(self, ctx, dt) f(0) end })",
    );
    adversary.frame();
    assert!(
        !adversary.errors().is_empty(),
        "the VM's own stack limit must surface as a script error"
    );
    // And the engine is still standing.
    adversary.frame();
}

// ── §4.6 row 2: allocation bomb ────────────────────────────────────────

#[test]
fn row_2_an_allocation_bomb_hits_the_memory_ceiling() {
    let budget = Budget {
        memory_bytes: 4 * 1024 * 1024,
        ..Budget::default()
    };
    let mut adversary = Adversary::with_budget(
        &fixed("local t = {} while true do table.insert(t, string.rep('x', 4096)) end"),
        budget,
    );
    adversary.frame();
    assert!(
        !adversary.errors().is_empty(),
        "the ceiling must be enforced"
    );
    adversary.frame();
}

// ── §4.6 row 3: filesystem, process, network ───────────────────────────

#[test]
fn row_3_nothing_that_reaches_outside_the_process_is_reachable() {
    // Not merely absent from the library set: *unreachable by name*, which
    // is the property that survives someone adding a library later.
    for name in [
        "io",
        "os",
        "package",
        "debug",
        "loadstring",
        "load",
        "loadfile",
        "dofile",
        "getfenv",
        "setfenv",
        "collectgarbage",
        "gcinfo",
        "print",
        "_G",
    ] {
        let mut adversary = Adversary::new(&fixed(&format!(
            "if type({name}) ~= 'nil' then ctx:log('REACHABLE') end"
        )));
        adversary.frame();
        assert!(
            !adversary.logs().contains(&"REACHABLE".to_string()),
            "`{name}` must not be reachable from a script"
        );
    }
}

#[test]
fn row_3_require_is_refused_at_compile_time_not_merely_absent() {
    // `require` is the one name on that list that a script *does* get —
    // ours, resolved against the asset graph. It is therefore guarded a
    // step earlier: any use the engine cannot follow by reading the source
    // is a compile error, which also rules out probing for it.
    let mut host = ScriptHost::new(Budget::default());
    for probe in [
        "local r = require\nreturn Script.define({})",
        "if type(require) ~= 'nil' then end\nreturn Script.define({})",
        "return Script.define({ escape = require })",
        "local n = 'io'\nlocal m = require(n)\nreturn Script.define({})",
    ] {
        let error = host
            .load_script(ScriptAssetId::mint(), "probe.luau", probe)
            .unwrap_err();
        assert!(
            error.to_string().contains("dependency graph"),
            "`{probe}` should have been refused: {error}"
        );
    }
    let _ = host.take_diagnostics();
}

// ── §4.6 row 4: external bytecode ──────────────────────────────────────

#[test]
fn row_4_a_cooked_artifact_from_another_runtime_is_never_loaded() {
    use somnium_core::script_cook::{CookedScript, hash_source};

    let source = "return Script.define({})";
    let artifact = CookedScript {
        fingerprint: "somnium-luau-1/deadbeefdeadbeef".into(),
        source_hash: hash_source(source),
        // Bytes that are emphatically not Luau bytecode. If a fingerprint
        // mismatch were ever treated as "load it anyway", this is what the
        // VM would be handed — and it does not validate its input.
        bytecode: vec![0xFF; 64],
    };
    assert!(
        !artifact.is_valid_for(source, "somnium-luau-1/0000000000000000"),
        "a different runtime's bytecode must be a cache miss"
    );
    assert!(
        !artifact.is_valid_for(
            "return Script.define({ schemaVersion = 2 })",
            &artifact.fingerprint
        ),
        "and so must bytecode for a source that has since changed"
    );
}

// ── §4.6 row 5: stale entity handle ────────────────────────────────────

#[test]
fn row_5_a_stale_handle_is_a_typed_rejection_not_a_panic() {
    let mut adversary = Adversary::new(
        "return Script.define({
            onFixedUpdate = function(self, ctx, dt)
                if self.saved == nil then
                    self.saved = ctx.entity
                    ctx:despawn(ctx.entity)
                else
                    -- The entity has been gone for a frame.
                    ctx:set(self.saved, 'somnium.Transform', 'translation', vector.create(1,2,3))
                end
            end,
        })",
    );
    adversary.frame();
    assert!(!adversary.world.is_alive(adversary.entity));
    // A stale handle must not take the engine with it.
    adversary.frame();
    adversary.frame();
}

#[test]
fn row_5_a_stale_handle_storm_costs_rejections_and_nothing_else() {
    let mut world = World::new();
    let corpses: Vec<Entity> = (0..256)
        .map(|_| world.spawn((Transform::default(),)))
        .collect();
    for entity in &corpses {
        world.despawn(*entity);
    }

    let registry = somnium_core::reflect_registry::component_registry();
    let mut buffer = somnium_script::command::CommandBuffer::new();
    let order = somnium_script::order::OrderKey::new(
        0,
        somnium_ecs::PersistentId::from_raw(1),
        InstanceUuid::from_raw(1),
    );
    buffer.begin(order);
    for entity in &corpses {
        buffer.push(somnium_script::command::ScriptCommand::Despawn { entity: *entity });
        buffer.push(somnium_script::command::ScriptCommand::SetFields {
            entity: *entity,
            component: somnium_ecs::StableId::new("somnium.Transform"),
            fields: somnium_ecs::ReflectObject::new(),
        });
    }
    buffer.end();

    let outcome =
        somnium_core::script_bridge::apply_commands(&mut world, &registry, buffer.drain_sorted());
    assert_eq!(outcome.applied, 0);
    assert_eq!(
        outcome.rejected.len(),
        512,
        "every one refused, none applied"
    );
    assert!(world.entities().next().is_none());
}

// ── §4.6 row 6: archetype invalidation / reentrancy ────────────────────

#[test]
fn row_6_structural_change_during_a_phase_cannot_invalidate_an_iteration() {
    // Twenty attachments, each spawning and despawning while every other
    // one is mid-phase. Nothing structural is allowed to happen until the
    // phase ends, which is what makes this safe rather than lucky.
    let mut host = ScriptHost::new(Budget::default());
    let asset = ScriptAssetId::mint();
    host.load_script(
        asset,
        "churn.luau",
        "return Script.define({
            onFixedUpdate = function(self, ctx, dt)
                self.t = ctx:spawn()
                if ctx.spawns ~= nil and self.prev ~= nil and ctx.spawns[self.prev] ~= nil then
                    ctx:despawn(ctx.spawns[self.prev])
                end
                self.prev = self.t
            end,
        })",
    )
    .unwrap();

    let mut world = World::new();
    for index in 0..20 {
        let entity = world.spawn((
            Name::new(&format!("Churn{index}")),
            Transform::default(),
            ScriptSet::new(),
        ));
        world
            .get_mut::<ScriptSet>(entity)
            .unwrap()
            .attach(ScriptAttachment::new(asset));
    }

    let mut services = HostServices::default();
    for step in 0..30 {
        let time = TimeSnapshot {
            step,
            ..TimeSnapshot::default()
        };
        let phase = PhaseInput {
            time,
            input: InputSnapshot::default(),
        };
        host.sync(&mut world, &phase, &mut services);
        host.fixed_update(&mut world, time, &InputSnapshot::default(), &mut services);
    }
    assert_eq!(host.runtime().live_instances(), 20);
}

// ── §4.6 row 8: an entity that despawns itself ─────────────────────────

#[test]
fn row_8_an_entity_that_despawns_itself_finishes_its_callback() {
    let mut adversary = Adversary::new(
        "return Script.define({
            onFixedUpdate = function(self, ctx, dt)
                ctx:despawn(ctx.entity)
                ctx:log('after')
            end,
            onDestroy = function(self, ctx) ctx:log('destroyed') end,
        })",
    );
    adversary.frame();
    let logs = adversary.logs();
    assert!(logs.contains(&"after".to_string()), "{logs:?}");
    assert!(logs.contains(&"destroyed".to_string()), "{logs:?}");
    assert_eq!(adversary.host.runtime().live_instances(), 0);
}

// ── §4.6 row 9: reload resource leak ───────────────────────────────────

#[test]
fn row_9_a_hundred_reload_cycles_retain_no_instances_or_resources() {
    let mut adversary = Adversary::new(&fixed("self.n = (self.n or 0) + 1"));
    adversary.frame();
    for cycle in 0..100 {
        adversary
            .host
            .reload_script(
                adversary.asset,
                "adversary.luau",
                format!(
                    "return Script.define({{ schemaVersion = {}, \
                     onFixedUpdate = function(self, ctx, dt) end }})",
                    cycle + 1
                ),
            )
            .unwrap();
        adversary.frame();
        assert_eq!(
            adversary.host.runtime().live_instances(),
            1,
            "cycle {cycle}"
        );
    }
    assert_eq!(adversary.host.runtime().owned_resources(), 0);
}

// ── §4.6 row 10: global contamination ──────────────────────────────────

#[test]
fn row_10_one_attachments_globals_are_invisible_to_another() {
    let mut host = ScriptHost::new(Budget::default());
    let writer = ScriptAssetId::mint();
    host.load_script(
        writer,
        "writer.luau",
        "smuggled = 'contraband'\nreturn Script.define({})",
    )
    .unwrap();
    let reader = ScriptAssetId::mint();
    host.load_script(
        reader,
        "reader.luau",
        "return Script.define({ onStart = function(self, ctx)
            ctx:log(if smuggled ~= nil then 'LEAKED' else 'isolated')
        end })",
    )
    .unwrap();

    let mut world = World::new();
    for asset in [writer, reader] {
        let entity = world.spawn((Transform::default(), ScriptSet::new()));
        world
            .get_mut::<ScriptSet>(entity)
            .unwrap()
            .attach(ScriptAttachment::new(asset));
    }
    let mut services = HostServices::default();
    host.sync(&mut world, &PhaseInput::default(), &mut services);

    let logs: Vec<String> = host.take_logs().into_iter().map(|l| l.message).collect();
    assert!(
        logs.contains(&"isolated".to_string()),
        "a global one attachment sets must not be visible to another: {logs:?}"
    );
}

#[test]
fn row_10_a_script_cannot_rewrite_a_shared_builtin_for_everyone_else() {
    let mut adversary = Adversary::new(&fixed(
        "local ok = pcall(function() math.floor = function() return 0 end end)
         ctx:log(if ok then 'REWROTE' else 'frozen')",
    ));
    adversary.frame();
    assert!(adversary.logs().contains(&"frozen".to_string()));
}

// ── §4.6 row 11/13: the capability manifest ────────────────────────────

#[test]
fn row_11_a_sandboxed_package_cannot_spawn_despawn_or_touch_physics() {
    let mut adversary = Adversary::new(
        "return Script.define({
            onFixedUpdate = function(self, ctx, dt)
                ctx:spawn()
                ctx:despawn(ctx.entity)
                ctx:applyForce(ctx.entity, vector.create(0, 100, 0))
                ctx:log('still running')
            end,
        })",
    );
    adversary
        .host
        .runtime_mut()
        .set_capabilities(adversary.asset, Capabilities::SANDBOXED);

    adversary.frame();
    let refused = adversary.host.take_rejections();
    assert_eq!(
        refused.len(),
        3,
        "spawn, despawn and applyForce must all be refused: {refused:?}"
    );
    assert!(refused.iter().all(|r| r.contains("capability")));
    assert!(
        adversary.world.is_alive(adversary.entity),
        "and the despawn it was refused must not have happened"
    );
    // The script itself is untouched — a capability refusal is not a fault.
    assert!(
        adversary
            .host
            .take_logs()
            .iter()
            .any(|l| l.message == "still running"),
        "a refused command must not fault the script"
    );
}

#[test]
fn row_11_a_project_script_keeps_the_capabilities_it_had() {
    let mut adversary = Adversary::new(&fixed("ctx:spawn()"));
    adversary
        .host
        .runtime_mut()
        .set_capabilities(adversary.asset, Capabilities::PROJECT);
    adversary.frame();
    assert!(adversary.host.take_rejections().is_empty());
    assert_eq!(adversary.world.entities().count(), 2);
}

// ── §4.6 row 12: one script starving the rest ──────────────────────────

#[test]
fn row_12_a_runaway_script_does_not_cost_its_peers_their_frame() {
    let mut host = ScriptHost::new(Budget::default());
    host.runtime_mut().set_failure_threshold(1);
    let spinner = ScriptAssetId::mint();
    host.load_script(spinner, "spin.luau", fixed("while true do end"))
        .unwrap();
    let worker = ScriptAssetId::mint();
    host.load_script(
        worker,
        "work.luau",
        "return Script.define({
            uses = { ['somnium.Transform'] = { 'translation' } },
            onFixedUpdate = function(self, ctx, dt)
                local t = ctx.self.transform
                t.translation = t.translation + vector.create(1, 0, 0)
            end,
        })",
    )
    .unwrap();

    let mut world = World::new();
    let mut moving = Entity::DANGLING;
    for asset in [spinner, worker] {
        let entity = world.spawn((Transform::default(), ScriptSet::new()));
        world
            .get_mut::<ScriptSet>(entity)
            .unwrap()
            .attach(ScriptAttachment::new(asset));
        if asset == worker {
            moving = entity;
        }
    }

    let mut services = HostServices::default();
    let time = TimeSnapshot::default();
    host.sync(&mut world, &PhaseInput::default(), &mut services);
    host.fixed_update(&mut world, time, &InputSnapshot::default(), &mut services);

    assert!(
        world.get::<Transform>(moving).unwrap().translation.x > 0.0,
        "the peer must have run and committed in the same phase"
    );
}

// ── §4.6 row 14: ABI drift ─────────────────────────────────────────────

#[test]
fn row_14_an_attachment_records_the_api_version_it_was_authored_against() {
    let attachment = ScriptAttachment::new(ScriptAssetId::mint());
    assert_eq!(
        attachment.api_version,
        somnium_script::attachment::CURRENT_API_VERSION
    );
    let mut host = ScriptHost::new(Budget::default());
    let asset = ScriptAssetId::mint();
    host.load_script(asset, "v.luau", "return Script.define({ apiVersion = 7 })")
        .unwrap();
    assert_eq!(
        host.runtime().asset_schema(asset).unwrap().api_version,
        7,
        "the version a module declares has to survive to where it can be compared"
    );
}

// ── The malformed-source corpus ────────────────────────────────────────

/// Sources that are wrong in every way the compiler and the descriptor
/// reader can be wrong. None may panic; all must produce a diagnostic.
const MALFORMED: &[&str] = &[
    "",
    "return",
    "return 1",
    "return nil",
    "return 'a string'",
    "return function() end",
    "return {}",
    "Script.define({})",
    "return Script.define(",
    "return Script.define({",
    "return Script.define({ onFixedUpdate = })",
    "return Script.define({ fields = 5 })",
    "return Script.define({ fields = { a = 5 } })",
    "return Script.define({ fields = { a = Field.number('not a number') } })",
    "return Script.define({ fields = { a = Field.nosuchkind(1) } })",
    "return Script.define({ uses = 5 })",
    "return Script.define({ uses = { [1] = 2 } })",
    "return Script.define({ onFixedUpdate = 5 })",
    "return Script.define({ apiVersion = 'one' })",
    "error('top level explosion')",
    "while true do end",
    "local t = {} t[1] = t return Script.define({ saveState = function() return t end })",
    "return Script.define({ requires = 1 })",
    "require()",
    "require(nil)",
    "require('')",
    "\u{feff}return Script.define({})",
    "return Script.define({ [1] = 2 })",
];

#[test]
fn the_malformed_corpus_produces_diagnostics_and_never_a_panic() {
    let mut host = ScriptHost::new(Budget::default());
    for (index, source) in MALFORMED.iter().enumerate() {
        let asset = ScriptAssetId::from_path(&format!("fuzz/{index}.luau"));
        // Whether it loads is not the assertion — some of these are legal
        // Luau that merely declares nothing. The assertion is that the
        // process is still here afterwards and the failure was reported
        // rather than swallowed.
        let outcome = host.load_script(asset, format!("fuzz/{index}.luau"), *source);
        if let Err(diagnostics) = outcome {
            assert!(
                !diagnostics.messages.is_empty(),
                "case {index} failed without saying why: {source:?}"
            );
        }
    }
    let _ = host.take_diagnostics();
}

#[test]
fn a_truncated_prefix_of_every_corpus_case_is_also_survivable() {
    // Half-written files are what a file watcher sees, so every prefix of
    // every case has to be as safe as the whole.
    let mut host = ScriptHost::new(Budget::default());
    let mut cases = 0;
    for source in MALFORMED {
        for cut in 1..=source.len() {
            if !source.is_char_boundary(cut) {
                continue;
            }
            let asset = ScriptAssetId::from_path(&format!("fuzz/prefix{cases}.luau"));
            let _ = host.load_script(asset, "fuzz.luau", &source[..cut]);
            cases += 1;
        }
    }
    assert!(cases > 100, "the corpus should be producing real coverage");
    let _ = host.take_diagnostics();
}

// ── Rust panics across the FFI boundary (§4.6 row 7) ───────────────────

#[test]
fn row_7_a_host_call_that_is_given_nonsense_returns_an_error_not_a_panic() {
    // Every one of these calls a host function with arguments of the wrong
    // shape. The boundary converts a bad argument into a script error; it
    // must never unwind into the C++ VM.
    let hits = Arc::new(AtomicU32::new(0));
    for body in [
        "ctx:set(ctx.entity, 'no.Such', 'field', 1)",
        "ctx:set(ctx.entity, 'somnium.Transform', 'nosuchfield', 1)",
        "ctx:set(ctx.entity, 'somnium.Mesh', 'index_count', 1)",
        "ctx:set(42, 'somnium.Transform', 'translation', vector.create(0,0,0))",
        "ctx:get(nil, 'somnium.Transform', 'translation')",
        "ctx:despawn('not an entity')",
        "ctx:applyForce(ctx.entity, 'not a vector')",
        "ctx:emit(nil)",
        "ctx:set(ctx.entity, 'somnium.Transform', 'translation', 0/0)",
    ] {
        let mut adversary = Adversary::new(&fixed(body));
        adversary.frame();
        // Some are rejected at the call, some at apply time. Either is a
        // pass; a panic or a silent success is not.
        let refused =
            !adversary.errors().is_empty() || !adversary.host.take_rejections().is_empty();
        assert!(refused, "`{body}` was accepted without complaint");
        hits.fetch_add(1, Ordering::Relaxed);
    }
    assert_eq!(hits.load(Ordering::Relaxed), 9);
}

#[test]
fn row_7_a_value_that_is_not_data_is_rendered_rather_than_smuggled() {
    // `ctx:log` deliberately accepts anything and renders it — going
    // through `tostring` would let a `__tostring` metamethod run arbitrary
    // script code inside the log path. What must never happen is a
    // coroutine or a function crossing into something *durable*.
    let mut adversary = Adversary::new(
        "return Script.define({
            onFixedUpdate = function(self, ctx, dt) ctx:log(coroutine.create(function() end)) end,
            saveState = function(self) return { escape = coroutine.create(function() end) } end,
        })",
    );
    adversary.frame();
    assert!(
        adversary
            .logs()
            .iter()
            .any(|line| line.contains("<thread>")),
        "logging one is fine; it is rendered, not converted"
    );
    assert!(
        adversary
            .host
            .runtime_mut()
            .export_state(adversary.instance)
            .is_err(),
        "saving one is not: a coroutine has no durable representation"
    );
}

#[test]
fn a_nan_never_reaches_a_component_or_physics() {
    let mut adversary = Adversary::new(&fixed(
        "ctx:applyForce(ctx.entity, vector.create(0/0, 0, 0))",
    ));
    adversary.frame();
    assert!(
        !adversary.host.take_rejections().is_empty(),
        "a NaN force is how a physics body vanishes to infinity three seconds later"
    );
}
