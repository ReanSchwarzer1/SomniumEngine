//! The contract every scripting runtime implements.
//!
//! Nothing in this file names a language. That is the whole point: the
//! engine talks to [`ScriptBackend`], and the one crate that knows what
//! Luau is implements it. Swapping the language means writing a second
//! implementation, not editing the ECS, the scene format, the undo stack,
//! the editor, or a single line of gameplay API.
//!
//! The trait is shaped by four requirements the plan calls out:
//!
//! * **[`describe`](ScriptBackend::describe) is separate from
//!   [`instantiate`](ScriptBackend::instantiate)**, and runs in a
//!   restricted environment with no world access, so opening a script in
//!   the editor cannot mutate the scene.
//! * **Callbacks are named by an enum**, not by string, so a backend can
//!   resolve them once and never look one up by name in a hot path.
//! * **State export and import are explicit and pure-data**, because that
//!   is the only thing that can survive a reload.
//! * **Budgets are set on the backend**, not passed per call, because a
//!   runaway script has to be stoppable from outside the call it is
//!   stuck in.

use std::fmt;
use std::time::Duration;

use crate::ids::{LanguageTag, ScriptAssetId, ScriptInstanceId};
use crate::order::OrderKey;
use crate::snapshot::{ScriptSnapshot, WorldView};
use crate::value::{FieldType, ScriptValue};
use crate::{attachment::PropertyBag, command::CommandBuffer};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Source and compiled modules
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A script asset's source text, as handed to a backend for compilation.
#[derive(Debug, Clone)]
pub struct ScriptSource {
    /// Durable asset id.
    pub id: ScriptAssetId,
    /// Which language this is written in.
    pub language: LanguageTag,
    /// Path as shown in diagnostics. Display only — module resolution
    /// goes through the asset graph, never through the filesystem.
    pub display_path: String,
    /// The text.
    pub text: String,
}

/// A handle to a module a backend has compiled.
///
/// Opaque by construction: the `handle` is meaningful only to the backend
/// that issued it. Keeping the compiled artifact behind a token is what
/// stops a runtime type leaking into the engine through the back door of
/// "just this one field".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CompiledModule {
    /// Which asset this came from.
    pub asset: ScriptAssetId,
    /// Which backend owns it.
    pub language: LanguageTag,
    /// Backend-private identifier.
    pub handle: u64,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Declared schema
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// One property a script declares for the editor to author.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptFieldSchema {
    /// Declared name; the key its value is stored under.
    pub name: String,
    /// Declared type.
    pub ty: FieldType,
    /// Value used when the attachment has no override.
    pub default: ScriptValue,
    /// Inclusive lower bound, for numeric properties.
    pub min: Option<f64>,
    /// Inclusive upper bound, for numeric properties.
    pub max: Option<f64>,
    /// Tooltip.
    pub description: Option<String>,
}

/// Which lifecycle callbacks a module actually defines.
///
/// Recorded once at describe time so the scheduler can skip an attachment
/// entirely for a phase it does not implement, instead of paying a call
/// into the VM to find nothing there.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CallbackMask(u16);

impl CallbackMask {
    /// Record that `callback` is defined.
    #[must_use]
    pub fn with(self, callback: Callback) -> Self {
        Self(self.0 | (1 << callback as u16))
    }

    /// Whether `callback` is defined.
    #[must_use]
    pub fn has(self, callback: Callback) -> bool {
        self.0 & (1 << callback as u16) != 0
    }

    /// Whether nothing is defined.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Everything the engine learns about a module without running it.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptSchema {
    /// Engine API generation the module was written against.
    pub api_version: u32,
    /// The author's own version for their property list, bumped when they
    /// change it in a way that needs migration.
    pub schema_version: u32,
    /// Declared properties, in declaration order.
    pub fields: Vec<ScriptFieldSchema>,
    /// Which callbacks exist.
    pub callbacks: CallbackMask,
    /// Components this script wants mirrored onto `ctx.self`, in
    /// declaration order, each with the fields it actually touches.
    ///
    /// Declaring them is what lets the engine resolve names once per
    /// attachment instead of once per access, and what keeps a script that
    /// touches nothing from paying for the machinery at all.
    ///
    /// An empty field list means "every readable field", which is the
    /// convenient default and the expensive one: mirroring a field costs
    /// a conversion in each direction every frame, so a script that names
    /// `translation` should not be paying to marshal a rotation
    /// quaternion it never reads.
    pub uses: Vec<ComponentUse>,
    /// Modules this one `require`s, in source order.
    ///
    /// Read out of the source **without running it**, which is what makes
    /// the dependency graph static — and therefore what lets a reload
    /// compute the blast radius of an edit before it touches anything,
    /// lets the cook know what to bundle, and lets cycle detection happen
    /// once on a graph rather than as a runtime guard.
    pub requires: Vec<String>,
}

impl ScriptSchema {
    /// Look a declared property up by name.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&ScriptFieldSchema> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Merge authored overrides over the declared defaults, dropping any
    /// override whose property no longer exists and reporting it.
    ///
    /// This is the schema-drift path: an author deleting a property must
    /// not make every scene that set it fail to load.
    #[must_use]
    pub fn resolve_properties(&self, authored: &PropertyBag) -> (PropertyBag, Vec<String>) {
        let mut resolved = PropertyBag::new();
        for field in &self.fields {
            let value = authored
                .get(&field.name)
                .filter(|v| field.ty.accepts(v))
                .cloned()
                .unwrap_or_else(|| field.default.clone());
            resolved.insert(field.name.clone(), value);
        }
        let dropped = authored
            .keys()
            .filter(|name| self.field(name).is_none())
            .cloned()
            .collect();
        (resolved, dropped)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Callbacks
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A lifecycle entry point.
///
/// An enum rather than a string because the alternative — resolving a
/// method by name on every invocation — is a measured anti-pattern in the
/// engines this design was drawn from, and it is the kind of cost that
/// only shows up once there are a thousand scripted entities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u16)]
pub enum Callback {
    /// Instance constructed. Peers may not have started yet.
    Init = 0,
    /// Every instance known at this point has been initialised.
    Start = 1,
    /// The attachment was switched on.
    Enable = 2,
    /// The attachment was switched off.
    Disable = 3,
    /// Deterministic gameplay, once per fixed step, before physics.
    FixedUpdate = 4,
    /// Presentation and non-deterministic gameplay, once per frame.
    Update = 5,
    /// A queued event was delivered.
    Event = 6,
    /// Teardown. Structural effects still commit at the safe point.
    Destroy = 7,
    /// Export declared state before a reload.
    SaveState = 8,
    /// Import declared state after a reload.
    LoadState = 9,
    /// Rewrite authored properties saved under an older schema version.
    ///
    /// Not a lifecycle phase — it is called on the *module*, with the
    /// attachment's property bag, during a reload. A rename is the case it
    /// exists for: dropping `speed` and adding `velocity` otherwise loses
    /// every value an author set, and only whoever made that change knows
    /// the two are the same field.
    MigrateState = 10,
}

impl Callback {
    /// The name a script author writes for this callback.
    #[must_use]
    pub const fn script_name(self) -> &'static str {
        match self {
            Self::Init => "onInit",
            Self::Start => "onStart",
            Self::Enable => "onEnable",
            Self::Disable => "onDisable",
            Self::FixedUpdate => "onFixedUpdate",
            Self::Update => "onUpdate",
            Self::Event => "onEvent",
            Self::Destroy => "onDestroy",
            Self::SaveState => "saveState",
            Self::LoadState => "loadState",
            Self::MigrateState => "migrateProperties",
        }
    }

    /// Every callback, in declaration order.
    #[must_use]
    pub const fn all() -> [Self; 11] {
        [
            Self::Init,
            Self::Start,
            Self::Enable,
            Self::Disable,
            Self::FixedUpdate,
            Self::Update,
            Self::Event,
            Self::Destroy,
            Self::SaveState,
            Self::LoadState,
            Self::MigrateState,
        ]
    }

    /// Whether this callback runs in the deterministic fixed phase and
    /// must therefore not see wall-clock time or frame-rate-dependent
    /// values.
    #[must_use]
    pub const fn is_deterministic_phase(self) -> bool {
        matches!(self, Self::FixedUpdate)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Diagnostics and errors
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// How bad a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Advice. Does not block.
    Hint,
    /// Suspicious. Does not block.
    Warning,
    /// Blocks compilation or instantiation.
    Error,
}

/// One compiler, type-checker or lint message, positioned in a source
/// file so the editor can make it clickable.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    /// Severity.
    pub severity: Severity,
    /// Asset the message is about.
    pub asset: ScriptAssetId,
    /// Path as shown to the author.
    pub display_path: String,
    /// One-based line, or 0 when the message is not positioned.
    pub line: u32,
    /// One-based column, or 0.
    pub column: u32,
    /// The message.
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}: {}", self.display_path, self.message)
        } else {
            write!(
                f,
                "{}:{}:{}: {}",
                self.display_path, self.line, self.column, self.message
            )
        }
    }
}

/// A batch of diagnostics from one compile or check.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Diagnostics {
    /// The messages, in source order where the backend can provide it.
    pub messages: Vec<Diagnostic>,
}

impl Diagnostics {
    /// Whether any message blocks.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.messages.iter().any(|m| m.severity == Severity::Error)
    }

    /// Whether there are no messages at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Append a message.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.messages.push(diagnostic);
    }
}

impl fmt::Display for Diagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, message) in self.messages.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{message}")?;
        }
        Ok(())
    }
}

/// A runtime failure inside a script.
///
/// Every variant is something the engine survives. A script that runs
/// away, allocates without bound, or throws is quarantined; it never
/// takes the frame with it.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptError {
    /// The script raised an error.
    Raised {
        /// Message as the script reported it.
        message: String,
        /// Stack trace with source positions, if the backend has one.
        traceback: Option<String>,
    },
    /// The instance exceeded its time budget and was interrupted.
    Deadline {
        /// The budget it blew.
        budget: Duration,
    },
    /// The instance exceeded its memory ceiling.
    OutOfMemory {
        /// The ceiling, in bytes.
        limit: usize,
    },
    /// A host call was given something it could not accept — the typed
    /// answer to a stale handle, a bad component name, or an out-of-range
    /// value.
    HostRejected {
        /// What was wrong.
        message: String,
    },
    /// The instance id does not name a live instance. Normal after a
    /// reload; a bug anywhere else.
    NoSuchInstance(ScriptInstanceId),
    /// The module does not define the callback that was requested.
    NoSuchCallback(Callback),
    /// The backend caught a panic crossing the FFI boundary and converted
    /// it rather than letting it unwind.
    HostPanic {
        /// Panic message, if it was a string.
        message: String,
    },
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Raised { message, traceback } => {
                write!(f, "script error: {message}")?;
                if let Some(trace) = traceback {
                    write!(f, "\n{trace}")?;
                }
                Ok(())
            }
            Self::Deadline { budget } => {
                write!(
                    f,
                    "script exceeded its {budget:?} budget and was interrupted"
                )
            }
            Self::OutOfMemory { limit } => {
                write!(f, "script exceeded its {limit} byte memory ceiling")
            }
            Self::HostRejected { message } => write!(f, "rejected by the engine: {message}"),
            Self::NoSuchInstance(id) => write!(f, "no live script {id}"),
            Self::NoSuchCallback(callback) => {
                write!(f, "module defines no `{}`", callback.script_name())
            }
            Self::HostPanic { message } => {
                write!(f, "engine panic inside a script call: {message}")
            }
        }
    }
}

impl std::error::Error for ScriptError {}

/// Resource limits applied to a backend.
///
/// Both halves matter. A per-instance deadline stops one runaway script;
/// a per-phase deadline stops a thousand scripts that are each just under
/// their own limit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    /// Wall-clock ceiling for one callback.
    pub per_call: Duration,
    /// Wall-clock ceiling for a whole phase.
    pub per_phase: Duration,
    /// Memory ceiling for the backend's VM, in bytes.
    pub memory_bytes: usize,
    /// Maximum commands one attachment may queue in one callback, so a
    /// cheap loop cannot flood the applier.
    pub max_commands: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            per_call: Duration::from_millis(2),
            per_phase: Duration::from_millis(8),
            memory_bytes: 64 * 1024 * 1024,
            max_commands: 4096,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// The backend
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// One component a script declared it uses, and which of its fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentUse {
    /// Stable component name, as the script wrote it.
    pub component: String,
    /// Field names to mirror. Empty means every readable field.
    pub fields: Vec<String>,
}

/// One attachment's slot in a phase.
#[derive(Debug, Clone, Copy)]
pub struct PhaseCall<'a> {
    /// Which live instance to call.
    pub instance: ScriptInstanceId,
    /// Where it sits in the deterministic order; the backend attributes
    /// the commands it emits to this key.
    pub order: OrderKey,
    /// What it sees. Time and input must match every other call in the
    /// phase — see [`ScriptBackend::invoke_phase`].
    pub snapshot: &'a ScriptSnapshot,
}

/// A scripting runtime.
///
/// One implementation per language. `Send` so a backend can be moved to a
/// worker thread later; deliberately **not** `Sync`, because a single VM
/// is a single-threaded object and pretending otherwise would invite the
/// mistake of assuming parallel script execution comes for free.
pub trait ScriptBackend: Send {
    /// Which language this backend runs.
    fn language(&self) -> LanguageTag;

    /// Compile source into a module. Does not run module top-level code
    /// against the world.
    ///
    /// # Errors
    ///
    /// The compiler's diagnostics, positioned in the source.
    fn compile(&mut self, source: &ScriptSource) -> Result<CompiledModule, Diagnostics>;

    /// Evaluate the module's descriptor in a restricted environment and
    /// report what it declares.
    ///
    /// The environment has no world access, so opening a script in the
    /// editor — or merely importing it — cannot change the scene.
    ///
    /// # Errors
    ///
    /// Diagnostics explaining why the descriptor could not be read.
    fn describe(&mut self, module: CompiledModule) -> Result<ScriptSchema, Diagnostics>;

    /// A string that changes whenever this runtime would produce
    /// different bytecode.
    ///
    /// Cooked bytecode is a **cache, never durable storage** — Luau's own
    /// `Bytecode.h` says indefinite backward compatibility is not
    /// provided — so anything that stores it has to be able to tell
    /// whether it is still valid, and recook from source when it is not.
    /// This is what that check compares.
    fn runtime_fingerprint(&self) -> String {
        String::new()
    }

    /// The bytecode of a compiled module, for a cook to store.
    ///
    /// `None` from a backend that has no such thing.
    fn bytecode(&self, module: CompiledModule) -> Option<Vec<u8>> {
        let _ = module;
        None
    }

    /// Adopt bytecode this backend produced earlier, skipping compilation.
    ///
    /// The `source` is still required, because everything except the
    /// parse still needs it: diagnostics, the dependency scan, and the
    /// ability to recook if the fingerprint no longer matches. Bytecode
    /// from anywhere other than this engine's own compiler is never
    /// loaded — the Luau VM assumes its input came from its own compiler
    /// and does not validate it.
    ///
    /// # Errors
    ///
    /// A fingerprint mismatch, or a backend that cannot take bytecode.
    fn load_bytecode(
        &mut self,
        source: &ScriptSource,
        bytecode: &[u8],
        fingerprint: &str,
    ) -> Result<CompiledModule, Diagnostics> {
        let _ = (bytecode, fingerprint);
        self.compile(source)
    }

    /// The module names a compiled module `require`s.
    ///
    /// Available straight after [`Self::compile`], before
    /// [`Self::describe`] — which is the order it has to be, because
    /// describing a module runs its top-level code and that code may
    /// `require`. Read from the source text, never from anything the
    /// script builds at run time, so a module cannot lie about what it
    /// depends on.
    fn module_requires(&self, module: CompiledModule) -> Vec<String> {
        let _ = module;
        Vec::new()
    }

    /// Bind a module's `require`s to the modules that satisfy them.
    ///
    /// Called after every asset in a graph has compiled, and again for a
    /// module's dependents when it is reloaded. Resolution is the
    /// **runtime's** job, not the backend's: names come out of
    /// [`ScriptSchema::requires`], the runtime maps them to assets and
    /// rejects cycles, and this hands the backend the answer.
    ///
    /// A backend with no module system can leave this alone.
    ///
    /// # Errors
    ///
    /// Whatever went wrong evaluating a dependency.
    fn link(
        &mut self,
        module: CompiledModule,
        imports: &[(String, CompiledModule)],
    ) -> Result<(), ScriptError> {
        let _ = (module, imports);
        Ok(())
    }

    /// Create a live instance of a module with its resolved properties.
    ///
    /// # Errors
    ///
    /// Whatever the script raised while constructing itself.
    fn instantiate(
        &mut self,
        id: ScriptInstanceId,
        module: CompiledModule,
        properties: &PropertyBag,
    ) -> Result<(), ScriptError>;

    /// Call the same entry point on many instances, as one phase.
    ///
    /// # Why this exists
    ///
    /// Setting up the script-facing context is not free — for the Luau
    /// backend it is roughly a dozen host closures, measured at ~0.67 µs
    /// each, against a Luau function call of ~0.12 µs. Built per
    /// *callback*, that context dominates: a thousand empty callbacks
    /// measured **14.3 ms against a 0.5 ms budget**, and essentially none
    /// of it was script execution.
    ///
    /// Built once per *phase* it disappears into the noise. So the
    /// scheduler hands the backend the whole phase and the backend sets up
    /// once — which is also the shape the design already assumed, since
    /// the phase boundary is where commands commit.
    ///
    /// # Contract
    ///
    /// Every snapshot in `calls` must carry the same [`TimeSnapshot`] and
    /// [`InputSnapshot`]: they are properties of the phase, not of an
    /// attachment, and a backend may build the context from the first
    /// call's copy.
    ///
    /// Failures are **returned, not propagated**. One attachment raising
    /// must not stop the others — that is error quarantine, and it is why
    /// this returns a list rather than a `Result`.
    ///
    /// The backend attributes each call's commands itself, via
    /// [`CommandBuffer::begin`] and [`CommandBuffer::end`]; the caller
    /// must not have a batch open.
    ///
    /// Two absences are treated differently, on purpose:
    ///
    /// * **the module does not define this callback** — a silent no-op.
    ///   Normal, and the reason [`CallbackMask`] exists; making it an
    ///   error would force the scheduler to filter twice and would fill
    ///   the log every frame with attachments that simply have no
    ///   `onUpdate`.
    /// * **no such live instance** — reported as a failure. It is either a
    ///   bug or a stale id surviving a reload, and both are worth seeing.
    fn invoke_phase(
        &mut self,
        callback: Callback,
        calls: &[PhaseCall<'_>],
        world: &dyn WorldView,
        commands: &mut CommandBuffer,
    ) -> Vec<(ScriptInstanceId, ScriptError)>;

    /// Call one lifecycle entry point.
    ///
    /// Sugar over [`Self::invoke_phase`] with a single call, and
    /// deliberately **not** a second implementation: when these were two
    /// code paths they drifted, and a feature that existed in the phase
    /// path was simply absent from this one. Tests and one-off callers use
    /// this; the scheduler uses the phase form directly.
    ///
    /// # Errors
    ///
    /// Whatever the script raised, or the budget it blew.
    fn invoke(
        &mut self,
        id: ScriptInstanceId,
        order: OrderKey,
        callback: Callback,
        snapshot: &ScriptSnapshot,
        world: &dyn WorldView,
        commands: &mut CommandBuffer,
    ) -> Result<(), ScriptError> {
        let calls = [PhaseCall {
            instance: id,
            order,
            snapshot,
        }];
        self.invoke_phase(callback, &calls, world, commands)
            .into_iter()
            .next()
            .map_or(Ok(()), |(_, err)| Err(err))
    }

    /// Ask an instance for its declared, pure-data state.
    ///
    /// Closures, coroutines and userdata are not state and must not be
    /// returned here; a backend that cannot express a value as a
    /// [`ScriptValue`] must drop it rather than smuggle it.
    ///
    /// # Errors
    ///
    /// Whatever the script raised.
    fn export_state(&mut self, id: ScriptInstanceId) -> Result<ScriptValue, ScriptError>;

    /// Rewrite an attachment's authored properties for a new schema
    /// version, using the module's own `migrateProperties`.
    ///
    /// Called on the module rather than on an instance, because the
    /// instance it is for does not exist yet — this runs between the old
    /// one being torn down and the new one being built.
    ///
    /// A backend with no migration support can leave this alone; the
    /// runtime only calls it when [`ScriptSchema::callbacks`] says the
    /// module declared one.
    ///
    /// # Errors
    ///
    /// Whatever the script raised.
    fn migrate_properties(
        &mut self,
        module: CompiledModule,
        properties: &PropertyBag,
        from_version: u32,
    ) -> Result<PropertyBag, ScriptError> {
        let _ = (module, from_version);
        Ok(properties.clone())
    }

    /// Give an instance previously exported state.
    ///
    /// # Errors
    ///
    /// Whatever the script raised.
    fn import_state(&mut self, id: ScriptInstanceId, state: ScriptValue)
    -> Result<(), ScriptError>;

    /// Destroy an instance and release everything it owns.
    fn unload(&mut self, id: ScriptInstanceId);

    /// Forget a compiled module. Called after every instance of it is
    /// gone, as the last step of a reload.
    fn release_module(&mut self, module: CompiledModule);

    /// Apply new resource limits.
    fn set_budget(&mut self, budget: Budget);

    /// Bytes currently attributed to this backend's VM.
    fn memory_used(&self) -> usize;

    /// Number of live instances, for leak assertions in reload tests.
    fn live_instances(&self) -> usize;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> ScriptSchema {
        ScriptSchema {
            api_version: 1,
            schema_version: 1,
            fields: vec![
                ScriptFieldSchema {
                    name: "speed".into(),
                    ty: FieldType::F64,
                    default: ScriptValue::F64(4.0),
                    min: Some(0.0),
                    max: Some(30.0),
                    description: None,
                },
                ScriptFieldSchema {
                    name: "target".into(),
                    ty: FieldType::Entity,
                    default: ScriptValue::Entity(None),
                    min: None,
                    max: None,
                    description: None,
                },
            ],
            callbacks: CallbackMask::default()
                .with(Callback::FixedUpdate)
                .with(Callback::Init),
            uses: Vec::new(),
            requires: Vec::new(),
        }
    }

    #[test]
    fn callback_mask_records_only_what_was_added() {
        let mask = schema().callbacks;
        assert!(mask.has(Callback::FixedUpdate));
        assert!(mask.has(Callback::Init));
        assert!(!mask.has(Callback::Update));
        assert!(!mask.has(Callback::Destroy));
        assert!(!mask.is_empty());
        assert!(CallbackMask::default().is_empty());
    }

    #[test]
    fn every_callback_fits_in_the_mask() {
        let mut mask = CallbackMask::default();
        for callback in Callback::all() {
            mask = mask.with(callback);
        }
        for callback in Callback::all() {
            assert!(mask.has(callback), "{callback:?} did not fit");
        }
    }

    #[test]
    fn only_fixed_update_is_the_deterministic_phase() {
        for callback in Callback::all() {
            assert_eq!(
                callback.is_deterministic_phase(),
                callback == Callback::FixedUpdate
            );
        }
    }

    #[test]
    fn authored_overrides_win_over_declared_defaults() {
        let mut authored = PropertyBag::new();
        authored.insert("speed".into(), ScriptValue::F64(11.0));
        let (resolved, dropped) = schema().resolve_properties(&authored);

        assert_eq!(resolved["speed"], ScriptValue::F64(11.0));
        assert_eq!(
            resolved["target"],
            ScriptValue::Entity(None),
            "default fills in"
        );
        assert!(dropped.is_empty());
    }

    #[test]
    fn an_override_of_the_wrong_type_falls_back_to_the_default() {
        let mut authored = PropertyBag::new();
        authored.insert("speed".into(), ScriptValue::Str("fast".into()));
        let (resolved, _) = schema().resolve_properties(&authored);
        assert_eq!(
            resolved["speed"],
            ScriptValue::F64(4.0),
            "a mistyped override must not become the live value"
        );
    }

    #[test]
    fn an_override_of_a_deleted_property_is_reported_not_fatal() {
        let mut authored = PropertyBag::new();
        authored.insert("speed".into(), ScriptValue::F64(2.0));
        authored.insert("removed_last_week".into(), ScriptValue::Bool(true));
        let (resolved, dropped) = schema().resolve_properties(&authored);

        assert_eq!(resolved.len(), 2, "resolved holds exactly the declared set");
        assert!(!resolved.contains_key("removed_last_week"));
        assert_eq!(dropped, vec!["removed_last_week".to_string()]);
    }

    #[test]
    fn diagnostics_distinguish_blocking_from_advisory() {
        let mut diagnostics = Diagnostics::default();
        assert!(diagnostics.is_empty());
        assert!(!diagnostics.has_errors());

        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            asset: ScriptAssetId::NONE,
            display_path: "a.luau".into(),
            line: 3,
            column: 7,
            message: "unused local".into(),
        });
        assert!(
            !diagnostics.has_errors(),
            "a warning must not block a reload"
        );

        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            asset: ScriptAssetId::NONE,
            display_path: "a.luau".into(),
            line: 9,
            column: 1,
            message: "expected `end`".into(),
        });
        assert!(diagnostics.has_errors());
        assert!(
            diagnostics
                .to_string()
                .contains("a.luau:9:1: expected `end`")
        );
    }

    #[test]
    fn an_unpositioned_diagnostic_still_prints_usefully() {
        let diagnostic = Diagnostic {
            severity: Severity::Error,
            asset: ScriptAssetId::NONE,
            display_path: "b.luau".into(),
            line: 0,
            column: 0,
            message: "module not found".into(),
        };
        assert_eq!(diagnostic.to_string(), "b.luau: module not found");
    }

    #[test]
    fn the_default_budget_is_stated_in_frame_terms() {
        let budget = Budget::default();
        assert!(
            budget.per_call < budget.per_phase,
            "one call cannot own the phase"
        );
        assert!(
            budget.per_phase < Duration::from_millis(17),
            "must fit inside 60 Hz"
        );
        assert!(budget.max_commands > 0);
        assert!(budget.memory_bytes > 0);
    }
}
