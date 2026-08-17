//! The scheduler and the instance registry: Phase 16-C's neutral half.
//!
//! [`ScriptRuntime`] owns the mapping from *authored data* — a
//! [`ScriptSet`](crate::attachment::ScriptSet) on an entity — to *live VM
//! objects*, and it drives them through the lifecycle in a documented
//! order. It knows nothing about Luau, and nothing about `World`: reads go
//! through [`WorldView`] and writes come out as a [`CommandBuffer`], which
//! is what lets the whole scheduler be tested without a window, a GPU or
//! a physics world.
//!
//! # The shape of a frame
//!
//! ```text
//! reconcile(views)          create instances for new attachments,
//!                           mark vanished ones for teardown
//!   ↓  (repeat to a fixed point, capped at MAX_INIT_CYCLES)
//! run_phase(Init)           Loaded      → Initialized
//! run_phase(Start)          Initialized → Started
//! run_phase(Enable/Disable) Started/Disabled ⇄ Enabled
//!   ↓  once per fixed step
//! run_phase(FixedUpdate)    → commands → apply → physics
//!   ↓  once per frame
//! run_phase(Update)
//! run_phase(Event)          delivers what the last apply produced
//!   ↓  the safe point
//! run_phase(Destroy) + collect()
//! ```
//!
//! # Two rules that are easy to get wrong
//!
//! **A state advance is not conditional on the callback existing.** A
//! script with no `onInit` still becomes `Initialized`; the
//! [`CallbackMask`] only decides whether the VM is entered at all. Tying
//! the transition to the callback would leave a whole class of scripts
//! permanently in `Loaded`.
//!
//! **The init fixed point is bounded.** Scripts spawned during
//! initialisation are initialised in the same frame, iterated until
//! nothing new appears — but a prefab that spawns itself would otherwise
//! hang the editor, so the loop is capped at [`MAX_INIT_CYCLES`] and the
//! chain that caused it is reported. The cap is the difference between a
//! bug and a hang.

use std::collections::BTreeMap;

use somnium_ecs::{Entity, PersistentId, ReflectObject};

use crate::attachment::PropertyBag;
use crate::backend::{
    Budget, Callback, CallbackMask, CompiledModule, Diagnostic, Diagnostics, PhaseCall,
    ScriptBackend, ScriptError, ScriptSchema, ScriptSource, Severity,
};
use crate::command::{CommandBuffer, SpawnToken};
use crate::ids::{InstanceUuid, LanguageTag, ScriptAssetId, ScriptInstanceId};
use crate::lifecycle::LifecycleState;
use crate::order::OrderKey;
use crate::ownership::{OwnedResource, OwnershipToken, ResourceLedger};
use crate::snapshot::{InputSnapshot, ScriptEvent, ScriptSnapshot, TimeSnapshot, WorldView};
use crate::value::ScriptValue;

/// How many times initialisation may iterate before the engine decides a
/// spawn chain is not converging.
///
/// Fyrox's number, adopted deliberately: high enough that no legitimate
/// prefab tree reaches it, low enough that hitting it costs a frame rather
/// than a session.
pub const MAX_INIT_CYCLES: u32 = 64;

/// How many consecutive failures quarantine an attachment.
pub const DEFAULT_FAILURE_THRESHOLD: u32 = 3;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Inputs
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// One authored attachment, as the engine reads it out of the world.
///
/// A flat copy rather than a borrow: the runtime is handed the whole
/// authored set once per reconcile, and holding borrows into ECS storage
/// across the phases that follow is exactly the thing this design exists
/// to prevent.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentView {
    /// The entity carrying the attachment.
    pub entity: Entity,
    /// That entity's durable id.
    pub persistent: PersistentId,
    /// The attachment's durable id.
    pub instance: InstanceUuid,
    /// Which script asset it runs.
    pub asset: ScriptAssetId,
    /// Whether the author has it switched on.
    pub enabled: bool,
    /// Authored ordering.
    pub execution_order: i32,
    /// Authored property overrides.
    pub properties: PropertyBag,
}

impl AttachmentView {
    /// The deterministic sort key for this attachment.
    #[must_use]
    pub const fn order(&self) -> OrderKey {
        OrderKey::new(self.execution_order, self.persistent, self.instance)
    }
}

/// Clock and input for one phase. Shared by every attachment in it,
/// because they are properties of the phase and not of an attachment.
#[derive(Debug, Clone, Default)]
pub struct PhaseInput {
    /// The clock.
    pub time: TimeSnapshot,
    /// Input state.
    pub input: InputSnapshot,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Reports
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// What one [`ScriptRuntime::reconcile`] changed.
#[derive(Debug, Default, PartialEq)]
pub struct ReconcileReport {
    /// Attachments that gained a live instance this pass.
    pub created: Vec<InstanceUuid>,
    /// Attachments marked for teardown because their authored data is gone.
    pub retired: Vec<InstanceUuid>,
    /// Attachments whose asset is not loaded. Retried on the next pass, so
    /// a scene that references a script that has not been imported yet
    /// still opens.
    pub missing_assets: Vec<(InstanceUuid, ScriptAssetId)>,
    /// Attachments whose module refused to instantiate.
    pub failed: Vec<(InstanceUuid, ScriptError)>,
}

impl ReconcileReport {
    /// Whether the pass changed the live set at all. The init fixed point
    /// runs until this is false.
    #[must_use]
    pub fn is_quiet(&self) -> bool {
        self.created.is_empty() && self.retired.is_empty()
    }
}

/// What the editor shows about one live attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceSummary {
    /// Durable attachment id.
    pub instance: InstanceUuid,
    /// The entity it is on.
    pub entity: Entity,
    /// The asset it runs.
    pub asset: ScriptAssetId,
    /// Where it is in its lifecycle.
    pub state: LifecycleState,
    /// Whether the error quarantine switched it off, as opposed to the
    /// author. The two are shown differently, because one is a bug report
    /// and the other is a choice.
    pub quarantined: bool,
    /// The author's `enabled` flag.
    pub authored_enabled: bool,
    /// How many failures in a row it has had.
    pub consecutive_failures: u32,
}

/// One attachment's failure, with enough context for a diagnostic that
/// names the script rather than the engine.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceFailure {
    /// The attachment.
    pub instance: InstanceUuid,
    /// The entity it is on.
    pub entity: Entity,
    /// The asset it runs.
    pub asset: ScriptAssetId,
    /// Which callback was running.
    pub callback: Callback,
    /// What went wrong.
    pub error: ScriptError,
}

/// What one [`ScriptRuntime::run_phase`] did.
#[derive(Debug, Default)]
pub struct PhaseReport {
    /// Attachments the VM was actually entered for.
    pub invoked: usize,
    /// Attachments whose lifecycle state moved.
    pub advanced: usize,
    /// Failures, in deterministic order.
    pub failures: Vec<InstanceFailure>,
    /// Attachments that crossed the failure threshold this phase and were
    /// switched off.
    pub quarantined: Vec<InstanceUuid>,
}

impl PhaseReport {
    /// Whether nothing went wrong.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.failures.is_empty()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Internal records
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

struct AssetRecord {
    source: ScriptSource,
    backend: usize,
    module: CompiledModule,
    schema: ScriptSchema,
}

struct InstanceRecord {
    entity: Entity,
    persistent: PersistentId,
    asset: ScriptAssetId,
    backend: usize,
    live: ScriptInstanceId,
    order: OrderKey,
    state: LifecycleState,
    /// The authored `enabled` flag. Quarantine is tracked separately so
    /// that clearing a fault restores the author's intent rather than
    /// silently leaving the attachment off.
    authored_enabled: bool,
    quarantined: bool,
    callbacks: CallbackMask,
    /// The property values this instance was built with. A change means
    /// the instance is rebuilt, which is how an editor property edit takes
    /// effect without a reload.
    properties: PropertyBag,
    consecutive_failures: u32,
    pending_destroy: bool,
    inbox: Vec<ScriptEvent>,
    spawn_results: Vec<(SpawnToken, Entity)>,
    rng_seed: u64,
}

impl InstanceRecord {
    fn wants_enabled(&self) -> bool {
        self.authored_enabled && !self.quarantined
    }
}

/// One instance's work in a phase, with everything it needs already
/// copied out so no borrow into the runtime survives the call.
struct Dispatch {
    instance: InstanceUuid,
    live: ScriptInstanceId,
    order: OrderKey,
    snapshot: ScriptSnapshot,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// The runtime
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Live scripts, their lifecycle, and the order they run in.
pub struct ScriptRuntime {
    backends: Vec<Box<dyn ScriptBackend>>,
    assets: BTreeMap<ScriptAssetId, AssetRecord>,
    /// `BTreeMap` because iteration order is part of the determinism
    /// promise, and a hash map's is not.
    instances: BTreeMap<InstanceUuid, InstanceRecord>,
    ledger: ResourceLedger,
    budget: Budget,
    failure_threshold: u32,
    diagnostics: Vec<Diagnostic>,
    next_event_sequence: u64,
    world_seed: u64,
}

impl ScriptRuntime {
    /// A runtime with no backends registered.
    #[must_use]
    pub fn new(budget: Budget) -> Self {
        Self {
            backends: Vec::new(),
            assets: BTreeMap::new(),
            instances: BTreeMap::new(),
            ledger: ResourceLedger::new(),
            budget,
            failure_threshold: DEFAULT_FAILURE_THRESHOLD,
            diagnostics: Vec::new(),
            next_event_sequence: 1,
            world_seed: 0x5EED_0000_0000_0001,
        }
    }

    /// Set how many consecutive failures quarantine an attachment.
    #[must_use]
    pub fn with_failure_threshold(mut self, threshold: u32) -> Self {
        self.set_failure_threshold(threshold);
        self
    }

    /// Change how many consecutive failures quarantine an attachment.
    ///
    /// Never zero: a threshold of zero would quarantine on success.
    pub fn set_failure_threshold(&mut self, threshold: u32) {
        self.failure_threshold = threshold.max(1);
    }

    /// Seed the per-attachment random streams. Same seed, same replay.
    pub fn set_world_seed(&mut self, seed: u64) {
        self.world_seed = seed;
    }

    /// Install a language.
    pub fn register_backend(&mut self, mut backend: Box<dyn ScriptBackend>) {
        backend.set_budget(self.budget);
        self.backends.push(backend);
    }

    /// Whether a language is available.
    #[must_use]
    pub fn has_backend(&self, language: LanguageTag) -> bool {
        self.backend_index(language).is_some()
    }

    fn backend_index(&self, language: LanguageTag) -> Option<usize> {
        self.backends
            .iter()
            .position(|backend| backend.language() == language)
    }

    /// Apply new limits to every backend.
    pub fn set_budget(&mut self, budget: Budget) {
        self.budget = budget;
        for backend in &mut self.backends {
            backend.set_budget(budget);
        }
    }

    // ── Assets ─────────────────────────────────────────────────────

    /// Compile a script asset and read its declaration.
    ///
    /// Both halves happen here because an asset the engine cannot describe
    /// is an asset it cannot attach: the editor needs the field list to
    /// draw the inspector, and instantiation needs the declared defaults.
    ///
    /// # Errors
    ///
    /// The compiler's or the descriptor's diagnostics.
    pub fn load_asset(&mut self, source: ScriptSource) -> Result<ScriptAssetId, Diagnostics> {
        let Some(backend) = self.backend_index(source.language) else {
            let diagnostics = single(
                source.id,
                &source.display_path,
                &format!("no backend is registered for `{}`", source.language),
            );
            self.diagnostics.extend(diagnostics.messages.iter().cloned());
            return Err(diagnostics);
        };

        let module = self.backends[backend].compile(&source).inspect_err(|d| {
            self.diagnostics.extend(d.messages.iter().cloned());
        })?;
        let schema = match self.backends[backend].describe(module) {
            Ok(schema) => schema,
            Err(diagnostics) => {
                // The module compiled but does not declare itself. Release
                // it rather than leaving a module nothing can instantiate.
                self.backends[backend].release_module(module);
                self.diagnostics
                    .extend(diagnostics.messages.iter().cloned());
                return Err(diagnostics);
            }
        };

        let id = source.id;
        // Replacing an asset in place would strand live instances against
        // a module that is about to be released; `reload_asset` is the path
        // that handles that, and it is what this defers to.
        if self.assets.contains_key(&id) {
            return self.reload_asset(source).map(|()| id);
        }
        self.assets.insert(
            id,
            AssetRecord {
                source,
                backend,
                module,
                schema,
            },
        );
        Ok(id)
    }

    /// The declaration of a loaded asset.
    #[must_use]
    pub fn asset_schema(&self, asset: ScriptAssetId) -> Option<&ScriptSchema> {
        self.assets.get(&asset).map(|record| &record.schema)
    }

    /// The source text of a loaded asset, for the editor.
    #[must_use]
    pub fn asset_source(&self, asset: ScriptAssetId) -> Option<&ScriptSource> {
        self.assets.get(&asset).map(|record| &record.source)
    }

    /// Whether an asset is loaded.
    #[must_use]
    pub fn is_asset_loaded(&self, asset: ScriptAssetId) -> bool {
        self.assets.contains_key(&asset)
    }

    /// Every loaded asset, in id order.
    pub fn assets(&self) -> impl Iterator<Item = (ScriptAssetId, &ScriptSource)> {
        self.assets
            .iter()
            .map(|(id, record)| (*id, &record.source))
    }

    /// Swap an asset's module under its live instances, carrying declared
    /// state across.
    ///
    /// This is the half of hot reload that has nothing to do with the
    /// filesystem, and it is proven by an in-process swap test. What 16-E
    /// adds on top is the watcher, the debounce, the dependency graph and
    /// the rollback generation — not this.
    ///
    /// The failure path is the point: **a module that does not compile
    /// leaves every live instance running.** Nothing about the world
    /// changes, and the caller gets diagnostics.
    ///
    /// # Errors
    ///
    /// The new source's diagnostics, with the old module still live.
    pub fn reload_asset(&mut self, source: ScriptSource) -> Result<(), Diagnostics> {
        let asset = source.id;
        let Some(existing) = self.assets.get(&asset) else {
            return self.load_asset(source).map(|_| ());
        };
        let backend = existing.backend;
        let previous = existing.module;

        // Compile and describe in the shadow, before anything live is
        // touched. A failure here returns with the world untouched.
        let module = self.backends[backend].compile(&source).inspect_err(|d| {
            self.diagnostics.extend(d.messages.iter().cloned());
        })?;
        let schema = match self.backends[backend].describe(module) {
            Ok(schema) => schema,
            Err(diagnostics) => {
                self.backends[backend].release_module(module);
                self.diagnostics
                    .extend(diagnostics.messages.iter().cloned());
                return Err(diagnostics);
            }
        };

        // From here the swap is committed.
        let affected: Vec<InstanceUuid> = self
            .instances
            .iter()
            .filter(|(_, record)| record.asset == asset && record.state.is_live())
            .map(|(id, _)| *id)
            .collect();

        let mut carried: Vec<(InstanceUuid, ScriptValue)> = Vec::with_capacity(affected.len());
        for id in &affected {
            let record = &self.instances[id];
            let state = self.backends[backend]
                .export_state(record.live)
                .unwrap_or(ScriptValue::Nil);
            carried.push((*id, state));
            // Owned resources do not survive a reload: a coroutine, a task
            // and an audio voice are all VM-lifetime things.
            self.ledger.release_all(*id);
            self.backends[backend].unload(record.live);
        }

        self.assets.insert(
            asset,
            AssetRecord {
                source,
                backend,
                module,
                schema: schema.clone(),
            },
        );

        for (id, state) in carried {
            let Some(record) = self.instances.get_mut(&id) else {
                continue;
            };
            let (resolved, dropped) = schema.resolve_properties(&record.properties);
            let live = ScriptInstanceId::next();
            match self.backends[backend].instantiate(live, module, &resolved) {
                Ok(()) => {
                    record.live = live;
                    record.callbacks = schema.callbacks;
                    // `loadState` before `onInit`, then the scheduler
                    // replays init/start/enable — the documented order.
                    let _ = self.backends[backend].import_state(live, state);
                    record.state = LifecycleState::Loaded;
                    record.consecutive_failures = 0;
                }
                Err(error) => {
                    record.state = LifecycleState::Destroyed;
                    self.diagnostics.push(diagnostic(
                        asset,
                        "<reload>",
                        &format!("{id} did not survive the reload: {error}"),
                    ));
                }
            }
            for name in dropped {
                self.diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    asset,
                    display_path: String::new(),
                    line: 0,
                    column: 0,
                    message: format!("property `{name}` no longer exists and was dropped"),
                });
            }
        }

        self.instances
            .retain(|_, record| record.state != LifecycleState::Destroyed);
        self.backends[backend].release_module(previous);
        Ok(())
    }

    // ── Diagnostics ────────────────────────────────────────────────

    /// Take everything worth showing in the output log.
    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.diagnostics)
    }

    /// How many blocking diagnostics are pending, for the status area.
    #[must_use]
    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count()
    }

    // ── Reconciliation ─────────────────────────────────────────────

    /// Make the live instance set match the authored one.
    ///
    /// Creates an instance for every attachment that does not have one,
    /// marks for teardown every instance whose attachment has gone, and
    /// rebuilds an instance whose authored properties changed. Idempotent:
    /// calling it twice with the same views does nothing the second time,
    /// which is what makes the init fixed point terminate.
    pub fn reconcile(&mut self, views: &[AttachmentView]) -> ReconcileReport {
        let mut report = ReconcileReport::default();
        let mut seen: Vec<InstanceUuid> = Vec::with_capacity(views.len());

        for view in views {
            seen.push(view.instance);

            if let Some(record) = self.instances.get_mut(&view.instance) {
                // Cheap authored updates that do not need a new VM object.
                record.entity = view.entity;
                record.persistent = view.persistent;
                record.order = view.order();
                record.authored_enabled = view.enabled;
                if record.properties == view.properties && record.asset == view.asset {
                    continue;
                }
                // A property or asset change means the instance is stale.
                // Reconcile runs at a phase boundary with no callback on
                // the stack, so the old VM object can go now rather than
                // being deferred — deferring it here would leave two
                // instances under one `InstanceUuid` for a frame.
                self.destroy_now(view.instance);
                report.retired.push(view.instance);
            }

            let Some(asset) = self.assets.get(&view.asset) else {
                report.missing_assets.push((view.instance, view.asset));
                continue;
            };
            let backend = asset.backend;
            let module = asset.module;
            let callbacks = asset.schema.callbacks;
            let (resolved, dropped) = asset.schema.resolve_properties(&view.properties);
            for name in dropped {
                self.diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    asset: view.asset,
                    display_path: String::new(),
                    line: 0,
                    column: 0,
                    message: format!(
                        "`{name}` is set on an attachment but the script no longer declares it"
                    ),
                });
            }

            let live = ScriptInstanceId::next();
            if let Err(error) = self.backends[backend].instantiate(live, module, &resolved) {
                self.diagnostics.push(diagnostic(
                    view.asset,
                    "<instantiate>",
                    &error.to_string(),
                ));
                report.failed.push((view.instance, error));
                continue;
            }

            self.instances.insert(
                view.instance,
                InstanceRecord {
                    entity: view.entity,
                    persistent: view.persistent,
                    asset: view.asset,
                    backend,
                    live,
                    order: view.order(),
                    state: LifecycleState::Loaded,
                    authored_enabled: view.enabled,
                    quarantined: false,
                    callbacks,
                    properties: view.properties.clone(),
                    consecutive_failures: 0,
                    pending_destroy: false,
                    inbox: Vec::new(),
                    spawn_results: Vec::new(),
                    rng_seed: seed_for(self.world_seed, view.instance),
                },
            );
            report.created.push(view.instance);
        }

        let vanished: Vec<InstanceUuid> = self
            .instances
            .keys()
            .filter(|id| !seen.contains(id))
            .copied()
            .collect();
        for id in vanished {
            self.retire(id, &mut report);
        }

        report
    }

    /// Mark one instance for teardown at the safe point.
    ///
    /// Deferred rather than immediate on purpose: an entity that despawns
    /// itself must finish the callback it is inside, and tearing the VM
    /// object down underneath a running call is how that turns into a
    /// crash instead of a feature.
    fn retire(&mut self, id: InstanceUuid, report: &mut ReconcileReport) {
        if let Some(record) = self.instances.get_mut(&id) {
            if !record.pending_destroy {
                record.pending_destroy = true;
                report.retired.push(id);
            }
        }
    }

    /// Release an instance immediately, without an `onDestroy`.
    ///
    /// Only for the rebuild path, where the attachment is not going away —
    /// it is being replaced with a fresh instance of the same authored
    /// data in the same pass. Anything that is actually *going away* goes
    /// through [`Self::retire`] and gets its destructor.
    fn destroy_now(&mut self, id: InstanceUuid) {
        if let Some(record) = self.instances.remove(&id) {
            self.ledger.release_all(id);
            self.backends[record.backend].unload(record.live);
        }
    }

    /// Mark for teardown every attachment on an entity that no longer
    /// exists.
    pub fn retire_dead_entities(&mut self, world: &dyn WorldView) {
        for record in self.instances.values_mut() {
            if !record.pending_destroy && !world.is_alive(record.entity) {
                record.pending_destroy = true;
            }
        }
    }

    /// Whether anything is waiting to be torn down.
    #[must_use]
    pub fn has_pending_destroy(&self) -> bool {
        self.instances
            .values()
            .any(|record| record.pending_destroy && record.state.is_live())
    }

    /// Whether any instance still owes an `onInit` or an `onStart`.
    ///
    /// Not the same question as "did the last reconcile create anything":
    /// a hot reload puts an existing attachment back into `Loaded` without
    /// creating one, and an init loop keyed on creation alone would leave
    /// every reloaded script permanently un-started.
    #[must_use]
    pub fn has_pending_init(&self) -> bool {
        self.instances.values().any(|record| {
            !record.pending_destroy
                && matches!(
                    record.state,
                    LifecycleState::Loaded | LifecycleState::Initialized
                )
        })
    }

    // ── Phases ─────────────────────────────────────────────────────

    /// Which instances take part in `callback`, in deterministic order.
    fn participants(&self, callback: Callback) -> Vec<InstanceUuid> {
        let mut chosen: Vec<(OrderKey, InstanceUuid)> = self
            .instances
            .iter()
            .filter(|(_, record)| participates(record, callback))
            .map(|(id, record)| (record.order, *id))
            .collect();
        chosen.sort_unstable();
        chosen.into_iter().map(|(_, id)| id).collect()
    }

    /// Run one lifecycle callback over every instance eligible for it.
    ///
    /// Failures are contained: one attachment raising costs that
    /// attachment its command batch and nothing else. Cross the failure
    /// threshold and it is switched off, logged, and skipped from then on.
    // Select, bucket, dispatch, then settle every participant's state — one
    // phase, in the order it happens. Splitting it would mean threading the
    // dispatch list and the failure list through three signatures to save a
    // line count, and would hide the fact that the settle pass runs for
    // *every* participant and not only the ones the VM was entered for,
    // which is the subtlety the function exists to get right.
    #[allow(clippy::too_many_lines)]
    pub fn run_phase(
        &mut self,
        callback: Callback,
        phase: &PhaseInput,
        world: &dyn WorldView,
        commands: &mut CommandBuffer,
    ) -> PhaseReport {
        let mut report = PhaseReport::default();
        let participants = self.participants(callback);
        if participants.is_empty() {
            return report;
        }

        // Bucket by backend, preserving order within each bucket. Command
        // apply order comes from `OrderKey` at drain time, so splitting the
        // phase across backends cannot change what the world sees.
        let mut buckets: BTreeMap<usize, Vec<Dispatch>> = BTreeMap::new();
        for id in &participants {
            let record = &self.instances[id];
            if !record.callbacks.has(callback) {
                // No such callback in the module: a silent no-op. The state
                // still advances below.
                continue;
            }
            buckets.entry(record.backend).or_default().push(Dispatch {
                instance: *id,
                live: record.live,
                order: record.order,
                snapshot: ScriptSnapshot {
                    time: phase.time,
                    input: phase.input.clone(),
                    self_entity: record.entity,
                    self_persistent: record.persistent,
                    self_components: BTreeMap::new(),
                    spawn_results: record.spawn_results.clone(),
                    events: if callback == Callback::Event {
                        record.inbox.clone()
                    } else {
                        Vec::new()
                    },
                    rng_seed: record.rng_seed,
                },
            });
        }

        let mut failed: Vec<(InstanceUuid, ScriptError)> = Vec::new();
        for (backend, dispatches) in &buckets {
            let calls: Vec<PhaseCall<'_>> = dispatches
                .iter()
                .map(|d| PhaseCall {
                    instance: d.live,
                    order: d.order,
                    snapshot: &d.snapshot,
                })
                .collect();
            report.invoked += calls.len();
            let errors = self.backends[*backend].invoke_phase(callback, &calls, world, commands);
            for (live, error) in errors {
                if let Some(dispatch) = dispatches.iter().find(|d| d.live == live) {
                    failed.push((dispatch.instance, error));
                }
            }
        }

        // Sorted so the diagnostics a frame produces are the same on every
        // run, not the order two backends happened to be visited in.
        failed.sort_by_key(|(id, _)| *id);

        for id in participants {
            let failure = failed.iter().find(|(failed_id, _)| *failed_id == id);
            let Some(record) = self.instances.get_mut(&id) else {
                continue;
            };

            // Consumed only by a phase the VM was actually entered for.
            // Clearing on a phase the module does not implement is how a
            // script with an `onFixedUpdate` and no `onUpdate` loses the
            // spawn results it was about to read: the update phase it does
            // not have would wipe them on the way past.
            if record.callbacks.has(callback) {
                if callback == Callback::Event {
                    record.inbox.clear();
                }
                if matches!(callback, Callback::FixedUpdate | Callback::Update) {
                    record.spawn_results.clear();
                }
            }

            if let Some((_, error)) = failure {
                record.consecutive_failures += 1;
                report.failures.push(InstanceFailure {
                    instance: id,
                    entity: record.entity,
                    asset: record.asset,
                    callback,
                    error: error.clone(),
                });
                let asset = record.asset;
                let over_threshold = record.consecutive_failures >= self.failure_threshold;
                if over_threshold && !record.quarantined {
                    record.quarantined = true;
                    // Quarantine is a hard stop for update phases; the
                    // instance stays alive so its state can still be
                    // inspected and it can be re-enabled after a fix.
                    if record.state == LifecycleState::Enabled {
                        record.state = LifecycleState::Disabled;
                    }
                    report.quarantined.push(id);
                    self.diagnostics.push(diagnostic(
                        asset,
                        "<runtime>",
                        &format!(
                            "{id} failed {} times in a row and was switched off",
                            self.failure_threshold
                        ),
                    ));
                }
                // A failed teardown still tears down: the alternative is an
                // instance that can never be released.
                if callback == Callback::Destroy {
                    record.state = LifecycleState::Destroyed;
                    report.advanced += 1;
                }
            } else {
                // Only a phase the VM was actually entered for counts as a
                // success. Otherwise a script that throws in
                // `onFixedUpdate` and has no `onUpdate` has its failure
                // count cleared by the update phase it does not implement,
                // and never reaches the quarantine threshold no matter how
                // often it fails.
                if record.callbacks.has(callback) {
                    record.consecutive_failures = 0;
                }
                if let Some(next) = advanced_state(callback) {
                    debug_assert!(
                        record.state.can_advance_to(next),
                        "{:?} → {next:?} is not a legal transition",
                        record.state
                    );
                    record.state = next;
                    report.advanced += 1;
                }
            }
        }

        report
    }

    /// Record that initialisation did not converge within
    /// [`MAX_INIT_CYCLES`], naming the spawn chain responsible.
    ///
    /// The loop itself belongs to the caller, because a script spawned in
    /// `onInit` does not exist in the world until the caller has applied
    /// its commands — only something holding `&mut World` can answer "what
    /// is attached now". What lives here is the cap and the report, so the
    /// number is stated once and the diagnostic names the offender rather
    /// than saying "too many cycles" and leaving whoever hits it to guess.
    pub fn report_init_did_not_settle(&mut self) {
        let mut stuck: Vec<String> = self
            .instances
            .values()
            .filter(|record| !record.state.has_started())
            .map(|record| record.asset.to_string())
            .collect();
        stuck.sort_unstable();
        stuck.dedup();
        self.diagnostics.push(Diagnostic {
            severity: Severity::Error,
            asset: ScriptAssetId::NONE,
            display_path: "<init>".into(),
            line: 0,
            column: 0,
            message: format!(
                "script initialisation did not settle after {MAX_INIT_CYCLES} cycles; \
                 still creating instances of: {}",
                if stuck.is_empty() {
                    "<unknown>".to_string()
                } else {
                    stuck.join(", ")
                }
            ),
        });
    }

    /// Deliver events produced by the last apply pass.
    ///
    /// Sequence numbers are assigned here, monotonically across the whole
    /// simulation, so a replay can assert that the same events arrived in
    /// the same order.
    pub fn queue_event(&mut self, name: String, source: Option<Entity>, payload: ReflectObject) {
        let event = ScriptEvent {
            name,
            sequence: self.next_event_sequence,
            source,
            payload,
        };
        self.next_event_sequence += 1;
        for record in self.instances.values_mut() {
            // Not `receives_updates()`: an event emitted from `onStart` is
            // queued while every instance is still `Started`, one phase
            // short of `Enabled`, and requiring `Enabled` here silently
            // drops it. Delivery is gated on *wanting* to be enabled;
            // dispatch is gated on actually being enabled, in
            // `participates`, so a switched-off attachment neither
            // receives nor accumulates.
            if record.wants_enabled()
                && record.state.has_started()
                && !record.pending_destroy
                && record.callbacks.has(Callback::Event)
            {
                record.inbox.push(event.clone());
            }
        }
    }

    /// Record which entities this phase's spawns produced, so the next
    /// snapshot can hand each script back what it asked for.
    pub fn record_spawns(&mut self, spawned: &[(OrderKey, SpawnToken, Entity)]) {
        for (order, token, entity) in spawned {
            if let Some(record) = self.instances.get_mut(&order.instance) {
                record.spawn_results.push((*token, *entity));
            }
        }
    }

    /// Tear down everything marked for destruction.
    ///
    /// Runs after `onDestroy`, at the safe point, and is the only place a
    /// VM object is released. Returns the resources each instance owned so
    /// the caller can stop the sounds and cancel the tasks.
    pub fn collect(&mut self) -> Vec<(InstanceUuid, Vec<OwnedResource>)> {
        let doomed: Vec<InstanceUuid> = self
            .instances
            .iter()
            .filter(|(_, record)| record.pending_destroy || record.state == LifecycleState::Destroyed)
            .map(|(id, _)| *id)
            .collect();

        let mut released = Vec::with_capacity(doomed.len());
        for id in doomed {
            let Some(record) = self.instances.remove(&id) else {
                continue;
            };
            // Subscriptions go before the VM object does, following Fyrox:
            // an event delivered to a half-torn-down instance is the bug
            // this ordering exists to prevent.
            let owned = self.ledger.release_all(id);
            self.backends[record.backend].unload(record.live);
            released.push((id, owned));
        }
        released
    }

    // ── Ownership ──────────────────────────────────────────────────

    /// Record that an attachment now holds an engine resource.
    pub fn acquire(&mut self, owner: InstanceUuid, resource: OwnedResource) -> OwnershipToken {
        self.ledger.acquire(owner, resource)
    }

    /// Give one resource back early.
    pub fn release(&mut self, token: OwnershipToken) -> Option<OwnedResource> {
        self.ledger.release(token)
    }

    /// How many engine resources are held across every attachment.
    #[must_use]
    pub fn owned_resources(&self) -> usize {
        self.ledger.total()
    }

    // ── Inspection ─────────────────────────────────────────────────

    /// Where one attachment is in its lifecycle.
    #[must_use]
    pub fn state_of(&self, instance: InstanceUuid) -> Option<LifecycleState> {
        self.instances.get(&instance).map(|record| record.state)
    }

    /// Every live attachment, in durable-id order.
    pub fn instances(&self) -> impl Iterator<Item = InstanceUuid> + '_ {
        self.instances.keys().copied()
    }

    /// One row of what the editor's attachment list shows.
    #[must_use]
    pub fn summary(&self, instance: InstanceUuid) -> Option<InstanceSummary> {
        let record = self.instances.get(&instance)?;
        Some(InstanceSummary {
            instance,
            entity: record.entity,
            asset: record.asset,
            state: record.state,
            quarantined: record.quarantined,
            authored_enabled: record.authored_enabled,
            consecutive_failures: record.consecutive_failures,
        })
    }

    /// Whether an attachment has been switched off by the error
    /// quarantine, as opposed to by its author.
    #[must_use]
    pub fn is_quarantined(&self, instance: InstanceUuid) -> bool {
        self.instances
            .get(&instance)
            .is_some_and(|record| record.quarantined)
    }

    /// Clear a quarantine, restoring the author's `enabled` flag.
    pub fn clear_quarantine(&mut self, instance: InstanceUuid) {
        if let Some(record) = self.instances.get_mut(&instance) {
            record.quarantined = false;
            record.consecutive_failures = 0;
        }
    }

    /// How many live instances there are. Reload tests assert on this
    /// returning to where it started.
    #[must_use]
    pub fn live_instances(&self) -> usize {
        self.instances.len()
    }

    /// Bytes attributed to every backend's VM.
    #[must_use]
    pub fn memory_used(&self) -> usize {
        self.backends.iter().map(|b| b.memory_used()).sum()
    }

    /// Ask an instance for its declared state.
    ///
    /// # Errors
    ///
    /// Whatever the script raised, or that there is no such instance.
    pub fn export_state(&mut self, instance: InstanceUuid) -> Result<ScriptValue, ScriptError> {
        let record = self
            .instances
            .get(&instance)
            .ok_or(ScriptError::HostRejected {
                message: format!("no live attachment {instance}"),
            })?;
        let (backend, live) = (record.backend, record.live);
        self.backends[backend].export_state(live)
    }

    /// Give an instance previously exported state.
    ///
    /// # Errors
    ///
    /// Whatever the script raised, or that there is no such instance.
    pub fn import_state(
        &mut self,
        instance: InstanceUuid,
        state: ScriptValue,
    ) -> Result<(), ScriptError> {
        let record = self
            .instances
            .get(&instance)
            .ok_or(ScriptError::HostRejected {
                message: format!("no live attachment {instance}"),
            })?;
        let (backend, live) = (record.backend, record.live);
        self.backends[backend].import_state(live, state)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Free functions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Whether an instance is eligible for a callback.
fn participates(record: &InstanceRecord, callback: Callback) -> bool {
    // Teardown outranks everything: an attachment on its way out does not
    // get another update, and it does get `onDisable` then `onDestroy` —
    // the same order a hot reload uses, so a script that releases things
    // in `onDisable` does not need a second copy of that code.
    if record.pending_destroy {
        return match callback {
            Callback::Disable => record.state == LifecycleState::Enabled,
            Callback::Destroy => record.state.is_live(),
            _ => false,
        };
    }
    match callback {
        Callback::Init => record.state == LifecycleState::Loaded,
        Callback::Start => record.state == LifecycleState::Initialized,
        Callback::Enable => {
            matches!(
                record.state,
                LifecycleState::Started | LifecycleState::Disabled
            ) && record.wants_enabled()
        }
        Callback::Disable => record.state == LifecycleState::Enabled && !record.wants_enabled(),
        Callback::FixedUpdate | Callback::Update => record.state.receives_updates(),
        Callback::Event => record.state.receives_updates() && !record.inbox.is_empty(),
        // Teardown of a live instance is requested by `pending_destroy`,
        // handled above.
        Callback::Destroy | Callback::SaveState | Callback::LoadState => false,
    }
}

/// The state an instance moves to when `callback` succeeds, if any.
///
/// Deliberately independent of whether the module *defines* the callback:
/// a script with no `onInit` still becomes `Initialized`. Tying the
/// transition to the function would strand every such script in `Loaded`.
const fn advanced_state(callback: Callback) -> Option<LifecycleState> {
    match callback {
        Callback::Init => Some(LifecycleState::Initialized),
        Callback::Start => Some(LifecycleState::Started),
        Callback::Enable => Some(LifecycleState::Enabled),
        Callback::Disable => Some(LifecycleState::Disabled),
        Callback::Destroy => Some(LifecycleState::Destroyed),
        Callback::FixedUpdate
        | Callback::Update
        | Callback::Event
        | Callback::SaveState
        | Callback::LoadState => None,
    }
}

/// A per-attachment random seed that is the same on every replay and
/// different for every attachment.
///
/// `SplitMix64`'s finaliser: cheap, and it decorrelates two attachments
/// whose uuids differ in one bit, which a plain `xor` would not.
fn seed_for(world_seed: u64, instance: InstanceUuid) -> u64 {
    let raw = instance.raw();
    #[allow(clippy::cast_possible_truncation)]
    let folded = (raw as u64) ^ ((raw >> 64) as u64);
    let mut z = world_seed.wrapping_add(folded).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn diagnostic(asset: ScriptAssetId, display_path: &str, message: &str) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        asset,
        display_path: display_path.to_owned(),
        line: 0,
        column: 0,
        message: message.to_owned(),
    }
}

fn single(asset: ScriptAssetId, display_path: &str, message: &str) -> Diagnostics {
    let mut diagnostics = Diagnostics::default();
    diagnostics.push(diagnostic(asset, display_path, message));
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_order_key_of_a_view_is_built_from_authored_data_only() {
        let view = AttachmentView {
            entity: Entity::DANGLING,
            persistent: PersistentId::from_raw(9),
            instance: InstanceUuid::from_raw(4),
            asset: ScriptAssetId::from_raw(1),
            enabled: true,
            execution_order: -3,
            properties: PropertyBag::new(),
        };
        let key = view.order();
        assert_eq!(key.execution_order, -3);
        assert_eq!(key.entity, PersistentId::from_raw(9));
        assert_eq!(key.instance, InstanceUuid::from_raw(4));
    }

    #[test]
    fn every_attachment_gets_a_distinct_stable_seed() {
        let a = seed_for(1, InstanceUuid::from_raw(1));
        let b = seed_for(1, InstanceUuid::from_raw(2));
        assert_ne!(a, b, "two attachments must not share a random stream");
        assert_eq!(a, seed_for(1, InstanceUuid::from_raw(1)), "and it replays");
        assert_ne!(a, seed_for(2, InstanceUuid::from_raw(1)), "world seed matters");
    }

    #[test]
    fn a_runtime_with_no_backend_refuses_the_asset_with_a_diagnostic() {
        let mut runtime = ScriptRuntime::new(Budget::default());
        let source = ScriptSource {
            id: ScriptAssetId::mint(),
            language: LanguageTag::LUAU,
            display_path: "a.luau".into(),
            text: String::new(),
        };
        let error = runtime.load_asset(source).unwrap_err();
        assert!(error.has_errors());
        assert!(error.to_string().contains("no backend"));
        assert_eq!(runtime.take_diagnostics().len(), 1);
    }

    #[test]
    fn an_attachment_whose_asset_is_missing_is_reported_not_dropped() {
        let mut runtime = ScriptRuntime::new(Budget::default());
        let view = AttachmentView {
            entity: Entity::DANGLING,
            persistent: PersistentId::from_raw(1),
            instance: InstanceUuid::from_raw(1),
            asset: ScriptAssetId::from_raw(77),
            enabled: true,
            execution_order: 0,
            properties: PropertyBag::new(),
        };
        let report = runtime.reconcile(std::slice::from_ref(&view));
        assert_eq!(report.missing_assets.len(), 1);
        assert!(report.created.is_empty());
        assert_eq!(runtime.live_instances(), 0);

        // The attachment is not forgotten: reconciling again retries it,
        // which is what makes "import the script later" work.
        let again = runtime.reconcile(std::slice::from_ref(&view));
        assert_eq!(again.missing_assets.len(), 1);
    }

    #[test]
    fn the_ownership_ledger_is_reachable_through_the_runtime() {
        let mut runtime = ScriptRuntime::new(Budget::default());
        let owner = InstanceUuid::from_raw(1);
        let token = runtime.acquire(owner, OwnedResource::Audio(3));
        assert_eq!(runtime.owned_resources(), 1);
        assert_eq!(runtime.release(token), Some(OwnedResource::Audio(3)));
        assert_eq!(runtime.owned_resources(), 0);
    }
}
