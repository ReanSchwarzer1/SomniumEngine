//! # Somnium Script — Luau
//!
//! The **only** crate in the workspace that knows what Luau is.
//!
//! Everything above it sees [`somnium_script`]'s neutral types. That is
//! not a convention, it is the exit strategy: replacing the language means
//! writing a sibling of this crate, not editing the ECS, the scene format,
//! the undo stack or a line of gameplay API.
//!
//! ## The sandbox, and one trap worth naming
//!
//! `StdLib::ALL_SAFE` is `u32::MAX` **under the `luau` feature** — it
//! includes `os` and `debug`. `Lua::new()` uses it. So the obvious way to
//! create a state gives scripts a wall clock and the debug library, and it
//! looks safe while doing it. This crate never calls `Lua::new()`; it
//! opens [`SAFE_LIBS`] explicitly.
//!
//! Beyond library selection:
//!
//! * `Lua::sandbox(true)` freezes the global table and the shared builtins
//!   after the engine API is registered, so one script cannot rewrite
//!   `math.floor` for every other script;
//! * each attachment gets its **own environment table**, so globals a
//!   script sets are private to it;
//! * an interrupt enforces a wall-clock deadline (see [`deadline`]);
//! * the VM has a memory ceiling.
//!
//! `os` is excluded rather than trimmed. Luau's `os` is already restricted
//! to `time`, `clock` and `date`, but all three are wall-clock reads, and
//! a fixed-step callback that can read the wall clock is not deterministic
//! no matter how harmless the function looks.
//!
//! ## Bytecode
//!
//! Source is compiled once, in-process, by the embedded compiler, and the
//! resulting bytecode is reused to instantiate. That is safe precisely
//! because we produced it: Luau's VM assumes bytecode came from its own
//! compiler and does not validate it. Bytecode from anywhere else — a
//! file, a mod, a network — is never loaded.

#![warn(missing_docs)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod convert;
pub mod deadline;
pub mod host;

use std::collections::HashMap;

use mlua::chunk::{ChunkMode, Compiler};
use mlua::{Function, Lua, LuaOptions, StdLib, Table, Value, VmState};
use somnium_script::attachment::PropertyBag;
use somnium_script::backend::{
    PhaseCall,
    Budget, Callback, CallbackMask, CompiledModule, Diagnostic, Diagnostics, ScriptBackend,
    ScriptError, ScriptFieldSchema, ScriptSchema, ScriptSource, Severity,
};
use somnium_script::command::CommandBuffer;
use somnium_script::ids::{LanguageTag, ScriptAssetId, ScriptInstanceId};
use somnium_script::snapshot::{ScriptSnapshot, WorldView};
use somnium_script::value::{FieldType, ScriptValue};

use crate::deadline::Deadline;

/// The standard libraries a Somnium script may use.
///
/// Deliberately **not** `StdLib::ALL_SAFE`, which is `u32::MAX` under
/// Luau and therefore includes `os` and `debug`. See the module docs.
///
/// A function rather than a constant only because `StdLib`'s `BitOr` is
/// not `const`.
#[must_use]
pub fn safe_libs() -> StdLib {
    StdLib::COROUTINE
        | StdLib::TABLE
        | StdLib::STRING
        | StdLib::UTF8
        | StdLib::BIT
        | StdLib::MATH
        | StdLib::BUFFER
        | StdLib::VECTOR
}

/// Create a Luau state with the safe library set and a memory ceiling.
///
/// Exposed for the crate's own tests; the backend builds its own.
///
/// # Errors
///
/// If the state cannot be created or the memory limit cannot be set.
pub fn new_sandboxed_state(memory_bytes: usize) -> mlua::Result<Lua> {
    let lua = Lua::new_with(safe_libs(), LuaOptions::default())?;
    lua.set_memory_limit(memory_bytes)?;
    Ok(lua)
}

/// A module this backend has compiled.
struct ModuleEntry {
    /// Which asset this came from. Read back by diagnostics.
    #[allow(dead_code)]
    asset: ScriptAssetId,
    display_path: String,
    /// Bytecode from our own compiler. Never from anywhere else.
    bytecode: Vec<u8>,
    /// Filled by the first `describe`, and reused by every instantiate.
    ///
    /// Instantiation needs it: an attachment that overrides none of a
    /// script's properties must still get the script's *declared*
    /// defaults, and those live in the schema. Caching it also means a
    /// thousand attachments of one asset evaluate the descriptor for its
    /// schema once rather than a thousand times.
    schema: Option<ScriptSchema>,
}

/// A live script object.
struct Instance {
    module: u64,
    /// The table the module returned from `Script.define`.
    descriptor: Table,
    /// Private global environment for this attachment.
    #[allow(dead_code)]
    env: Table,
    /// Entry points resolved **once**, at instantiation.
    ///
    /// Looking a callback up by name on every call is the measured
    /// anti-pattern this array exists to avoid; at a thousand scripted
    /// entities it is a string hash and a table lookup per entity per
    /// phase, for an answer that cannot change until the next reload.
    callbacks: [Option<Function>; 10],
    /// The entity handle this instance hands to `ctx.entity`, cached.
    ///
    /// An attachment's entity does not change, but the handle is userdata
    /// and constructing one allocates. Rebuilding it per callback cost
    /// about 0.5 µs against a 0.5 µs total budget — half the frame's
    /// script time spent re-wrapping a number that had not moved.
    entity_handle: Option<(somnium_ecs::Entity, mlua::AnyUserData)>,
}

/// The Luau backend.
pub struct LuauBackend {
    lua: Lua,
    modules: HashMap<u64, ModuleEntry>,
    instances: HashMap<ScriptInstanceId, Instance>,
    next_handle: u64,
    budget: Budget,
    deadline: Deadline,
}

impl LuauBackend {
    /// Build a backend for one trust domain.
    ///
    /// One state per trust domain, not one global state and not one per
    /// instance: a global state is the anti-pattern (every script sharing
    /// mutable globals), and per-instance states would multiply the
    /// runtime's fixed cost by the entity count for isolation that
    /// per-attachment environments already provide.
    ///
    /// # Errors
    ///
    /// If the VM cannot be created or the engine API cannot be installed.
    pub fn new(budget: Budget) -> Result<Self, ScriptError> {
        let lua = new_sandboxed_state(budget.memory_bytes).map_err(host::to_script_error)?;

        // The API has to be installed *before* sandboxing, because
        // sandboxing is what makes the global table read-only.
        host::install_api(&lua).map_err(host::to_script_error)?;

        let deadline = Deadline::new();
        let interrupt_deadline = deadline.clone();
        lua.set_interrupt(move |_| {
            if interrupt_deadline.should_stop() {
                // An error from the interrupt is what actually unwinds a
                // script that has no intention of returning.
                return Err(mlua::Error::RuntimeError(
                    "script exceeded its time budget".into(),
                ));
            }
            Ok(VmState::Continue)
        });

        lua.sandbox(true).map_err(host::to_script_error)?;

        Ok(Self {
            lua,
            modules: HashMap::new(),
            instances: HashMap::new(),
            next_handle: 1,
            budget,
            deadline,
        })
    }

    /// Run `body` with the deadline armed, converting a trip into the
    /// typed error rather than an anonymous runtime error.
    fn with_deadline<R>(
        &self,
        budget: std::time::Duration,
        body: impl FnOnce() -> mlua::Result<R>,
    ) -> Result<R, ScriptError> {
        self.deadline.arm(budget);
        let result = body();
        self.deadline.disarm();
        match result {
            Ok(value) => Ok(value),
            Err(err) if self.deadline.tripped() => {
                let _ = err;
                Err(ScriptError::Deadline { budget })
            }
            Err(err) => Err(host::to_script_error(err)),
        }
    }

    /// Load a module's bytecode into a fresh function bound to `env`.
    ///
    /// A function carries its environment and Luau has no `setfenv`, so
    /// per-instance isolation means building a new function per instance.
    /// That is why bytecode is cached rather than source: this path runs
    /// once per attachment, and re-compiling the text each time would put
    /// the parser on the instantiation path.
    fn load_with_env(&self, module: &ModuleEntry, env: Table) -> mlua::Result<Function> {
        self.lua
            .load(&module.bytecode[..])
            .set_name(&module.display_path)
            .set_mode(ChunkMode::Binary)
            .set_environment(env)
            .into_function()
    }

    /// Evaluate a module and return the table it declared.
    fn evaluate(&self, module: &ModuleEntry, env: Table) -> mlua::Result<Table> {
        let function = self.load_with_env(module, env)?;
        let returned: Value = function.call(())?;
        match returned {
            Value::Table(table) => Ok(table),
            other => Err(mlua::Error::RuntimeError(format!(
                "a script must `return Script.define{{...}}`; this one returned {}",
                other.type_name()
            ))),
        }
    }

    fn module(&self, module: CompiledModule) -> Result<&ModuleEntry, ScriptError> {
        self.modules
            .get(&module.handle)
            .ok_or_else(|| ScriptError::HostRejected {
                message: format!("module {} is not loaded", module.handle),
            })
    }

    /// The cached `ctx.entity` userdata for an instance, built on first
    /// use and rebuilt only if the attachment moves to another entity.
    fn entity_handle(
        &mut self,
        id: ScriptInstanceId,
        entity: somnium_ecs::Entity,
    ) -> Option<mlua::AnyUserData> {
        let fresh = match self.instances.get(&id)?.entity_handle {
            Some((cached, ref handle)) if cached == entity => return Some(handle.clone()),
            _ => self
                .lua
                .create_userdata(crate::convert::EntityHandle(entity))
                .ok()?,
        };
        let instance = self.instances.get_mut(&id)?;
        instance.entity_handle = Some((entity, fresh.clone()));
        Some(fresh)
    }

    fn instance(&self, id: ScriptInstanceId) -> Result<&Instance, ScriptError> {
        self.instances
            .get(&id)
            .ok_or(ScriptError::NoSuchInstance(id))
    }
}

impl ScriptBackend for LuauBackend {
    fn language(&self) -> LanguageTag {
        LanguageTag::LUAU
    }

    fn compile(&mut self, source: &ScriptSource) -> Result<CompiledModule, Diagnostics> {
        // `Compiler::new()` defaults to optimisation level 1 and debug
        // level 1, which is what gives tracebacks real line numbers.
        // Debug information is kept even in release: a stack trace that
        // says "?" is worth nothing to whoever has to fix the script.
        let compiler = Compiler::new();
        let bytecode = compiler.compile(&source.text).map_err(|err| {
            let mut diagnostics = Diagnostics::default();
            diagnostics.push(host::diagnostic_from_error(&err, source));
            diagnostics
        })?;

        let handle = self.next_handle;
        self.next_handle += 1;
        self.modules.insert(
            handle,
            ModuleEntry {
                asset: source.id,
                display_path: source.display_path.clone(),
                bytecode,
                schema: None,
            },
        );
        Ok(CompiledModule {
            asset: source.id,
            language: LanguageTag::LUAU,
            handle,
        })
    }

    fn describe(&mut self, module: CompiledModule) -> Result<ScriptSchema, Diagnostics> {
        let entry = self.module(module).map_err(|err| {
            host::single_diagnostic(module.asset, "<unknown>", &err.to_string())
        })?;

        // The descriptor runs in an environment with **no world access**.
        // Opening a script in the editor, or merely importing one to read
        // its property list, must not be able to change the scene — so the
        // API surface it can see does not include the means.
        let env = host::describe_environment(&self.lua).map_err(|err| {
            host::single_diagnostic(module.asset, &entry.display_path, &err.to_string())
        })?;

        let descriptor = self
            .with_deadline(self.budget.per_call, || self.evaluate(entry, env))
            .map_err(|err| {
                host::single_diagnostic(module.asset, &entry.display_path, &err.to_string())
            })?;

        let schema = host::schema_from_descriptor(&descriptor).map_err(|message| {
            host::single_diagnostic(module.asset, &entry.display_path, &message)
        })?;
        if let Some(entry) = self.modules.get_mut(&module.handle) {
            entry.schema = Some(schema.clone());
        }
        Ok(schema)
    }

    fn instantiate(
        &mut self,
        id: ScriptInstanceId,
        module: CompiledModule,
        properties: &PropertyBag,
    ) -> Result<(), ScriptError> {
        // A script's declared defaults are part of its schema, so an
        // attachment that overrides nothing still needs it.
        if self.modules.get(&module.handle).is_some_and(|e| e.schema.is_none()) {
            self.describe(module)
                .map_err(|d| ScriptError::HostRejected {
                    message: d.to_string(),
                })?;
        }
        let entry = self.module(module)?;
        let resolved = match &entry.schema {
            Some(schema) => schema.resolve_properties(properties).0,
            None => properties.clone(),
        };

        let env = host::instance_environment(&self.lua).map_err(host::to_script_error)?;
        let descriptor =
            self.with_deadline(self.budget.per_call, || self.evaluate(entry, env.clone()))?;

        // Resolved property values land on the descriptor table itself, so
        // a script reads `self.speed` rather than `self.props.speed`.
        for (name, value) in &resolved {
            let lua_value = convert::to_lua(&self.lua, value).map_err(host::to_script_error)?;
            descriptor
                .set(name.as_str(), lua_value)
                .map_err(host::to_script_error)?;
        }

        let callbacks = host::resolve_callbacks(&descriptor).map_err(host::to_script_error)?;

        self.instances.insert(
            id,
            Instance {
                module: module.handle,
                descriptor,
                env,
                callbacks,
                entity_handle: None,
            },
        );
        Ok(())
    }


    fn invoke(
        &mut self,
        id: ScriptInstanceId,
        callback: Callback,
        snapshot: &ScriptSnapshot,
        world: &dyn WorldView,
        commands: &mut CommandBuffer,
    ) -> Result<(), ScriptError> {
        let instance = self.instance(id)?;
        let Some(function) = instance.callbacks[callback as usize].clone() else {
            return Err(ScriptError::NoSuchCallback(callback));
        };
        let descriptor = instance.descriptor.clone();

        let budget = self.budget.per_call;
        self.deadline.arm(budget);
        let result = host::call_with_context(
            &self.lua,
            &function,
            &descriptor,
            callback,
            snapshot,
            world,
            commands,
        );
        self.deadline.disarm();

        match result {
            Ok(()) => Ok(()),
            Err(_) if self.deadline.tripped() => Err(ScriptError::Deadline { budget }),
            Err(err) => Err(host::to_script_error(err)),
        }
    }

    fn invoke_phase(
        &mut self,
        callback: Callback,
        calls: &[PhaseCall<'_>],
        world: &dyn WorldView,
        commands: &mut CommandBuffer,
    ) -> Vec<(ScriptInstanceId, ScriptError)> {
        let mut failures = Vec::new();
        if calls.is_empty() {
            return failures;
        }

        debug_assert!(
            calls
                .iter()
                .all(|c| c.snapshot.time == calls[0].snapshot.time
                    && c.snapshot.input == calls[0].snapshot.input),
            "every call in a phase must share its time and input; the \
             context is built once from the first"
        );

        // Resolve callables and entity handles before entering the scope,
        // so the borrow of `self.instances` ends before `commands` is
        // borrowed mutably — and so neither allocates inside the loop.
        let resolved: Vec<Option<(Function, Table, mlua::AnyUserData)>> = calls
            .iter()
            .map(|call| {
                let entry = self.instances.get(&call.instance).and_then(|instance| {
                    instance.callbacks[callback as usize]
                        .clone()
                        .map(|function| (function, instance.descriptor.clone()))
                })?;
                let handle = self.entity_handle(call.instance, call.snapshot.self_entity)?;
                Some((entry.0, entry.1, handle))
            })
            .collect();

        let budget = self.budget.per_call;
        let deadline = self.deadline.clone();
        let lua = &self.lua;
        let command_cell = std::cell::RefCell::new(commands);
        let command_ref = &command_cell;

        // One scope and one `ctx` for the whole phase. See
        // `ScriptBackend::invoke_phase` for the measurement that made this
        // the shape it is.
        let scope_result = lua.scope(|scope| {
            let ctx = host::build_ctx(lua, scope, calls[0].snapshot, world, command_ref)?;

            for (call, entry) in calls.iter().zip(&resolved) {
                let Some((function, descriptor, entity)) = entry else {
                    // No such instance, or the module does not define this
                    // callback. Neither is an error worth reporting every
                    // frame — the scheduler filters on the callback mask.
                    continue;
                };

                ctx.set("entity", entity)?;
                command_ref.borrow_mut().begin(call.order);
                deadline.arm(budget);
                let outcome =
                    host::dispatch(lua, function, descriptor, &ctx, callback, call.snapshot);
                deadline.disarm();
                command_ref.borrow_mut().end();

                if let Err(err) = outcome {
                    // Error quarantine: this attachment's whole batch is
                    // discarded so the world never sees half of what it
                    // intended, and every other attachment still lands.
                    command_ref.borrow_mut().discard_from(call.order);
                    let err = if deadline.tripped() {
                        ScriptError::Deadline { budget }
                    } else {
                        host::to_script_error(err)
                    };
                    failures.push((call.instance, err));
                }
            }
            Ok(())
        });

        if let Err(err) = scope_result {
            // Building the context failed, which is an engine fault rather
            // than any one script's. Attribute it to every call rather
            // than swallowing it.
            let err = host::to_script_error(err);
            failures.extend(calls.iter().map(|call| (call.instance, err.clone())));
        }
        failures
    }

    fn export_state(&mut self, id: ScriptInstanceId) -> Result<ScriptValue, ScriptError> {
        let instance = self.instance(id)?;
        let Some(function) = instance.callbacks[Callback::SaveState as usize].clone() else {
            // A script with no declared state is not an error; it simply
            // has nothing to carry across a reload.
            return Ok(ScriptValue::Nil);
        };
        let descriptor = instance.descriptor.clone();
        let value: Value =
            self.with_deadline(self.budget.per_call, || function.call(descriptor))?;

        // Anything the script tried to smuggle out that is not pure data
        // is refused here rather than at the point it fails to survive a
        // reload.
        convert::from_lua(&value).map_err(|message| ScriptError::HostRejected {
            message: format!("saveState returned something that is not durable data: {message}"),
        })
    }

    fn import_state(
        &mut self,
        id: ScriptInstanceId,
        state: ScriptValue,
    ) -> Result<(), ScriptError> {
        let instance = self.instance(id)?;
        let Some(function) = instance.callbacks[Callback::LoadState as usize].clone() else {
            return Ok(());
        };
        let descriptor = instance.descriptor.clone();
        let value = convert::to_lua(&self.lua, &state).map_err(host::to_script_error)?;
        self.with_deadline(self.budget.per_call, || function.call((descriptor, value)))
    }

    fn unload(&mut self, id: ScriptInstanceId) {
        // Dropping the instance drops its descriptor, environment and
        // cached callables, which is every reference this crate holds into
        // the VM for it. What is left is the VM's own garbage, which the
        // collector reclaims — `live_instances` is what a reload test
        // asserts on, and it goes down here.
        self.instances.remove(&id);
    }

    fn release_module(&mut self, module: CompiledModule) {
        debug_assert!(
            !self
                .instances
                .values()
                .any(|instance| instance.module == module.handle),
            "a module must not be released while instances of it are live"
        );
        self.modules.remove(&module.handle);
    }

    fn set_budget(&mut self, budget: Budget) {
        self.budget = budget;
        // A failure here means the ceiling could not be lowered because
        // the VM already holds more than that, which is information the
        // caller cannot act on mid-frame; the ceiling stays where it was.
        let _ = self.lua.set_memory_limit(budget.memory_bytes);
    }

    fn memory_used(&self) -> usize {
        self.lua.used_memory()
    }

    fn live_instances(&self) -> usize {
        self.instances.len()
    }
}

/// Read a declared field's type from its descriptor table.
///
/// Kept here rather than in [`host`] because it is the one place the
/// engine's [`FieldType`] vocabulary meets the script's, and a reader
/// looking for "what types can a script declare" should find one list.
pub(crate) fn field_type_from_kind(kind: &str) -> Option<FieldType> {
    Some(match kind {
        "boolean" => FieldType::Bool,
        "integer" => FieldType::I64,
        "number" => FieldType::F64,
        "string" => FieldType::Str,
        "vec2" => FieldType::Vec2,
        "vec3" => FieldType::Vec3,
        "vec4" => FieldType::Vec4,
        "quat" => FieldType::Quat,
        "color" => FieldType::Color,
        "entity" => FieldType::Entity,
        "asset" => FieldType::Asset,
        _ => return None,
    })
}

/// Build a [`ScriptFieldSchema`] list in declaration order.
pub(crate) fn empty_schema() -> ScriptSchema {
    ScriptSchema {
        api_version: somnium_script::attachment::CURRENT_API_VERSION,
        schema_version: 1,
        fields: Vec::<ScriptFieldSchema>::new(),
        callbacks: CallbackMask::default(),
    }
}

/// Build a one-message diagnostic batch at error severity.
pub(crate) fn error_diagnostic(
    asset: ScriptAssetId,
    display_path: &str,
    message: &str,
) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        asset,
        display_path: display_path.to_owned(),
        line: 0,
        column: 0,
        message: message.to_owned(),
    }
}
