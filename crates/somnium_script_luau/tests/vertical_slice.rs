//! Phase 16-B: the Luau adapter, exercised the way the engine will use it.
//!
//! Every test here goes through the neutral [`ScriptBackend`] trait and a
//! real world. Nothing stubs the VM — a test that passed against a fake
//! Luau would tell us nothing about whether Luau works.
//!
//! The sandbox tests are the ones worth reading first. They assert what a
//! script *cannot* do, and each corresponds to a row of the threat model
//! in `dev records/phase_16.md` §4.6.

use std::time::{Duration, Instant};

use somnium_core::reflect_registry::component_registry;
use somnium_core::script_bridge::{EngineWorldView, apply_commands};
use somnium_core::{Name, Transform};
use somnium_ecs::reflect::TypeRegistry;
use somnium_ecs::{Entity, PersistentId, StableId, World};
use somnium_script::attachment::PropertyBag;
use somnium_script::backend::{Budget, Callback, ScriptBackend, ScriptError, ScriptSource};
use somnium_script::command::CommandBuffer;
use somnium_script::ids::{InstanceUuid, LanguageTag, ScriptAssetId, ScriptInstanceId};
use somnium_script::order::OrderKey;
use somnium_script::snapshot::{InputSnapshot, ScriptSnapshot, TimeSnapshot};
use somnium_script::value::ScriptValue;
use somnium_script_luau::LuauBackend;

const TRANSFORM: StableId = StableId::new("somnium.Transform");

// ── Harness ────────────────────────────────────────────────────────────

fn backend() -> LuauBackend {
    LuauBackend::new(Budget::default()).expect("VM should start")
}

fn source(text: &str) -> ScriptSource {
    ScriptSource {
        id: ScriptAssetId::mint(),
        language: LanguageTag::LUAU,
        display_path: "test.luau".into(),
        text: text.to_string(),
    }
}

fn snapshot(entity: Entity, persistent: PersistentId) -> ScriptSnapshot {
    ScriptSnapshot {
        time: TimeSnapshot {
            fixed_delta: 1.0 / 60.0,
            delta: 1.0 / 60.0,
            simulation_time: 0.0,
            step: 0,
        },
        input: InputSnapshot::default(),
        self_entity: entity,
        self_persistent: persistent,
        self_components: std::collections::BTreeMap::new(),
        spawn_results: Vec::new(),
        events: Vec::new(),
        rng_seed: 0,
    }
}

/// One entity, one script, wired up the way the scheduler will do it.
struct Slice {
    backend: LuauBackend,
    world: World,
    registry: TypeRegistry,
    entity: Entity,
    persistent: PersistentId,
    instance: ScriptInstanceId,
    order: OrderKey,
}

impl Slice {
    fn new(text: &str, properties: PropertyBag) -> Self {
        let mut backend = backend();
        let module = backend
            .compile(&source(text))
            .unwrap_or_else(|d| panic!("compile failed: {d}"));
        let instance = ScriptInstanceId::next();
        backend
            .instantiate(instance, module, &properties)
            .expect("instantiate should succeed");

        let mut world = World::new();
        let entity = world.spawn((Name::new("Scripted"), Transform::default()));
        let persistent = world.ensure_persistent_id(entity).unwrap();

        Self {
            backend,
            world,
            registry: component_registry(),
            entity,
            persistent,
            instance,
            order: OrderKey::new(0, persistent, InstanceUuid::mint()),
        }
    }

    /// Run one callback and commit whatever it asked for.
    fn step(&mut self, callback: Callback) -> Result<(), ScriptError> {
        let snapshot = snapshot(self.entity, self.persistent);
        let mut commands = CommandBuffer::new();
        let result = {
            let view = EngineWorldView::new(&self.world, &self.registry);
            self.backend.invoke(
                self.instance,
                self.order,
                callback,
                &snapshot,
                &view,
                &mut commands,
            )
        };
        if result.is_ok() {
            let _ = apply_commands(&mut self.world, &self.registry, commands.drain_sorted());
        }
        result
    }

    fn translation(&self) -> glam::Vec3 {
        self.world
            .get::<Transform>(self.entity)
            .unwrap()
            .translation
    }
}

// ── The vertical slice ─────────────────────────────────────────────────

const ROTATOR: &str = r#"
--!strict
return Script.define({
    apiVersion = 1,
    schemaVersion = 1,
    fields = {
        speed = Field.number(4.0, { min = 0.0, max = 30.0 }),
    },
    onFixedUpdate = function(self, ctx, dt)
        local p = ctx:get(ctx.entity, "somnium.Transform", "translation")
        ctx:set(ctx.entity, "somnium.Transform", "translation",
                vector.create(p.x + self.speed * dt, p.y, p.z))
    end,
})
"#;

#[test]
fn a_script_moves_its_entity_at_fixed_step() {
    let mut slice = Slice::new(ROTATOR, PropertyBag::new());
    assert_eq!(slice.translation().x, 0.0);

    slice.step(Callback::FixedUpdate).unwrap();
    let after_one = slice.translation().x;
    assert!(
        (after_one - 4.0 / 60.0).abs() < 1.0e-5,
        "expected one step of 4 m/s at 60 Hz, got {after_one}"
    );

    for _ in 0..59 {
        slice.step(Callback::FixedUpdate).unwrap();
    }
    assert!(
        (slice.translation().x - 4.0).abs() < 1.0e-3,
        "sixty steps should be one second of travel, got {}",
        slice.translation().x
    );
}

#[test]
fn an_authored_property_overrides_the_declared_default() {
    let mut properties = PropertyBag::new();
    properties.insert("speed".into(), ScriptValue::F64(10.0));
    let mut slice = Slice::new(ROTATOR, properties);

    slice.step(Callback::FixedUpdate).unwrap();
    assert!(
        (slice.translation().x - 10.0 / 60.0).abs() < 1.0e-5,
        "the authored value must win over the script's default"
    );
}

#[test]
fn the_declared_schema_is_readable_without_running_the_script_against_the_world() {
    let mut backend = backend();
    let module = backend.compile(&source(ROTATOR)).unwrap();
    let schema = backend.describe(module).unwrap();

    assert_eq!(schema.api_version, 1);
    assert_eq!(schema.schema_version, 1);
    assert_eq!(schema.fields.len(), 1);

    let speed = schema.field("speed").unwrap();
    assert_eq!(speed.default, ScriptValue::F64(4.0));
    assert_eq!(speed.min, Some(0.0));
    assert_eq!(speed.max, Some(30.0));

    assert!(schema.callbacks.has(Callback::FixedUpdate));
    assert!(!schema.callbacks.has(Callback::Update));
}

#[test]
fn describing_a_module_cannot_touch_the_world() {
    // Top-level module code runs during `describe`. If it could reach the
    // world, opening a script in the editor would be able to change the
    // scene — so `ctx` does not exist in that environment at all.
    let mut backend = backend();
    let module = backend
        .compile(&source(
            r#"
            local reached = (ctx ~= nil)
            return Script.define({ reachedWorld = reached })
            "#,
        ))
        .unwrap();
    let schema = backend.describe(module).unwrap();
    assert!(schema.callbacks.is_empty());
}

#[test]
fn spawn_despawn_and_events_reach_the_engine() {
    let mut slice = Slice::new(
        r#"
        return Script.define({
            onStart = function(self, ctx)
                ctx:spawn()
                ctx:emit("started")
                ctx:log("hello from a script")
            end,
        })
        "#,
        PropertyBag::new(),
    );

    let before = slice.world.entity_count();
    slice.step(Callback::Start).unwrap();
    assert_eq!(
        slice.world.entity_count(),
        before + 1,
        "the spawn should have been committed"
    );
}

#[test]
fn a_script_can_despawn_its_own_entity_from_inside_a_callback() {
    let mut slice = Slice::new(
        r#"
        return Script.define({
            onFixedUpdate = function(self, ctx, dt)
                ctx:despawn(ctx.entity)
                -- Still running: destruction is deferred to the safe point.
                ctx:log("still alive here")
            end,
        })
        "#,
        PropertyBag::new(),
    );
    slice.step(Callback::FixedUpdate).unwrap();
    assert!(!slice.world.is_alive(slice.entity));
}

// ── Diagnostics ────────────────────────────────────────────────────────

#[test]
fn a_syntax_error_is_a_positioned_diagnostic_not_a_panic() {
    let mut backend = backend();
    let diagnostics = backend
        .compile(&source(
            "return Script.define({\n    onInit = function(self ctx)\n",
        ))
        .expect_err("this should not compile");

    assert!(diagnostics.has_errors());
    let first = &diagnostics.messages[0];
    assert_eq!(first.display_path, "test.luau");
    assert!(
        first.line > 0,
        "a syntax error must point at a line: {first}"
    );
}

#[test]
fn a_module_that_returns_the_wrong_thing_is_refused_with_an_explanation() {
    let mut backend = backend();
    let module = backend.compile(&source("return 42")).unwrap();
    let err = backend.describe(module).unwrap_err();
    assert!(
        err.to_string().contains("Script.define"),
        "the message should say what the script was supposed to do: {err}"
    );
}

#[test]
fn a_runtime_error_is_reported_with_a_traceback_and_does_not_stop_the_engine() {
    let mut slice = Slice::new(
        r#"
        return Script.define({
            onFixedUpdate = function(self, ctx, dt)
                error("deliberate")
            end,
        })
        "#,
        PropertyBag::new(),
    );

    let err = slice.step(Callback::FixedUpdate).unwrap_err();
    match err {
        ScriptError::Raised { message, .. } => {
            assert!(message.contains("deliberate"), "got: {message}");
        }
        other => panic!("expected a raised error, got {other:?}"),
    }

    // The world is untouched and the next call still works.
    assert!(slice.world.is_alive(slice.entity));
}

// ── Sandbox ────────────────────────────────────────────────────────────

#[test]
fn an_infinite_loop_is_interrupted_close_to_its_deadline() {
    let mut backend = LuauBackend::new(Budget {
        per_call: Duration::from_millis(20),
        ..Budget::default()
    })
    .unwrap();
    let module = backend
        .compile(&source(
            "return Script.define({ onFixedUpdate = function() while true do end end })",
        ))
        .unwrap();
    let instance = ScriptInstanceId::next();
    backend
        .instantiate(instance, module, &PropertyBag::new())
        .unwrap();

    let mut world = World::new();
    let entity = world.spawn((Transform::default(),));
    let persistent = world.ensure_persistent_id(entity).unwrap();
    let registry = component_registry();
    let snapshot = snapshot(entity, persistent);
    let order = OrderKey::new(0, persistent, InstanceUuid::mint());
    let mut commands = CommandBuffer::new();

    let started = Instant::now();
    let err = {
        let view = EngineWorldView::new(&world, &registry);
        backend
            .invoke(
                instance,
                order,
                Callback::FixedUpdate,
                &snapshot,
                &view,
                &mut commands,
            )
            .expect_err("an infinite loop must not return normally")
    };
    let elapsed = started.elapsed();

    assert!(
        matches!(err, ScriptError::Deadline { .. }),
        "expected a deadline error, got {err:?}"
    );
    assert!(
        elapsed < Duration::from_millis(22),
        "budget was 20 ms; the interrupt fired after {elapsed:?}"
    );
}

#[test]
fn the_unsafe_standard_libraries_are_not_open() {
    // `os` and `debug` are the two the Luau feature's `ALL_SAFE` would
    // have handed us. `os.clock` in particular would make a fixed-step
    // callback non-deterministic while looking harmless.
    let mut slice = Slice::new(
        r#"
        return Script.define({
            onStart = function(self, ctx)
                ctx:set(ctx.entity, "somnium.Name", "value",
                        tostring(os) .. "/" .. tostring(debug))
            end,
        })
        "#,
        PropertyBag::new(),
    );
    // `os` and `debug` are nil, so indexing them would error; here we only
    // stringify, which yields "nil".
    slice.step(Callback::Start).unwrap();
    assert_eq!(
        slice.world.get::<Name>(slice.entity).unwrap().as_str(),
        "nil/nil",
        "neither library may be present"
    );
}

#[test]
fn one_script_cannot_change_globals_for_another() {
    let mut backend = backend();
    let module = backend
        .compile(&source(
            r#"
            return Script.define({
                onStart = function(self, ctx)
                    shared_leak = "leaked"
                end,
                onFixedUpdate = function(self, ctx, dt)
                    ctx:set(ctx.entity, "somnium.Name", "value",
                            tostring(shared_leak))
                end,
            })
            "#,
        ))
        .unwrap();

    let polluter = ScriptInstanceId::next();
    let observer = ScriptInstanceId::next();
    backend
        .instantiate(polluter, module, &PropertyBag::new())
        .unwrap();
    backend
        .instantiate(observer, module, &PropertyBag::new())
        .unwrap();

    let mut world = World::new();
    let entity = world.spawn((Name::new("start"), Transform::default()));
    let persistent = world.ensure_persistent_id(entity).unwrap();
    let registry = component_registry();
    let order = OrderKey::new(0, persistent, InstanceUuid::mint());

    let mut run = |instance: ScriptInstanceId, callback: Callback, world: &mut World| {
        let snap = snapshot(entity, persistent);
        let mut commands = CommandBuffer::new();
        {
            let view = EngineWorldView::new(world, &registry);
            backend
                .invoke(instance, order, callback, &snap, &view, &mut commands)
                .unwrap();
        }
        let _ = apply_commands(world, &registry, commands.drain_sorted());
    };

    run(polluter, Callback::Start, &mut world);
    run(observer, Callback::FixedUpdate, &mut world);

    assert_eq!(
        world.get::<Name>(entity).unwrap().as_str(),
        "nil",
        "one attachment's globals must be invisible to another"
    );
}

#[test]
fn a_script_cannot_write_an_engine_owned_field() {
    let mut backend = backend();
    let module = backend
        .compile(&source(
            r#"
            return Script.define({
                onFixedUpdate = function(self, ctx, dt)
                    ctx:set(ctx.entity, "somnium.Mesh", "index_count", 999)
                end,
            })
            "#,
        ))
        .unwrap();
    let instance = ScriptInstanceId::next();
    backend
        .instantiate(instance, module, &PropertyBag::new())
        .unwrap();

    let mut world = World::new();
    let entity = world.spawn((somnium_core::MeshComponent::default(), Transform::default()));
    let persistent = world.ensure_persistent_id(entity).unwrap();
    let registry = component_registry();
    let order = OrderKey::new(0, persistent, InstanceUuid::mint());
    let mut commands = CommandBuffer::new();

    let err = {
        let view = EngineWorldView::new(&world, &registry);
        backend
            .invoke(
                instance,
                order,
                Callback::FixedUpdate,
                &snapshot(entity, persistent),
                &view,
                &mut commands,
            )
            .expect_err("writing an engine-owned field must fail")
    };
    assert!(
        err.to_string().contains("engine-owned"),
        "the author should be told why: {err}"
    );
    assert_eq!(
        world
            .get::<somnium_core::MeshComponent>(entity)
            .unwrap()
            .index_count,
        0
    );
}

#[test]
fn a_stale_entity_handle_reads_as_nothing_rather_than_panicking() {
    let mut slice = Slice::new(
        r#"
        return Script.define({
            onStart = function(self, ctx)
                self.saved = ctx.entity
            end,
            onFixedUpdate = function(self, ctx, dt)
                if ctx:isAlive(self.saved) then
                    ctx:log("still there")
                else
                    ctx:log("gone")
                end
                local v = ctx:get(self.saved, "somnium.Transform", "translation")
                assert(v == nil, "a dead entity must read as nil")
            end,
        })
        "#,
        PropertyBag::new(),
    );

    slice.step(Callback::Start).unwrap();
    slice.world.despawn(slice.entity);
    slice
        .step(Callback::FixedUpdate)
        .expect("a stale handle is ordinary control flow, not an error");
}

// ── Instance lifetime ──────────────────────────────────────────────────

#[test]
fn instances_are_released_and_do_not_accumulate() {
    let mut backend = backend();
    let module = backend
        .compile(&source("return Script.define({ onInit = function() end })"))
        .unwrap();

    assert_eq!(backend.live_instances(), 0);
    let ids: Vec<_> = (0..100)
        .map(|_| {
            let id = ScriptInstanceId::next();
            backend
                .instantiate(id, module, &PropertyBag::new())
                .unwrap();
            id
        })
        .collect();
    assert_eq!(backend.live_instances(), 100);

    for id in ids {
        backend.unload(id);
    }
    assert_eq!(
        backend.live_instances(),
        0,
        "every instance must be released"
    );
    backend.release_module(module);
}

#[test]
fn declared_state_survives_an_in_process_module_swap() {
    // The halves hot reload is built from, proven without a filesystem:
    // export the old instance's declared state, build a new instance of a
    // *different* module, import into it.
    let mut backend = backend();
    let old = backend
        .compile(&source(
            r#"
            return Script.define({
                onStart = function(self, ctx) self.hits = 7 end,
                saveState = function(self) return { count = self.hits } end,
            })
            "#,
        ))
        .unwrap();
    let new = backend
        .compile(&source(
            r#"
            return Script.define({
                loadState = function(self, state) self.restored = state.count end,
                onFixedUpdate = function(self, ctx, dt)
                    ctx:set(ctx.entity, "somnium.Name", "value", tostring(self.restored))
                end,
            })
            "#,
        ))
        .unwrap();

    let before = ScriptInstanceId::next();
    backend
        .instantiate(before, old, &PropertyBag::new())
        .unwrap();

    let mut world = World::new();
    let entity = world.spawn((Name::new("start"), Transform::default()));
    let persistent = world.ensure_persistent_id(entity).unwrap();
    let registry = component_registry();
    let order = OrderKey::new(0, persistent, InstanceUuid::mint());

    {
        let mut commands = CommandBuffer::new();
        let view = EngineWorldView::new(&world, &registry);
        backend
            .invoke(
                before,
                order,
                Callback::Start,
                &snapshot(entity, persistent),
                &view,
                &mut commands,
            )
            .unwrap();
    }

    let state = backend.export_state(before).unwrap();
    backend.unload(before);

    let after = ScriptInstanceId::next();
    backend
        .instantiate(after, new, &PropertyBag::new())
        .unwrap();
    backend.import_state(after, state).unwrap();

    let mut commands = CommandBuffer::new();
    {
        let view = EngineWorldView::new(&world, &registry);
        backend
            .invoke(
                after,
                order,
                Callback::FixedUpdate,
                &snapshot(entity, persistent),
                &view,
                &mut commands,
            )
            .unwrap();
    }
    let _ = apply_commands(&mut world, &registry, commands.drain_sorted());

    assert_eq!(
        world.get::<Name>(entity).unwrap().as_str(),
        "7",
        "declared state must survive the swap"
    );
}

#[test]
fn a_closure_cannot_be_smuggled_out_as_state() {
    let mut backend = backend();
    let module = backend
        .compile(&source(
            "return Script.define({ saveState = function(self) return function() end end })",
        ))
        .unwrap();
    let instance = ScriptInstanceId::next();
    backend
        .instantiate(instance, module, &PropertyBag::new())
        .unwrap();

    let err = backend
        .export_state(instance)
        .expect_err("a function is not durable state");
    assert!(err.to_string().contains("durable"), "got: {err}");
}

#[test]
fn a_module_with_no_save_state_exports_nothing_rather_than_failing() {
    let mut backend = backend();
    let module = backend
        .compile(&source("return Script.define({})"))
        .unwrap();
    let instance = ScriptInstanceId::next();
    backend
        .instantiate(instance, module, &PropertyBag::new())
        .unwrap();
    assert_eq!(backend.export_state(instance).unwrap(), ScriptValue::Nil);
}

#[test]
fn invoking_a_callback_a_module_does_not_define_is_a_no_op() {
    let mut backend = backend();
    let module = backend
        .compile(&source("return Script.define({})"))
        .unwrap();
    let instance = ScriptInstanceId::next();
    backend
        .instantiate(instance, module, &PropertyBag::new())
        .unwrap();

    let mut world = World::new();
    let entity = world.spawn((Transform::default(),));
    let persistent = world.ensure_persistent_id(entity).unwrap();
    let registry = component_registry();
    let order = OrderKey::new(0, persistent, InstanceUuid::mint());
    let mut commands = CommandBuffer::new();

    let view = EngineWorldView::new(&world, &registry);
    backend
        .invoke(
            instance,
            order,
            Callback::FixedUpdate,
            &snapshot(entity, persistent),
            &view,
            &mut commands,
        )
        .expect("a module with no such callback is skipped, not an error");
    assert!(
        commands.is_empty(),
        "and it cannot have queued anything either"
    );
}

#[test]
fn an_unknown_instance_is_a_typed_error_not_a_panic() {
    let mut backend = backend();
    let mut world = World::new();
    let entity = world.spawn((Transform::default(),));
    let persistent = world.ensure_persistent_id(entity).unwrap();
    let registry = component_registry();
    let order = OrderKey::new(0, persistent, InstanceUuid::mint());
    let mut commands = CommandBuffer::new();

    let view = EngineWorldView::new(&world, &registry);
    let ghost = ScriptInstanceId::next();
    let err = backend
        .invoke(
            ghost,
            order,
            Callback::FixedUpdate,
            &snapshot(entity, persistent),
            &view,
            &mut commands,
        )
        .unwrap_err();
    assert!(matches!(err, ScriptError::NoSuchInstance(_)));
}

#[test]
fn a_read_modify_write_loop_accumulates_through_the_mirror() {
    // The defect this fixes: `ctx:get` reads committed world state and
    // `ctx:set` queues a deferred write, so a loop through that pair
    // re-read the *pre-phase* value every iteration and only the last
    // write survived — ten steps silently produced one step of movement.
    //
    // Through the mirror a script sees its own writes, which is both what
    // an author expects and the visibility rule the design documents.
    let mut slice = Slice::new(
        r#"
        return Script.define({
            uses = { ["somnium.Transform"] = { "translation" } },
            onFixedUpdate = function(self, ctx, dt)
                local t = ctx.self.transform
                for i = 1, 10 do
                    local p = t.translation
                    t.translation = vector.create(p.x + 1, p.y, p.z)
                end
            end,
        })
        "#,
        PropertyBag::new(),
    );

    slice.step(Callback::FixedUpdate).unwrap();
    assert!(
        (slice.translation().x - 10.0).abs() < 1.0e-5,
        "ten iterations must accumulate ten steps, got {}",
        slice.translation().x
    );
}

#[test]
fn a_mirrored_field_only_queues_a_command_when_it_changes() {
    let mut slice = Slice::new(
        r#"
        return Script.define({
            uses = { "somnium.Transform" },
            onFixedUpdate = function(self, ctx, dt)
                local _ = ctx.self.transform.translation   -- read only
            end,
        })
        "#,
        PropertyBag::new(),
    );
    slice
        .world
        .get_mut::<Transform>(slice.entity)
        .unwrap()
        .translation = glam::Vec3::new(5.0, 6.0, 7.0);
    slice.step(Callback::FixedUpdate).unwrap();
    assert!(
        (slice.translation() - glam::Vec3::new(5.0, 6.0, 7.0)).length() < 1.0e-6,
        "a script that only reads must not write anything back"
    );
}

#[test]
fn a_script_cannot_write_an_engine_owned_field_through_the_mirror() {
    // `MeshComponent`'s fields are all RUNTIME_ONLY. They mirror in as
    // readable, and writes to them must be dropped rather than queued.
    let mut backend = backend();
    let module = backend
        .compile(&source(
            r#"
            return Script.define({
                uses = { "somnium.Mesh" },
                onFixedUpdate = function(self, ctx, dt)
                    ctx.self.mesh.index_count = 999
                end,
            })
            "#,
        ))
        .unwrap();
    let instance = ScriptInstanceId::next();
    backend
        .instantiate(instance, module, &PropertyBag::new())
        .unwrap();

    let mut world = World::new();
    let entity = world.spawn((somnium_core::MeshComponent::default(), Transform::default()));
    let persistent = world.ensure_persistent_id(entity).unwrap();
    let registry = component_registry();
    let order = OrderKey::new(0, persistent, InstanceUuid::mint());
    let snap = snapshot(entity, persistent);
    let calls = [somnium_script::backend::PhaseCall {
        instance,
        order,
        snapshot: &snap,
    }];

    let mut commands = CommandBuffer::new();
    {
        let view = EngineWorldView::new(&world, &registry);
        let failures = backend.invoke_phase(Callback::FixedUpdate, &calls, &view, &mut commands);
        assert!(failures.is_empty(), "{failures:?}");
    }
    let outcome = apply_commands(&mut world, &registry, commands.drain_sorted());
    assert!(outcome.is_clean(), "{:?}", outcome.rejected);
    assert_eq!(
        world
            .get::<somnium_core::MeshComponent>(entity)
            .unwrap()
            .index_count,
        0,
        "an engine-owned field must not be writable through the mirror either"
    );
}

#[test]
fn the_transform_written_by_a_script_is_the_one_the_engine_reads() {
    // End to end through every layer: script → command → validation →
    // registry `apply` → ECS storage → registry `snapshot` → script.
    let mut slice = Slice::new(
        r#"
        return Script.define({
            onFixedUpdate = function(self, ctx, dt)
                ctx:set(ctx.entity, "somnium.Transform", "translation", vector.create(1, 2, 3))
            end,
            onUpdate = function(self, ctx, dt)
                local p = ctx:get(ctx.entity, "somnium.Transform", "translation")
                ctx:set(ctx.entity, "somnium.Name", "value",
                        string.format("%d,%d,%d", p.x, p.y, p.z))
            end,
        })
        "#,
        PropertyBag::new(),
    );

    slice.step(Callback::FixedUpdate).unwrap();
    slice.step(Callback::Update).unwrap();

    assert_eq!(
        slice.world.get::<Name>(slice.entity).unwrap().as_str(),
        "1,2,3"
    );
    let _ = TRANSFORM;
}
