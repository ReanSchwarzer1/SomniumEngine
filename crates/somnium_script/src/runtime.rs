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
use crate::capability::Capabilities;
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
    /// The module this one replaced, kept for exactly one generation.
    ///
    /// One, not a history: the case it exists for is "the reload compiled
    /// but the new code is broken, put it back". Keeping more would mean
    /// holding VM objects for edits nobody is going to return to.
    previous: Option<CompiledModule>,
    /// The source of that previous module, so a rollback can re-describe
    /// it rather than trusting a cached schema.
    previous_source: Option<ScriptSource>,
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
    /// The script's own schema version at the moment those properties were
    /// resolved. A reload compares against the new one to decide whether
    /// migration is needed.
    schema_version: u32,
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
    /// Module name → asset, for resolving `require`.
    module_keys: BTreeMap<String, ScriptAssetId>,
    /// Asset → the path it was loaded from, recorded when a load is
    /// *attempted*.
    ///
    /// Separate from `assets` on purpose: the diagnostic that most needs a
    /// readable name is the `require` cycle, and in a cycle neither file
    /// has finished loading, so neither is in `assets` to be named from.
    module_paths: BTreeMap<ScriptAssetId, String>,
    /// The static dependency graph: asset → what it requires.
    ///
    /// Built from source at compile time, never observed at run time. It
    /// is what tells a reload the blast radius of an edit before anything
    /// is touched, and what makes cycle detection one pass over a graph
    /// rather than a guard that fires on the unlucky path.
    dependencies: BTreeMap<ScriptAssetId, Vec<ScriptAssetId>>,
    /// `require`s that named a module nothing is loaded under, so the
    /// caller can go and import it.
    unresolved: Vec<(ScriptAssetId, String)>,
    /// Attachments whose authored properties a migration rewrote.
    ///
    /// These have to reach the **world**, not just the live instance: the
    /// next reconcile compares the instance's properties against the
    /// authored ones and rebuilds on a difference, so a migration that
    /// only updated the instance would be undone on the very next frame —
    /// and would never be saved. The runtime cannot touch the world, so it
    /// reports and the host writes.
    migrated: Vec<(InstanceUuid, PropertyBag)>,
    /// What a script may ask the engine for when its package says nothing.
    default_capabilities: Capabilities,
    /// Per-package overrides, keyed by asset.
    capabilities: BTreeMap<ScriptAssetId, Capabilities>,
    /// Cooked bytecode offered for the next load of an asset. Consumed on
    /// use, and ignored entirely if its fingerprint does not match.
    offered: BTreeMap<ScriptAssetId, (String, Vec<u8>)>,
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
            module_keys: BTreeMap::new(),
            module_paths: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            unresolved: Vec::new(),
            migrated: Vec::new(),
            // A project's own scripts, until something says otherwise.
            // The mod tier is the case that narrows this, and it does not
            // exist yet; defaulting to the narrow set would mean every
            // ordinary script started out unable to spawn anything.
            default_capabilities: Capabilities::PROJECT,
            capabilities: BTreeMap::new(),
            offered: BTreeMap::new(),
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

    // ── Capabilities ───────────────────────────────────────────────

    /// What a script may ask for when its package declares nothing.
    pub fn set_default_capabilities(&mut self, capabilities: Capabilities) {
        self.default_capabilities = capabilities;
    }

    /// Grant one script package exactly this set.
    pub fn set_capabilities(&mut self, asset: ScriptAssetId, capabilities: Capabilities) {
        self.capabilities.insert(asset, capabilities);
    }

    /// What one asset is allowed to do.
    #[must_use]
    pub fn capabilities_of(&self, asset: ScriptAssetId) -> Capabilities {
        self.capabilities
            .get(&asset)
            .copied()
            .unwrap_or(self.default_capabilities)
    }

    /// What the attachment behind an [`OrderKey`] is allowed to do.
    ///
    /// Keyed by the order key because that is what a queued command
    /// carries: the command applier knows who emitted it and nothing else,
    /// which is exactly enough.
    #[must_use]
    pub fn capabilities_for(&self, order: OrderKey) -> Capabilities {
        self.instances
            .get(&order.instance)
            .map_or(self.default_capabilities, |record| {
                self.capabilities_of(record.asset)
            })
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
            self.diagnostics
                .extend(diagnostics.messages.iter().cloned());
            return Err(diagnostics);
        };

        let id = source.id;
        // Replacing an asset in place would strand live instances against
        // a module that is about to be released; `reload_asset` is the path
        // that handles that, and it is what this defers to.
        if self.assets.contains_key(&id) {
            return self.reload_asset(source).map(|()| id);
        }

        // The name has to be registered before the link pass, so that a
        // module can be required by something loaded after it *and* by
        // something loaded before it once that is relinked.
        self.module_keys
            .insert(module_key(&source.display_path), id);
        self.module_paths.insert(id, source.display_path.clone());
        let (module, schema) = self.build(backend, &source)?;
        self.assets.insert(
            id,
            AssetRecord {
                source,
                backend,
                module,
                schema,
                previous: None,
                previous_source: None,
            },
        );
        // Anything that failed to resolve this module by name earlier can
        // now succeed.
        self.relink_all();
        Ok(id)
    }

    /// Offer a cooked artifact for the next [`Self::load_asset`] of this
    /// source.
    ///
    /// Not a separate load path: the artifact is a *hint*, and if its
    /// fingerprint does not match this runtime the backend compiles from
    /// source and nobody notices. That is what makes bytecode a cache
    /// rather than storage.
    pub fn offer_bytecode(&mut self, asset: ScriptAssetId, fingerprint: String, bytecode: Vec<u8>) {
        self.offered.insert(asset, (fingerprint, bytecode));
    }

    /// The fingerprint a cook must record beside its bytes.
    #[must_use]
    pub fn runtime_fingerprint(&self, language: LanguageTag) -> Option<String> {
        self.backend_index(language)
            .map(|index| self.backends[index].runtime_fingerprint())
    }

    /// The bytecode of a loaded asset, for a cook to store.
    #[must_use]
    pub fn bytecode_of(&self, asset: ScriptAssetId) -> Option<Vec<u8>> {
        let record = self.assets.get(&asset)?;
        self.backends[record.backend].bytecode(record.module)
    }

    /// Compile, link and describe one source, in that order.
    ///
    /// The order is forced: describing a module runs its top-level code,
    /// and that code may `require`, so the imports have to be bound first.
    /// The requires themselves come out of the source text at compile
    /// time, which is what breaks the circularity.
    fn build(
        &mut self,
        backend: usize,
        source: &ScriptSource,
    ) -> Result<(CompiledModule, ScriptSchema), Diagnostics> {
        let module = match self.offered.remove(&source.id) {
            Some((fingerprint, bytecode)) => self.backends[backend]
                .load_bytecode(source, &bytecode, &fingerprint)
                .inspect_err(|d| self.diagnostics.extend(d.messages.iter().cloned()))?,
            None => self.backends[backend].compile(source).inspect_err(|d| {
                self.diagnostics.extend(d.messages.iter().cloned());
            })?,
        };

        if let Err(diagnostics) = self.link_module(source.id, &source.display_path, module) {
            self.backends[backend].release_module(module);
            self.diagnostics
                .extend(diagnostics.messages.iter().cloned());
            return Err(diagnostics);
        }

        match self.backends[backend].describe(module) {
            Ok(schema) => Ok((module, schema)),
            Err(diagnostics) => {
                // The module compiled but does not declare itself. Release
                // it rather than leaving a module nothing can instantiate.
                self.backends[backend].release_module(module);
                self.dependencies.remove(&source.id);
                self.diagnostics
                    .extend(diagnostics.messages.iter().cloned());
                Err(diagnostics)
            }
        }
    }

    /// Resolve one module's `require`s, reject cycles, and hand the
    /// backend the answer.
    /// `display_path` is passed rather than looked up, because the most
    /// common caller is an asset **being loaded for the first time** — it
    /// is not in `assets` yet, and without its path a sibling `require`
    /// has nothing to be relative to.
    fn link_module(
        &mut self,
        asset: ScriptAssetId,
        display_path: &str,
        module: CompiledModule,
    ) -> Result<(), Diagnostics> {
        let backend = module_backend(&self.backends, module.language)
            .ok_or_else(|| single(asset, "<link>", "no backend is registered for this module"))?;
        let names = self.backends[backend].module_requires(module);

        // ── 1. Names to assets ───────────────────────────────────────
        //
        // Through `module_keys`, which is populated the moment a load is
        // *attempted*, not when it succeeds. That matters for a cycle: two
        // files that require each other both fail their first link, and if
        // resolution needed a loaded target neither edge would ever be
        // recorded and the cycle would report as "not loaded" forever.
        let mut edges = Vec::with_capacity(names.len());
        for name in &names {
            let Some(target) = self.resolve_module(display_path, name) else {
                self.unresolved.push((asset, name.clone()));
                return Err(single(
                    asset,
                    display_path,
                    &format!("`require(\"{name}\")` names no loaded script"),
                ));
            };
            edges.push(target);
        }

        // ── 2. The cycle check, on the graph that is about to exist ──
        //
        // Recorded before the availability check below, and deliberately
        // *left* recorded if that check fails: the half-built graph is how
        // the second file in a cycle discovers the first one's edge.
        let previous = self.dependencies.insert(asset, edges.clone());
        if let Some(cycle) = self.find_cycle(asset) {
            match previous {
                Some(old) => self.dependencies.insert(asset, old),
                None => self.dependencies.remove(&asset),
            };
            return Err(single(
                asset,
                display_path,
                &format!("`require` cycle: {}", cycle.join(" → ")),
            ));
        }

        // ── 3. Are they actually loaded? ─────────────────────────────
        let mut imports = Vec::with_capacity(names.len());
        for (name, target) in names.iter().zip(&edges) {
            let Some(record) = self.assets.get(target) else {
                self.unresolved.push((asset, name.clone()));
                return Err(single(
                    asset,
                    display_path,
                    &format!("`require(\"{name}\")` names a script that is not loaded"),
                ));
            };
            imports.push((name.clone(), record.module));
        }

        self.backends[backend]
            .link(module, &imports)
            .map_err(|error| single(asset, display_path, &error.to_string()))
    }

    /// Re-link every asset, so a newly loaded module satisfies whoever was
    /// waiting for it. Cheap: linking is a name lookup and a handle copy.
    fn relink_all(&mut self) {
        let all: Vec<(ScriptAssetId, String, CompiledModule)> = self
            .assets
            .iter()
            .map(|(id, record)| (*id, record.source.display_path.clone(), record.module))
            .collect();
        self.unresolved.clear();
        for (id, path, module) in all {
            // A failure here is already recorded in `unresolved`, and the
            // asset stays loaded but unlinked — which is exactly the state
            // "its dependency has not been imported yet" should produce.
            let _ = self.link_module(id, &path, module);
        }
    }

    /// Map a module name written by a script onto a loaded asset.
    ///
    /// Three spellings, in order of specificity: relative to the requiring
    /// file's folder, then the project-relative path, then the same with
    /// the content root's name prepended — so `require("scripts/util")`
    /// finds `assets/scripts/util.luau` without the author writing
    /// `assets/` in every file.
    fn resolve_module(&self, requester: &str, name: &str) -> Option<ScriptAssetId> {
        let folder = module_key(requester)
            .rsplit_once('/')
            .map(|(dir, _)| dir.to_string());

        let mut candidates = Vec::with_capacity(3);
        if let Some(folder) = folder {
            candidates.push(format!("{folder}/{name}"));
        }
        candidates.push(name.to_string());
        candidates.push(format!("assets/{name}"));

        candidates
            .into_iter()
            .find_map(|candidate| self.module_keys.get(&module_key(&candidate)).copied())
    }

    /// The first `require` cycle reachable from `start`, as a path.
    fn find_cycle(&self, start: ScriptAssetId) -> Option<Vec<String>> {
        let mut stack = vec![start];
        let mut visiting = vec![start];
        let mut done: Vec<ScriptAssetId> = Vec::new();
        // Iterative DFS: a deep graph must not blow the Rust stack, and a
        // cycle is exactly the case where a naive recursion would not
        // terminate.
        let mut frames: Vec<usize> = vec![0];
        while let Some(&node) = stack.last() {
            let index = *frames.last().unwrap_or(&0);
            let children = self.dependencies.get(&node).cloned().unwrap_or_default();
            if index >= children.len() {
                stack.pop();
                frames.pop();
                visiting.retain(|id| *id != node);
                done.push(node);
                continue;
            }
            *frames.last_mut().unwrap() += 1;
            let child = children[index];
            if visiting.contains(&child) {
                let mut path: Vec<String> = visiting.iter().map(|id| self.name_of(*id)).collect();
                path.push(self.name_of(child));
                return Some(path);
            }
            if done.contains(&child) {
                continue;
            }
            stack.push(child);
            visiting.push(child);
            frames.push(0);
        }
        None
    }

    fn name_of(&self, asset: ScriptAssetId) -> String {
        self.assets
            .get(&asset)
            .map(|record| record.source.display_path.clone())
            .or_else(|| self.module_paths.get(&asset).cloned())
            .unwrap_or_else(|| asset.to_string())
    }

    /// Every asset that would have to be rebuilt if `asset` changed —
    /// itself, plus everything that requires it, transitively.
    ///
    /// This is the blast radius, and computing it from the static graph is
    /// why the graph has to be static.
    #[must_use]
    pub fn blast_radius(&self, asset: ScriptAssetId) -> Vec<ScriptAssetId> {
        let mut affected = vec![asset];
        let mut index = 0;
        while index < affected.len() {
            let current = affected[index];
            index += 1;
            for (dependent, deps) in &self.dependencies {
                if deps.contains(&current) && !affected.contains(dependent) {
                    affected.push(*dependent);
                }
            }
        }
        affected
    }

    /// What one module requires, resolved.
    #[must_use]
    pub fn dependencies_of(&self, asset: ScriptAssetId) -> Vec<ScriptAssetId> {
        self.dependencies.get(&asset).cloned().unwrap_or_default()
    }

    /// `require`s that named nothing loaded, so the caller can import them
    /// and try again.
    pub fn take_unresolved(&mut self) -> Vec<(ScriptAssetId, String)> {
        std::mem::take(&mut self.unresolved)
    }

    /// Authored property bags a migration rewrote, for the caller to write
    /// back into the world. See the field's documentation for why this is
    /// not optional.
    pub fn take_migrated_properties(&mut self) -> Vec<(InstanceUuid, PropertyBag)> {
        std::mem::take(&mut self.migrated)
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
        self.assets.iter().map(|(id, record)| (*id, &record.source))
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
        let previous_module = existing.module;
        let previous_source = existing.source.clone();
        let superseded = existing.previous;

        // ── 1–3. The shadow build ────────────────────────────────────
        //
        // Compile, link and describe before anything live is touched. A
        // failure here returns with **every live instance still running**
        // and nothing about the world changed — that is the property this
        // ordering exists for, and it is what makes the reload key safe to
        // lean on.
        let (module, schema) = self.build(backend, &source)?;

        // From here the swap is committed.
        self.assets.insert(
            asset,
            AssetRecord {
                source,
                backend,
                module,
                schema,
                previous: Some(previous_module),
                previous_source: Some(previous_source),
            },
        );
        // The generation before last is now unreachable and can go.
        if let Some(stale) = superseded {
            self.backends[backend].release_module(stale);
        }

        // ── The blast radius ─────────────────────────────────────────
        //
        // Everything that requires this module, transitively, holds a
        // reference to what the old one evaluated to. Relinking clears
        // that, and their instances are rebuilt alongside this module's.
        let affected_assets = self.blast_radius(asset);
        for dependent in &affected_assets {
            if *dependent == asset {
                continue;
            }
            if let Some(record) = self.assets.get(dependent) {
                let (id, path, module) = (
                    *dependent,
                    record.source.display_path.clone(),
                    record.module,
                );
                let _ = self.link_module(id, &path, module);
            }
        }

        self.rebuild_instances(&affected_assets);
        Ok(())
    }

    /// Steps 4–8 of the reload: carry state across, tear the old VM
    /// objects down, build the new ones, migrate, and put the state back.
    ///
    /// The lifecycle is *not* replayed here — the instances are left in
    /// `Loaded` and the scheduler runs `onInit`, `onStart` and `onEnable`
    /// at the next sync, which is the frame boundary the plan asks the
    /// swap to commit at.
    fn rebuild_instances(&mut self, assets: &[ScriptAssetId]) {
        let affected: Vec<InstanceUuid> = self
            .instances
            .iter()
            .filter(|(_, record)| assets.contains(&record.asset) && record.state.is_live())
            .map(|(id, _)| *id)
            .collect();

        let mut carried: Vec<(InstanceUuid, ScriptValue, u32)> = Vec::with_capacity(affected.len());
        for id in &affected {
            let record = &self.instances[id];
            let (backend, live) = (record.backend, record.live);
            let from_version = record.schema_version;
            let state = self.backends[backend]
                .export_state(live)
                .unwrap_or(ScriptValue::Nil);
            carried.push((*id, state, from_version));
            // Owned resources do not survive a reload: a coroutine, a task
            // and an audio voice are all VM-lifetime things. Released
            // before the VM object, so nothing is delivered to a
            // half-torn-down instance.
            self.ledger.release_all(*id);
            self.backends[backend].unload(live);
        }

        for (id, state, from_version) in carried {
            let Some(record) = self.instances.get(&id) else {
                continue;
            };
            let asset = record.asset;
            let Some(entry) = self.assets.get(&asset) else {
                continue;
            };
            let (backend, module, schema) = (entry.backend, entry.module, entry.schema.clone());
            let properties = record.properties.clone();

            // ── 7. Property migration ────────────────────────────────
            let (migrated, notes) =
                self.migrate_properties(backend, module, &schema, &properties, from_version, asset);
            for note in notes {
                self.diagnostics.push(note);
            }
            if migrated != properties {
                self.migrated.push((id, migrated.clone()));
            }
            let (resolved, dropped) = schema.resolve_properties(&migrated);
            for name in dropped {
                self.diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    asset,
                    display_path: String::new(),
                    line: 0,
                    column: 0,
                    message: format!(
                        "property `{name}` no longer exists in the script and was dropped"
                    ),
                });
            }

            let live = ScriptInstanceId::next();
            match self.backends[backend].instantiate(live, module, &resolved) {
                Ok(()) => {
                    // ── 8. `loadState`, then let the scheduler replay
                    // `onInit`, `onStart`, `onEnable`.
                    let _ = self.backends[backend].import_state(live, state);
                    if let Some(record) = self.instances.get_mut(&id) {
                        record.live = live;
                        record.callbacks = schema.callbacks;
                        record.properties = migrated;
                        record.schema_version = schema.schema_version;
                        record.state = LifecycleState::Loaded;
                        record.consecutive_failures = 0;
                    }
                }
                Err(error) => {
                    if let Some(record) = self.instances.get_mut(&id) {
                        record.state = LifecycleState::Destroyed;
                    }
                    self.diagnostics.push(diagnostic(
                        asset,
                        "<reload>",
                        &format!("{id} did not survive the reload: {error}"),
                    ));
                }
            }
        }

        self.instances
            .retain(|_, record| record.state != LifecycleState::Destroyed);
    }

    /// Run the script's own property migration, if its schema version
    /// moved and it declared one.
    ///
    /// A rename is the case this exists for: dropping `speed` and adding
    /// `velocity` loses every value an author set, and only the person who
    /// made that change knows they are the same field.
    fn migrate_properties(
        &mut self,
        backend: usize,
        module: CompiledModule,
        schema: &ScriptSchema,
        properties: &PropertyBag,
        from_version: u32,
        asset: ScriptAssetId,
    ) -> (PropertyBag, Vec<Diagnostic>) {
        if from_version == schema.schema_version || !schema.callbacks.has(Callback::MigrateState) {
            return (properties.clone(), Vec::new());
        }
        match self.backends[backend].migrate_properties(module, properties, from_version) {
            Ok(mut migrated) => (
                {
                    // The bag came back through a runtime with one number
                    // type, so a float that went in as `9.0` can come back
                    // as `9`. The declared type is the authority, and this
                    // is what stops the scene *recording* the wrong one.
                    for (name, value) in &mut migrated {
                        if let Some(field) = schema.field(name) {
                            *value = field.ty.coerce(value.clone());
                        }
                    }
                    migrated
                },
                vec![Diagnostic {
                    severity: Severity::Hint,
                    asset,
                    display_path: String::new(),
                    line: 0,
                    column: 0,
                    message: format!(
                        "migrated authored properties from schema version {from_version} to {}",
                        schema.schema_version
                    ),
                }],
            ),
            Err(error) => (
                properties.clone(),
                vec![diagnostic(
                    asset,
                    "<migrate>",
                    &format!("migrateProperties failed, keeping the authored values: {error}"),
                )],
            ),
        }
    }

    /// Put the previous generation of a module back.
    ///
    /// The case this exists for is "it compiled, and it is wrong". One
    /// generation deep, because that is the edit anyone is going to
    /// return to.
    ///
    /// # Errors
    ///
    /// That there is nothing to roll back to, or the old source's
    /// diagnostics — which would mean the old module no longer compiles,
    /// and is worth hearing about.
    pub fn rollback_asset(&mut self, asset: ScriptAssetId) -> Result<(), Diagnostics> {
        let Some(record) = self.assets.get(&asset) else {
            return Err(single(asset, "<rollback>", "no such script"));
        };
        let Some(source) = record.previous_source.clone() else {
            return Err(single(
                asset,
                "<rollback>",
                "there is no previous generation of this script to return to",
            ));
        };
        // Deliberately the ordinary reload path: rolling back is loading
        // the old text, and a second code path here would be a second set
        // of bugs.
        self.reload_asset(source)
    }

    /// Whether a script has a generation to roll back to.
    #[must_use]
    pub fn can_rollback(&self, asset: ScriptAssetId) -> bool {
        self.assets
            .get(&asset)
            .is_some_and(|record| record.previous_source.is_some())
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
            let schema_version = asset.schema.schema_version;
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
                self.diagnostics
                    .push(diagnostic(view.asset, "<instantiate>", &error.to_string()));
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
                    schema_version,
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
            .filter(|(_, record)| {
                record.pending_destroy || record.state == LifecycleState::Destroyed
            })
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
        // handled above. State export/import and property migration are
        // driven explicitly by the reload, not by a phase.
        Callback::Destroy | Callback::SaveState | Callback::LoadState | Callback::MigrateState => {
            false
        }
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
        | Callback::LoadState
        | Callback::MigrateState => None,
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
    let mut z = world_seed
        .wrapping_add(folded)
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The key a module is looked up by: lower-cased, forward slashes, and no
/// script extension.
///
/// The same file has to resolve to the same module on Windows and Linux,
/// and `require("scripts/util")` has to find `scripts/util.luau`.
fn module_key(display_path: &str) -> String {
    let normalised = display_path.replace('\\', "/").to_ascii_lowercase();
    normalised
        .strip_suffix(".luau")
        .or_else(|| normalised.strip_suffix(".lua"))
        .unwrap_or(&normalised)
        .trim_start_matches("./")
        .to_string()
}

/// Which backend runs a language. A free function so it can be called
/// while another field of the runtime is borrowed.
fn module_backend(backends: &[Box<dyn ScriptBackend>], language: LanguageTag) -> Option<usize> {
    backends
        .iter()
        .position(|backend| backend.language() == language)
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
        assert_ne!(
            a,
            seed_for(2, InstanceUuid::from_raw(1)),
            "world seed matters"
        );
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
