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
//! can answer "what is attached now", so the init loop is here â€” while
//! the cap it obeys, [`MAX_INIT_CYCLES`], and the diagnostic it raises
//! belong to the runtime, so the number is stated once.
//!
//! # Physics and audio are routed, not called
//!
//! `ApplyForce` names an entity; Jolt wants a body id, and the map from
//! one to the other is game-layer knowledge â€” `somnium_core` has no
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

/// Scripts, wired to the engine.
pub struct ScriptHost {
    runtime: ScriptRuntime,
    registry: TypeRegistry,
    force_router: Option<ForceRouter>,
    /// Where each imported script came from, so a reload can re-read it.
    script_paths: BTreeMap<ScriptAssetId, PathBuf>,
    /// Sound assets a script may name, and where they live on disk.
    audio_paths: BTreeMap<ScriptAssetId, String>,
    voices: BTreeMap<u64, SoundHandle>,
    next_voice: u64,
    logs: Vec<ScriptLogLine>,
    /// Rejections from the last apply, kept so the editor can show why a
    /// script's write did not land.
    rejections: Vec<String>,
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
            audio_paths: BTreeMap::new(),
            voices: BTreeMap::new(),
            next_voice: 1,
            logs,
            rejections: Vec::new(),
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
            audio_paths: BTreeMap::new(),
            voices: BTreeMap::new(),
            next_voice: 1,
            logs: Vec::new(),
            rejections: Vec::new(),
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

    /// The scheduler underneath, mutably â€” asset loading, quarantine
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
    /// The id comes from the path, not the bytes â€” see
    /// [`ScriptAssetId::from_path`] for why. Importing the same file twice
    /// is a reload, not a second asset.
    ///
    /// # Errors
    ///
    /// A read failure or the compiler's diagnostics.
    pub fn import_script_file(
        &mut self,
        path: &std::path::Path,
    ) -> Result<ScriptAssetId, Diagnostics> {
        let display = display_path(path);
        let asset = ScriptAssetId::from_path(&display);
        let text = std::fs::read_to_string(path).map_err(|error| {
            let mut diagnostics = Diagnostics::default();
            diagnostics.push(host_diagnostic(
                asset,
                &format!("cannot read {display}: {error}"),
            ));
            self.runtime_mut()
                .take_diagnostics()
                .into_iter()
                .for_each(drop);
            diagnostics
        })?;
        self.script_paths.insert(asset, path.to_path_buf());
        if self.runtime.is_asset_loaded(asset) {
            self.reload_script(asset, display, text).map(|()| asset)
        } else {
            self.load_script(asset, display, text)
        }
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

    // â”€â”€ The frame â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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
            failures.extend(self.run_phase(Callback::Init, world, phase, services).failures);
            failures.extend(self.run_phase(Callback::Start, world, phase, services).failures);
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
        // here â€” one diff, one place.
        report
            .failures
            .extend(self.run_phase(Callback::Enable, world, phase, services).failures);
        report
            .failures
            .extend(self.run_phase(Callback::Disable, world, phase, services).failures);

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
        let failures = self
            .run_phase(Callback::Update, world, &phase, services)
            .failures;
        self.teardown(world, &phase, services);
        failures
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
        // `onDisable` first, then `onDestroy` â€” the same order a hot reload
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
        let mut commands = CommandBuffer::new();
        let report = {
            // The view borrows the world immutably for exactly the
            // duration of the callbacks; nothing it hands out survives.
            let view = EngineWorldView::new(world, &self.registry);
            self.runtime.run_phase(callback, phase, &view, &mut commands)
        };

        for failure in &report.failures {
            self.logs.push(ScriptLogLine {
                level: LogLevel::Error,
                instance: failure.instance,
                message: format!("{} raised: {}", failure.callback.script_name(), failure.error),
            });
        }

        let outcome = apply_commands(world, &self.registry, commands.drain_sorted());
        self.absorb(outcome, world, services);
        report
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
            self.rejections.push(format!(
                "{:?}: {}",
                rejection.reason, rejection.detail
            ));
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

    // â”€â”€ Output â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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
    /// id â€” the save-game half of the reload contract.
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
/// Ordering does not matter here â€” [`OrderKey`] decides run order â€” but
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

