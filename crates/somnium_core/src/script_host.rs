//! Phase 16-C: the thing that drives scripts from the frame loop.
//!
//! [`ScriptRuntime`] owns the lifecycle and the order; it deliberately
//! knows nothing about `World`, `PhysicsWorld` or `AudioEngine`.
//! [`ScriptHost`] is where those meet: it reads the authored
//! [`ScriptSet`]s out of the world, runs a phase, applies the commands
//! that came back, and routes the side effects the applier hands over as
//! data.
//!
//! # Where the fixed point lives, and why it is here
//!
//! A script spawned during `onInit` does not exist in the world until its
//! spawn command has been applied. Only something holding `&mut World`
//! can answer "what is attached now", so the init loop is here — while
//! the cap it obeys, [`MAX_INIT_CYCLES`], and the diagnostic it raises
//! belong to the runtime, so the number is stated once.
//!
//! # Physics and audio are routed, not called
//!
//! `ApplyForce` names an entity; Jolt wants a body id, and the map from
//! one to the other is game-layer knowledge — `somnium_core` has no
//! rigid-body component. So the host takes a [`ForceRouter`] the game
//! installs once, rather than guessing. An uninstalled router turns a
//! force into a rejection with a message that says so, which is a much
//! better failure than a silent no-op.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use somnium_audio::engine::AudioEngine;
use somnium_audio::sound::{SoundHandle, SoundSettings};
use somnium_ecs::reflect::TypeRegistry;
use somnium_ecs::{Entity, World};
use somnium_physics::world::PhysicsWorld;
use somnium_script::attachment::ScriptSet;
use somnium_script::backend::{Budget, Callback, Diagnostic, Diagnostics, ScriptSource, Severity};
use somnium_script::command::{CommandBuffer, ForceMode, LogLevel};
use somnium_script::ids::{InstanceUuid, LanguageTag, ScriptAssetId};
use somnium_script::lifecycle::LifecycleState;
use somnium_script::ownership::OwnedResource;
use somnium_script::runtime::{
    AttachmentView, InstanceFailure, MAX_INIT_CYCLES, PhaseInput, ScriptRuntime,
};
use somnium_script::snapshot::{InputSnapshot, TimeSnapshot};
use somnium_script::value::ScriptValue;

use crate::reflect_registry::component_registry;
use crate::script_bridge::{ApplyOutcome, EngineWorldView, apply_commands};

/// Turns a script's `applyForce` into whatever the game's physics
/// representation needs.
///
/// Installed by game code, because the entity-to-body mapping is game
/// code's. See the module docs.
pub type ForceRouter =
    Box<dyn FnMut(&World, &mut PhysicsWorld, Entity, glam::Vec3, ForceMode) + Send>;

/// One line a script wrote to the output log.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptLogLine {
    /// Severity as the script asked for it.
    pub level: LogLevel,
    /// Which attachment wrote it.
    pub instance: InstanceUuid,
    /// The text.
    pub message: String,
}

impl std::fmt::Display for ScriptLogLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tag = match self.level {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        };
        write!(f, "[script {tag}] {}", self.message)
    }
}

/// The subsystems a phase's side effects are routed into.
///
/// Both optional so the whole host runs headlessly in a test, which is
/// where most of its behaviour is actually proven.
#[derive(Default)]
pub struct HostServices<'a> {
    /// Physics, for `applyForce`.
    pub physics: Option<&'a mut PhysicsWorld>,
    /// Audio, for `playAudio`.
    pub audio: Option<&'a mut AudioEngine>,
}

/// What one `sync` did, for the caller's log and the editor's status area.
#[derive(Debug, Default)]
pub struct SyncReport {
    /// How many init/start cycles the fixed point needed.
    pub cycles: u32,
    /// Instances created across every cycle.
    pub created: usize,
    /// Instances torn down.
    pub destroyed: usize,
    /// Attachments referencing an asset that is not loaded.
    pub missing_assets: Vec<(InstanceUuid, ScriptAssetId)>,
    /// Whether [`MAX_INIT_CYCLES`] stopped the loop rather than
    /// convergence.
    pub hit_cap: bool,
    /// Failures from any lifecycle callback in this sync.
    pub failures: Vec<InstanceFailure>,
}

/// What scripting cost this frame, for the Phase 29 overlay.
///
/// CPU only, and named as such: there is no GPU side to a script, and a
/// row that implied otherwise would be worse than no row. `break-on-error`
/// is deliberately not here — it is not claimed at MVP.
#[derive(Debug, Default, Clone, Copy)]
pub struct ScriptStats {
    /// Wall time in the fixed phases, including the command apply.
    pub fixed_ms: f32,
    /// Wall time in the variable phase.
    pub update_ms: f32,
    /// Wall time reconciling, initialising and tearing down.
    pub sync_ms: f32,
    /// Callbacks the VM was actually entered for.
    pub calls: u32,
    /// Commands applied.
    pub commands: u32,
    /// Callbacks that raised.
    pub errors: u32,
    /// Live instances.
    pub instances: usize,
    /// Bytes attributed to the VM.
    pub vm_bytes: usize,
}

impl ScriptStats {
    /// Total script CPU time this frame.
    #[must_use]
    pub fn total_ms(&self) -> f32 {
        self.fixed_ms + self.update_ms + self.sync_ms
    }
}

/// Which phase group a measurement belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Meter {
    Sync,
    Fixed,
    Update,
}

/// Scripts, wired to the engine.
pub struct ScriptHost {
    runtime: ScriptRuntime,
    registry: TypeRegistry,
    force_router: Option<ForceRouter>,
    /// Where each imported script came from, so a reload can re-read it.
    script_paths: BTreeMap<ScriptAssetId, PathBuf>,
    /// What each of those files looked like when it was last loaded, and
    /// whether a change is still in flight. See [`ScriptHost::poll_file_changes`].
    watched: BTreeMap<ScriptAssetId, WatchState>,
    /// Sound assets a script may name, and where they live on disk.
    audio_paths: BTreeMap<ScriptAssetId, String>,
    voices: BTreeMap<u64, SoundHandle>,
    next_voice: u64,
    logs: Vec<ScriptLogLine>,
    /// Rejections from the last apply, kept so the editor can show why a
    /// script's write did not land.
    rejections: Vec<String>,
    /// Whether imports consult and write the bytecode cache.
    cook_enabled: bool,
    /// This frame's cost, accumulated as phases run.
    stats: ScriptStats,
    /// Which group `run_phase` is currently attributing time to.
    meter: Meter,
}

impl Default for ScriptHost {
    fn default() -> Self {
        Self::new(Budget::default())
    }
}

impl ScriptHost {
    /// A host with the Luau backend installed.
    ///
    /// A backend that fails to start is reported as a diagnostic rather
    /// than a panic: an engine that cannot bring up a VM should still open
    /// the editor and say why.
    #[must_use]
    pub fn new(budget: Budget) -> Self {
        let mut runtime = ScriptRuntime::new(budget);
        let mut logs = Vec::new();
        match somnium_script_luau::LuauBackend::new(budget) {
            Ok(backend) => runtime.register_backend(Box::new(backend)),
            Err(error) => logs.push(ScriptLogLine {
                level: LogLevel::Error,
                instance: InstanceUuid::NONE,
                message: format!("the Luau runtime failed to start: {error}"),
            }),
        }
        Self {
            runtime,
            registry: component_registry(),
            force_router: None,
            script_paths: BTreeMap::new(),
            watched: BTreeMap::new(),
            audio_paths: BTreeMap::new(),
            voices: BTreeMap::new(),
            next_voice: 1,
            logs,
            rejections: Vec::new(),
            cook_enabled: true,
            stats: ScriptStats::default(),
            meter: Meter::Sync,
        }
    }

    /// A host with no backend at all, for tests that only exercise the
    /// scheduler.
    #[must_use]
    pub fn headless(budget: Budget) -> Self {
        Self {
            runtime: ScriptRuntime::new(budget),
            registry: component_registry(),
            force_router: None,
            script_paths: BTreeMap::new(),
            watched: BTreeMap::new(),
            audio_paths: BTreeMap::new(),
            voices: BTreeMap::new(),
            next_voice: 1,
            logs: Vec::new(),
            rejections: Vec::new(),
            cook_enabled: true,
            stats: ScriptStats::default(),
            meter: Meter::Sync,
        }
    }

    /// The component schemas scripts and the scene format both read.
    #[must_use]
    pub fn registry(&self) -> &TypeRegistry {
        &self.registry
    }

    /// The scheduler underneath, for inspection and for the editor.
    #[must_use]
    pub fn runtime(&self) -> &ScriptRuntime {
        &self.runtime
    }

    /// The scheduler underneath, mutably — asset loading, quarantine
    /// clearing and state export all go through it.
    pub fn runtime_mut(&mut self) -> &mut ScriptRuntime {
        &mut self.runtime
    }

    /// Install the game's entity-to-body mapping.
    pub fn set_force_router(&mut self, router: ForceRouter) {
        self.force_router = Some(router);
    }

    /// Let scripts name a sound asset.
    pub fn register_audio(&mut self, asset: ScriptAssetId, path: impl Into<String>) {
        self.audio_paths.insert(asset, path.into());
    }

    /// Compile and describe a `.luau` file's text.
    ///
    /// # Errors
    ///
    /// The compiler's diagnostics, positioned in the source.
    pub fn load_script(
        &mut self,
        asset: ScriptAssetId,
        display_path: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<ScriptAssetId, Diagnostics> {
        self.runtime.load_asset(ScriptSource {
            id: asset,
            language: LanguageTag::LUAU,
            display_path: display_path.into(),
            text: text.into(),
        })
    }

    /// Import a `.luau` file from disk.
    ///
    /// The id comes from the path, not the bytes — see
    /// [`ScriptAssetId::from_path`] for why. Importing the same file twice
    /// is a reload, not a second asset.
    ///
    /// # Errors
    ///
    /// A read failure or the compiler's diagnostics.
    pub fn import_script_file(&mut self, path: &Path) -> Result<ScriptAssetId, Diagnostics> {
        self.import_with_dependencies(path, 0)
    }

    /// How deep a `require` chain may be followed while importing.
    ///
    /// Cycles are rejected by the runtime on the static graph. This bounds
    /// the *import* recursion, which walks a directory tree it did not
    /// create and therefore has to answer for one.
    const MAX_IMPORT_DEPTH: u32 = 16;

    fn import_with_dependencies(
        &mut self,
        path: &Path,
        depth: u32,
    ) -> Result<ScriptAssetId, Diagnostics> {
        let display = display_path(path);
        let asset = ScriptAssetId::from_path(&display);
        let text = std::fs::read_to_string(path).map_err(|error| {
            let mut diagnostics = Diagnostics::default();
            diagnostics.push(host_diagnostic(
                asset,
                &format!("cannot read {display}: {error}"),
            ));
            diagnostics
        })?;
        self.script_paths.insert(asset, path.to_path_buf());
        self.remember_signature(asset, path);

        // Phase 16-F: offer the cook, if there is a valid one. A miss
        // costs nothing — the backend compiles from source and the cache
        // is rewritten below.
        self.offer_cached_bytecode(asset, &text);

        let first = self.load_or_reload(asset, &display, &text);
        if first.is_ok() {
            self.store_cooked(asset, &text);
            return first;
        }

        // A `require` naming a script nobody has imported yet is not the
        // author's problem to solve by hand. The link pass reports which
        // names it could not find; import those and try once more. A
        // second failure is a real one.
        let missing = self.runtime.take_unresolved();
        if missing.is_empty() || depth >= Self::MAX_IMPORT_DEPTH {
            return first;
        }
        let folder = path.parent().map(Path::to_path_buf);
        let root = std::env::current_dir().unwrap_or_default();
        for (_, name) in missing {
            let candidates = [
                folder.as_ref().map(|dir| dir.join(format!("{name}.luau"))),
                Some(root.join(format!("{name}.luau"))),
                Some(root.join("assets").join(format!("{name}.luau"))),
            ];
            if let Some(found) = candidates.into_iter().flatten().find(|p| p.is_file()) {
                let _ = self.import_with_dependencies(&found, depth + 1);
            }
        }
        let second = self.load_or_reload(asset, &display, &text);
        if second.is_ok() {
            self.store_cooked(asset, &text);
        }
        second
    }

    // ── The cook (16-F) ────────────────────────────────────────────

    /// Whether the bytecode cache is in use. Off leaves the source path,
    /// which is what development wants and what a debugger needs.
    pub fn set_bytecode_cache(&mut self, enabled: bool) {
        self.cook_enabled = enabled;
    }

    /// Hand the runtime a cached artifact, if there is one that matches
    /// both this runtime and this exact source.
    fn offer_cached_bytecode(&mut self, asset: ScriptAssetId, text: &str) {
        if !self.cook_enabled {
            return;
        }
        let Some(fingerprint) = self.runtime.runtime_fingerprint(LanguageTag::LUAU) else {
            return;
        };
        let path = crate::script_cook::cache_path(&crate::script_cook::cache_dir(), asset);
        let Some(cooked) = crate::script_cook::read_cooked(&path) else {
            return;
        };
        if cooked.is_valid_for(text, &fingerprint) {
            self.runtime
                .offer_bytecode(asset, cooked.fingerprint, cooked.bytecode);
        }
    }

    /// Record what this load compiled, so the next one can skip it.
    fn store_cooked(&mut self, asset: ScriptAssetId, text: &str) {
        if !self.cook_enabled {
            return;
        }
        let (Some(fingerprint), Some(bytecode)) = (
            self.runtime.runtime_fingerprint(LanguageTag::LUAU),
            self.runtime.bytecode_of(asset),
        ) else {
            return;
        };
        let cooked = crate::script_cook::CookedScript {
            fingerprint,
            source_hash: crate::script_cook::hash_source(text),
            bytecode,
        };
        let path = crate::script_cook::cache_path(&crate::script_cook::cache_dir(), asset);
        // A cache that cannot be written is not a failure worth telling
        // anyone about: the next load compiles from source, which is what
        // it would have done anyway.
        let _ = crate::script_cook::write_cooked(&path, &cooked);
    }

    fn load_or_reload(
        &mut self,
        asset: ScriptAssetId,
        display: &str,
        text: &str,
    ) -> Result<ScriptAssetId, Diagnostics> {
        if self.runtime.is_asset_loaded(asset) {
            self.reload_script(asset, display, text).map(|()| asset)
        } else {
            self.load_script(asset, display, text)
        }
    }

    // ── The watcher (16-E) ─────────────────────────────────────────

    /// Note a file's current shape, so a later change is detectable.
    fn remember_signature(&mut self, asset: ScriptAssetId, path: &Path) {
        let signature = file_signature(path);
        self.watched.insert(
            asset,
            WatchState {
                loaded: signature,
                pending: None,
                changed_at: None,
            },
        );
    }

    /// Which imported scripts have changed on disk **and settled**.
    ///
    /// The debounce is not decoration. An editor saving a file often
    /// writes it in more than one go, and a watcher without one
    /// recompiles a half-written file and reports a syntax error the
    /// author never made. A change is reported only once its size and
    /// mtime have stopped moving for `settle`.
    pub fn poll_file_changes(&mut self, settle: std::time::Duration) -> Vec<ScriptAssetId> {
        let now = std::time::Instant::now();
        let paths: Vec<(ScriptAssetId, PathBuf)> = self
            .script_paths
            .iter()
            .map(|(id, path)| (*id, path.clone()))
            .collect();

        let mut settled = Vec::new();
        for (asset, path) in paths {
            let current = file_signature(&path);
            let Some(state) = self.watched.get_mut(&asset) else {
                continue;
            };
            if current == state.loaded {
                state.pending = None;
                state.changed_at = None;
            } else if state.pending == Some(current) {
                if state
                    .changed_at
                    .is_some_and(|at| now.duration_since(at) >= settle)
                {
                    state.loaded = current;
                    state.pending = None;
                    state.changed_at = None;
                    settled.push(asset);
                }
            } else {
                // Still being written. Restart the clock.
                state.pending = Some(current);
                state.changed_at = Some(now);
            }
        }
        settled
    }

    /// Poll, then reload whatever settled — the editor's per-frame call.
    ///
    /// Returns `(reloaded, failed)`. A file that no longer compiles keeps
    /// its live instances running and only publishes diagnostics, which is
    /// what makes this safe to run every frame.
    pub fn reload_changed(&mut self, settle: std::time::Duration) -> (usize, usize) {
        let changed = self.poll_file_changes(settle);
        if changed.is_empty() {
            return (0, 0);
        }
        let (mut ok, mut failed) = (0, 0);
        for asset in changed {
            let Some(path) = self.script_paths.get(&asset).cloned() else {
                continue;
            };
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    if self.reload_script(asset, display_path(&path), text).is_ok() {
                        ok += 1;
                    } else {
                        failed += 1;
                    }
                }
                Err(error) => {
                    failed += 1;
                    self.logs.push(ScriptLogLine {
                        level: LogLevel::Error,
                        instance: InstanceUuid::NONE,
                        message: format!("cannot read {}: {error}", path.display()),
                    });
                }
            }
        }
        (ok, failed)
    }

    /// Recompile every imported script from its file on disk.
    ///
    /// Returns how many recompiled cleanly. A file that no longer compiles
    /// keeps its live instances running and publishes diagnostics, which
    /// is the property that makes this safe to bind to a key.
    pub fn reload_all_from_disk(&mut self) -> (usize, usize) {
        let paths: Vec<(ScriptAssetId, std::path::PathBuf)> = self
            .script_paths
            .iter()
            .map(|(id, path)| (*id, path.clone()))
            .collect();
        let (mut ok, mut failed) = (0, 0);
        for (asset, path) in paths {
            let display = display_path(&path);
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    if self.reload_script(asset, display, text).is_ok() {
                        ok += 1;
                    } else {
                        failed += 1;
                    }
                }
                Err(error) => {
                    failed += 1;
                    self.logs.push(ScriptLogLine {
                        level: LogLevel::Error,
                        instance: InstanceUuid::NONE,
                        message: format!("cannot read {display}: {error}"),
                    });
                }
            }
        }
        (ok, failed)
    }

    /// Where an imported script came from.
    #[must_use]
    pub fn script_path(&self, asset: ScriptAssetId) -> Option<&std::path::Path> {
        self.script_paths.get(&asset).map(PathBuf::as_path)
    }

    /// Recompile a script under its live instances, carrying declared
    /// state across. A compile failure leaves everything running.
    ///
    /// # Errors
    ///
    /// The new source's diagnostics, with the old module still live.
    pub fn reload_script(
        &mut self,
        asset: ScriptAssetId,
        display_path: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<(), Diagnostics> {
        self.runtime.reload_asset(ScriptSource {
            id: asset,
            language: LanguageTag::LUAU,
            display_path: display_path.into(),
            text: text.into(),
        })
    }

    // ── The frame ──────────────────────────────────────────────────

    /// Reconcile the live set with the authored one and run the lifecycle
    /// up to `Enabled`.
    ///
    /// Call once per frame, before the update phases. Cheap when nothing
    /// changed: the first reconcile creates nothing and the loop exits.
    pub fn sync(
        &mut self,
        world: &mut World,
        phase: &PhaseInput,
        services: &mut HostServices<'_>,
    ) -> SyncReport {
        let mut report = SyncReport::default();
        self.meter = Meter::Sync;

        // Phase 16-E: a reload's property migration rewrote authored data,
        // and authored data lives in the world. Written back before the
        // reconcile below, which would otherwise see the instance and the
        // scene disagree, rebuild from the scene, and undo the migration
        // on the very frame it happened.
        self.apply_migrated_properties(world);

        // Anything on a despawned entity is on its way out whether or not
        // the authored data changed.
        {
            let view = EngineWorldView::new(world, &self.registry);
            self.runtime.retire_dead_entities(&view);
        }

        let mut created = 0usize;
        let mut destroyed = 0usize;
        let mut missing = Vec::new();
        let mut failures = Vec::new();
        let (cycles, hit_cap) = run_to_fixed_point(|| {
            let views = collect_attachments(world);
            let reconciled = self.runtime.reconcile(&views);
            created += reconciled.created.len();
            destroyed += reconciled.retired.len();
            missing.extend(reconciled.missing_assets);
            for (instance, error) in reconciled.failed {
                self.logs.push(ScriptLogLine {
                    level: LogLevel::Error,
                    instance,
                    message: format!("failed to instantiate: {error}"),
                });
            }
            if !self.runtime.has_pending_init() {
                return false;
            }
            failures.extend(
                self.run_phase(Callback::Init, world, phase, services)
                    .failures,
            );
            failures.extend(
                self.run_phase(Callback::Start, world, phase, services)
                    .failures,
            );
            // Only a pass that *created* something can lead to more work:
            // a reload puts existing attachments back into `Loaded`, and
            // that settles in one pass rather than iterating.
            !reconciled.created.is_empty()
        });
        report.created = created;
        report.destroyed = destroyed;
        report.missing_assets = missing;
        report.failures = failures;
        report.cycles = cycles;
        report.hit_cap = hit_cap;
        if hit_cap {
            self.runtime.report_init_did_not_settle();
        }

        // The author's `enabled` flag, and quarantine, are both applied
        // here — one diff, one place.
        report.failures.extend(
            self.run_phase(Callback::Enable, world, phase, services)
                .failures,
        );
        report.failures.extend(
            self.run_phase(Callback::Disable, world, phase, services)
                .failures,
        );

        report.destroyed += self.teardown(world, phase, services);
        report
    }

    /// Run `onFixedUpdate` and commit what it asked for.
    ///
    /// Called from the fixed-step accumulator, **before** `physics.step`,
    /// so a force a script applies is integrated by the same step it was
    /// applied in.
    pub fn fixed_update(
        &mut self,
        world: &mut World,
        time: TimeSnapshot,
        input: &InputSnapshot,
        services: &mut HostServices<'_>,
    ) -> Vec<InstanceFailure> {
        let phase = PhaseInput {
            time,
            input: input.clone(),
        };
        self.meter = Meter::Fixed;
        let mut failures = self
            .run_phase(Callback::FixedUpdate, world, &phase, services)
            .failures;
        // Events emitted this step are delivered in the same step, after
        // every fixed callback has run, so delivery order does not depend
        // on who emitted first.
        failures.extend(
            self.run_phase(Callback::Event, world, &phase, services)
                .failures,
        );
        self.teardown(world, &phase, services);
        failures
    }

    /// Run `onUpdate` and commit what it asked for.
    pub fn update(
        &mut self,
        world: &mut World,
        time: TimeSnapshot,
        input: &InputSnapshot,
        services: &mut HostServices<'_>,
    ) -> Vec<InstanceFailure> {
        let phase = PhaseInput {
            time,
            input: input.clone(),
        };
        self.meter = Meter::Update;
        let failures = self
            .run_phase(Callback::Update, world, &phase, services)
            .failures;
        self.teardown(world, &phase, services);
        failures
    }

    /// Write a migration's rewritten property bags into the `ScriptSet`s
    /// they came from.
    ///
    /// This is what makes a rename survive a save as well as a reload: the
    /// scene is the authority on authored data, so a migration that only
    /// reached the live instance would be lost the moment anything
    /// re-read the world.
    fn apply_migrated_properties(&mut self, world: &mut World) {
        let migrated = self.runtime.take_migrated_properties();
        if migrated.is_empty() {
            return;
        }
        let scripted: Vec<Entity> = world
            .entities()
            .filter(|entity| world.get::<ScriptSet>(*entity).is_some())
            .collect();
        for (instance, properties) in migrated {
            for entity in &scripted {
                let Some(mut set) = world.get::<ScriptSet>(*entity).cloned() else {
                    continue;
                };
                let Some(attachment) = set.get_mut(instance) else {
                    continue;
                };
                attachment.properties = properties.clone();
                let _ = world.insert_component(*entity, set);
                break;
            }
        }
    }

    /// Tear every live instance down, as Stop does.
    ///
    /// Reconciling against an empty authored set is the whole mechanism:
    /// every attachment looks vanished, gets `onDisable` and `onDestroy`,
    /// and gives back what it owned. The authored data in the world is
    /// untouched, so the next Play rebuilds all of it.
    pub fn shutdown(&mut self, world: &mut World, services: &mut HostServices<'_>) -> usize {
        let phase = PhaseInput::default();
        self.runtime.reconcile(&[]);
        self.teardown(world, &phase, services)
    }

    /// Send an event to every enabled attachment that listens.
    pub fn emit(&mut self, name: impl Into<String>, source: Option<Entity>) {
        self.runtime
            .queue_event(name.into(), source, somnium_ecs::ReflectObject::new());
    }

    /// Destroy everything the world no longer has authored data for, and
    /// release what those attachments owned.
    fn teardown(
        &mut self,
        world: &mut World,
        phase: &PhaseInput,
        services: &mut HostServices<'_>,
    ) -> usize {
        if !self.runtime.has_pending_destroy() {
            return 0;
        }
        // `onDisable` first, then `onDestroy` — the same order a hot reload
        // uses, so a script that gives things back in `onDisable` does not
        // need a second copy of that code in its destructor. Both run while
        // the instance is still whole; the VM object goes after, at the
        // safe point.
        self.run_phase(Callback::Disable, world, phase, services);
        self.run_phase(Callback::Destroy, world, phase, services);
        let released = self.runtime.collect();
        let count = released.len();
        for (instance, owned) in released {
            for resource in owned {
                self.release(instance, &resource);
            }
        }
        count
    }

    /// Give one owned engine resource back.
    fn release(&mut self, instance: InstanceUuid, resource: &OwnedResource) {
        match resource {
            OwnedResource::Audio(handle) => {
                if let Some(mut voice) = self.voices.remove(handle) {
                    voice.stop();
                }
            }
            OwnedResource::Entity(_) | OwnedResource::Task(_) => {}
            OwnedResource::Subscription(name) => {
                self.logs.push(ScriptLogLine {
                    level: LogLevel::Debug,
                    instance,
                    message: format!("dropped subscription to `{name}`"),
                });
            }
        }
    }

    /// One phase, end to end: run, apply, route.
    fn run_phase(
        &mut self,
        callback: Callback,
        world: &mut World,
        phase: &PhaseInput,
        services: &mut HostServices<'_>,
    ) -> somnium_script::runtime::PhaseReport {
        let started = std::time::Instant::now();
        let mut commands = CommandBuffer::new();
        let report = {
            // The view borrows the world immutably for exactly the
            // duration of the callbacks; nothing it hands out survives.
            let view = EngineWorldView::new(world, &self.registry);
            self.runtime
                .run_phase(callback, phase, &view, &mut commands)
        };

        for failure in &report.failures {
            self.logs.push(ScriptLogLine {
                level: LogLevel::Error,
                instance: failure.instance,
                message: format!(
                    "{} raised: {}",
                    failure.callback.script_name(),
                    failure.error
                ),
            });
        }

        // Phase 16-F: the capability manifest, enforced once, here.
        //
        // At the command boundary rather than in the bindings: every
        // effect a script can have on the world is a command, so this is
        // exhaustive, and a new command variant cannot be added without
        // declaring what it needs.
        let mut queued = commands.drain_sorted();
        queued.retain(|command| {
            let granted = self.runtime.capabilities_for(command.order);
            let needed = command.command.required_capability();
            if granted.allows(needed) {
                return true;
            }
            self.rejections.push(format!(
                "`{}` needs the `{}` capability, which this script package \
                 does not have",
                command.command.name(),
                needed.name()
            ));
            false
        });

        let outcome = apply_commands(world, &self.registry, queued);
        self.stats.commands += u32::try_from(outcome.applied).unwrap_or(u32::MAX);
        self.absorb(outcome, world, services);

        // Measured around the whole phase — the VM call *and* the apply —
        // because "how much did scripting cost this frame" is the
        // question, and the marshalling either side of the call is most of
        // the answer (see the 16-B budget record).
        #[allow(clippy::cast_possible_truncation)]
        let elapsed = started.elapsed().as_secs_f32() * 1000.0;
        match self.meter {
            Meter::Sync => self.stats.sync_ms += elapsed,
            Meter::Fixed => self.stats.fixed_ms += elapsed,
            Meter::Update => self.stats.update_ms += elapsed,
        }
        self.stats.calls += u32::try_from(report.invoked).unwrap_or(u32::MAX);
        self.stats.errors += u32::try_from(report.failures.len()).unwrap_or(u32::MAX);
        report
    }

    /// This frame's cost so far.
    #[must_use]
    pub fn stats(&self) -> ScriptStats {
        ScriptStats {
            instances: self.runtime.live_instances(),
            vm_bytes: self.runtime.memory_used(),
            ..self.stats
        }
    }

    /// Zero the per-frame counters. Called once per frame by the engine,
    /// after the overlay has read them.
    pub fn begin_frame(&mut self) {
        self.stats = ScriptStats::default();
    }

    /// Route one apply pass's side effects into the engine.
    fn absorb(
        &mut self,
        outcome: ApplyOutcome,
        world: &mut World,
        services: &mut HostServices<'_>,
    ) {
        self.runtime.record_spawns(&outcome.spawned);

        for (order, level, message) in outcome.logs {
            self.logs.push(ScriptLogLine {
                level,
                instance: order.instance,
                message,
            });
        }

        for rejection in outcome.rejected {
            self.rejections
                .push(format!("{:?}: {}", rejection.reason, rejection.detail));
        }

        for event in outcome.events {
            self.runtime
                .queue_event(event.name, Some(event.source), event.payload);
        }

        if let Some(physics) = services.physics.as_deref_mut() {
            if let Some(router) = self.force_router.as_mut() {
                for (entity, force, mode) in outcome.forces {
                    router(world, physics, entity, glam::Vec3::from(force), mode);
                }
            } else if !outcome.forces.is_empty() {
                self.rejections.push(
                    "applyForce: no force router is installed, so the engine has no way to \
                     find this entity's rigid body (see ScriptHost::set_force_router)"
                        .to_string(),
                );
            }
        }

        if let Some(audio) = services.audio.as_deref_mut() {
            for (order, asset, volume) in outcome.audio {
                let Some(path) = self.audio_paths.get(&asset) else {
                    self.rejections
                        .push(format!("playAudio: no sound is registered as {asset}"));
                    continue;
                };
                let settings = SoundSettings {
                    volume: f64::from(volume),
                    looping: false,
                };
                match audio.play(path, settings) {
                    Ok(handle) => {
                        let id = self.next_voice;
                        self.next_voice += 1;
                        self.voices.insert(id, handle);
                        // Tagged with the attachment that asked, so
                        // teardown stops it rather than leaving a destroyed
                        // script's sound playing forever.
                        self.runtime
                            .acquire(order.instance, OwnedResource::Audio(id));
                    }
                    Err(error) => self.rejections.push(format!("playAudio {path}: {error}")),
                }
            }
        }

        // Despawned entities take their attachments with them.
        if !outcome.despawned.is_empty() {
            let view = EngineWorldView::new(world, &self.registry);
            self.runtime.retire_dead_entities(&view);
        }
    }

    // ── Output ─────────────────────────────────────────────────────

    /// Take everything scripts wrote to the log this frame.
    pub fn take_logs(&mut self) -> Vec<ScriptLogLine> {
        std::mem::take(&mut self.logs)
    }

    /// Take the compiler and runtime diagnostics.
    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        self.runtime.take_diagnostics()
    }

    /// Take the commands the applier refused, with the reason.
    pub fn take_rejections(&mut self) -> Vec<String> {
        std::mem::take(&mut self.rejections)
    }

    /// Where one attachment is in its lifecycle, for the editor.
    #[must_use]
    pub fn state_of(&self, instance: InstanceUuid) -> Option<LifecycleState> {
        self.runtime.state_of(instance)
    }

    /// Export every live attachment's declared state, keyed by its durable
    /// id — the save-game half of the reload contract.
    pub fn export_states(&mut self) -> BTreeMap<InstanceUuid, ScriptValue> {
        let ids: Vec<InstanceUuid> = self.live_instance_ids();
        let mut out = BTreeMap::new();
        for id in ids {
            if let Ok(state) = self.runtime.export_state(id) {
                if state != ScriptValue::Nil {
                    out.insert(id, state);
                }
            }
        }
        out
    }

    /// Give exported state back to the attachments it came from.
    pub fn import_states(&mut self, states: &BTreeMap<InstanceUuid, ScriptValue>) {
        for (id, state) in states {
            if let Err(error) = self.runtime.import_state(*id, state.clone()) {
                self.logs.push(ScriptLogLine {
                    level: LogLevel::Warn,
                    instance: *id,
                    message: format!("loadState refused the saved value: {error}"),
                });
            }
        }
    }

    /// Every live attachment, in durable-id order.
    #[must_use]
    pub fn live_instance_ids(&self) -> Vec<InstanceUuid> {
        self.runtime.instances().collect()
    }
}

/// Iterate `cycle` until it reports that it created nothing, or until
/// [`MAX_INIT_CYCLES`] passes have run.
///
/// Extracted so the cap is provable without a VM: the loop's contract is
/// "converge, or stop and say so", and that is a property of the loop
/// rather than of any script. Returns the number of *creating* passes and
/// whether the cap stopped it.
pub fn run_to_fixed_point(mut cycle: impl FnMut() -> bool) -> (u32, bool) {
    let mut passes = 0;
    while cycle() {
        passes += 1;
        if passes >= MAX_INIT_CYCLES {
            return (passes, true);
        }
    }
    (passes, false)
}

/// Read every authored attachment out of the world, in world order.
///
/// Ordering does not matter here — [`OrderKey`] decides run order — but
/// durable ids do: an attachment with no [`PersistentId`] on its entity
/// has no stable tiebreak, so one is minted the first time it is seen.
#[must_use]
pub fn collect_attachments(world: &mut World) -> Vec<AttachmentView> {
    let scripted: Vec<Entity> = world
        .entities()
        .filter(|entity| world.get::<ScriptSet>(*entity).is_some())
        .collect();

    let mut views = Vec::new();
    for entity in scripted {
        // Minting migrates the entity to another archetype, which is why
        // the entity list above is materialised first.
        let Ok(persistent) = world.ensure_persistent_id(entity) else {
            continue;
        };
        let Some(set) = world.get::<ScriptSet>(entity) else {
            continue;
        };
        for attachment in &set.attachments {
            views.push(AttachmentView {
                entity,
                persistent,
                instance: attachment.instance,
                asset: attachment.asset,
                enabled: attachment.enabled,
                execution_order: attachment.execution_order,
                properties: attachment.properties.clone(),
            });
        }
    }
    views
}

/// What a file looks like, without reading it.
///
/// Modification time and size — enough to notice an edit, cheap enough to
/// check every frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileSignature {
    /// Last modified, and how big.
    Present(std::time::SystemTime, u64),
    /// Not there. A signature like any other: deleting a script is a
    /// change, and the reload that follows reports the read failure.
    Missing,
}

/// What a watched file looked like, and whether a change is still in
/// flight.
#[derive(Debug, Clone)]
struct WatchState {
    /// Signature of the text currently compiled.
    loaded: FileSignature,
    /// A different signature seen but not yet settled.
    pending: Option<FileSignature>,
    /// When `pending` was first seen.
    changed_at: Option<std::time::Instant>,
}

fn file_signature(path: &Path) -> FileSignature {
    let Ok(meta) = std::fs::metadata(path) else {
        return FileSignature::Missing;
    };
    meta.modified().map_or(FileSignature::Missing, |at| {
        FileSignature::Present(at, meta.len())
    })
}

/// A path as the editor and the diagnostics show it: project-relative
/// where possible, forward slashes, so the same file reads the same on
/// every platform and produces the same [`ScriptAssetId`].
#[must_use]
pub fn display_path(path: &Path) -> String {
    let relative = std::env::current_dir()
        .ok()
        .and_then(|cwd| path.strip_prefix(&cwd).ok().map(Path::to_path_buf))
        .unwrap_or_else(|| path.to_path_buf());
    relative.to_string_lossy().replace('\\', "/")
}

/// The source a new script starts from.
///
/// Strict mode, one declared property and an empty `onFixedUpdate` — the
/// smallest thing that is already correct, rather than an empty file the
/// author has to look up the shape of.
pub const NEW_SCRIPT_TEMPLATE: &str = r#"--!strict

return Script.define({
	apiVersion = 1,
	schemaVersion = 1,

	-- Components this script touches on its own entity. Naming the fields
	-- rather than the whole component is what keeps the per-frame cost to
	-- the values you actually use.
	uses = { ["somnium.Transform"] = { "translation" } },

	-- Properties the Details panel will draw for you.
	fields = {
		speed = Field.number(1.0, { min = 0.0, max = 100.0 }),
	},

	onFixedUpdate = function(self, ctx, dt)
		-- `ctx.self.transform` is a plain table: read it, change it, and
		-- the engine turns the difference into a command at the end of the
		-- phase.
	end,
})
"#;

/// Build a diagnostic the editor can show for a host-level problem.
#[must_use]
pub fn host_diagnostic(asset: ScriptAssetId, message: &str) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        asset,
        display_path: "<engine>".into(),
        line: 0,
        column: 0,
        message: message.to_owned(),
    }
}
