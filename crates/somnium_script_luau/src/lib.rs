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
pub mod modules;

use std::collections::HashMap;

use mlua::chunk::{ChunkMode, Compiler};
use mlua::{Function, Lua, LuaOptions, StdLib, Table, Value, VmState};
use somnium_script::attachment::PropertyBag;
use somnium_script::backend::{
    Budget, Callback, CallbackMask, CompiledModule, Diagnostic, Diagnostics, PhaseCall,
    ScriptBackend, ScriptError, ScriptFieldSchema, ScriptSchema, ScriptSource, Severity,
};
use somnium_script::command::CommandBuffer;
use somnium_script::ids::{LanguageTag, ScriptAssetId, ScriptInstanceId};
use somnium_script::snapshot::WorldView;
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
    /// Module names this one `require`s, read out of the source at compile
    /// time.
    requires: Vec<String>,
    /// What those names resolved to, filled in by `link`.
    imports: Vec<(String, u64)>,
    /// The value this module returned, evaluated once per trust domain and
    /// frozen.
    ///
    /// Once per *domain*, not once per attachment: that is what makes a
    /// shared library shared. Frozen for the same reason the global table
    /// is — one attachment must not be able to rewrite a helper for every
    /// other attachment in the game.
    evaluated: Option<Value>,
    /// Filled by the first `describe`, and reused by every instantiate.
    ///
    /// Instantiation needs it: an attachment that overrides none of a
    /// script's properties must still get the script's *declared*
    /// defaults, and those live in the schema. Caching it also means a
    /// thousand attachments of one asset evaluate the descriptor for its
    /// schema once rather than a thousand times.
    schema: Option<ScriptSchema>,
}

/// One field of a mirrored component.
struct MirrorField {
    id: somnium_ecs::FieldId,
    /// Pre-interned Lua key. Interning it per access costs 130–190 ns;
    /// this costs nothing after the first frame.
    key: mlua::LuaString,
    writable: bool,
    /// The declared type, resolved once when the mirror is built.
    ///
    /// Needed because a shape conversion cannot distinguish a quaternion
    /// from four numbers, and the schema is the only thing that can. See
    /// [`somnium_ecs::reflect::FieldType::coerce`].
    ty: somnium_ecs::reflect::FieldType,
}

/// One component mirrored onto `ctx.self` for an attachment.
struct Mirror {
    component: somnium_ecs::StableId,
    /// The sub-table a script reads and writes, e.g. `ctx.self.transform`.
    table: Table,
    fields: Vec<MirrorField>,
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
    callbacks: [Option<Function>; host::CALLBACK_SLOTS],
    /// The entity handle this instance hands to `ctx.entity`, cached.
    ///
    /// An attachment's entity does not change, but the handle is userdata
    /// and constructing one allocates. Rebuilding it per callback cost
    /// about 0.5 µs against a 0.5 µs total budget — half the frame's
    /// script time spent re-wrapping a number that had not moved.
    entity_handle: Option<(somnium_ecs::Entity, mlua::AnyUserData)>,
    /// Components this script declared it uses, unresolved.
    uses: Vec<somnium_script::backend::ComponentUse>,
    /// Resolved on the first call, when a `WorldView` is finally available.
    /// `Some(empty)` means "resolved, uses nothing" and is not retried.
    mirrors: Option<Vec<Mirror>>,
    /// The table bound to `ctx.self`, owning the per-component sub-tables.
    self_table: Table,
}

/// The `ctx.self` key a component mirrors under.
///
/// `"somnium.Transform"` becomes `transform`, `"game.Health"` becomes
/// `health`. The stable id stays the durable name; this is only how a
/// script spells it, and keeping it short is the point of having it.
fn mirror_key(stable_name: &str) -> String {
    let last = stable_name.rsplit('.').next().unwrap_or(stable_name);
    let mut chars = last.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Interned keys for the per-call hot path.
struct Keys {
    entity: mlua::LuaString,
    zelf: mlua::LuaString,
    spawns: mlua::LuaString,
}

/// The Luau backend.
pub struct LuauBackend {
    lua: Lua,
    /// Pre-interned keys for the fields rewritten on every call.
    ///
    /// A `&str` key makes Lua intern the string on every access —
    /// measured at 264 ns for a `set` against 100 ns with a cached key.
    /// On a path that runs once per attachment per phase that is most of
    /// the budget.
    keys: Keys,
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

        let keys = Keys {
            entity: lua.create_string("entity").map_err(host::to_script_error)?,
            zelf: lua.create_string("self").map_err(host::to_script_error)?,
            spawns: lua.create_string("spawns").map_err(host::to_script_error)?,
        };

        Ok(Self {
            lua,
            keys,
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

    /// Evaluate everything a module `require`s, and hand back the values
    /// its `require` will return.
    fn resolve_imports(&mut self, handle: u64, depth: u32) -> mlua::Result<Vec<(String, Value)>> {
        let imports = self
            .modules
            .get(&handle)
            .map(|entry| entry.imports.clone())
            .unwrap_or_default();
        let mut resolved = Vec::with_capacity(imports.len());
        for (name, dependency) in imports {
            resolved.push((name, self.evaluate_module(dependency, depth + 1)?));
        }
        Ok(resolved)
    }

    /// Run a required module once and freeze what it returned.
    ///
    /// Once per **trust domain**, not once per attachment: a shared helper
    /// that were re-evaluated per attachment would not be shared, and two
    /// attachments would disagree about anything it kept. Frozen for the
    /// same reason the globals are — one attachment must not be able to
    /// rewrite a helper for everyone else.
    fn evaluate_module(&mut self, handle: u64, depth: u32) -> mlua::Result<Value> {
        if let Some(cached) = self.modules.get(&handle).and_then(|e| e.evaluated.clone()) {
            return Ok(cached);
        }
        if depth > MAX_MODULE_DEPTH {
            // The runtime rejects cycles on the static graph before we get
            // here. This is the belt to that braces: a graph that somehow
            // arrived cyclic must not blow the Rust stack.
            return Err(mlua::Error::RuntimeError(format!(
                "module graph nests deeper than {MAX_MODULE_DEPTH} levels; \
                 the dependency graph should have rejected this as a cycle"
            )));
        }

        let resolved = self.resolve_imports(handle, depth)?;
        let env = self.module_environment(&resolved)?;
        let value = {
            let entry = self.modules.get(&handle).ok_or_else(|| {
                mlua::Error::RuntimeError(format!("module {handle} is not loaded"))
            })?;
            self.load_with_env(entry, env)?.call::<Value>(())?
        };
        if let Value::Table(table) = &value {
            table.set_readonly(true);
        }
        if let Some(entry) = self.modules.get_mut(&handle) {
            entry.evaluated = Some(value.clone());
        }
        Ok(value)
    }

    /// A private environment chained to the frozen globals, with `require`
    /// bound to exactly this module's resolved imports.
    ///
    /// `require` is a table lookup, not a searcher: resolution already
    /// happened, in Rust, against the asset graph. A script therefore
    /// cannot reach a module it did not declare, cannot reach the
    /// filesystem, and cannot reach a native library.
    fn module_environment(&self, imports: &[(String, Value)]) -> mlua::Result<Table> {
        let env = host::instance_environment(&self.lua)?;
        let table = self.lua.create_table()?;
        for (name, value) in imports {
            table.set(name.as_str(), value.clone())?;
        }
        table.set_readonly(true);
        env.set(
            "require",
            self.lua.create_function(move |_, name: String| {
                let value: Value = table.get(name.as_str())?;
                if value.is_nil() {
                    return Err(mlua::Error::RuntimeError(format!(
                        "`{name}` is not one of this script's declared requires; \
                         module names are resolved before the script runs"
                    )));
                }
                Ok(value)
            })?,
        )?;
        Ok(env)
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

    /// Resolve an instance's declared `uses` into mirrors.
    ///
    /// Deferred to the first call because resolution needs a `WorldView`,
    /// which only exists during a phase. Done once; a script that declares
    /// nothing gets an empty list and never pays again.
    fn resolve_mirrors(&mut self, id: ScriptInstanceId, world: &dyn WorldView) {
        let Some(instance) = self.instances.get(&id) else {
            return;
        };
        if instance.mirrors.is_some() {
            return;
        }
        let uses = instance.uses.clone();
        let self_table = instance.self_table.clone();

        let mut mirrors = Vec::with_capacity(uses.len());
        for declared in &uses {
            let name = &declared.component;
            let Some(component) = world.component_by_name(name) else {
                // An unknown component is a script bug, but not a fatal
                // one: the mirror is simply absent and `ctx.self.x` reads
                // nil, which is what an author sees and can act on.
                continue;
            };
            let Ok(table) = self.lua.create_table() else {
                continue;
            };
            let Ok(key) = self.lua.create_string(mirror_key(name)) else {
                continue;
            };
            if self_table.raw_set(&key, &table).is_err() {
                continue;
            }
            let fields = world
                .script_fields(component)
                .into_iter()
                .filter(|(field_name, _, _)| {
                    declared.fields.is_empty() || declared.fields.contains(field_name)
                })
                .filter_map(|(field_name, field_id, writable)| {
                    Some(MirrorField {
                        id: field_id,
                        key: self.lua.create_string(&field_name).ok()?,
                        writable,
                        ty: world.field_type(component, field_id)?,
                    })
                })
                .collect();
            mirrors.push(Mirror {
                component,
                table,
                fields,
            });
        }

        if let Some(instance) = self.instances.get_mut(&id) {
            instance.mirrors = Some(mirrors);
        }
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

        // The dependency graph is read out of the text, not out of a run.
        // A `require` the engine cannot resolve statically is a compile
        // error here rather than a surprise at frame four hundred.
        let requires = match modules::parse_requires(&source.text) {
            Ok(sites) => sites.into_iter().map(|site| site.module).collect(),
            Err((line, message)) => {
                let mut diagnostics = Diagnostics::default();
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    asset: source.id,
                    display_path: source.display_path.clone(),
                    line,
                    column: 0,
                    message,
                });
                return Err(diagnostics);
            }
        };

        let handle = self.next_handle;
        self.next_handle += 1;
        self.modules.insert(
            handle,
            ModuleEntry {
                asset: source.id,
                display_path: source.display_path.clone(),
                bytecode,
                requires,
                imports: Vec::new(),
                evaluated: None,
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
        let path = self
            .modules
            .get(&module.handle)
            .map_or_else(|| "<unknown>".to_string(), |e| e.display_path.clone());
        let fail = |message: String| host::single_diagnostic(module.asset, &path, &message);

        // The descriptor runs in an environment with **no world access**.
        // Opening a script in the editor, or merely importing one to read
        // its property list, must not be able to change the scene — so the
        // API surface it can see does not include the means. It does get
        // `require`, because a module's declaration may legitimately be
        // built from a shared table, and evaluating a dependency is
        // subject to the same restriction.
        let imports = self
            .resolve_imports(module.handle, 0)
            .map_err(|err| fail(err.to_string()))?;
        let env = self
            .module_environment(&imports)
            .map_err(|err| fail(err.to_string()))?;

        let descriptor = {
            let entry = self
                .modules
                .get(&module.handle)
                .ok_or_else(|| fail(format!("module {} is not loaded", module.handle)))?;
            self.with_deadline(self.budget.per_call, || self.evaluate(entry, env))
                .map_err(|err| fail(err.to_string()))?
        };

        let mut schema = host::schema_from_descriptor(&descriptor).map_err(fail)?;
        // The graph comes out of the source scan, not the descriptor: a
        // script's `require`s are a property of its text, and reading them
        // from a table the script itself built would let it lie about them.
        schema.requires = self
            .modules
            .get(&module.handle)
            .map(|entry| entry.requires.clone())
            .unwrap_or_default();
        if let Some(entry) = self.modules.get_mut(&module.handle) {
            entry.schema = Some(schema.clone());
        }
        Ok(schema)
    }

    fn runtime_fingerprint(&self) -> String {
        runtime_fingerprint()
    }

    fn bytecode(&self, module: CompiledModule) -> Option<Vec<u8>> {
        self.modules
            .get(&module.handle)
            .map(|entry| entry.bytecode.clone())
    }

    fn load_bytecode(
        &mut self,
        source: &ScriptSource,
        bytecode: &[u8],
        fingerprint: &str,
    ) -> Result<CompiledModule, Diagnostics> {
        if fingerprint != runtime_fingerprint() {
            // Not an error the caller has to handle specially — recooking
            // from source is always available, and is what the cache is a
            // cache *of*.
            return self.compile(source);
        }
        // The dependency scan still runs: the graph is a property of the
        // source, not of the bytecode, and the cook does not store it.
        let requires = match modules::parse_requires(&source.text) {
            Ok(sites) => sites.into_iter().map(|site| site.module).collect(),
            Err(_) => return self.compile(source),
        };

        let handle = self.next_handle;
        self.next_handle += 1;
        self.modules.insert(
            handle,
            ModuleEntry {
                asset: source.id,
                display_path: source.display_path.clone(),
                bytecode: bytecode.to_vec(),
                requires,
                imports: Vec::new(),
                evaluated: None,
                schema: None,
            },
        );
        Ok(CompiledModule {
            asset: source.id,
            language: LanguageTag::LUAU,
            handle,
        })
    }

    fn module_requires(&self, module: CompiledModule) -> Vec<String> {
        self.modules
            .get(&module.handle)
            .map(|entry| entry.requires.clone())
            .unwrap_or_default()
    }

    fn link(
        &mut self,
        module: CompiledModule,
        imports: &[(String, CompiledModule)],
    ) -> Result<(), ScriptError> {
        let Some(entry) = self.modules.get_mut(&module.handle) else {
            return Err(ScriptError::HostRejected {
                message: format!("module {} is not loaded", module.handle),
            });
        };
        entry.imports = imports
            .iter()
            .map(|(name, dependency)| (name.clone(), dependency.handle))
            .collect();
        // Relinking invalidates whatever this module last evaluated to.
        // That is how a reload of a shared helper reaches everything that
        // required it: the runtime relinks the dependents, and their next
        // instantiate re-evaluates against the new module.
        entry.evaluated = None;
        Ok(())
    }

    fn instantiate(
        &mut self,
        id: ScriptInstanceId,
        module: CompiledModule,
        properties: &PropertyBag,
    ) -> Result<(), ScriptError> {
        // A script's declared defaults are part of its schema, so an
        // attachment that overrides nothing still needs it.
        if self
            .modules
            .get(&module.handle)
            .is_some_and(|e| e.schema.is_none())
        {
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

        let imports = self
            .resolve_imports(module.handle, 0)
            .map_err(host::to_script_error)?;
        let env = self
            .module_environment(&imports)
            .map_err(host::to_script_error)?;
        let entry = self.module(module)?;
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
        let uses = entry
            .schema
            .as_ref()
            .map(|schema| schema.uses.clone())
            .unwrap_or_default();
        let self_table = self.lua.create_table().map_err(host::to_script_error)?;

        self.instances.insert(
            id,
            Instance {
                module: module.handle,
                descriptor,
                env,
                callbacks,
                entity_handle: None,
                uses,
                mirrors: None,
                self_table,
            },
        );
        Ok(())
    }

    // Resolve pass, mirror in, dispatch, mirror out, quarantine — one
    // phase, in the order it happens. Splitting it would mean threading
    // the scope, the command cell and the staging buffer through four
    // signatures to save a line count, and would hide the ordering that
    // is the whole point of the function.
    #[allow(clippy::too_many_lines)]
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

        // ── The resolve pass ─────────────────────────────────────────
        //
        // Everything that needs `&mut self` happens here, once, before the
        // scope: caching the entity userdata and building the mirrors.
        // After it, the phase touches nothing mutably — which is what lets
        // the loop below hold a *reference* to each instance instead of
        // looking it up in a hash map three times per call.
        //
        // Those three lookups were measured at ~75–100 ns per call, about
        // 12% of the mirror overhead; `context.md` §17.18.3 records the
        // per-callback cost model these come out of.
        for call in calls {
            if self.instances.contains_key(&call.instance) {
                self.entity_handle(call.instance, call.snapshot.self_entity);
                self.resolve_mirrors(call.instance, world);
            }
        }

        let budget = self.budget.per_call;
        let deadline = self.deadline.clone();
        // Reborrowed immutably. Nothing inside the scope mutates the
        // backend, so this costs nothing and buys the borrow checker's
        // permission to keep `&Instance` across the whole loop.
        let this: &Self = self;
        let (lua, keys) = (&this.lua, &this.keys);

        let mut resolved: Vec<Option<&Instance>> = Vec::with_capacity(calls.len());
        for call in calls {
            let Some(instance) = this.instances.get(&call.instance) else {
                // A stale id is worth seeing; see the trait docs.
                failures.push((call.instance, ScriptError::NoSuchInstance(call.instance)));
                resolved.push(None);
                continue;
            };
            // The module simply has no such callback, or the entity handle
            // could not be built. Neither is an error.
            let runnable =
                instance.callbacks[callback as usize].is_some() && instance.entity_handle.is_some();
            resolved.push(runnable.then_some(instance));
        }

        let command_cell = std::cell::RefCell::new(commands);
        let command_ref = &command_cell;
        // Reused across the whole phase so mirroring allocates nothing.
        let mut staged: Vec<ScriptValue> = Vec::new();

        // One scope and one `ctx` for the whole phase. See
        // `ScriptBackend::invoke_phase` for the measurement that made this
        // the shape it is.
        let scope_result = lua.scope(|scope| {
            let ctx = host::build_ctx(lua, scope, calls[0].snapshot, world, command_ref)?;
            // Whether `ctx.spawns` currently holds a table, so the common
            // case of nobody having spawned anything costs nothing.
            let mut spawns_bound = false;

            for (call, entry) in calls.iter().zip(&resolved) {
                let Some(instance) = entry else {
                    // Already accounted for in the resolve pass above.
                    continue;
                };
                // Both were checked when `resolved` was built; a phase
                // that reached here without them would be a bug in the
                // resolve pass rather than in a script.
                let Some(function) = instance.callbacks[callback as usize].as_ref() else {
                    continue;
                };
                let Some((_, entity)) = instance.entity_handle.as_ref() else {
                    continue;
                };
                let descriptor = &instance.descriptor;

                ctx.raw_set(&keys.entity, entity.clone())?;

                // ── Spawn results ────────────────────────────────────
                //
                // A spawn cannot return an entity: the entity does not
                // exist until the commit point. The script gets a token
                // straight away and finds `ctx.spawns[token]` filled in on
                // the next phase.
                //
                // Per attachment, so it is rebound here rather than built
                // with the rest of `ctx` — but *only when it changes*. The
                // overwhelmingly common case is that no attachment in the
                // phase spawned anything, and clearing a key that is
                // already nil still costs a hash and a write barrier per
                // call. Tracking whether the key is currently set makes
                // that case free, which is the same lesson §11.4 records
                // about per-call work that looks too small to matter.
                if call.snapshot.spawn_results.is_empty() {
                    if spawns_bound {
                        ctx.raw_set(&keys.spawns, Value::Nil)?;
                        spawns_bound = false;
                    }
                } else {
                    let spawns = lua.create_table()?;
                    for (token, spawned) in &call.snapshot.spawn_results {
                        spawns.raw_set(token.0, convert::EntityHandle(*spawned))?;
                    }
                    ctx.raw_set(&keys.spawns, spawns)?;
                    spawns_bound = true;
                }

                command_ref.borrow_mut().begin(call.order);

                // ── Mirror in ────────────────────────────────────────
                //
                // The entity's own components become plain Luau tables
                // before the call, so a script reading and writing them is
                // doing table access rather than a host call.
                //
                // It also fixes a real defect in the `ctx:get`/`ctx:set`
                // pair. Because writes are deferred, a read-modify-write
                // loop through those re-read the *pre-phase* value every
                // iteration and only the last write survived — the loop
                // silently computed one step instead of ten. Through the
                // mirror a script sees its own writes, which is the
                // documented visibility rule.
                staged.clear();
                if let Some(mirrors) = instance.mirrors.as_ref() {
                    if !mirrors.is_empty() {
                        ctx.raw_set(&keys.zelf, instance.self_table.clone())?;
                    }
                    for mirror in mirrors {
                        for field in &mirror.fields {
                            let value = world
                                .read_field_id(
                                    call.snapshot.self_entity,
                                    mirror.component,
                                    field.id,
                                )
                                .unwrap_or(ScriptValue::Nil);
                            mirror
                                .table
                                .raw_set(&field.key, convert::to_lua(lua, &value)?)?;
                            staged.push(value);
                        }
                    }
                }

                deadline.arm(budget);
                let outcome =
                    host::dispatch(lua, function, descriptor, &ctx, callback, call.snapshot);
                deadline.disarm();

                // ── Mirror out ───────────────────────────────────────
                //
                // Only what actually changed becomes a command, and only
                // fields the schema marks script-writable are considered.
                // A script that reads its transform and writes nothing
                // queues nothing.
                if outcome.is_ok() {
                    if let Some(mirrors) = instance.mirrors.as_ref() {
                        let mut index = 0;
                        for mirror in mirrors {
                            let mut changed = somnium_ecs::ReflectObject::new();
                            for field in &mirror.fields {
                                let before = &staged[index];
                                index += 1;
                                if !field.writable {
                                    continue;
                                }
                                let raw: Value = mirror.table.raw_get(&field.key)?;
                                let Ok(after) = convert::from_lua(&raw) else {
                                    continue;
                                };
                                // Re-tag before comparing: a rotation read
                                // out as `Quat` and written back as a table
                                // narrows to `Vec4`, and without this every
                                // frame would look like a change *and* be
                                // rejected by the schema.
                                let after = field.ty.coerce(after);
                                if after != *before {
                                    changed.insert(field.id, after);
                                }
                            }
                            if !changed.is_empty() {
                                command_ref.borrow_mut().push(
                                    somnium_script::command::ScriptCommand::SetFields {
                                        entity: call.snapshot.self_entity,
                                        component: mirror.component,
                                        fields: changed,
                                    },
                                );
                            }
                        }
                    }
                }

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

    fn migrate_properties(
        &mut self,
        module: CompiledModule,
        properties: &PropertyBag,
        from_version: u32,
    ) -> Result<PropertyBag, ScriptError> {
        // Evaluated fresh rather than borrowed from an instance: the
        // instance this is for has already been torn down and its
        // replacement does not exist yet. A reload is not a hot path.
        let imports = self
            .resolve_imports(module.handle, 0)
            .map_err(host::to_script_error)?;
        let env = self
            .module_environment(&imports)
            .map_err(host::to_script_error)?;
        let entry = self.module(module)?;
        let descriptor = self.with_deadline(self.budget.per_call, || self.evaluate(entry, env))?;

        let Ok(Value::Function(migrate)) =
            descriptor.get::<Value>(Callback::MigrateState.script_name())
        else {
            return Ok(properties.clone());
        };

        let bag = self.lua.create_table().map_err(host::to_script_error)?;
        for (name, value) in properties {
            let lua_value = convert::to_lua(&self.lua, value).map_err(host::to_script_error)?;
            bag.set(name.as_str(), lua_value)
                .map_err(host::to_script_error)?;
        }

        let returned: Value = self.with_deadline(self.budget.per_call, || {
            migrate.call((descriptor.clone(), bag, from_version))
        })?;

        // A migration that returns nothing usable keeps the old bag rather
        // than silently emptying an author's work.
        match convert::from_lua(&returned) {
            Ok(ScriptValue::Map(entries)) => Ok(entries.into_iter().collect()),
            Ok(ScriptValue::Nil) => Ok(properties.clone()),
            Ok(other) => Err(ScriptError::HostRejected {
                message: format!(
                    "migrateProperties must return a table of property values, got {}",
                    other.kind()
                ),
            }),
            Err(message) => Err(ScriptError::HostRejected { message }),
        }
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
        uses: Vec::new(),
        requires: Vec::new(),
    }
}

/// A fingerprint that changes whenever this build would produce different
/// bytecode.
///
/// # Why a probe and not a version string
///
/// The obvious implementation is `format!("mlua {MLUA_VERSION} luau
/// {LUAU_VERSION}")`, and it is wrong in the way that matters: those are
/// constants a human keeps in sync, so the one time they are stale is the
/// one time the cache is invalid and nothing notices. Stale bytecode
/// handed to the Luau VM is undefined behaviour — the VM assumes its input
/// came from its own compiler and does not validate it.
///
/// So this compiles a fixed probe and hashes the result. If the compiler,
/// its options, or the bytecode format change *at all*, the bytes change
/// and the fingerprint changes with them. Nothing has to be remembered.
#[must_use]
pub fn runtime_fingerprint() -> String {
    /// Exercises constants, upvalues, a table, a call and a loop, so a
    /// change to almost any part of the emitter moves the hash.
    const PROBE: &str = "local t = {1,2,3} local s = 0 \
                         for i, v in ipairs(t) do s += v * i end \
                         return function() return s end";

    let bytes = Compiler::new().compile(PROBE).unwrap_or_default();
    // FNV-1a. Not cryptographic and does not need to be: this compares a
    // build against itself.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in &bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // The prefix is the *cache layout* version, which is ours to bump when
    // what we store beside the bytecode changes.
    format!("somnium-luau-1/{hash:016x}")
}

/// How deep a module graph may nest before the evaluator gives up.
///
/// The runtime rejects cycles on the static graph, so reaching this is a
/// bug rather than a script's mistake — but a Rust stack overflow is not
/// an acceptable way to find that out.
const MAX_MODULE_DEPTH: u32 = 32;

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
