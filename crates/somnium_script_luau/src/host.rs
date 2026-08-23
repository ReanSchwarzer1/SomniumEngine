//! The engine API as scripts see it, and the call plumbing behind it.
//!
//! # Two environments, and why
//!
//! A module is evaluated twice in its life: once to read its declaration
//! ([`describe_environment`]) and once per attachment to become a live
//! instance ([`instance_environment`]). The first has **no world access at
//! all** — not a read, not a write. That is what makes opening a script in
//! the editor, or importing one to list its properties, incapable of
//! changing the scene. Top-level module code runs in both, and only the
//! second one can do anything.
//!
//! # `ctx` is scoped, not stored
//!
//! The world view and the command buffer are borrowed for exactly one
//! callback. `Lua::scope` lets those borrows reach script-callable
//! functions without either leaking or needing a raw-pointer bridge: when
//! the scope ends the functions are invalidated, so a script that squirrels
//! `ctx` away in a global gets an error on the next frame rather than a
//! dangling read.

use std::cell::RefCell;

use mlua::{Function, Lua, Table, Value, Variadic};
use somnium_ecs::reflect::ReflectObject;
use somnium_script::backend::{
    Callback, CallbackMask, Diagnostic, Diagnostics, ScriptError, ScriptFieldSchema, ScriptSchema,
    ScriptSource, Severity,
};
use somnium_script::command::{CommandBuffer, ForceMode, LogLevel, ScriptCommand};
use somnium_script::ids::ScriptAssetId;
use somnium_script::snapshot::{ScriptSnapshot, WorldView};
use somnium_script::value::ScriptValue;

use crate::convert::{self, EntityHandle};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Errors and diagnostics
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Convert an `mlua` error into the engine's typed error.
///
/// Memory exhaustion gets its own variant because it is the one runtime
/// failure whose fix is a budget change rather than a code change.
#[must_use]
pub fn to_script_error(err: mlua::Error) -> ScriptError {
    match err {
        // `limit` is filled in by the backend, which is the only thing
        // that knows the ceiling it set.
        mlua::Error::MemoryError(_) => ScriptError::OutOfMemory { limit: 0 },
        mlua::Error::SyntaxError { message, .. } => ScriptError::Raised {
            message,
            traceback: None,
        },
        mlua::Error::CallbackError { traceback, cause } => ScriptError::Raised {
            message: cause.to_string(),
            traceback: Some(traceback),
        },
        other => ScriptError::Raised {
            message: other.to_string(),
            traceback: None,
        },
    }
}

/// Turn a compiler error into a positioned diagnostic.
///
/// Luau writes `path:line: message`, so the position is recovered from the
/// text rather than invented. A message that does not parse keeps its
/// whole text and reports line 0, which the editor renders as
/// file-level — better than pointing confidently at the wrong line.
#[must_use]
pub fn diagnostic_from_error(err: &mlua::Error, source: &ScriptSource) -> Diagnostic {
    let raw = match err {
        mlua::Error::SyntaxError { message, .. } => message.clone(),
        other => other.to_string(),
    };
    let (line, message) = parse_position(&raw);
    Diagnostic {
        severity: Severity::Error,
        asset: source.id,
        display_path: source.display_path.clone(),
        line,
        column: 0,
        message,
    }
}

/// Pull a `:line:` out of a Luau diagnostic, if it has one.
fn parse_position(raw: &str) -> (u32, String) {
    // Shapes seen from Luau: `[string "name"]:12: msg` and `name:12: msg`.
    let after_bracket = raw.rfind("]:").map(|i| i + 2);
    let start = after_bracket.unwrap_or(0);
    let rest = &raw[start..];
    let Some(colon) = rest.find(':') else {
        return (0, raw.to_owned());
    };
    let (head, tail) = rest.split_at(colon);
    let digits = if after_bracket.is_some() {
        head.trim()
    } else {
        head.rsplit(':').next().unwrap_or("").trim()
    };
    match digits.parse::<u32>() {
        Ok(line) => (line, tail.trim_start_matches(':').trim().to_owned()),
        Err(_) => (0, raw.to_owned()),
    }
}

/// One-message diagnostic batch.
#[must_use]
pub fn single_diagnostic(asset: ScriptAssetId, display_path: &str, message: &str) -> Diagnostics {
    let mut diagnostics = Diagnostics::default();
    diagnostics.push(crate::error_diagnostic(asset, display_path, message));
    diagnostics
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// The declaration API
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Install the globals every script sees, before the state is sandboxed.
///
/// Two of them: `Script`, whose only job is `define`, and `Field`, which
/// builds property descriptors. Both are pure — neither touches the world
/// — which is what lets the same API serve the describe pass and the live
/// pass without a second surface to keep in sync.
///
/// # Errors
///
/// If a table or function cannot be created.
pub fn install_api(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();

    // ── Close the doors the default library set leaves open ─────────
    //
    // Opening only the safe `StdLib` flags is not sufficient: several
    // globals arrive with the base library and each of them defeats
    // something this design relies on. Measured, not assumed — see the
    // `enumerate_globals` test, which fails if a future Luau adds another.
    //
    //   getfenv / setfenv  reach and rewrite *another* function's
    //                      environment, which is the entire mechanism
    //                      keeping one attachment's globals private;
    //   loadstring         compiles arbitrary text at runtime, so no
    //                      static analysis or cook step can see it, and
    //                      the "only our compiler makes bytecode" rule
    //                      stops being true;
    //   require            resolves modules outside the asset graph;
    //   collectgarbage
    //   gcinfo             let a script force a full GC every frame,
    //                      which is a frame-time denial of service that
    //                      no time budget catches because the pause lands
    //                      outside the script's own call;
    //   print              writes to stdout, bypassing the output log and
    //                      the attribution that says which script wrote
    //                      it. `ctx:log` is the sanctioned path;
    //   _G                 hands out the global table itself.
    //
    // Removed before `sandbox(true)`, because sandboxing is what freezes
    // the table and after it nothing can be removed.
    for name in REMOVED_GLOBALS {
        globals.set(*name, mlua::Value::Nil)?;
    }

    let script = lua.create_table()?;
    // `define` returns its argument. The indirection exists so a module's
    // shape is declared rather than discovered: the engine reads one table
    // instead of guessing which globals were meant to be callbacks.
    script.set(
        "define",
        lua.create_function(|_, descriptor: Table| Ok(descriptor))?,
    )?;
    globals.set("Script", script)?;

    let field = lua.create_table()?;
    for kind in [
        "boolean", "integer", "number", "string", "vec2", "vec3", "vec4", "quat", "color",
        "entity", "asset",
    ] {
        let kind_owned = kind.to_string();
        field.set(
            kind,
            lua.create_function(move |lua, (default, options): (Value, Option<Table>)| {
                let descriptor = lua.create_table()?;
                descriptor.set("__field", true)?;
                descriptor.set("kind", kind_owned.as_str())?;
                descriptor.set("default", default)?;
                if let Some(options) = options {
                    for key in ["min", "max", "description"] {
                        let value: Value = options.get(key)?;
                        if !value.is_nil() {
                            descriptor.set(key, value)?;
                        }
                    }
                }
                Ok(descriptor)
            })?,
        )?;
    }
    globals.set("Field", field)?;

    Ok(())
}

/// Globals that arrive with the base library and must not survive.
///
/// Public so the sandbox test can assert on exactly this list rather than
/// on a copy of it that could drift.
pub const REMOVED_GLOBALS: &[&str] = &[
    "getfenv",
    "setfenv",
    "loadstring",
    "require",
    "collectgarbage",
    "gcinfo",
    "print",
    "_G",
];

/// An environment for reading a module's declaration.
///
/// Inherits the frozen globals — `Script`, `Field`, `math`, `table` — and
/// nothing else. There is no `ctx` here and no way to obtain one, so
/// top-level module code cannot reach the world.
///
/// # Errors
///
/// If the table or its metatable cannot be created.
pub fn describe_environment(lua: &Lua) -> mlua::Result<Table> {
    child_of_globals(lua)
}

/// An environment for a live instance.
///
/// Structurally the same as the describe environment: private globals
/// chained to the frozen shared ones. The difference between the two
/// passes is not the environment, it is that a live instance is *called*
/// with a `ctx` and a describe pass never is.
///
/// # Errors
///
/// If the table or its metatable cannot be created.
pub fn instance_environment(lua: &Lua) -> mlua::Result<Table> {
    child_of_globals(lua)
}

/// A fresh table that reads through to the frozen globals but writes to
/// itself — so one attachment's globals are private to it.
fn child_of_globals(lua: &Lua) -> mlua::Result<Table> {
    let env = lua.create_table()?;
    let meta = lua.create_table()?;
    meta.set("__index", lua.globals())?;
    env.set_metatable(Some(meta))?;
    Ok(env)
}

/// Read a module's descriptor table into the engine's schema type.
///
/// # Errors
///
/// A message naming what was wrong with the declaration.
pub fn schema_from_descriptor(descriptor: &Table) -> Result<ScriptSchema, String> {
    let mut schema = crate::empty_schema();

    if let Ok(Some(uses)) = descriptor.get::<Option<Table>>("uses") {
        // Two spellings, because they answer different needs:
        //
        //   uses = { "somnium.Transform" }                       -- all fields
        //   uses = { ["somnium.Transform"] = { "translation" } } -- just these
        //
        // The list form is what an author reaches for first; the map form
        // is what they reach for once a profile says the mirror is costing
        // them. Both are parsed here so neither is a second API.
        let mut declared: Vec<somnium_script::backend::ComponentUse> = Vec::new();
        for pair in uses.pairs::<Value, Value>() {
            let (key, value) = pair.map_err(|e| format!("`uses`: {e}"))?;
            match (key, value) {
                (Value::Integer(_), Value::String(name)) => {
                    declared.push(somnium_script::backend::ComponentUse {
                        component: name.to_str().map_err(|e| e.to_string())?.to_string(),
                        fields: Vec::new(),
                    });
                }
                (Value::String(name), Value::Table(fields)) => {
                    let mut names = Vec::new();
                    for field in fields.sequence_values::<String>() {
                        names.push(field.map_err(|e| format!("`uses` field list: {e}"))?);
                    }
                    declared.push(somnium_script::backend::ComponentUse {
                        component: name.to_str().map_err(|e| e.to_string())?.to_string(),
                        fields: names,
                    });
                }
                _ => {
                    return Err("`uses` entries must be a component name, or a                                 component name mapped to a list of field names"
                        .to_string());
                }
            }
        }
        // Sorted so the mirror layout does not depend on Lua table order.
        declared.sort_by(|a, b| a.component.cmp(&b.component));
        schema.uses = declared;
    }

    if let Ok(Some(version)) = descriptor.get::<Option<u32>>("apiVersion") {
        schema.api_version = version;
    }
    if let Ok(Some(version)) = descriptor.get::<Option<u32>>("schemaVersion") {
        schema.schema_version = version;
    }

    if let Ok(Some(fields)) = descriptor.get::<Option<Table>>("fields") {
        // Sorted by name so the property list — and therefore the editor's
        // field order and the generated declarations — is the same on
        // every run. Lua table iteration order is not.
        let mut declared: Vec<(String, Table)> = Vec::new();
        for pair in fields.pairs::<String, Table>() {
            let (name, spec) = pair.map_err(|e| format!("field list: {e}"))?;
            declared.push((name, spec));
        }
        declared.sort_by(|a, b| a.0.cmp(&b.0));

        for (name, spec) in declared {
            schema.fields.push(field_schema(&name, &spec)?);
        }
    }

    let mut callbacks = CallbackMask::default();
    for callback in Callback::all() {
        match descriptor.get::<Value>(callback.script_name()) {
            Ok(Value::Function(_)) => callbacks = callbacks.with(callback),
            Ok(Value::Nil) | Err(_) => {}
            Ok(other) => {
                // A callback name bound to something uncallable would
                // otherwise be *silently absent* — the mask simply would
                // not have it, and the author would find out at frame four
                // hundred that their update never ran. Luau's dynamic
                // typing means a genuinely changed argument list cannot be
                // caught here; this is the part that can be.
                return Err(format!(
                    "`{}` must be a function; this declares a {}",
                    callback.script_name(),
                    other.type_name()
                ));
            }
        }
    }
    schema.callbacks = callbacks;

    Ok(schema)
}

/// Widen a value to the type its field declares, where that is
/// unambiguous. Only integer-to-float: everything else is either already
/// right, or genuinely a mistake the caller should hear about.
fn coerce_to_declared(ty: &somnium_script::value::FieldType, value: ScriptValue) -> ScriptValue {
    use somnium_script::value::FieldType;
    match (ty, &value) {
        #[allow(clippy::cast_precision_loss)]
        (FieldType::F64, ScriptValue::I64(v)) => ScriptValue::F64(*v as f64),
        _ => value,
    }
}

fn field_schema(name: &str, spec: &Table) -> Result<ScriptFieldSchema, String> {
    let kind: String = spec
        .get("kind")
        .map_err(|_| format!("`{name}` is not a Field descriptor; use `Field.number(...)`"))?;
    let ty = crate::field_type_from_kind(&kind)
        .ok_or_else(|| format!("`{name}` has unknown field kind `{kind}`"))?;

    let default_value: Value = spec.get("default").unwrap_or(Value::Nil);
    let default = convert::from_lua(&default_value)
        .map_err(|e| format!("`{name}` has an invalid default: {e}"))?;
    // Luau has one number type, so `Field.number(4.0)` is indistinguishable
    // from `Field.number(4)` by the time it reaches here. The declared type
    // decides; otherwise a default of `4.0` would be stored as an integer
    // and compare unequal to the same value read back later.
    let default = coerce_to_declared(&ty, default);

    // A default that does not match its own declared type is a script bug
    // that would otherwise surface as a mysterious value in the editor.
    if !matches!(default, ScriptValue::Nil) && !ty.accepts(&default) {
        return Err(format!(
            "`{name}` declares {} but its default is {}",
            ty.name(),
            default.kind()
        ));
    }

    Ok(ScriptFieldSchema {
        name: name.to_owned(),
        ty,
        default,
        min: spec.get::<Option<f64>>("min").unwrap_or(None),
        max: spec.get::<Option<f64>>("max").unwrap_or(None),
        description: spec.get::<Option<String>>("description").unwrap_or(None),
    })
}

/// One slot per [`Callback`]. Derived rather than written down, so adding
/// a callback cannot leave an array one short — which it did, and the
/// symptom was an index-out-of-bounds panic rather than anything that
/// pointed at the enum.
pub const CALLBACK_SLOTS: usize = Callback::all().len();

/// Resolve every lifecycle entry point once.
///
/// # Errors
///
/// If the descriptor cannot be read.
pub fn resolve_callbacks(descriptor: &Table) -> mlua::Result<[Option<Function>; CALLBACK_SLOTS]> {
    let mut resolved: [Option<Function>; CALLBACK_SLOTS] = Default::default();
    for callback in Callback::all() {
        if let Ok(Value::Function(function)) = descriptor.get::<Value>(callback.script_name()) {
            resolved[callback as usize] = Some(function);
        }
    }
    Ok(resolved)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Calling into a script
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Invoke `function` with the argument shape its callback expects.
///
/// # Errors
///
/// Whatever the script raised.
pub fn dispatch(
    lua: &Lua,
    function: &Function,
    descriptor: &Table,
    ctx: &Table,
    callback: Callback,
    snapshot: &ScriptSnapshot,
) -> mlua::Result<()> {
    match callback {
        Callback::FixedUpdate => function.call::<()>((descriptor, ctx, snapshot.time.fixed_delta)),
        Callback::Update => function.call::<()>((descriptor, ctx, snapshot.time.delta)),
        Callback::Event => {
            let events = lua.create_table()?;
            for (i, event) in snapshot.events.iter().enumerate() {
                let table = lua.create_table()?;
                table.set("name", event.name.as_str())?;
                table.set("sequence", event.sequence)?;
                events.set(i + 1, table)?;
            }
            function.call::<()>((descriptor, ctx, events))
        }
        _ => function.call::<()>((descriptor, ctx)),
    }
}

/// Render a value for the output log without going through `tostring`,
/// which the sandbox may not expose and which would let a `__tostring`
/// metamethod run arbitrary script code inside the log path.
fn format_for_log(value: &Value) -> String {
    match value {
        Value::Nil => "nil".into(),
        Value::Boolean(v) => v.to_string(),
        Value::Integer(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => v.to_string_lossy(),
        Value::Vector(v) => format!("({}, {}, {})", v.x(), v.y(), v.z()),
        other => format!("<{}>", other.type_name()),
    }
}

// One block per `ctx` member. Splitting it would scatter the engine's
// entire script-facing surface across several functions, which is the one
// thing a reader of this file most needs to see in one place.
#[allow(clippy::too_many_lines)]
/// Build the table every callback in a phase receives.
///
/// Everything here is phase-level: the host closures, the input
/// helpers, the clock. The only per-attachment field is `entity`, which
/// [`rebind_ctx`] overwrites between calls. That split is the whole
/// reason a thousand callbacks cost a thousand Luau calls rather than
/// twelve thousand closure allocations.
///
/// # Errors
///
/// If any table or host function cannot be created.
pub fn build_ctx<'scope, 'env>(
    lua: &Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    snapshot: &ScriptSnapshot,
    world: &'env dyn WorldView,
    commands: &'env RefCell<&'env mut CommandBuffer>,
) -> mlua::Result<Table> {
    let ctx = lua.create_table()?;
    ctx.set("entity", EntityHandle(snapshot.self_entity))?;
    ctx.set("time", snapshot.time.simulation_time)?;
    ctx.set("fixedDelta", snapshot.time.fixed_delta)?;
    ctx.set("step", snapshot.time.step)?;

    // ── Input ────────────────────────────────────────────────────
    let input = lua.create_table()?;
    {
        let keys_down = snapshot.input.keys_down.clone();
        input.set(
            "isKeyDown",
            scope.create_function(move |_, (_self, key): (Table, u32)| {
                Ok(keys_down.binary_search(&key).is_ok())
            })?,
        )?;
        let keys_pressed = snapshot.input.keys_pressed.clone();
        input.set(
            "isKeyPressed",
            scope.create_function(move |_, (_self, key): (Table, u32)| {
                Ok(keys_pressed.binary_search(&key).is_ok())
            })?,
        )?;
        let mouse_down = snapshot.input.mouse_down.clone();
        input.set(
            "isMouseDown",
            scope.create_function(move |_, (_self, button): (Table, u8)| {
                Ok(mouse_down.binary_search(&button).is_ok())
            })?,
        )?;
        // Plain numbers rather than a call, because look code reads them
        // every step and a host call for a value already sitting in the
        // snapshot would be two hundred nanoseconds for nothing.
        //
        // Pixels since the previous fixed step, already accumulated — a
        // script must not have to know the frame rate to aim.
        input.set("mouseDeltaX", snapshot.input.mouse_delta[0])?;
        input.set("mouseDeltaY", snapshot.input.mouse_delta[1])?;
    }
    ctx.set("input", input)?;

    // ── Reads ────────────────────────────────────────────────────
    ctx.set(
        "isAlive",
        scope.create_function(move |_, (_ctx, entity): (Table, EntityHandle)| {
            Ok(world.is_alive(entity.0))
        })?,
    )?;
    ctx.set(
        "get",
        scope.create_function(
            move |lua, (_ctx, entity, component, field): (Table, EntityHandle, String, String)| {
                let Some(stable) = world.component_by_name(&component) else {
                    return Ok(Value::Nil);
                };
                match world.read_field(entity.0, stable, &field) {
                    Some(value) => convert::to_lua(lua, &value),
                    None => Ok(Value::Nil),
                }
            },
        )?,
    )?;

    // ── Writes, all deferred ─────────────────────────────────────
    ctx.set(
        "set",
        scope.create_function(
            move |_,
                  (_ctx, entity, component, field, value): (
                Table,
                EntityHandle,
                String,
                String,
                Value,
            )| {
                let stable = world.component_by_name(&component).ok_or_else(|| {
                    mlua::Error::RuntimeError(format!("unknown component `{component}`"))
                })?;
                let field_id = world.field_by_name(stable, &field).ok_or_else(|| {
                    mlua::Error::RuntimeError(format!("`{component}` has no field `{field}`"))
                })?;
                // Refused here as well as at apply time. The script
                // author gets the error at the line that wrote it,
                // instead of a rejection in a log after the frame.
                if !world.is_field_writable(stable, &field) {
                    return Err(mlua::Error::RuntimeError(format!(
                        "`{component}.{field}` is engine-owned and cannot be set by a script"
                    )));
                }
                let parsed = convert::from_lua(&value).map_err(mlua::Error::RuntimeError)?;
                // The declared type is the only thing that can tell a
                // quaternion from four numbers; see `FieldType::coerce`.
                let parsed = world
                    .field_type(stable, field_id)
                    .map_or(parsed.clone(), |ty| ty.coerce(parsed));
                let mut record = ReflectObject::new();
                record.insert(field_id, parsed);
                commands.borrow_mut().push(ScriptCommand::SetFields {
                    entity: entity.0,
                    component: stable,
                    fields: record,
                });
                Ok(())
            },
        )?,
    )?;

    ctx.set(
        "despawn",
        scope.create_function(move |_, (_ctx, entity): (Table, EntityHandle)| {
            commands
                .borrow_mut()
                .push(ScriptCommand::Despawn { entity: entity.0 });
            Ok(())
        })?,
    )?;

    ctx.set(
        "spawn",
        scope.create_function(move |_, _ctx: Table| {
            let mut buffer = commands.borrow_mut();
            let token = buffer.new_spawn_token();
            buffer.push(ScriptCommand::Spawn {
                token,
                components: Vec::new(),
            });
            // The token, not an entity: the entity does not exist until
            // the commit point, and the next snapshot is where the
            // script learns what it got.
            Ok(token.0)
        })?,
    )?;

    ctx.set(
        "applyForce",
        scope.create_function(
            move |_,
                  (_ctx, entity, force, impulse): (
                Table,
                EntityHandle,
                mlua::Vector,
                Option<bool>,
            )| {
                commands.borrow_mut().push(ScriptCommand::ApplyForce {
                    entity: entity.0,
                    force: [force.x(), force.y(), force.z()],
                    mode: if impulse.unwrap_or(false) {
                        ForceMode::Impulse
                    } else {
                        ForceMode::Force
                    },
                });
                Ok(())
            },
        )?,
    )?;

    ctx.set(
        "emit",
        scope.create_function(move |_, (_ctx, name): (Table, String)| {
            commands.borrow_mut().push(ScriptCommand::EmitEvent {
                name,
                payload: ReflectObject::new(),
            });
            Ok(())
        })?,
    )?;

    // `print` is not available — the sandbox does not open it — so this
    // is the only way a script talks to the output log, and every line
    // is attributed to the attachment that wrote it.
    for (name, level) in [
        ("log", LogLevel::Info),
        ("warn", LogLevel::Warn),
        ("error", LogLevel::Error),
    ] {
        ctx.set(
            name,
            scope.create_function(move |_, (_ctx, parts): (Table, Variadic<Value>)| {
                let message = parts
                    .iter()
                    .map(format_for_log)
                    .collect::<Vec<_>>()
                    .join(" ");
                commands
                    .borrow_mut()
                    .push(ScriptCommand::Log { level, message });
                Ok(())
            })?,
        )?;
    }
    Ok(ctx)
}
