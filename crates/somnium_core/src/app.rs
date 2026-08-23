use std::sync::Arc;

use tracing::{debug, error, info, warn};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
#[cfg(target_os = "windows")]
use winit::platform::windows::WindowAttributesExtWindows;
use winit::window::{Window, WindowAttributes, WindowId};

use somnium_audio::engine::AudioEngine;
use somnium_physics::body::{BodyId, MotionType, RigidBodyDescriptor};
use somnium_physics::shape::ColliderShape;
use somnium_physics::{config::PhysicsConfig, world::PhysicsWorld};
use somnium_renderer::{GizmoAxis, GizmoMode, RenderContext, SomniumRenderer, gizmo_hit_test};
use somnium_ui::{ColorField, EditorEvent, LightInspectorState, TerrainInspectorState, UiManager};

use crate::config::EngineConfig;
use crate::context::{EngineContext, SimulationClock, SimulationState};
use crate::editor_commands::{
    CreateEntityCmd, CreateLandscapeCmd, DeleteEntityCmd, EntitySnapshot, SetLightCmd,
    SetTransformCmd, TerrainEditCmd, TerrainRestoreOp, TerrainRestoreQueue, UndoStack,
};
use crate::error::EngineError;
use crate::event::{EngineEvent, translate_window_event};
use crate::time::TimeState;
use crate::{
    BuoyantVessel, CameraSettingsComponent, FoliageComponent, LightComponent, LightType,
    MaterialComponent, MeshComponent, MeshKind, Name, Parent, ParticleEmitter,
    PostProcessComponent, TerrainComponent, Transform, WaterComponent, WorldTransform,
    look_rotation_neg_z, simulate_particles,
};
use somnium_ecs::World;
use somnium_renderer::terrain::brush::{BrushMode, TerrainBrush, apply_paint, apply_sculpt};

/// Maintain the scene-wide post-process component as an actual singleton.
///
/// Legacy/imported scenes can contain duplicates and New Scene used to contain
/// none. Prefer the selected component so an inspector edit is never discarded;
/// otherwise keep the oldest ECS entity and remove the extras.
fn normalize_post_process_singleton(
    world: &mut World,
    selected_entity: &mut Option<somnium_ecs::Entity>,
) {
    let entities: Vec<_> = world
        .entities()
        .filter(|entity| world.get::<PostProcessComponent>(*entity).is_some())
        .collect();
    if entities.is_empty() {
        world.spawn((
            Transform::from_translation(glam::Vec3::ZERO),
            Name::new("Post Processing"),
            WorldTransform::identity(),
            PostProcessComponent::default(),
        ));
        return;
    }
    let keeper = selected_entity
        .filter(|selected| entities.contains(selected))
        .unwrap_or(entities[0]);
    for entity in entities {
        if entity != keeper {
            world.despawn(entity);
        }
    }
}

/// An offline path trace can only converge while the scene is stationary.
/// Preserve the user's transport state, hold fixed-step simulation for as long
/// as the path tracer is active, then restore that state on exit.
fn synchronize_path_trace_pause(
    active: bool,
    clock: &mut SimulationClock,
    accumulator: &mut f32,
    previous_state: &mut Option<SimulationState>,
) {
    if active {
        if previous_state.is_none() {
            *previous_state = Some(clock.state);
        }
        clock.state = SimulationState::Paused;
        *accumulator = 0.0;
    } else if let Some(previous) = previous_state.take() {
        clock.state = previous;
        *accumulator = 0.0;
    }
}

#[cfg(test)]
mod content_target_tests {
    use super::resolve_content_target;
    use std::path::Path;

    fn root() -> &'static Path {
        Path::new("/project/assets")
    }

    #[test]
    fn a_name_lands_in_the_folder_that_was_right_clicked() {
        let path = resolve_content_target(root(), "scripts", "Enemy", Some("luau")).unwrap();
        assert!(path.ends_with("Enemy.luau"), "{path:?}");
        assert!(path.to_string_lossy().contains("scripts"), "{path:?}");

        let at_root = resolve_content_target(root(), "", "Shared", None).unwrap();
        assert!(at_root.ends_with("Shared"), "{at_root:?}");
    }

    #[test]
    fn the_script_extension_is_added_but_never_doubled() {
        for typed in ["Enemy", "Enemy.luau", "Enemy.LUAU"] {
            let path = resolve_content_target(root(), "", typed, Some("luau")).unwrap();
            assert!(
                path.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("luau")),
                "{typed} produced {path:?}"
            );
            assert!(
                !path.to_string_lossy().to_ascii_lowercase().contains(".luau.luau"),
                "{typed} produced {path:?}"
            );
        }
    }

    #[test]
    fn a_name_cannot_escape_the_content_root() {
        // The whole reason this function exists: a modal takes a free
        // string, and the drawer has no undo.
        for attempt in ["../secrets", "..\\secrets", "..", ".", "a/b", "a\\b"] {
            assert!(
                resolve_content_target(root(), "", attempt, None).is_err(),
                "`{attempt}` should have been refused"
            );
        }
    }

    #[test]
    fn an_empty_or_blank_name_is_refused() {
        assert!(resolve_content_target(root(), "", "", None).is_err());
        assert!(resolve_content_target(root(), "", "   ", None).is_err());
    }

    #[test]
    fn surrounding_whitespace_is_trimmed_rather_than_kept() {
        let path = resolve_content_target(root(), "", "  Enemy  ", Some("luau")).unwrap();
        assert!(path.ends_with("Enemy.luau"), "{path:?}");
    }

    #[test]
    fn a_windows_reserved_name_is_refused_whatever_the_extension() {
        // These are created happily and then cannot be opened again.
        for reserved in ["con", "NUL", "Aux.luau", "com1"] {
            assert!(
                resolve_content_target(root(), "", reserved, Some("luau")).is_err(),
                "`{reserved}` should have been refused"
            );
        }
        assert!(
            resolve_content_target(root(), "", "console", Some("luau")).is_ok(),
            "a name that merely starts with a reserved word is fine"
        );
    }
}

#[cfg(test)]
mod post_process_singleton_tests {
    use super::*;

    fn count(world: &World) -> usize {
        world
            .entities()
            .filter(|entity| world.get::<PostProcessComponent>(*entity).is_some())
            .count()
    }

    #[test]
    fn missing_post_process_is_created() {
        let mut world = World::new();
        normalize_post_process_singleton(&mut world, &mut None);
        assert_eq!(count(&world), 1);
    }

    #[test]
    fn selected_duplicate_is_the_one_preserved() {
        let mut world = World::new();
        let first = world.spawn((PostProcessComponent::default(),));
        let mut selected_settings = PostProcessComponent::default();
        selected_settings.bloom_intensity = 0.73;
        let selected = world.spawn((selected_settings,));
        let mut selection = Some(selected);
        normalize_post_process_singleton(&mut world, &mut selection);
        assert_eq!(count(&world), 1);
        assert!(!world.is_alive(first));
        assert_eq!(
            world
                .get::<PostProcessComponent>(selected)
                .unwrap()
                .bloom_intensity,
            0.73
        );
    }

    #[test]
    fn path_trace_pause_preserves_and_restores_transport_state() {
        let mut clock = SimulationClock {
            state: SimulationState::Playing,
            ..SimulationClock::default()
        };
        let mut accumulator = 0.5;
        let mut previous = None;

        synchronize_path_trace_pause(true, &mut clock, &mut accumulator, &mut previous);
        assert_eq!(clock.state, SimulationState::Paused);
        assert_eq!(previous, Some(SimulationState::Playing));
        assert_eq!(accumulator, 0.0);

        synchronize_path_trace_pause(false, &mut clock, &mut accumulator, &mut previous);
        assert_eq!(clock.state, SimulationState::Playing);
        assert_eq!(previous, None);

        clock.state = SimulationState::Paused;
        synchronize_path_trace_pause(true, &mut clock, &mut accumulator, &mut previous);
        assert_eq!(previous, Some(SimulationState::Paused));
        synchronize_path_trace_pause(false, &mut clock, &mut accumulator, &mut previous);
        assert_eq!(clock.state, SimulationState::Paused);
    }
}

/// State captured when the user begins dragging a gizmo axis handle.
#[derive(Clone)]
struct GizmoDragState {
    axis: GizmoAxis,
    mode: GizmoMode,
    entity_index: u32,
    start_transform: Transform,
    /// Scalar along the drag axis from gizmo origin at drag start (translate/scale).
    start_axis_param: f32,
    /// Angle in the ring plane at drag start, in radians (rotate).
    start_angle: f32,
    /// Ring-plane tangent vector (rotate).
    ring_tangent: glam::Vec3,
    /// Ring-plane bitangent vector (rotate).
    ring_bitangent: glam::Vec3,
    /// Gizmo world position at drag start.
    gizmo_pos: glam::Vec3,
}

/// State captured while a terrain brush stroke is in progress (Phase 14D).
///
/// On stroke start, the full heightmap (or splatmap) is snapshotted; the
/// affected region accumulates as the stroke moves. On release, the old/new
/// data of just that region is pushed as a [`TerrainEditCmd`].
struct TerrainStroke {
    terrain_id: u32,
    is_paint: bool,
    start_heights: Vec<f32>,
    start_texels: Vec<somnium_renderer::terrain::textures::SplatTexel>,
    /// Union of all touched (vertex or texel) regions, inclusive.
    region: Option<(u32, u32, u32, u32)>,
}

/// Trait to be implemented by the user's game.
pub trait GameApp {
    /// Called once when the engine starts.
    fn on_init(&mut self, _ctx: &mut EngineContext) {}

    /// Called for every window event.
    fn on_event(&mut self, _ctx: &mut EngineContext, _event: &EngineEvent) {}

    /// Called every frame for game logic.
    fn on_update(&mut self, _ctx: &mut EngineContext) {}

    /// Called once per deterministic fixed simulation step, immediately before
    /// Jolt advances. Buoyancy and other forces belong here rather than in the
    /// variable-rate editor update.
    fn on_fixed_update(&mut self, _ctx: &mut EngineContext) {}

    /// Called every frame for UI and debug rendering.
    fn on_render(&mut self, _ctx: &mut EngineContext) {}

    /// Called just before the engine shuts down.
    fn on_shutdown(&mut self) {}

    /// Called after a version-2 map factory finishes (drawer double-click or tests).
    fn on_map_loaded(&mut self, _ctx: &mut EngineContext, _result: &crate::MapLoadResult) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecycleState {
    Uninitialized,
    Running,
    Suspended,
    ShuttingDown,
}

/// The foliage palette: what the brush can paint (Phase 17F).
///
/// Fixed for now. Once there is a content drawer this becomes whatever the
/// project has imported, which is why the brush stores a palette *index* rather
/// than anything about the mesh itself.
///
/// All four are CC0 from Poly Haven — see ATTRIBUTION.md.
pub const FOLIAGE_PALETTE: [(&str, &str); 4] = [
    (
        "Grass Medium",
        "assets/foliage/grass_medium_01/grass_medium_01_2k.gltf",
    ),
    (
        "Grass Bermuda",
        "assets/foliage/grass_bermuda_01/grass_bermuda_01_2k.gltf",
    ),
    (
        "Fir Sapling",
        "assets/foliage/fir_sapling/fir_sapling_2k.gltf",
    ),
    (
        "Island Tree",
        "assets/foliage/island_tree_02/island_tree_02_2k.gltf",
    ),
];

/// One drawable piece of a palette entry.
///
/// A single foliage model is rarely one primitive: a sapling is a trunk plus a
/// separate twig mesh, a tree is trunk, branches and leaves. `local` is the
/// piece's transform within the model, composed under the instance transform.
#[derive(Clone, Copy)]
struct FoliagePart {
    vertex_offset: u32,
    index_offset: u32,
    index_count: u32,
    material_id: u32,
    local: glam::Mat4,
    /// Cutout / foliage material — the part LOD should drop first (leaves,
    /// twigs). Bark stays until the impostor band, then we keep only this
    /// being false. Index-count is the fallback when the glTF did not mark
    /// any part this way.
    is_leaf: bool,
}

/// The central engine controller that manages the lifecycle and orchestration of all subsystems.
pub struct Engine<G: GameApp> {
    game: Box<G>,
    config: EngineConfig,
    time: TimeState,
    world: World,
    /// Phase 16-A: the engine's one reflected description of its
    /// components. Built once at startup and shared by scene
    /// serialization, the script boundary, and (when it exists) the
    /// reflection inspector — the whole point being that there is exactly
    /// one of these.
    type_registry: somnium_ecs::reflect::TypeRegistry,
    physics: Option<PhysicsWorld>,
    audio: Option<AudioEngine>,
    window: Option<Arc<Window>>,
    render_ctx: Option<RenderContext>,
    renderer: Option<SomniumRenderer>,
    ui_manager: Option<UiManager>,
    selected_entity: Option<somnium_ecs::entity::Entity>,
    state: LifecycleState,
    /// Bounded command history for editor undo/redo (128-command capacity).
    undo_stack: UndoStack,
    /// Current cursor position in physical pixels.
    cursor_pos: (f32, f32),
    /// Current window dimensions in physical pixels (updated on resize).
    viewport_size: (f32, f32),
    /// Active gizmo drag state (Some while LMB is held on a gizmo axis).
    gizmo_drag: Option<GizmoDragState>,

    /// State at the start of an inspector drag-scrub, so the whole gesture
    /// collapses into one undo entry instead of one per pixel of travel.
    /// `(entity index, value before the drag)`.
    /// Uploaded geometry per palette entry, filled in the first time each one
    /// is painted — loading four scanned models up front would add seconds to
    /// startup for meshes the user may never place.
    foliage_meshes: [Option<Vec<FoliagePart>>; FOLIAGE_PALETTE.len()],
    /// Palette entries whose import failed, so we stop retrying them.
    foliage_failed: [bool; FOLIAGE_PALETTE.len()],
    /// Phase 17F: the foliage brush.
    foliage_brush: somnium_renderer::terrain::foliage_paint::FoliageBrush,
    /// When true, dragging in the viewport paints foliage instead of sculpting.
    pub foliage_paint_active: bool,
    /// Erase instead of add.
    pub foliage_erase: bool,
    /// Scratch list for this frame's visible foliage, reused so a field of
    /// instances does not allocate a fresh vector every frame.
    foliage_batch: Vec<(FoliagePart, glam::Mat4, bool)>,
    /// Advances per dab so a held brush keeps generating fresh candidates
    /// rather than retrying the same rejected points.
    foliage_stroke_seed: u32,
    /// True while the left button is held during a foliage stroke.
    foliage_painting: bool,
    /// Phase 17B: static heightfield body per terrain, with the terrain
    /// revision it was built from so it is only rebuilt after a real edit.
    terrain_colliders: std::collections::HashMap<u32, (u64, BodyId)>,

    scrub_transform: Option<(u32, Transform)>,
    scrub_light: Option<(u32, LightComponent)>,
    /// Phase 11.5M: receiver for captured tracing events forwarded to the output log.
    log_rx: Option<std::sync::mpsc::Receiver<crate::log_capture::LogEntry>>,
    /// Exact modifier snapshot for registry-backed global shortcuts.
    shortcut_modifiers: somnium_ui::message::Modifiers,
    /// Cached default material ID for editor-created mesh entities.
    default_material_id: Option<u32>,
    /// Phase 14F: terrain edit mode (F6 or terrain tool button activates).
    terrain_edit_active: bool,
    /// Phase 14D: current terrain brush settings.
    terrain_brush: TerrainBrush,
    /// Inspector debug view (same codes as `SOMNIUM_SHADOW_DEBUG`; 0 = env).
    terrain_debug_view: f32,
    /// Active brush stroke (Some while LMB is held in terrain edit mode).
    terrain_stroke: Option<TerrainStroke>,
    /// Restore ops produced by `TerrainEditCmd` undo/redo, applied before render.
    terrain_restore_queue: TerrainRestoreQueue,
    /// Phase 20B: editor camera speed as a normalized 0..1 slider position.
    /// Game code reads the mapped speed via `EngineContext::camera_speed`.
    camera_speed_norm: f32,
    /// Viewport toolbar 3D resolution preset. 0 = Native (window pixels).
    viewport_resolution: usize,
    /// UE-style editor transport state and deterministic gameplay time.
    simulation_clock: SimulationClock,
    /// Transport state temporarily held while the offline path tracer
    /// accumulates a stationary scene.
    path_trace_previous_simulation_state: Option<SimulationState>,
    /// True from Play until Stop, including while a play session is paused.
    /// Editor-only overlays and authoring tools stay disabled for the session.
    play_session_active: bool,
    /// Carries fractional wall-clock time between 60 Hz physics steps.
    simulation_accumulator: f32,
    /// True after a mutating editor action until Save or New.
    scene_dirty: bool,
    /// Title-bar close requested a shutdown.
    ui_wants_exit: bool,
    /// Map load completed this frame; game code seeds fly-cam / boat after ECS reset.
    pending_map_load: Option<crate::MapLoadResult>,
    // ── Phase 16-C: scripting ────────────────────────────────────────────
    /// Live scripts: the lifecycle, the scheduler and the command applier.
    ///
    /// Public so game code can load assets and install the entity-to-body
    /// mapping `applyForce` needs; the engine drives the phases.
    pub scripts: crate::script_host::ScriptHost,
    /// Held keys and buttons as scripts see them. Separate from the
    /// editor's own input handling because a script's view is a *sampled*
    /// snapshot per fixed step, not an event stream.
    script_input: crate::script_input::ScriptInputTracker,
    /// The authored world as it was when Play was pressed. Stop restores
    /// it, which is what stops a script dirtying the edit-time scene.
    play_checkpoint: Option<crate::script_input::WorldCheckpoint>,
    /// Fixed steps elapsed in this play session. Part of the deterministic
    /// clock a script sees, and reset by Stop.
    script_step: u64,
}

impl<G: GameApp + 'static> Engine<G> {
    /// Start the engine loop. This will take control of the current thread.
    pub fn run(config: EngineConfig, game: G) -> Result<(), EngineError> {
        // Phase 11.5M: install both the fmt layer and the log-capture layer.
        let (capture_layer, log_rx) = crate::log_capture::make_log_capture();
        {
            use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
            tracing_subscriber::registry()
                .with(tracing_subscriber::fmt::layer())
                .with(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
                )
                .with(capture_layer)
                .try_init()
                .ok();
        }

        info!(
            title = %config.window_title,
            size = ?config.window_size,
            target_fps = ?config.target_fps,
            vsync = config.vsync,
            "Somnium Engine starting"
        );

        let event_loop = EventLoop::new().map_err(|e| EngineError::EventLoop(e.to_string()))?;

        let initial_vp = (config.window_size.0 as f32, config.window_size.1 as f32);
        let mut engine = Self {
            game: Box::new(game),
            time: TimeState::new(config.target_fps),
            config,
            world: World::new(),
            type_registry: crate::reflect_registry::component_registry(),
            physics: None,
            audio: None,
            window: None,
            render_ctx: None,
            renderer: None,
            ui_manager: None,
            selected_entity: None,
            state: LifecycleState::Uninitialized,
            undo_stack: UndoStack::new(128),
            cursor_pos: (0.0, 0.0),
            viewport_size: initial_vp,
            gizmo_drag: None,
            foliage_meshes: std::array::from_fn(|_| None),
            foliage_failed: [false; FOLIAGE_PALETTE.len()],
            foliage_brush: somnium_renderer::terrain::foliage_paint::FoliageBrush::default(),
            foliage_paint_active: false,
            foliage_erase: false,
            foliage_batch: Vec::new(),
            foliage_stroke_seed: 0,
            foliage_painting: false,
            terrain_colliders: std::collections::HashMap::new(),
            scrub_transform: None,
            scrub_light: None,
            log_rx: Some(log_rx),
            shortcut_modifiers: somnium_ui::message::Modifiers::default(),
            default_material_id: None,
            terrain_edit_active: false,
            terrain_brush: TerrainBrush::default(),
            terrain_debug_view: 0.0,
            terrain_stroke: None,
            terrain_restore_queue: TerrainRestoreQueue::default(),
            camera_speed_norm: crate::DEFAULT_CAMERA_SPEED_NORM,
            viewport_resolution: std::env::var("SOMNIUM_VIEWPORT_RES")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)
                .min(4),
            simulation_clock: SimulationClock::default(),
            path_trace_previous_simulation_state: None,
            play_session_active: false,
            simulation_accumulator: 0.0,
            scene_dirty: false,
            ui_wants_exit: false,
            pending_map_load: None,
            scripts: crate::script_host::ScriptHost::default(),
            script_input: crate::script_input::ScriptInputTracker::new(),
            play_checkpoint: None,
            script_step: 0,
        };

        event_loop
            .run_app(&mut engine)
            .map_err(|e| EngineError::EventLoop(e.to_string()))?;

        info!("Somnium Engine shut down cleanly");
        Ok(())
    }
}

impl<G: GameApp> Engine<G> {
    // ── Phase 16-C: driving scripts from the frame loop ──────────────────

    /// The clock a script sees. Fixed-step callbacks are handed
    /// `fixed_delta` and simulation time and nothing else, because those
    /// are the only two values that are the same on a replay.
    fn script_time(
        &self,
        fixed_dt: f32,
        dt: f32,
    ) -> somnium_script::snapshot::TimeSnapshot {
        somnium_script::snapshot::TimeSnapshot {
            fixed_delta: fixed_dt,
            delta: dt,
            simulation_time: f64::from(self.simulation_clock.elapsed_seconds),
            step: self.script_step,
        }
    }

    fn sync_scripts(&mut self, dt: f32) {
        let phase = somnium_script::runtime::PhaseInput {
            time: self.script_time(self.simulation_clock.fixed_delta_seconds, dt),
            input: self.script_input.snapshot(),
        };
        let mut services = crate::script_host::HostServices {
            physics: self.physics.as_mut(),
            audio: self.audio.as_mut(),
        };
        let report = self.scripts.sync(&mut self.world, &phase, &mut services);
        if report.hit_cap {
            warn!("script initialisation hit its cycle cap; see the Output Log");
        }
    }

    fn script_fixed_update(&mut self, fixed_dt: f32, dt: f32) {
        let time = self.script_time(fixed_dt, dt);
        let input = self.script_input.snapshot();
        let mut services = crate::script_host::HostServices {
            physics: self.physics.as_mut(),
            audio: self.audio.as_mut(),
        };
        self.scripts
            .fixed_update(&mut self.world, time, &input, &mut services);
    }

    fn script_update(&mut self, dt: f32) {
        let time = self.script_time(self.simulation_clock.fixed_delta_seconds, dt);
        let input = self.script_input.snapshot();
        let mut services = crate::script_host::HostServices {
            physics: self.physics.as_mut(),
            audio: self.audio.as_mut(),
        };
        self.scripts
            .update(&mut self.world, time, &input, &mut services);
    }

    /// Phase 16-E: recompile scripts whose file changed and settled.
    ///
    /// A quarter of a second is long enough to cover an editor that
    /// writes a file in several chunks and short enough that the reload
    /// feels immediate. Nothing here can break a running session: a file
    /// that no longer compiles keeps its live instances and publishes
    /// diagnostics.
    fn poll_script_reloads(&mut self) {
        const SETTLE: std::time::Duration = std::time::Duration::from_millis(250);
        let (reloaded, failed) = self.scripts.reload_changed(SETTLE);
        if reloaded == 0 && failed == 0 {
            return;
        }
        if let Some(ui) = self.ui_manager.as_mut() {
            if failed == 0 {
                ui.clear_script_errors();
                ui.push_toast(&format!("Reloaded {reloaded} script(s)"));
            } else {
                ui.push_toast(&format!(
                    "{failed} script(s) still failing — see the Output Log"
                ));
            }
        }
    }

    /// Move everything scripts produced into the editor's Output Log.
    ///
    /// Always drained, even while stopped, so a compile error raised by an
    /// import is not stuck in a buffer until the next Play.
    fn drain_script_output(&mut self) {
        let logs = self.scripts.take_logs();
        let diagnostics = self.scripts.take_diagnostics();
        let rejections = self.scripts.take_rejections();
        if logs.is_empty() && diagnostics.is_empty() && rejections.is_empty() {
            return;
        }
        let errors = diagnostics
            .iter()
            .filter(|d| d.severity == somnium_script::backend::Severity::Error)
            .count();
        if let Some(ui) = self.ui_manager.as_mut() {
            for line in logs {
                ui.append_log(&line.to_string());
            }
            for diagnostic in &diagnostics {
                ui.append_log(&format!("[script] {diagnostic}"));
            }
            for rejection in rejections {
                ui.append_log(&format!("[script rejected] {rejection}"));
            }
            if errors > 0 {
                ui.set_script_error_count(errors);
            }
        }
    }

    // ── Phase 16-D: the editor's scripting actions ───────────────────────

    /// How many attachments the selection carries, if it carries a
    /// `ScriptSet` at all.
    fn selected_script_count(&self) -> Option<usize> {
        let entity = self.selected_entity?;
        self.world
            .get::<somnium_script::attachment::ScriptSet>(entity)
            .map(somnium_script::attachment::ScriptSet::len)
    }

    /// Run one script `EditorCommand` against the selection.
    ///
    /// Every scripting edit goes through the undo stack, so Ctrl+Z covers
    /// attaching a behaviour the same way it covers moving an object.
    fn push_script_command(
        &mut self,
        build: impl FnOnce(u32) -> Box<dyn crate::editor_commands::EditorCommand>,
    ) {
        let Some(entity) = self.selected_entity else {
            return;
        };
        let command = build(entity.index());
        self.undo_stack
            .push(command, &mut self.world, &mut self.selected_entity);
        self.scene_dirty = true;
    }

    /// Import a `.luau` file and attach it to the selection.
    fn attach_script(&mut self, path: &std::path::Path) {
        match self.scripts.import_script_file(path) {
            Ok(asset) => {
                if self.selected_entity.is_none() {
                    if let Some(ui) = &mut self.ui_manager {
                        ui.push_toast("Select an entity first, then click the script");
                    }
                    return;
                }
                self.push_script_command(|entity| {
                    Box::new(crate::editor_commands::AttachScriptCmd::new(entity, asset))
                });
                if let Some(ui) = &mut self.ui_manager {
                    ui.push_toast(&format!(
                        "Attached {}",
                        path.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    ));
                }
            }
            Err(diagnostics) => {
                let errors = diagnostics.messages.len();
                if let Some(ui) = &mut self.ui_manager {
                    for message in &diagnostics.messages {
                        ui.append_log(&format!("[script] {message}"));
                    }
                    ui.set_script_error_count(errors);
                    ui.push_toast("Script failed to compile — see the Output Log");
                }
            }
        }
        self.drain_script_output();
    }

    // ── Content Drawer authoring ─────────────────────────────────────────

    /// Resolve a name typed in the drawer to a path that is safe to write.
    ///
    /// Returns `None` — having already told the author why — when the name
    /// would escape the content root, is empty, or names something that
    /// already exists. **Nothing here overwrites.** Losing someone's file
    /// to a mistyped name in a modal is not a recoverable mistake, and the
    /// drawer has no undo.
    fn content_target(
        &mut self,
        parent: &str,
        name: &str,
        force_extension: Option<&str>,
    ) -> Option<std::path::PathBuf> {
        let root = std::env::current_dir().unwrap_or_default().join("assets");
        match resolve_content_target(&root, parent, name, force_extension) {
            Ok(path) if path.exists() => {
                self.report_content_error(&path, "something with that name is already here");
                None
            }
            Ok(path) => Some(path),
            Err(reason) => {
                self.report_content_error(std::path::Path::new(name), reason);
                None
            }
        }
    }

    /// Refresh the drawer and say what happened.
    fn after_content_change(&mut self, message: &str) {
        if let Some(ui) = self.ui_manager.as_mut() {
            ui.refresh_content();
            ui.push_toast(message);
        }
    }

    /// Report a content operation that could not be done.
    fn report_content_error(&mut self, path: &std::path::Path, reason: &str) {
        let text = format!("{}: {reason}", path.display());
        error!("{text}");
        if let Some(ui) = self.ui_manager.as_mut() {
            ui.append_log(&format!("[content] {text}"));
            ui.push_toast(reason);
        }
    }

    /// Write a new `.luau` file from the template and attach it.
    ///
    /// Never overwrites: a name that exists gets a numeric suffix. Losing
    /// someone's script to a menu click is not a recoverable mistake.
    fn create_script(&mut self) {
        let folder = std::env::current_dir()
            .unwrap_or_default()
            .join("assets")
            .join("scripts");
        if let Err(error) = std::fs::create_dir_all(&folder) {
            error!("cannot create {}: {error}", folder.display());
            return;
        }
        let mut path = folder.join("NewScript.luau");
        let mut suffix = 1;
        while path.exists() {
            path = folder.join(format!("NewScript{suffix}.luau"));
            suffix += 1;
        }
        if let Err(error) = std::fs::write(&path, crate::script_host::NEW_SCRIPT_TEMPLATE) {
            error!("cannot write {}: {error}", path.display());
            return;
        }
        info!("Created {}", path.display());
        self.attach_script(&path);
        if let Some(ui) = &mut self.ui_manager {
            ui.refresh_content();
        }
    }

    /// Apply one property edit, recording it only when the gesture ends.
    fn set_script_property(
        &mut self,
        index: usize,
        field: String,
        value: somnium_script::value::ScriptValue,
        live: bool,
    ) {
        let Some(entity) = self.selected_entity else {
            return;
        };
        if live {
            // Mid-drag: apply it, do not record it. The gesture's final
            // value arrives once with `live == false` and becomes the one
            // undo step — the same convention `SetInspectorValue` uses.
            if let Some(set) = self
                .world
                .get_mut::<somnium_script::attachment::ScriptSet>(entity)
            {
                if let Some(attachment) = set.attachments.get_mut(index) {
                    attachment.properties.insert(field, value);
                }
            }
            self.scene_dirty = true;
            return;
        }
        self.push_script_command(|entity_index| {
            Box::new(crate::editor_commands::SetScriptPropertyCmd::new(
                entity_index,
                index,
                field,
                value,
            ))
        });
    }

    /// Describe the selection's attachments for the Details panel.
    ///
    /// Every row here comes from the script's own declaration. Nothing in
    /// this function names a property, and adding one to a `.luau` file
    /// changes nothing in Rust.
    fn script_inspector_state(&self) -> Option<somnium_ui::ScriptInspectorState> {
        let entity = self.selected_entity?;
        let set = self
            .world
            .get::<somnium_script::attachment::ScriptSet>(entity)?;

        let mut attachments = Vec::with_capacity(set.attachments.len());
        for attachment in &set.attachments {
            let schema = self.scripts.runtime().asset_schema(attachment.asset);
            let asset_name = self
                .scripts
                .script_path(attachment.asset)
                .and_then(|path| path.file_name())
                .map_or_else(
                    || attachment.asset.to_string(),
                    |name| name.to_string_lossy().into_owned(),
                );

            let quarantined = self.scripts.runtime().is_quarantined(attachment.instance);
            let status = match self.scripts.state_of(attachment.instance) {
                Some(state) if quarantined => {
                    format!("{} — quarantined after repeated errors", state.name())
                }
                Some(state) => state.name().to_string(),
                None if schema.is_none() => "asset not imported".to_string(),
                None => "not running (press Play)".to_string(),
            };

            let fields = schema.map_or_else(Vec::new, |schema| {
                schema
                    .fields
                    .iter()
                    .map(|field| {
                        let authored = attachment.properties.get(&field.name);
                        let value = authored.unwrap_or(&field.default);
                        somnium_ui::ScriptFieldRow {
                            name: field.name.clone(),
                            kind: script_field_kind(value, field),
                            description: field.description.clone(),
                        }
                    })
                    .collect()
            });

            attachments.push(somnium_ui::ScriptAttachmentRow {
                asset_name,
                status,
                enabled: attachment.enabled,
                quarantined,
                fields,
            });
        }
        Some(somnium_ui::ScriptInspectorState { attachments })
    }

    /// Capture the authored world so Stop can put it back, and start the
    /// script clock from zero.
    fn begin_play_session(&mut self) {
        self.play_checkpoint = Some(crate::script_input::WorldCheckpoint::capture(
            &mut self.world,
            &self.type_registry,
        ));
        self.script_step = 0;
        self.scripts.runtime_mut().set_world_seed(SCRIPT_WORLD_SEED);
    }

    /// Tear every script down and restore the world exactly as it was.
    fn end_play_session(&mut self) {
        let mut services = crate::script_host::HostServices {
            physics: self.physics.as_mut(),
            audio: self.audio.as_mut(),
        };
        self.scripts.shutdown(&mut self.world, &mut services);
        if let Some(checkpoint) = self.play_checkpoint.take() {
            checkpoint.restore(&mut self.world, &self.type_registry);
        }
        self.script_step = 0;
        self.drain_script_output();
    }
}

/// How one declared property is drawn.
///
/// Numbers and booleans are editable; everything else is shown read-only
/// rather than hidden, because an absent row looks like the script forgot
/// to declare the property and a visible one says "not authorable yet".
fn script_field_kind(
    value: &somnium_script::value::ScriptValue,
    field: &somnium_script::backend::ScriptFieldSchema,
) -> somnium_ui::ScriptFieldKind {
    use somnium_script::value::ScriptValue as V;
    match value {
        #[allow(clippy::cast_possible_truncation)]
        V::F64(v) => somnium_ui::ScriptFieldKind::Number {
            value: *v as f32,
            min: field.min.map(|m| m as f32),
            max: field.max.map(|m| m as f32),
        },
        #[allow(clippy::cast_precision_loss)]
        V::I64(v) => somnium_ui::ScriptFieldKind::Number {
            value: *v as f32,
            min: field.min.map(|m| m as f32),
            max: field.max.map(|m| m as f32),
        },
        V::Bool(v) => somnium_ui::ScriptFieldKind::Bool(*v),
        V::Str(v) => somnium_ui::ScriptFieldKind::Text(v.clone()),
        V::Nil => somnium_ui::ScriptFieldKind::Text("unset".into()),
        other => somnium_ui::ScriptFieldKind::Text(other.kind().to_string()),
    }
}

/// Turn a name typed in the Content Drawer into a path inside the content
/// root, or say why it cannot be one.
///
/// Separated from the engine so the rules are testable without a window.
/// They are worth testing: this is the only place in the editor where a
/// string an author typed becomes a filesystem path, and the drawer has
/// no undo.
///
/// # Errors
///
/// A message fit to show in a toast.
fn resolve_content_target(
    root: &std::path::Path,
    parent: &str,
    name: &str,
    force_extension: Option<&str>,
) -> Result<std::path::PathBuf, &'static str> {
    let leaf = name.trim();
    if leaf.is_empty() {
        return Err("a name cannot be empty");
    }
    // A separator would let `../..` walk out of the content root, and
    // creating into a nested folder is not what the menu offers anyway.
    if leaf.contains(['/', '\\']) || leaf == "." || leaf == ".." {
        return Err("a name cannot contain a path separator");
    }
    // Windows reserves these whatever the extension, and a file named
    // after one is created and then unopenable.
    const RESERVED: &[&str] = &[
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "lpt1", "lpt2", "lpt3",
    ];
    let stem = leaf.split('.').next().unwrap_or(leaf).to_ascii_lowercase();
    if RESERVED.contains(&stem.as_str()) {
        return Err("that name is reserved by the operating system");
    }

    let mut path = if parent.is_empty() {
        root.to_path_buf()
    } else {
        root.join(parent)
    }
    .join(leaf);

    if let Some(extension) = force_extension {
        if path
            .extension()
            .is_none_or(|e| !e.eq_ignore_ascii_case(extension))
        {
            // `set_extension` replaces rather than appends, which is what
            // turns `Player.controller` into `Player.luau` rather than
            // `Player.controller.luau`.
            path.set_extension(extension);
        }
    }

    // Belt and braces on the separator check: whatever the name was, the
    // result has to still be under the content root.
    if !path.starts_with(root) {
        return Err("that would land outside the content folder");
    }
    Ok(path)
}

/// Open the OS file browser at `path`, selecting it where the platform
/// supports that.
///
/// Deliberately *reveal*, not *open*. Opening a `.luau` means launching
/// whatever the OS has associated with the extension, which on a fresh
/// machine is nothing, and on a developer's machine is a coin toss.
/// Choosing an editor is its own sub-phase; showing someone where the
/// file is costs nothing and is never wrong.
///
/// # Errors
///
/// A message naming what the platform said.
fn reveal_in_file_browser(path: &std::path::Path) -> Result<(), String> {
    if !path.exists() {
        return Err("that file is no longer there".to_string());
    }
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer")
        .arg("/select,")
        .arg(path)
        .spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open")
        // No "select the file" on the freedesktop side, so open the
        // folder it is in rather than the file itself — which would hand
        // it to whatever `.luau` is associated with, if anything.
        .arg(path.parent().unwrap_or(path))
        .spawn();

    // `explorer` returns a non-zero exit code even when it succeeds, so
    // the spawn is what is checked and the status deliberately is not.
    result.map(|_| ()).map_err(|error| error.to_string())
}

/// The seed every play session's script random streams derive from.
///
/// Fixed rather than clock-derived: Phase 16 promises that the same build
/// on the same platform replays identically, and a wall-clock seed would
/// break that on the first frame.
const SCRIPT_WORLD_SEED: u64 = 0x536F_6D6E_6975_6D01;

impl<G: GameApp> ApplicationHandler for Engine<G> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Phase 16-F: keep the generated `.d.luau` current.
        //
        // Rewritten only when it differs from what is on disk, so an
        // ordinary run does not dirty the working tree — and so a
        // component added to the registry updates it without anyone
        // having to remember to regenerate.
        {
            let path = crate::script_decls::default_declarations_path();
            let generated = crate::script_decls::generate_declarations(&self.type_registry);
            let current = std::fs::read_to_string(&path).is_ok_and(|on_disk| on_disk == generated);
            if !current {
                match std::fs::create_dir_all(path.parent().unwrap_or(&path))
                    .and_then(|()| std::fs::write(&path, &generated))
                {
                    Ok(()) => info!("Wrote script declarations to {}", path.display()),
                    Err(error) => warn!("could not write {}: {error}", path.display()),
                }
            }
        }

        if self.physics.is_none() {
            self.physics = Some(PhysicsWorld::new(PhysicsConfig::default()));
        }

        if self.audio.is_none() {
            match AudioEngine::new() {
                Ok(engine) => self.audio = Some(engine),
                Err(e) => tracing::error!("Failed to initialize audio: {}", e),
            }
        }

        if self.state == LifecycleState::Running {
            debug!("Engine resumed from suspend");
            let mut ctx = EngineContext::new(
                &self.time,
                &self.config,
                &mut self.world,
                self.physics.as_mut().unwrap(),
                self.audio.as_mut().unwrap(),
                self.render_ctx.as_ref(),
                self.renderer.as_mut(),
                &mut self.selected_entity,
                self.ui_manager.as_mut().unwrap(),
                crate::camera_speed_from_normalized(self.camera_speed_norm),
                self.simulation_clock,
                &mut self.scripts,
            );
            self.game.on_event(&mut ctx, &EngineEvent::Resumed);
            return;
        }

        info!("Creating window");

        let size = LogicalSize::new(self.config.window_size.0, self.config.window_size.1);
        let mut attrs = WindowAttributes::default()
            .with_title("Somnium Engine")
            .with_inner_size(size)
            .with_resizable(self.config.resizable)
            .with_decorations(false);
        #[cfg(target_os = "windows")]
        {
            attrs = attrs.with_undecorated_shadow(true);
        }

        match event_loop.create_window(attrs) {
            Ok(window) => {
                let window = Arc::new(window);
                if std::env::var("SOMNIUM_MAXIMIZE").as_deref() == Ok("1") {
                    window.set_maximized(true);
                    info!("Window maximized (SOMNIUM_MAXIMIZE=1)");
                }
                self.window = Some(Arc::clone(&window));

                info!("Initializing rendering subsystems...");
                let render_ctx = pollster::block_on(RenderContext::new(Arc::clone(&window)));
                let renderer = SomniumRenderer::new(&render_ctx);

                let ui_manager = UiManager::new(
                    &render_ctx.device,
                    render_ctx.config.format,
                    1,
                    &render_ctx.queue,
                    Arc::clone(&window),
                );

                self.render_ctx = Some(render_ctx);
                self.renderer = Some(renderer);
                self.ui_manager = Some(ui_manager);

                self.state = LifecycleState::Running;

                let mut ctx = EngineContext::new(
                    &self.time,
                    &self.config,
                    &mut self.world,
                    self.physics.as_mut().unwrap(),
                    self.audio.as_mut().unwrap(),
                    self.render_ctx.as_ref(),
                    self.renderer.as_mut(),
                    &mut self.selected_entity,
                    self.ui_manager.as_mut().unwrap(),
                    crate::camera_speed_from_normalized(self.camera_speed_norm),
                    self.simulation_clock,
                    &mut self.scripts,
                );
                self.game.on_init(&mut ctx);

                if ctx.should_exit {
                    self.initiate_shutdown(event_loop);
                    return;
                }
            }
            Err(err) => {
                error!(%err, "Failed to create window");
                event_loop.exit();
            }
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        if self.state == LifecycleState::Running {
            debug!("Engine suspended");
            self.state = LifecycleState::Suspended;
            let mut ctx = EngineContext::new(
                &self.time,
                &self.config,
                &mut self.world,
                self.physics.as_mut().unwrap(),
                self.audio.as_mut().unwrap(),
                self.render_ctx.as_ref(),
                self.renderer.as_mut(),
                &mut self.selected_entity,
                self.ui_manager.as_mut().unwrap(),
                crate::camera_speed_from_normalized(self.camera_speed_norm),
                self.simulation_clock,
                &mut self.scripts,
            );
            self.game.on_event(&mut ctx, &EngineEvent::Suspended);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.state != LifecycleState::Running {
            return;
        }

        // Always track cursor position (needed for gizmo picking every frame).
        if let WindowEvent::CursorMoved { position, .. } = &event {
            self.cursor_pos = (position.x as f32, position.y as f32);
        }

        // Track the exact modifier state for registry-backed shortcuts.  Extra
        // modifiers must not accidentally match a destructive command.
        if let WindowEvent::ModifiersChanged(m) = &event {
            let state = m.state();
            self.shortcut_modifiers = somnium_ui::message::Modifiers {
                ctrl: state.control_key(),
                shift: state.shift_key(),
                alt: state.alt_key(),
                logo: state.super_key(),
            };
        }

        // Handle Resizing
        if let WindowEvent::Resized(size) = &event {
            self.viewport_size = (size.width as f32, size.height as f32);
            if let Some(r_ctx) = &mut self.render_ctx {
                r_ctx.resize(size.width, size.height);
            }
            if let (Some(r), Some(c)) = (&mut self.renderer, &self.render_ctx) {
                let (sw, sh) = somnium_renderer::scene_size_for_preset(
                    size.width,
                    size.height,
                    self.viewport_resolution,
                );
                r.resize(c, sw, sh);
            }
            if let (Some(ui), Some(window)) = (&mut self.ui_manager, &self.window) {
                ui.reposition_panels(window);
            }
        }

        // ── 1. Registered editor shortcuts FIRST (never array-position dispatch) ──
        if let WindowEvent::KeyboardInput { event: key_ev, .. } = &event {
            if key_ev.state == winit::event::ElementState::Pressed && !key_ev.repeat {
                use winit::keyboard::PhysicalKey;
                if let PhysicalKey::Code(code) = key_ev.physical_key {
                    let chord = somnium_ui::commands::Chord::from_winit(
                        code,
                        self.shortcut_modifiers.command(),
                        self.shortcut_modifiers.shift,
                        self.shortcut_modifiers.alt,
                        false,
                    );
                    let action = chord
                        .and_then(|chord| somnium_ui::commands::registry().binding(chord))
                        .map(|command| command.action);
                    use somnium_ui::commands::CommandAction as A;
                    match action {
                        Some(A::NewScene) => {
                            if let Some(ui) = &mut self.ui_manager {
                                ui.set_scene_dirty(self.scene_dirty);
                                ui.prompt_unsaved_new();
                            } else {
                                self.handle_editor_event(EditorEvent::NewScene);
                            }
                            return;
                        }
                        Some(A::SaveScene) => { self.handle_editor_event(EditorEvent::SaveScene); return; }
                        Some(A::Undo) => { self.handle_editor_event(EditorEvent::Undo); return; }
                        Some(A::Redo) => { self.handle_editor_event(EditorEvent::Redo); return; }
                        Some(A::DeleteSelected) => { self.handle_editor_event(EditorEvent::DeleteSelected); return; }
                        Some(A::DuplicateSelected) => { self.handle_editor_event(EditorEvent::DuplicateSelected); return; }
                        Some(A::SetGizmoMode(mode)) => { self.handle_editor_event(EditorEvent::SetGizmoMode(mode)); return; }
                        Some(A::ToggleTerrainEdit) => { self.handle_editor_event(EditorEvent::ToggleTerrainEdit); return; }
                        Some(A::ToggleFoliage) => { self.handle_editor_event(EditorEvent::ToggleFoliage); return; }
                        Some(A::ReloadScripts) => { self.handle_editor_event(EditorEvent::ReloadScripts); return; }
                        _ => {}
                    }
                }
            }
        }

        // ── 2. Other shortcuts (non-Ctrl) ──────────────────────────────────────
        if let WindowEvent::KeyboardInput { event: key_ev, .. } = &event {
            if key_ev.state == winit::event::ElementState::Pressed && !key_ev.repeat {
                use winit::keyboard::{KeyCode as WKC, PhysicalKey};
                if let PhysicalKey::Code(code) = key_ev.physical_key {
                    match code {
                        WKC::BracketLeft if self.terrain_edit_active => {
                            self.terrain_brush.radius = (self.terrain_brush.radius / 1.25).max(0.5);
                            info!("Brush radius: {:.1} m", self.terrain_brush.radius);
                        }
                        WKC::BracketRight if self.terrain_edit_active => {
                            self.terrain_brush.radius =
                                (self.terrain_brush.radius * 1.25).min(128.0);
                            info!("Brush radius: {:.1} m", self.terrain_brush.radius);
                        }
                        WKC::Minus if self.terrain_edit_active => {
                            self.terrain_brush.strength =
                                (self.terrain_brush.strength - 0.1).max(0.05);
                            info!("Brush strength: {:.2}", self.terrain_brush.strength);
                        }
                        WKC::Equal if self.terrain_edit_active => {
                            self.terrain_brush.strength =
                                (self.terrain_brush.strength + 0.1).min(1.0);
                            info!("Brush strength: {:.2}", self.terrain_brush.strength);
                        }
                        WKC::Comma if self.terrain_edit_active => {
                            self.terrain_brush.paint_layer =
                                self.terrain_brush.paint_layer.checked_sub(1).unwrap_or(
                                    somnium_renderer::terrain::textures::TERRAIN_LAYER_COUNT
                                        as usize
                                        - 1,
                                );
                            info!("Paint layer: {}", self.terrain_brush.paint_layer);
                        }
                        WKC::Period if self.terrain_edit_active => {
                            self.terrain_brush.paint_layer = (self.terrain_brush.paint_layer + 1)
                                % somnium_renderer::terrain::textures::TERRAIN_LAYER_COUNT as usize;
                            info!("Paint layer: {}", self.terrain_brush.paint_layer);
                        }
                        WKC::Digit1 if self.terrain_edit_active => self.set_terrain_tool(0),
                        WKC::Digit2 if self.terrain_edit_active => self.set_terrain_tool(1),
                        WKC::Digit3 if self.terrain_edit_active => self.set_terrain_tool(2),
                        WKC::Digit4 if self.terrain_edit_active => self.set_terrain_tool(3),
                        WKC::Digit5 if self.terrain_edit_active => self.set_terrain_tool(4),
                        WKC::Digit6 if self.terrain_edit_active => self.set_terrain_tool(5),
                        _ => {}
                    }
                }
            }
        }

        // ── 3. Route to native UI; return early if consumed ──────────────────
        let ui_consumed = if let Some(ui) = &mut self.ui_manager {
            ui.process_os_event(&event)
        } else {
            false
        };
        if ui_consumed {
            return;
        }

        // ── 3.4 Foliage brush (Phase 17F) — takes priority over sculpting ────
        if !self.play_session_active && self.foliage_paint_active {
            if let WindowEvent::MouseInput {
                state: winit::event::ElementState::Pressed,
                button: winit::event::MouseButton::Left,
                ..
            } = &event
            {
                self.foliage_painting = self.paint_foliage_dab();
                if self.foliage_painting {
                    return;
                }
            }
            if let WindowEvent::MouseInput {
                state: winit::event::ElementState::Released,
                button: winit::event::MouseButton::Left,
                ..
            } = &event
            {
                if self.foliage_painting {
                    self.foliage_painting = false;
                    return;
                }
            }
            // Dragging keeps dabbing, which is what makes it a brush rather
            // than a stamp.
            if self.foliage_painting {
                if let WindowEvent::CursorMoved { .. } = &event {
                    self.paint_foliage_dab();
                    return;
                }
            }
        }

        // ── 3.5 Terrain brush stroke (Phase 14D) — takes priority over gizmo ──
        if !self.play_session_active && self.terrain_edit_active {
            if let WindowEvent::MouseInput {
                state: winit::event::ElementState::Pressed,
                button: winit::event::MouseButton::Left,
                ..
            } = &event
            {
                if self.begin_terrain_stroke() {
                    return;
                }
            }
            if let WindowEvent::MouseInput {
                state: winit::event::ElementState::Released,
                button: winit::event::MouseButton::Left,
                ..
            } = &event
            {
                if self.end_terrain_stroke() {
                    return;
                }
            }
        }

        // ── 4. Gizmo LMB pick / drag-end ────────────────────────────────────
        let mut gizmo_consumed = false;

        if !self.play_session_active {
            if let WindowEvent::MouseInput {
                state: winit::event::ElementState::Pressed,
                button: winit::event::MouseButton::Left,
                ..
            } = &event
            {
                let drag = try_start_gizmo_drag(
                    self.renderer.as_ref(),
                    &self.world,
                    &self.selected_entity,
                    self.cursor_pos,
                    self.viewport_size,
                );
                if drag.is_some() {
                    self.gizmo_drag = drag;
                    gizmo_consumed = true;
                }
            }

            if let WindowEvent::MouseInput {
                state: winit::event::ElementState::Released,
                button: winit::event::MouseButton::Left,
                ..
            } = &event
            {
                if let Some(drag) = self.gizmo_drag.take() {
                    if let Some(entity) = self.world.find_entity_by_index(drag.entity_index) {
                        let final_t = self
                            .world
                            .get::<Transform>(entity)
                            .copied()
                            .unwrap_or(drag.start_transform);
                        let cmd = Box::new(SetTransformCmd::new(
                            drag.entity_index,
                            drag.start_transform,
                            final_t,
                        ));
                        self.undo_stack.push_silent(cmd);
                    }
                    gizmo_consumed = true;
                }
            }
        }

        if gizmo_consumed {
            return;
        }

        // ── 5. Forward remaining events to game ──────────────────────────────
        if let Some(engine_event) = translate_window_event(&event) {
            // Phase 16-C: scripts see a sampled snapshot per fixed step
            // rather than the event stream, so the tracker folds every
            // event in here and the phase reads it later.
            self.script_input.observe(&engine_event);
            let mut ctx = EngineContext::new(
                &self.time,
                &self.config,
                &mut self.world,
                self.physics.as_mut().unwrap(),
                self.audio.as_mut().unwrap(),
                self.render_ctx.as_ref(),
                self.renderer.as_mut(),
                &mut self.selected_entity,
                self.ui_manager.as_mut().unwrap(),
                crate::camera_speed_from_normalized(self.camera_speed_norm),
                self.simulation_clock,
                &mut self.scripts,
            );
            self.game.on_event(&mut ctx, &engine_event);
            let speed_request = ctx.camera_speed_request;

            if ctx.should_exit {
                self.initiate_shutdown(event_loop);
                return;
            }

            if matches!(engine_event, EngineEvent::WindowCloseRequested) {
                self.initiate_shutdown(event_loop);
            }

            // Phase 20B: apply a camera-speed change requested by game code
            // (RMB + wheel) and keep the toolbar slider in sync with it.
            if let Some(norm) = speed_request {
                self.camera_speed_norm = norm;
                let speed = crate::camera_speed_from_normalized(norm);
                if let Some(ui) = &mut self.ui_manager {
                    ui.update_camera_speed(norm, speed);
                }
            }
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        if self.state != LifecycleState::Running {
            return;
        }

        let engine_event: Option<EngineEvent> = match event {
            winit::event::DeviceEvent::MouseMotion { delta } => Some(EngineEvent::MouseMotion {
                delta_x: delta.0 as f32,
                delta_y: delta.1 as f32,
            }),
            _ => None,
        };

        if let Some(ev) = engine_event {
            self.script_input.observe(&ev);
            let mut ctx = EngineContext::new(
                &self.time,
                &self.config,
                &mut self.world,
                self.physics.as_mut().unwrap(),
                self.audio.as_mut().unwrap(),
                self.render_ctx.as_ref(),
                self.renderer.as_mut(),
                &mut self.selected_entity,
                self.ui_manager.as_mut().unwrap(),
                crate::camera_speed_from_normalized(self.camera_speed_norm),
                self.simulation_clock,
                &mut self.scripts,
            );
            self.game.on_event(&mut ctx, &ev);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.state != LifecycleState::Running {
            return;
        }

        self.time.tick();
        let dt = self.time.delta_time().as_secs_f32();
        // Phase 16-F: the script profiler counters are per frame, and the
        // frame starts here.
        self.scripts.begin_frame();

        let path_tracer_active = self
            .world
            .entities()
            .find_map(|entity| self.world.get::<PostProcessComponent>(entity))
            .is_some_and(|post| post.path_tracer);
        synchronize_path_trace_pause(
            path_tracer_active,
            &mut self.simulation_clock,
            &mut self.simulation_accumulator,
            &mut self.path_trace_previous_simulation_state,
        );

        // Editor-world simulations (water, buoyancy, particles, previews) run
        // in ordinary Edit mode. Pause is the only state that freezes time;
        // Play remains a game-state distinction, not an animation power switch.
        // Phase 16-C: reconcile the live script set with the authored one
        // before any phase runs, so an attachment added this frame is
        // initialised this frame. Only while Play is running — scripts are
        // not allowed to dirty the edit-time scene.
        if self.simulation_clock.state == SimulationState::Playing {
            self.sync_scripts(dt);
        }

        if self.simulation_clock.state != SimulationState::Paused {
            self.simulation_accumulator += dt.min(0.1);
            let fixed_dt = self.simulation_clock.fixed_delta_seconds;
            while self.simulation_accumulator >= fixed_dt {
                {
                    let mut ctx = EngineContext::new(
                        &self.time,
                        &self.config,
                        &mut self.world,
                        self.physics.as_mut().unwrap(),
                        self.audio.as_mut().unwrap(),
                        self.render_ctx.as_ref(),
                        self.renderer.as_mut(),
                        &mut self.selected_entity,
                        self.ui_manager.as_mut().unwrap(),
                        crate::camera_speed_from_normalized(self.camera_speed_norm),
                        self.simulation_clock,
                        &mut self.scripts,
                    );
                    self.game.on_fixed_update(&mut ctx);
                }
                // `onFixedUpdate` runs here, inside the accumulator and
                // **before** `physics.step`, so a force a script applies is
                // integrated by the same step that applied it. This is the
                // deterministic hook; nothing about the loop needed
                // restructuring to get it.
                if self.simulation_clock.state == SimulationState::Playing {
                    // Jolt → components, so a script sees the velocity it
                    // actually has after last step's collisions rather
                    // than the one it asked for.
                    if let Some(physics) = self.physics.as_ref() {
                        crate::character::read_physics_into_world(&mut self.world, physics);
                    }
                    self.script_fixed_update(fixed_dt, dt);
                    // Components → Jolt, after the command apply, so a
                    // script's write is the last word before integration.
                    if let Some(physics) = self.physics.as_mut() {
                        crate::character::write_world_into_physics(&self.world, physics);
                    }
                    self.script_input.end_step();
                    self.script_step += 1;
                }
                if let Some(physics) = self.physics.as_mut() {
                    physics.step(fixed_dt);
                }
                self.simulation_clock.elapsed_seconds += fixed_dt;
                self.simulation_accumulator -= fixed_dt;
            }
        }

        // ── Gizmo drag: update entity transform each frame while dragging ────
        let drag_result: Option<(u32, Transform)> = self.gizmo_drag.as_ref().and_then(|drag| {
            let (cam, inv_vp) = self
                .renderer
                .as_ref()
                .map(|r| (r.camera_pos, r.view_proj.inverse()))
                .unwrap_or((glam::Vec3::ZERO, glam::Mat4::IDENTITY));
            let new_t = apply_gizmo_drag(drag, cam, inv_vp, self.cursor_pos, self.viewport_size);
            Some((drag.entity_index, new_t))
        });
        if let Some((idx, new_t)) = drag_result {
            if let Some(entity) = self.world.find_entity_by_index(idx) {
                if let Some(t) = self.world.get_mut::<Transform>(entity) {
                    *t = new_t;
                }
            }
            if let Some(r) = &mut self.renderer {
                r.set_gizmo_world_pos(new_t.translation);
            }
        }

        {
            let mut ctx = EngineContext::new(
                &self.time,
                &self.config,
                &mut self.world,
                self.physics.as_mut().unwrap(),
                self.audio.as_mut().unwrap(),
                self.render_ctx.as_ref(),
                self.renderer.as_mut(),
                &mut self.selected_entity,
                self.ui_manager.as_mut().unwrap(),
                crate::camera_speed_from_normalized(self.camera_speed_norm),
                self.simulation_clock,
                &mut self.scripts,
            );
            self.game.on_update(&mut ctx);
        }

        // Phase 16-C: the variable-rate script phase, and the one place
        // script output reaches the editor's Output Log.
        if self.simulation_clock.state == SimulationState::Playing {
            self.script_update(dt);
        }
        // Phase 16-E: pick up edits made outside the editor. Runs while
        // stopped as well as while playing — an author fixing a compile
        // error wants to see it clear without pressing anything, and a
        // file that still does not compile costs nothing but a diagnostic.
        self.poll_script_reloads();
        self.drain_script_output();

        // Phase IV-C: ECS membership is authoritative for renderer-owned water
        // data. Delete drops textures; undo/redo recreates them from the small
        // stable descriptor, so no stale GPU handle survives in a component.
        let water_descriptors: Vec<_> = self
            .world
            .entities()
            .filter_map(|entity| self.world.get::<WaterComponent>(entity).copied())
            .filter(|water| water.enabled && water.water_id != u32::MAX && water.preset != 0)
            .map(WaterComponent::descriptor)
            .collect();
        if let (Some(renderer), Some(render_ctx)) = (&mut self.renderer, &self.render_ctx) {
            let active: std::collections::HashSet<u32> = water_descriptors
                .iter()
                .map(|descriptor| descriptor.water_id)
                .collect();
            for descriptor in water_descriptors {
                if let Err(error) = renderer.ensure_water_body(render_ctx, descriptor) {
                    warn!(
                        "Failed to restore water body {}: {error}",
                        descriptor.water_id
                    );
                }
            }
            renderer.water_bodies.retain_ids(&active);
        }

        // ── Update native UI panels with current frame state ─────────────────
        {
            let all_entities: Vec<somnium_ecs::Entity> = self.world.entities().collect();
            let mut names: Vec<(u32, String, Option<u32>)> = all_entities
                .iter()
                .map(|&e| {
                    let name = self
                        .world
                        .get::<Name>(e)
                        .map(|n| n.as_str().to_owned())
                        .unwrap_or_else(|| format!("Entity {}", e.index()));
                    let parent = self.world.get::<Parent>(e).and_then(|p| {
                        if p.entity == somnium_ecs::Entity::DANGLING {
                            None
                        } else {
                            Some(p.entity.index())
                        }
                    });
                    (e.index(), name, parent)
                })
                .collect();
            names.sort_by(|a, b| a.1.to_ascii_lowercase().cmp(&b.1.to_ascii_lowercase()));
            let mut children: std::collections::HashMap<u32, Vec<u32>> =
                std::collections::HashMap::new();
            let mut name_of: std::collections::HashMap<u32, String> =
                std::collections::HashMap::new();
            for (id, name, parent) in &names {
                name_of.insert(*id, name.clone());
                if let Some(p) = parent {
                    children.entry(*p).or_default().push(*id);
                }
            }
            fn walk(
                id: u32,
                depth: u8,
                name_of: &std::collections::HashMap<u32, String>,
                children: &std::collections::HashMap<u32, Vec<u32>>,
                out: &mut Vec<(u32, String, u8, bool)>,
            ) {
                let has = children.get(&id).map(|c| !c.is_empty()).unwrap_or(false);
                out.push((
                    id,
                    name_of.get(&id).cloned().unwrap_or_default(),
                    depth,
                    has,
                ));
                if let Some(kids) = children.get(&id) {
                    for kid in kids {
                        walk(*kid, depth.saturating_add(1), name_of, children, out);
                    }
                }
            }
            let mut tree = Vec::new();
            for (id, _, parent) in &names {
                let is_root = match parent {
                    None => true,
                    Some(p) => !name_of.contains_key(p),
                };
                if is_root {
                    walk(*id, 0, &name_of, &children, &mut tree);
                }
            }
            let selected_idx = self.selected_entity.map(|e| e.index());
            let sel_t = self
                .selected_entity
                .and_then(|e| self.world.get::<Transform>(e).copied());
            let sel_camera_settings = self
                .selected_entity
                .and_then(|e| self.world.get::<CameraSettingsComponent>(e).copied());
            let sel_camera = sel_camera_settings.map(|c| c.frustum_cull);
            // Phase DOOM-F.
            let sel_camera_dynres = sel_camera_settings
                .map(|c| (c.dynamic_resolution, c.dynamic_target_ms, c.dynamic_floor));
            // Phase 15A1: post-processing settings for the inspector.
            let sel_post = self
                .selected_entity
                .and_then(|e| self.world.get::<PostProcessComponent>(e).copied())
                .map(|pp| somnium_ui::PostInspectorState {
                    values: [
                        pp.ev100,
                        pp.exposure_compensation,
                        pp.vignette_strength,
                        pp.ca_strength,
                        pp.ibl_intensity,
                    ],
                    vignette: pp.vignette_enabled,
                    chromatic: pp.ca_enabled,
                    fxaa: pp.fxaa_enabled,
                    cel_shading: pp.cel_shading,
                    taa: pp.taa_enabled,
                    gtao: pp.gtao_enabled,
                    restir: pp.restir_enabled,
                    restir_gi: pp.restir_gi_enabled,
                    rt_reflect: pp.rt_reflect_enabled,
                    rt_refract: pp.rt_refract_enabled,
                    pcss: pp.pcss_enabled,
                    contact_shadows: pp.contact_shadows_enabled,
                    cas: pp.cas_enabled,
                    motion_blur: pp.motion_blur_enabled,
                    bloom: pp.bloom_enabled,
                    dof: pp.dof_enabled,
                    volumetrics: pp.volumetrics_enabled,
                    physical_camera: pp.use_physical_camera,
                    shafts: pp.light_shafts,
                    world_cache: pp.world_cache,
                    specular_gi: pp.specular_gi,
                    path_tracer: pp.path_tracer,
                    mesh_sdf: pp.mesh_sdf,
                    probes: pp.probes,
                    analytic_grad: pp.analytic_grad,
                    fsr: pp.fsr_enabled,
                    fsr_sharpness: pp.fsr_sharpness,
                    cache_intensity: pp.cache_intensity,
                    cache_cell: pp.cache_cell_size,
                    spec_rough: pp.spec_roughness,
                    path_bounces: pp.path_bounces as f32,
                    probe_intensity: pp.probe_intensity,
                    shaft_intensity: pp.shaft_intensity,
                    extras: [
                        pp.bloom_intensity,
                        pp.dof_focus_distance,
                        pp.temperature,
                        pp.contrast,
                        pp.saturation,
                        pp.grain,
                        pp.fog_density,
                        pp.fog_height_falloff,
                        pp.fog_asymmetry,
                        pp.tint,
                        pp.lift,
                        pp.gamma,
                        pp.gain,
                        pp.aperture_f_stops,
                        // Shown as the denominator: 0.01 s reads as 100.
                        if pp.shutter_speed_s > 0.0 {
                            1.0 / pp.shutter_speed_s
                        } else {
                            0.0
                        },
                        pp.sensitivity_iso,
                        pp.gtao_radius,
                        pp.gtao_intensity,
                        pp.cas_sharpness,
                        pp.cas_strength,
                        pp.motion_blur_shutter,
                        pp.restir_gi_intensity,
                    ],
                    auto_exposure: pp.auto_exposure,
                    tonemapper: pp.tonemapper.label(),
                });
            // Phase 17C: terrain layer + foliage settings for the inspector.
            let sel_terrain = self.selected_entity.and_then(|e| {
                let tc = self.world.get::<TerrainComponent>(e)?;
                let r = self.renderer.as_ref()?;
                let t = r.terrain(tc.terrain_id)?;
                let tile = |i: usize| t.layers.get(i).map_or(1.0, |l| l.tiling);
                let paint = self.terrain_brush.paint_layer;
                Some(TerrainInspectorState {
                    paint_layer: paint as f32,
                    tile: tile(paint),
                    relief: t.parallax_scale,
                    wetness: t.wetness,
                    debug_view: self.terrain_debug_view,
                    macro_strength: t.macro_strength,
                    brush: match self.terrain_brush.mode {
                        BrushMode::Raise => 0,
                        BrushMode::Lower => 1,
                        BrushMode::Smooth => 2,
                        BrushMode::Flatten => 3,
                        BrushMode::Noise => 4,
                        BrushMode::Paint => 5,
                    },
                    terrain_edit: self.terrain_edit_active,
                    terrain_paint: self.terrain_edit_active
                        && self.terrain_brush.mode == BrushMode::Paint,
                    foliage_paint: self.foliage_paint_active,
                    hex_tiling: t.hex_tiling,
                    parallax: t.parallax_scale > 0.0,
                    clipmap: r
                        .clipmaps
                        .get(tc.terrain_id as usize)
                        .map(|c| c.enabled)
                        .unwrap_or(false),
                    lod_morph: t.lod_morph,
                    morph_start: t.lod_morph_start,
                })
            });
            let brush = self.foliage_brush;
            let paint_on = self.foliage_paint_active;
            let erase_on = self.foliage_erase;
            let single_on = brush.single;
            let sel_foliage = self
                .selected_entity
                .and_then(|e| self.world.get::<FoliageComponent>(e).copied())
                .map(|f| {
                    (
                        [
                            brush.density,
                            brush.radius,
                            brush.max_slope_deg,
                            f32::from(brush.kind),
                            brush.scale_min,
                            brush.scale_max,
                            f.foliage_shadow_distance,
                            f.cull_distance,
                            f.lod_distance,
                            f.impostor_distance,
                        ],
                        [f.enabled, paint_on, erase_on, single_on],
                    )
                });
            let sel_water = self
                .selected_entity
                .and_then(|entity| self.world.get::<WaterComponent>(entity).copied())
                .map(|water| {
                    [
                        water.surface_level,
                        water.max_depth,
                        water.clarity,
                        water.amplitude,
                        water.roughness,
                        water.ssr_strength,
                        water.rt_reflect_strength,
                        water.reflect_debug,
                        water.wave_length_a,
                        water.wave_length_b,
                        water.wave_speed,
                        water.wave_steepness,
                        water.wind_speed,
                        water.foam_decay,
                        water.foam_threshold,
                        water.spectrum_blend,
                        water.edge_scale,
                        water.anisotropy,
                        water.caustic_strength,
                    ]
                });
            let sel_vessel = self
                .selected_entity
                .and_then(|entity| self.world.get::<BuoyantVessel>(entity).copied())
                .map(|vessel| {
                    [
                        vessel.buoyancy_per_sample,
                        vessel.linear_drag,
                        vessel.angular_drag,
                        vessel.propulsion_force,
                        vessel.draft,
                        vessel.righting,
                    ]
                });

            // Phase 13E: light properties for the inspector (angles in degrees).
            let sel_light = self
                .selected_entity
                .and_then(|e| self.world.get::<LightComponent>(e).copied())
                .map(|lc| LightInspectorState {
                    values: [
                        lc.intensity,
                        lc.range,
                        lc.inner_angle.to_degrees(),
                        lc.outer_angle.to_degrees(),
                        lc.tint().x,
                        lc.tint().y,
                        lc.tint().z,
                        lc.moon_intensity,
                        lc.source_radius,
                        lc.area_width,
                        lc.area_height,
                    ],
                    kelvin: lc.color_temperature_k,
                    directional: lc.light_type == LightType::Directional,
                    show_cone: lc.light_type == LightType::Spot,
                    show_width: matches!(lc.light_type, LightType::Rect | LightType::Tube),
                    show_height: lc.light_type == LightType::Rect,
                });
            // Phase 16-D: the Scripts section, built from what each
            // attached script declared. Computed before the `ui` borrow
            // because it reads the world and the script host.
            let sel_scripts = self.script_inspector_state();
            if let Some(ui) = &mut self.ui_manager {
                ui.update_outliner_tree(&tree, selected_idx);
                ui.set_fps(self.time.fps());
                // Phase 26-Zeta: the status bar is an instrument panel, so it
                // gets the same per-frame facts the Outliner does.
                ui.set_status_stats(tree.len(), self.time.fps());
                ui.set_status_selection(
                    selected_idx
                        .and_then(|idx| tree.iter().find(|(id, ..)| *id == idx))
                        .map(|(_, name, ..)| name.as_str()),
                );
                if let Some(t) = sel_t {
                    let (rx, ry, rz) = t.rotation.to_euler(glam::EulerRot::XYZ);
                    ui.update_inspector(
                        selected_idx,
                        Some(t.translation.to_array()),
                        Some([rx.to_degrees(), ry.to_degrees(), rz.to_degrees()]),
                        Some(t.scale.to_array()),
                    );
                } else {
                    ui.update_inspector(None, None, None, None);
                }
                ui.update_light_inspector(sel_light);
                ui.update_camera_inspector(sel_camera, sel_camera_dynres);
                ui.update_post_inspector(sel_post);
                ui.update_terrain_inspector(sel_terrain);
                ui.update_water_inspector(sel_water);
                ui.update_vessel_inspector(sel_vessel);
                ui.update_foliage_inspector(sel_foliage);
                let water_iris = self.selected_entity.and_then(|entity| {
                    let water = self.world.get::<WaterComponent>(entity)?;
                    Some((
                        water.deep_color,
                        water.shallow_color,
                        water.edge_color,
                        water.absorption,
                        water.scattering,
                        water.underwater_enabled,
                        [
                            water.wave_dir_a[0],
                            water.wave_dir_a[1],
                            water.wave_dir_b[0],
                            water.wave_dir_b[1],
                        ],
                    ))
                });
                ui.update_water_iris(water_iris);
                let particle = self.selected_entity.and_then(|entity| {
                    let p = self.world.get::<ParticleEmitter>(entity)?;
                    Some((p.color_start, p.color_end))
                });
                ui.update_particle_inspector(particle);
                let material = self.selected_entity.and_then(|entity| {
                    let id = self.world.get::<MaterialComponent>(entity)?.id;
                    self.renderer
                        .as_ref()
                        .and_then(|r| r.materials_pool.get(id))
                        .map(|m| m.base_color)
                });
                ui.update_material_inspector(material);
                ui.update_script_inspector(sel_scripts);
                ui.set_scene_dirty(self.scene_dirty);
                // Phase 26-Zeta-G. Must run after the update_* writes above:
                // the first value a field is seen holding becomes its revert
                // baseline, so observing it earlier would baseline an empty
                // inspector.
                ui.refresh_modified_dots();
                ui.refresh_inspector_filter();
            }
        }

        // Phase 29: the overlay is refreshed every frame rather than on
        // selection changes like the inspectors above it — the numbers move
        // whether or not anything was clicked.
        {
            let rows = self
                .renderer
                .as_ref()
                .filter(|r| r.profiler.enabled())
                .map(|r| {
                    let p = &r.profiler;
                    let mut rows: Vec<somnium_ui::ProfilerRow> = Vec::new();
                    rows.push(somnium_ui::ProfilerRow {
                        label: "— GPU —".to_string(),
                        value: String::new(),
                        depth: 0,
                    });
                    rows.extend(p.results().iter().map(|s| somnium_ui::ProfilerRow {
                        label: s.name.to_string(),
                        value: format!("{:.3} ms", s.ms),
                        depth: s.depth.saturating_add(1),
                    }));
                    if p.results().is_empty() {
                        rows.push(somnium_ui::ProfilerRow {
                            label: "collecting…".to_string(),
                            value: String::new(),
                            depth: 1,
                        });
                    }
                    rows.push(somnium_ui::ProfilerRow {
                        label: "unattributed".to_string(),
                        value: format!("{:.3} ms", p.unattributed_ms()),
                        depth: 1,
                    });
                    // Phase 16-F. CPU only, and labelled as such: there
                    // is no GPU side to a script, and a row under "GPU"
                    // that was really wall time would be a lie that took
                    // someone a morning to find.
                    let script = self.scripts.stats();
                    rows.push(somnium_ui::ProfilerRow {
                        label: "— Scripts (CPU) —".to_string(),
                        value: format!("{:.3} ms", script.total_ms()),
                        depth: 0,
                    });
                    for (label, value) in [
                        ("fixed", format!("{:.3} ms", script.fixed_ms)),
                        ("update", format!("{:.3} ms", script.update_ms)),
                        ("sync + teardown", format!("{:.3} ms", script.sync_ms)),
                        ("calls", script.calls.to_string()),
                        ("commands applied", script.commands.to_string()),
                        ("errors", script.errors.to_string()),
                        ("live instances", script.instances.to_string()),
                        ("VM memory", format!("{} KiB", script.vm_bytes / 1024)),
                    ] {
                        rows.push(somnium_ui::ProfilerRow {
                            label: label.to_string(),
                            value,
                            depth: 1,
                        });
                    }
                    rows.push(somnium_ui::ProfilerRow {
                        label: "— Graph —".to_string(),
                        value: String::new(),
                        depth: 0,
                    });
                    let graph = p
                        .results()
                        .iter()
                        .filter(|s| s.depth <= 1)
                        .map(|s| s.name)
                        .collect::<Vec<_>>()
                        .join(" → ");
                    if graph.is_empty() {
                        rows.push(somnium_ui::ProfilerRow {
                            label: "collecting…".to_string(),
                            value: String::new(),
                            depth: 1,
                        });
                    } else {
                        rows.push(somnium_ui::ProfilerRow {
                            label: graph,
                            value: String::new(),
                            depth: 1,
                        });
                    }
                    rows.push(somnium_ui::ProfilerRow {
                        label: "— CPU —".to_string(),
                        value: String::new(),
                        depth: 0,
                    });
                    if p.cpu_results().is_empty() {
                        rows.push(somnium_ui::ProfilerRow {
                            label: "collecting…".to_string(),
                            value: String::new(),
                            depth: 1,
                        });
                    } else {
                        rows.extend(p.cpu_results().iter().map(|s| somnium_ui::ProfilerRow {
                            label: s.name.to_string(),
                            value: format!("{:.3} ms", s.ms),
                            depth: s.depth.saturating_add(1),
                        }));
                    }
                    let c = p.last_counters;
                    let frustum_tag = if SomniumRenderer::cpu_frustum_env_off() {
                        " [forced-off]"
                    } else if !r.cpu_frustum_active() {
                        " [off]"
                    } else {
                        ""
                    };
                    rows.push(somnium_ui::ProfilerRow {
                        label: "draws".to_string(),
                        value: c.draw_calls.to_string(),
                        depth: 0,
                    });
                    rows.push(somnium_ui::ProfilerRow {
                        label: "triangles".to_string(),
                        value: c.triangles.to_string(),
                        depth: 0,
                    });
                    rows.push(somnium_ui::ProfilerRow {
                        label: "terrain chunks".to_string(),
                        value: format!(
                            "{} vis / {} cpu-cull{frustum_tag}",
                            c.terrain_chunks, c.terrain_cpu_culled
                        ),
                        depth: 0,
                    });
                    rows.push(somnium_ui::ProfilerRow {
                        label: "TLAS instances".to_string(),
                        value: c.tlas_instances.to_string(),
                        depth: 0,
                    });
                    rows.push(somnium_ui::ProfilerRow {
                        label: "shadow casters".to_string(),
                        value: format!("{} / {}", c.shadow_casters, c.draw_calls),
                        depth: 0,
                    });
                    rows
                });
            if let Some(ui) = &mut self.ui_manager {
                ui.update_profiler(rows.as_deref());
            }
        }

        if let Some(ui) = &mut self.ui_manager {
            if let Some(window) = &self.window {
                ui.begin_frame(window);
            }
        }

        {
            let mut ctx = EngineContext::new(
                &self.time,
                &self.config,
                &mut self.world,
                self.physics.as_mut().unwrap(),
                self.audio.as_mut().unwrap(),
                self.render_ctx.as_ref(),
                self.renderer.as_mut(),
                &mut self.selected_entity,
                self.ui_manager.as_mut().unwrap(),
                crate::camera_speed_from_normalized(self.camera_speed_norm),
                self.simulation_clock,
                &mut self.scripts,
            );
            self.game.on_render(&mut ctx);

            if ctx.should_exit {
                self.initiate_shutdown(event_loop);
                return;
            }
        }

        // ── Particle simulation (Phase 11.5J) ────────────────────────────────
        {
            let frame = self.time.frame_count();
            let particle_dt = if self.simulation_clock.state != SimulationState::Paused {
                dt
            } else {
                0.0
            };
            let gpu_particles = simulate_particles(&mut self.world, particle_dt, frame);
            if let Some(r) = &mut self.renderer {
                r.set_particles(gpu_particles);
            }
        }

        // ── Terrain editing + submission (Phase 14) ──────────────────────────
        self.apply_terrain_restores();
        if self.play_session_active {
            self.gizmo_drag = None;
            self.terrain_stroke = None;
            self.foliage_painting = false;
            if let Some(r) = &mut self.renderer {
                for terrain in &mut r.terrains {
                    terrain.brush_cursor = [0.0; 4];
                }
            }
        } else {
            self.update_terrain_editing(dt);
        }
        self.submit_terrains();
        self.submit_foliage();
        self.sync_terrain_colliders();

        // ── Light gizmos (Phase 13E) ─────────────────────────────────────────
        if !self.play_session_active {
            self.submit_light_gizmos();
        }

        // ── Post-processing settings (Phase 15A1) ────────────────────────────
        self.apply_post_process();
        self.apply_camera_settings();

        if let (Some(r), Some(c), Some(ui), Some(window)) = (
            &mut self.renderer,
            &self.render_ctx,
            &mut self.ui_manager,
            &self.window,
        ) {
            r.set_editor_overlays_enabled(!self.play_session_active);
            r.time = self.simulation_clock.elapsed_seconds;
            r.render(c, ui, window);
        }

        // ── Drain editor events and apply to ECS ──────────────────────────────
        {
            let mut events: Vec<EditorEvent> = Vec::new();
            if let Some(ui) = &mut self.ui_manager {
                while let Some(ev) = ui.poll_editor_event() {
                    events.push(ev);
                }
            }
            for ev in events {
                self.handle_editor_event(ev);
            }
            if let Some(result) = self.pending_map_load.take() {
                let mut ctx = EngineContext::new(
                    &self.time,
                    &self.config,
                    &mut self.world,
                    self.physics.as_mut().unwrap(),
                    self.audio.as_mut().unwrap(),
                    self.render_ctx.as_ref(),
                    self.renderer.as_mut(),
                    &mut self.selected_entity,
                    self.ui_manager.as_mut().unwrap(),
                    crate::camera_speed_from_normalized(self.camera_speed_norm),
                    self.simulation_clock,
                    &mut self.scripts,
                );
                self.game.on_map_loaded(&mut ctx, &result);
            }
        }
        if self.ui_wants_exit {
            self.initiate_shutdown(event_loop);
            return;
        }

        // ── Forward log entries to the output log panel ───────────────────────
        {
            let mut entries: Vec<String> = Vec::new();
            if let Some(rx) = &self.log_rx {
                while let Ok(entry) = rx.try_recv() {
                    entries.push(format!("[{}] {}", entry.level, entry.message));
                }
            }
            if let Some(ui) = &mut self.ui_manager {
                for entry in &entries {
                    ui.append_log(entry);
                }
            }
        }

        self.time.wait_for_frame_budget();

        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if self.state != LifecycleState::ShuttingDown {
            warn!("Engine exiting without explicit shutdown — calling on_shutdown");
            self.game.on_shutdown();
        }
        self.state = LifecycleState::ShuttingDown;
        info!(
            frames = self.time.frame_count(),
            elapsed = ?self.time.elapsed(),
            "Somnium Engine exiting"
        );
    }
}

impl<G: GameApp> Engine<G> {
    fn initiate_shutdown(&mut self, event_loop: &ActiveEventLoop) {
        if self.state == LifecycleState::ShuttingDown {
            return;
        }
        info!("Initiating engine shutdown");
        self.state = LifecycleState::ShuttingDown;
        self.game.on_shutdown();
        event_loop.exit();
    }

    // ── Phase 14: terrain editing helpers ────────────────────────────────────

    /// The selected entity's terrain component, if it has one.
    fn selected_terrain(&self) -> Option<TerrainComponent> {
        let entity = self.selected_entity?;
        self.world.get::<TerrainComponent>(entity).copied()
    }

    /// Model matrix of the selected terrain entity.
    fn selected_terrain_model(&self) -> Option<glam::Mat4> {
        let entity = self.selected_entity?;
        self.world.get::<Transform>(entity).map(|t| t.to_matrix())
    }

    /// World-space cursor ray (origin, direction) from the camera through
    /// the current cursor position.
    fn cursor_ray(&self) -> Option<(glam::Vec3, glam::Vec3)> {
        let r = self.renderer.as_ref()?;
        let inv_vp = r.view_proj.inverse();
        let world_pt = ndc_to_world(
            self.cursor_pos.0,
            self.cursor_pos.1,
            self.viewport_size.0,
            self.viewport_size.1,
            &inv_vp,
        );
        let dir = (world_pt - r.camera_pos).normalize_or_zero();
        (dir != glam::Vec3::ZERO).then_some((r.camera_pos, dir))
    }

    /// Select a terrain brush tool by index (0-5 = `BrushMode` order) and
    /// activate terrain edit mode if a terrain is selected.
    fn set_terrain_tool(&mut self, tool: u8) {
        self.terrain_brush.mode = match tool {
            0 => BrushMode::Raise,
            1 => BrushMode::Lower,
            2 => BrushMode::Smooth,
            3 => BrushMode::Flatten,
            4 => BrushMode::Noise,
            _ => BrushMode::Paint,
        };
        if self.selected_terrain().is_some() {
            self.terrain_edit_active = true;
        }
        self.foliage_paint_active = false;
        info!("Terrain tool: {}", self.terrain_brush.mode.label());
    }

    /// Begin a brush stroke under the cursor. Returns true if a stroke started.
    fn begin_terrain_stroke(&mut self) -> bool {
        let Some(tc) = self.selected_terrain() else {
            return false;
        };
        let Some(model) = self.selected_terrain_model() else {
            return false;
        };
        let Some((origin, dir)) = self.cursor_ray() else {
            return false;
        };
        let Some(renderer) = self.renderer.as_mut() else {
            return false;
        };
        let Some(terrain) = renderer.terrain_mut(tc.terrain_id) else {
            return false;
        };

        terrain.model = model; // keep raycast in sync with the entity transform
        let Some(hit) = terrain.raycast(origin, dir) else {
            return false;
        };

        let is_paint = self.terrain_brush.mode == BrushMode::Paint;
        // Flatten levels toward the raw height under the initial hit point.
        if self.terrain_brush.mode == BrushMode::Flatten {
            let local = model.inverse().transform_point3(hit);
            self.terrain_brush.target_height =
                terrain.world_height_at(local.x, local.z) / terrain.desc.height_scale.max(0.001);
        }

        self.terrain_stroke = Some(TerrainStroke {
            terrain_id: tc.terrain_id,
            is_paint,
            start_heights: if is_paint {
                Vec::new()
            } else {
                terrain.heightmap.clone()
            },
            start_texels: if is_paint {
                terrain.splatmap.data.clone()
            } else {
                Vec::new()
            },
            region: None,
        });
        true
    }

    /// Apply the active stroke each frame and update the brush cursor uniform.
    fn update_terrain_editing(&mut self, dt: f32) {
        // Phase 17F: the foliage brush gets the same ring. Painting blind was
        // the single worst thing about the first cut of it — you could not see
        // where a stroke would land until after it landed.
        if self.foliage_paint_active {
            let Some(tc) = self.selected_terrain() else {
                return;
            };
            let Some(model) = self.selected_terrain_model() else {
                return;
            };
            let ray = self.cursor_ray();
            let radius = self.foliage_brush.radius;
            let Some(renderer) = self.renderer.as_mut() else {
                return;
            };
            renderer.clear_gizmo();
            let Some(terrain) = renderer.terrain_mut(tc.terrain_id) else {
                return;
            };
            terrain.model = model;
            match ray.and_then(|(o, d)| terrain.raycast(o, d)) {
                Some(hit) => terrain.brush_cursor = [hit.x, hit.z, radius, 3.0],
                None => terrain.brush_cursor = [0.0; 4],
            }
            return;
        }

        if !self.terrain_edit_active {
            // Make sure no stale cursor ring stays visible.
            if let (Some(tc), Some(r)) = (self.selected_terrain(), self.renderer.as_mut()) {
                if let Some(t) = r.terrain_mut(tc.terrain_id) {
                    t.brush_cursor = [0.0; 4];
                }
            }
            return;
        }
        let Some(tc) = self.selected_terrain() else {
            return;
        };
        let Some(model) = self.selected_terrain_model() else {
            return;
        };
        let ray = self.cursor_ray();
        let brush = self.terrain_brush;
        let stroking = self.terrain_stroke.is_some();
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        // Phase 14F-1: regular transform gizmos are hidden in terrain mode.
        renderer.clear_gizmo();
        let Some(terrain) = renderer.terrain_mut(tc.terrain_id) else {
            return;
        };
        terrain.model = model;

        let hit = ray.and_then(|(o, d)| terrain.raycast(o, d));
        let Some(hit) = hit else {
            terrain.brush_cursor = [0.0; 4];
            return;
        };

        // Cursor ring: green for sculpt modes, blue for paint (Phase 14D-3).
        let mode_flag = if brush.mode == BrushMode::Paint {
            2.0
        } else {
            1.0
        };
        terrain.brush_cursor = [hit.x, hit.z, brush.radius, mode_flag];

        if stroking {
            let local = model.inverse().transform_point3(hit);
            let region = if brush.mode == BrushMode::Paint {
                apply_paint(terrain, &brush, local.x, local.z, dt)
            } else {
                apply_sculpt(terrain, &brush, local.x, local.z, dt)
            };
            if let (Some(rg), Some(stroke)) = (region, self.terrain_stroke.as_mut()) {
                stroke.region = Some(match stroke.region {
                    None => rg,
                    Some(acc) => (
                        acc.0.min(rg.0),
                        acc.1.min(rg.1),
                        acc.2.max(rg.2),
                        acc.3.max(rg.3),
                    ),
                });
            }
        }
    }

    /// Finish the active stroke and push an undo command. Returns true if a
    /// stroke was finished.
    fn end_terrain_stroke(&mut self) -> bool {
        let Some(stroke) = self.terrain_stroke.take() else {
            return false;
        };
        let Some(region) = stroke.region else {
            return true;
        };
        let Some(renderer) = self.renderer.as_ref() else {
            return true;
        };
        let Some(terrain) = renderer.terrain(stroke.terrain_id) else {
            return true;
        };

        let (x0, z0, x1, z1) = region;
        let cmd: Box<dyn crate::editor_commands::EditorCommand> = if stroke.is_paint {
            let row_w = terrain.splatmap.width;
            let extract = |data: &[somnium_renderer::terrain::textures::SplatTexel]| -> Vec<somnium_renderer::terrain::textures::SplatTexel> {
                let mut out = Vec::with_capacity(((x1 - x0 + 1) * (z1 - z0 + 1)) as usize);
                for z in z0..=z1 {
                    let start = (z * row_w + x0) as usize;
                    out.extend_from_slice(&data[start..start + (x1 - x0 + 1) as usize]);
                }
                out
            };
            Box::new(TerrainEditCmd::paint(
                stroke.terrain_id,
                region,
                extract(&stroke.start_texels),
                extract(&terrain.splatmap.data),
                self.terrain_restore_queue.clone(),
            ))
        } else {
            let row_w = terrain.desc.total_vertices_x();
            let extract = |data: &[f32]| -> Vec<f32> {
                let mut out = Vec::with_capacity(((x1 - x0 + 1) * (z1 - z0 + 1)) as usize);
                for z in z0..=z1 {
                    let start = (z * row_w + x0) as usize;
                    out.extend_from_slice(&data[start..start + (x1 - x0 + 1) as usize]);
                }
                out
            };
            Box::new(TerrainEditCmd::sculpt(
                stroke.terrain_id,
                region,
                extract(&stroke.start_heights),
                extract(&terrain.heightmap),
                self.terrain_restore_queue.clone(),
            ))
        };
        // Stroke effects are already applied — record for undo only.
        self.undo_stack.push_silent(cmd);
        if stroke.is_paint {
            if let Some(r) = self.renderer.as_mut() {
                if let Some(t) = r.terrain_mut(stroke.terrain_id) {
                    t.invalidate_unique_colour();
                }
            }
        }
        true
    }

    /// Apply queued terrain restores produced by `TerrainEditCmd` undo/redo.
    fn apply_terrain_restores(&mut self) {
        let ops: Vec<TerrainRestoreOp> = match self.terrain_restore_queue.lock() {
            Ok(mut q) => q.drain(..).collect(),
            Err(_) => return,
        };
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        for op in ops {
            match op {
                TerrainRestoreOp::Heights {
                    terrain_id,
                    region,
                    heights,
                } => {
                    let Some(terrain) = renderer.terrain_mut(terrain_id) else {
                        continue;
                    };
                    let (x0, z0, x1, z1) = region;
                    let row_w = terrain.desc.total_vertices_x();
                    let w = (x1 - x0 + 1) as usize;
                    for (i, z) in (z0..=z1).enumerate() {
                        let dst = (z * row_w + x0) as usize;
                        terrain.heightmap[dst..dst + w]
                            .copy_from_slice(&heights[i * w..(i + 1) * w]);
                    }
                    terrain.mark_region_dirty(x0, z0, x1, z1);
                }
                TerrainRestoreOp::Splat {
                    terrain_id,
                    region,
                    texels,
                } => {
                    let Some(terrain) = renderer.terrain_mut(terrain_id) else {
                        continue;
                    };
                    let (x0, z0, x1, z1) = region;
                    let row_w = terrain.splatmap.width;
                    let w = (x1 - x0 + 1) as usize;
                    for (i, z) in (z0..=z1).enumerate() {
                        let dst = (z * row_w + x0) as usize;
                        terrain.splatmap.data[dst..dst + w]
                            .copy_from_slice(&texels[i * w..(i + 1) * w]);
                    }
                    terrain.splatmap.mark_dirty(x0, z0, x1, z1);
                    terrain.invalidate_unique_colour();
                    terrain.edit_revision = terrain.edit_revision.wrapping_add(1);
                }
            }
        }
    }

    /// File > Import Model: pick a glTF/GLB and spawn it into the scene.
    ///
    /// glTF node transforms are relative to the scene origin, so a normally
    /// authored model lands at world (0, 0, 0) while multi-part models keep
    /// their relative layout. One entity is spawned per renderable node, named
    /// from the glTF node so it is identifiable in the outliner.
    fn import_model(&mut self) {
        // Native modal picker. It blocks the event loop for as long as it is
        // open, which is fine for an editor action.
        let Some(path) = rfd::FileDialog::new()
            .set_title("Import 3D Model")
            .add_filter("glTF model", &["glb", "gltf"])
            .pick_file()
        else {
            return; // cancelled
        };
        let path_str = path.to_string_lossy().to_string();

        let Some((renderer, render_ctx)) = self.renderer.as_mut().zip(self.render_ctx.as_ref())
        else {
            warn!("Cannot import before the renderer is ready");
            return;
        };

        let scene = match somnium_asset::load_gltf(&path_str) {
            Ok(scene) => scene,
            Err(e) => {
                warn!("Failed to import {}: {}", path_str, e);
                return;
            }
        };

        let uploaded = renderer.upload_scene(render_ctx, &scene);
        if uploaded.is_empty() {
            warn!("{} contained no renderable meshes", path_str);
            return;
        }

        let count = uploaded.len();
        for node in uploaded {
            let (scale, rotation, translation) = node.transform.to_scale_rotation_translation();
            let name = if node.entity_name.is_empty() {
                Name::new("Imported Mesh")
            } else {
                Name::new(&node.entity_name)
            };
            let entity = self.world.spawn((
                Transform {
                    translation,
                    rotation,
                    scale,
                },
                name,
                WorldTransform::identity(),
                MeshComponent {
                    vertex_offset: node.vertex_offset,
                    index_offset: node.index_offset,
                    index_count: node.index_count,
                },
                MaterialComponent {
                    id: node.material_id,
                },
            ));
            // Select the last node so the import is immediately visible in the
            // inspector and the gizmo lands on it.
            self.selected_entity = Some(entity);
        }

        info!("Imported {} ({} mesh nodes)", path_str, count);
    }

    /// Push the scene's post-processing settings to the renderer (Phase 15A1).
    ///
    /// Driven by the first entity carrying a `PostProcessComponent`. With no
    /// Legacy scenes with none or several are normalized to one before the
    /// selected settings are copied into the renderer.
    fn apply_post_process(&mut self) {
        normalize_post_process_singleton(&mut self.world, &mut self.selected_entity);
        // Prefer the selected Post Processing entity. This makes the details
        // panel authoritative even if an imported legacy scene accidentally
        // contains a duplicate; falling back to the first keeps old scenes
        // working when another entity is selected.
        let settings = self
            .selected_entity
            .and_then(|e| self.world.get::<PostProcessComponent>(e).copied())
            .or_else(|| {
                self.world
                    .entities()
                    .find_map(|e| self.world.get::<PostProcessComponent>(e).copied())
            });
        if let (Some(pp), Some(r)) = (settings, self.renderer.as_mut()) {
            // Phase 24A: exposure is now derived from EV100 rather than being a
            // free multiplier. Auto-exposure overrides it on the GPU from the
            // metered histogram, so this value is what a manual camera would use
            // and the fallback if metering has not produced a reading yet.
            r.exposure = pp.exposure_multiplier();
            r.auto_exposure = pp.auto_exposure;
            // Adaptation is per second, so it needs the real frame time or the
            // eye adjusts at a rate that depends on frame rate.
            r.frame_delta_time = self.time.delta_time().as_secs_f32();
            r.tonemapper = pp.tonemapper.as_index();
            r.exposure_compensation = pp.exposure_compensation;
            r.shading_mode = u32::from(pp.cel_shading)
                | if pp.pcss_enabled { 2 } else { 0 }
                | if pp.contact_shadows_enabled { 4 } else { 0 }
                | if pp.analytic_grad { 8 } else { 0 };
            let path_active = pp.path_tracer && r.raytrace_pass.supported();
            // The path tracer already owns temporal accumulation. Feeding that
            // result through FSR/TAA (and then motion blur) accumulates history
            // a second time and is the source of the reported afterimages.
            // The current experimental wgpu-ffx backend corrupts low-luminance
            // geometry once the directional sun is below the horizon (stars
            // survive while the rest collapses into a broad black band). Keep
            // the authored FSR request, but contain that backend defect with
            // the stable temporal fallback until the embedded FSR shaders are
            // updated. Daylight FSR and real-resolution upscaling remain live.
            let fsr_safe_for_lighting = r.light_direction.y > 0.0;
            r.fsr_pass
                .set_enabled(pp.fsr_enabled && !path_active && fsr_safe_for_lighting);
            r.fsr_pass.sharpness = pp.fsr_sharpness;
            // Use the pass's effective state, not the authored request: on a
            // device without FSR features, pp.fsr_enabled may be true while
            // the pass correctly declined it. TAA/CAS must still be allowed.
            let fsr_active = r.fsr_pass.enabled;
            let fsr_fallback = pp.fsr_enabled && !path_active && !fsr_active;
            r.taa_pass
                .set_enabled((pp.taa_enabled || fsr_fallback) && !fsr_active && !path_active);
            r.gtao_pass.enabled = pp.gtao_enabled && !path_active;
            r.bloom_pass.enabled = pp.bloom_enabled;
            r.bloom_pass.intensity = pp.bloom_intensity;
            r.dof_pass.enabled = pp.dof_enabled && !path_active;
            r.dof_pass.focus_distance = pp.dof_focus_distance;
            r.dof_pass.f_stop = pp.aperture_f_stops;
            r.restir_pass.enabled = pp.restir_enabled && !path_active;
            r.restir_gi_pass.enabled =
                pp.restir_gi_enabled && r.restir_gi_pass.supported() && !path_active;
            r.water_reflection_pass.enabled =
                pp.rt_reflect_enabled && r.water_reflection_pass.supported();
            r.water_reflection_pass.refract_enabled =
                pp.rt_refract_enabled && r.water_reflection_pass.supported();
            r.cas_pass.enabled = pp.cas_enabled && !fsr_active;
            r.cas_pass.sharpness = pp.cas_sharpness;
            r.cas_pass.strength = pp.cas_strength;
            r.motion_blur_pass.enabled = pp.motion_blur_enabled && !path_active;
            r.motion_blur_pass.shutter = pp.motion_blur_shutter;
            r.restir_gi_pass.intensity = pp.restir_gi_intensity;
            r.gtao_pass.radius = pp.gtao_radius;
            r.gtao_pass.intensity = pp.gtao_intensity;
            r.volumetric_pass.enabled = pp.volumetrics_enabled && !path_active;
            r.volumetric_pass.fog.density = pp.fog_density;
            r.volumetric_pass.fog.height_falloff = pp.fog_height_falloff;
            r.volumetric_pass.fog.asymmetry = pp.fog_asymmetry;
            r.volumetric_pass.fog.shafts = pp.light_shafts;
            r.volumetric_pass.fog.shaft_intensity = pp.shaft_intensity;
            {
                use somnium_renderer::pass::lighting_extra::{
                    FLAG_CACHE, FLAG_PATH, FLAG_PROBES, FLAG_SDF, FLAG_SPECULAR,
                };
                let rt = r.raytrace_pass.supported();
                let mut flags = 0u32;
                if path_active {
                    // Path tracing replaces the frame. Baking caches/probes or
                    // tracing the separate specular buffer underneath it only
                    // wastes work and risks cross-mode history contamination.
                    flags = FLAG_PATH;
                } else {
                    if pp.world_cache && rt {
                        flags |= FLAG_CACHE;
                    }
                    if pp.specular_gi && rt {
                        flags |= FLAG_SPECULAR;
                    }
                    if pp.mesh_sdf && !pp.world_cache {
                        flags |= FLAG_SDF;
                    }
                    if pp.probes && rt {
                        flags |= FLAG_PROBES;
                    }
                }
                r.lighting_extra_pass.flags = flags;
                r.lighting_extra_pass.intensity = pp.cache_intensity;
                r.lighting_extra_pass.probe_intensity = pp.probe_intensity;
                r.lighting_extra_pass.cell_size = pp.cache_cell_size;
                r.lighting_extra_pass.spec_rough = pp.spec_roughness;
                r.lighting_extra_pass.path_bounces = pp.path_bounces;
            }
            r.grading = somnium_renderer::pass::postprocess::Grading {
                temperature: pp.temperature,
                tint: pp.tint,
                contrast: pp.contrast,
                saturation: pp.saturation,
                gain: pp.gain,
                lift: pp.lift,
                gamma: pp.gamma,
                grain: pp.grain,
                time: self.time.elapsed().as_secs_f32(),
            };
            r.vignette_strength = pp.effective_vignette();
            r.chromatic_aberration = pp.effective_ca();
            r.fxaa_enabled = pp.fxaa_enabled;
            // Phase 22C: rides along with the sun in the directional-light
            // buffer, so every pass that lights anything picks it up.
            r.set_ibl_intensity(pp.ibl_intensity);
        }
    }

    /// Push CPU frustum and dynamic-resolution settings from the Camera entity
    /// (Phase CR-C, extended by DOOM-F).
    fn apply_camera_settings(&mut self) {
        let settings = self
            .world
            .entities()
            .find_map(|e| self.world.get::<CameraSettingsComponent>(e).copied());
        let Some(cam) = settings else { return };
        if let Some(r) = self.renderer.as_mut() {
            r.set_cpu_frustum(cam.frustum_cull);
        }
        // Separate borrow because the dynamic-resolution setter needs the
        // render context as well — switching the controller off resizes the
        // scene targets back to the base extent there and then.
        if let (Some(r), Some(c)) = (self.renderer.as_mut(), self.render_ctx.as_ref()) {
            r.set_dynamic_resolution(
                c,
                cam.dynamic_resolution,
                cam.dynamic_target_ms,
                cam.dynamic_floor,
            );
        }
    }

    /// Keep a static heightfield collider in step with every terrain
    /// (Phase 17B).
    ///
    /// Rebuilding the shape means rebuilding Jolt's acceleration tree over a
    /// quarter of a million samples, so it is gated on the terrain's edit
    /// revision. Sculpting rebuilds it once the stroke lands, not per frame.
    fn sync_terrain_colliders(&mut self) {
        let terrains: Vec<(u32, glam::Vec3)> = self
            .world
            .entities()
            .filter_map(|e| {
                let tc = self.world.get::<TerrainComponent>(e)?;
                let pos = self
                    .world
                    .get::<Transform>(e)
                    .map_or(glam::Vec3::ZERO, |t| t.translation);
                Some((tc.terrain_id, pos))
            })
            .collect();

        // A terrain that has gone away should not leave a collider behind.
        let live: std::collections::HashSet<u32> = terrains.iter().map(|(id, _)| *id).collect();
        let stale: Vec<u32> = self
            .terrain_colliders
            .keys()
            .copied()
            .filter(|id| !live.contains(id))
            .collect();
        for id in stale {
            if let Some((_, body)) = self.terrain_colliders.remove(&id) {
                if let Some(p) = self.physics.as_mut() {
                    p.destroy_body(body);
                }
            }
        }

        for (terrain_id, position) in terrains {
            let revision = self
                .renderer
                .as_ref()
                .and_then(|r| r.terrain(terrain_id))
                .map_or(0, |t| t.edit_revision);
            if self
                .terrain_colliders
                .get(&terrain_id)
                .is_some_and(|(rev, _)| *rev == revision)
            {
                continue;
            }

            let Some(renderer) = self.renderer.as_ref() else {
                continue;
            };
            let Some(terrain) = renderer.terrain(terrain_id) else {
                continue;
            };
            let (samples, sample_count, scale) = terrain.heightfield();

            // Drop the old body first: two overlapping static surfaces would
            // fight over every contact.
            if let Some((_, old)) = self.terrain_colliders.remove(&terrain_id) {
                if let Some(p) = self.physics.as_mut() {
                    p.destroy_body(old);
                }
            }

            let Some(physics) = self.physics.as_mut() else {
                continue;
            };
            let body = physics.create_body(RigidBodyDescriptor {
                shape: ColliderShape::HeightField {
                    samples,
                    sample_count,
                    scale,
                },
                position,
                motion_type: MotionType::Static,
                object_layer: somnium_physics::layer::LAYER_NON_MOVING,
                ..Default::default()
            });
            self.terrain_colliders.insert(terrain_id, (revision, body));
            info!("Terrain {terrain_id}: collider rebuilt ({sample_count}x{sample_count} samples)",);
        }
    }

    /// Submit every painted foliage instance (Phase 17F).
    ///
    /// Instances are ordinary draw commands, so they inherit the Phase 15
    /// pipeline — indirect draws, frustum, Hi-Z and per-cluster culling —
    /// without foliage needing to know any of it exists.
    fn submit_foliage(&mut self) {
        let camera_ws = self
            .renderer
            .as_ref()
            .map_or(glam::Vec3::ZERO, |r| r.camera_pos);
        let terrains: Vec<(u32, glam::Mat4, f32, f32, f32, f32)> = self
            .world
            .entities()
            .filter_map(|e| {
                let tc = self.world.get::<TerrainComponent>(e)?;
                let fc = self.world.get::<FoliageComponent>(e)?;
                if !fc.enabled {
                    return None;
                }
                let model = self
                    .world
                    .get::<Transform>(e)
                    .map_or(glam::Mat4::IDENTITY, Transform::to_matrix);
                Some((
                    tc.terrain_id,
                    model,
                    fc.cull_distance,
                    fc.foliage_shadow_distance,
                    fc.lod_distance,
                    fc.impostor_distance,
                ))
            })
            .collect();

        if let Some(r) = self.renderer.as_mut() {
            r.profiler.cpu_begin("Foliage");
        }

        for (terrain_id, model, cull_distance, shadow_distance, lod_distance, impostor_distance) in
            terrains
        {
            // Which palette entries this terrain actually uses, so nothing is
            // loaded for a kind that has never been painted.
            let kinds: Vec<u8> = {
                let Some(t) = self.renderer.as_ref().and_then(|r| r.terrain(terrain_id)) else {
                    continue;
                };
                let mut k: Vec<u8> = t.painted_foliage.iter().map(|p| p.kind).collect();
                k.sort_unstable();
                k.dedup();
                k
            };
            for kind in &kinds {
                self.ensure_palette_mesh(*kind);
            }

            let Some(t) = self.renderer.as_ref().and_then(|r| r.terrain(terrain_id)) else {
                continue;
            };
            // Phase 17G: reject distant instances before they become draws.
            let camera_local = model.inverse().transform_point3(camera_ws);
            let cull_sq = if cull_distance > 0.0 {
                cull_distance * cull_distance
            } else {
                f32::MAX
            };
            let shadow_sq = if shadow_distance > 0.0 {
                shadow_distance * shadow_distance
            } else {
                f32::MAX
            };
            self.foliage_batch.clear();
            for inst in &t.painted_foliage {
                let d = inst.position - camera_local;
                // Horizontal distance: flying up should not make ground cover
                // vanish out from under you.
                if d.x * d.x + d.z * d.z > cull_sq {
                    continue;
                }
                let Some(Some(parts)) = self.foliage_meshes.get(inst.kind as usize) else {
                    continue;
                };
                // Terrain-local placement composed with the terrain's own
                // transform, so moving the terrain carries its foliage.
                let placement = glam::Mat4::from_scale_rotation_translation(
                    glam::Vec3::splat(inst.scale),
                    glam::Quat::from_rotation_y(inst.yaw),
                    inst.position,
                );
                // Horizontal, like the draw cull above and for the same
                // reason: climbing a hill should not switch every shadow below
                // you back on.
                let casts = d.x * d.x + d.z * d.z <= shadow_sq;
                let dist = (d.x * d.x + d.z * d.z).sqrt();
                // Three mesh LODs. The old "impostor" was an untextured ground
                // plane rotated toward the camera — a black triangle that FSR
                // then ghosted. Past impostor_distance keep the solid parts
                // (trunk / branches). Past lod_distance drop the leaf/twig
                // cutouts. Index-count is only the fallback when the glTF did
                // not mark any part as foliage.
                let keep_only =
                    impostor_distance > 0.0 && dist > impostor_distance && parts.len() > 1;
                let drop_heavy =
                    !keep_only && lod_distance > 0.0 && dist > lod_distance && parts.len() > 1;
                let has_leaf = parts.iter().any(|p| p.is_leaf);
                let cheapest = parts
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, p)| p.index_count)
                    .map(|(i, _)| i);
                let heaviest = parts
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, p)| p.index_count)
                    .map(|(i, _)| i);
                for (i, part) in parts.iter().enumerate() {
                    if keep_only {
                        if has_leaf {
                            if part.is_leaf {
                                continue;
                            }
                        } else if cheapest != Some(i) {
                            continue;
                        }
                    }
                    if drop_heavy {
                        if has_leaf {
                            if part.is_leaf {
                                continue;
                            }
                        } else if heaviest == Some(i) {
                            continue;
                        }
                    }
                    self.foliage_batch
                        .push((*part, model * placement * part.local, casts));
                }
            }
            self.foliage_batch.sort_by_key(|(part, _, _)| {
                (part.vertex_offset, part.index_offset, part.material_id)
            });

            if let Some(r) = self.renderer.as_mut() {
                for (part, transform, casts_shadow) in self.foliage_batch.drain(..) {
                    r.submit(somnium_renderer::command::DrawCommand {
                        sort_key: somnium_renderer::command::SortKey::new(0, 0, 0),
                        vertex_offset: part.vertex_offset,
                        index_offset: part.index_offset,
                        index_count: part.index_count,
                        material_id: part.material_id,
                        transform,
                        casts_shadow,
                    });
                }
            }
        }
        if let Some(r) = self.renderer.as_mut() {
            r.profiler.cpu_end();
        }
    }

    /// Load and upload one palette entry, the first time it is painted.
    fn ensure_palette_mesh(&mut self, kind: u8) {
        let idx = kind as usize;
        if idx >= FOLIAGE_PALETTE.len()
            || self.foliage_meshes[idx].is_some()
            || self.foliage_failed[idx]
        {
            return;
        }
        let (name, path) = FOLIAGE_PALETTE[idx];
        if !std::path::Path::new(path).exists() {
            warn!("Foliage: {name} is not installed at {path}");
            return;
        }
        let (Some(renderer), Some(ctx)) = (&mut self.renderer, &self.render_ctx) else {
            return;
        };
        let mut scene = match somnium_asset::load_gltf(path) {
            Ok(s) => s,
            Err(e) => {
                warn!("Foliage: could not load {path}: {e}");
                // Remember the failure. Without this every brush dab retries
                // the import, and a model that cannot load turns each stroke
                // into a stall plus a wall of identical warnings.
                self.foliage_failed[idx] = true;
                return;
            }
        };

        // Vegetation is almost always exported as BLEND — Poly Haven's grass
        // and leaves are. Left that way it goes through the sorted forward
        // pass: per-object-sorted draws with no depth write, so blades sort
        // wrongly against each other, cast no shadows, and skip GPU culling.
        // Re-tagging as MASK moves them to the visibility buffer where they
        // belong. These particular models are modelled blades with JPEG
        // textures and carry no alpha, so the cutout never fires — the win is
        // the opaque path. Alpha-carded assets get the clipping for free.
        for m in &mut scene.materials {
            if m.alpha_mode == somnium_asset::AlphaMode::Blend {
                m.alpha_mode = somnium_asset::AlphaMode::Mask;
                if !(m.alpha_cutoff > 0.0 && m.alpha_cutoff < 1.0) {
                    m.alpha_cutoff = 0.5;
                }
                // Palette entries are known vegetation. Re-routing a material
                // to the opaque visibility path must retain the semantic data
                // that activates curved normals, the roughness floor, and
                // two-sided transmission in deferred shading. Limit this to
                // the originally blended pieces so opaque trunks stay solid.
                m.foliage = true;
                m.double_sided = true;
                if m.transmission <= 0.0 {
                    m.transmission = 0.5;
                }
            }
        }

        let uploaded = renderer.upload_scene(ctx, &scene);
        if uploaded.is_empty() {
            return;
        }

        // `upload_scene` returns one entry per *primitive*. A glTF node is
        // often several: a sapling is `branches` plus `twigs`, a tree is trunk,
        // branches and leaves. Taking only the biggest primitive planted trees
        // with no trunk, and for the island tree picked a 714k-triangle leaf
        // mesh that renders as nothing.
        //
        // Meanwhile these models are also *collections* — the grass files hold
        // seventeen separate tufts, and instancing all of them would multiply
        // every dab by seventeen. So: group primitives by node, take the one
        // node with the most geometry, and keep all of its primitives.
        //
        // Primitives of one node share that node's transform exactly, which is
        // what the grouping keys on.
        let mut groups: Vec<(glam::Mat4, Vec<&somnium_renderer::renderer::UploadedNode>)> =
            Vec::new();
        for n in &uploaded {
            match groups.iter_mut().find(|(m, _)| *m == n.transform) {
                Some((_, v)) => v.push(n),
                None => groups.push((n.transform, vec![n])),
            }
        }
        let Some((node_xform, parts)) = groups
            .into_iter()
            .max_by_key(|(_, v)| v.iter().map(|n| u64::from(n.index_count)).sum::<u64>())
        else {
            return;
        };

        // The node's own transform is factored out and reapplied per part, so a
        // model authored away from its origin still plants where you click.
        let inv_node = node_xform.inverse();
        let built: Vec<FoliagePart> = parts
            .iter()
            .map(|n| {
                let is_leaf = scene
                    .nodes
                    .iter()
                    .find(|sn| sn.name == n.entity_name)
                    .and_then(|sn| sn.material_index)
                    .and_then(|i| scene.materials.get(i))
                    .is_some_and(|m| {
                        m.foliage
                            || m.alpha_mode != somnium_asset::AlphaMode::Opaque
                            || m.alpha_cutoff > 0.0
                    });
                FoliagePart {
                    vertex_offset: n.vertex_offset,
                    index_offset: n.index_offset,
                    index_count: n.index_count,
                    material_id: n.material_id,
                    local: inv_node * n.transform,
                    is_leaf,
                }
            })
            .collect();
        let tris: u32 = built.iter().map(|p| p.index_count / 3).sum();
        info!(
            "Foliage: loaded {name} ({} parts, {tris} triangles)",
            built.len()
        );
        self.foliage_meshes[idx] = Some(built);
    }

    /// Apply one dab of the foliage brush under the cursor (Phase 17F).
    ///
    /// Returns true when a terrain was hit, so the caller knows the click was
    /// consumed by painting rather than falling through to selection.
    fn paint_foliage_dab(&mut self) -> bool {
        let Some(tc) = self.selected_terrain() else {
            return false;
        };
        let Some(model) = self.selected_terrain_model() else {
            return false;
        };
        let Some((origin, dir)) = self.cursor_ray() else {
            return false;
        };

        let brush = self.foliage_brush;
        let erase = self.foliage_erase;
        let seed = self.foliage_stroke_seed;
        self.foliage_stroke_seed = seed.wrapping_add(1);

        let Some(renderer) = self.renderer.as_mut() else {
            return false;
        };
        let Some(terrain) = renderer.terrain_mut(tc.terrain_id) else {
            return false;
        };
        terrain.model = model; // keep the raycast in sync with the entity
        let Some(hit) = terrain.raycast(origin, dir) else {
            return false;
        };
        // `raycast` marches in terrain-local space but transforms the result
        // back to world before returning. Painted instances are stored local,
        // because `submit_foliage` composes them with the terrain's transform —
        // so the hit has to come back down. Skipping this applied the terrain
        // offset twice and dropped every stroke a terrain-width away from the
        // cursor.
        let local_hit = model.inverse().transform_point3(hit);
        let center = [local_hit.x, local_hit.z];

        if erase {
            let removed = somnium_renderer::terrain::foliage_paint::erase(
                &mut terrain.painted_foliage,
                center,
                brush.radius,
                Some(brush.kind),
            );
            if removed > 0 {
                info!("Foliage: erased {removed}");
            }
            return true;
        }

        // The brush needs to read ground heights while writing the instance
        // list, and both live on the terrain. Moving the list out for the
        // duration keeps the borrows disjoint without copying the heightmap.
        let mut painted = std::mem::take(&mut terrain.painted_foliage);
        let added = somnium_renderer::terrain::foliage_paint::paint(
            &mut painted,
            &brush,
            center,
            seed,
            |x, z| terrain.ground_sample(x, z),
        );
        terrain.painted_foliage = painted;
        if added > 0 {
            info!(
                "Foliage: painted {added} of {} (total {})",
                FOLIAGE_PALETTE[brush.kind as usize % FOLIAGE_PALETTE.len()].0,
                terrain.painted_foliage.len(),
            );
        }
        true
    }

    /// Queue a light gizmo for every light entity (Phase 13E).
    ///
    /// The selected light draws at full brightness so it stands out while being
    /// positioned; the rest are dimmed. Direction follows the same convention as
    /// shading: the light travels along the entity's forward axis (`-Z`).
    fn submit_light_gizmos(&mut self) {
        use somnium_renderer::pass::light_gizmo::{LightGizmoDesc, LightGizmoKind};

        let selected_idx = self.selected_entity.map(|e| e.index());
        let gizmos: Vec<LightGizmoDesc> = self
            .world
            .entities()
            .filter_map(|e| {
                let light = self.world.get::<LightComponent>(e)?;
                let transform = self.world.get::<Transform>(e)?;
                let kind = match light.light_type {
                    LightType::Directional => LightGizmoKind::Directional,
                    LightType::Point | LightType::Rect | LightType::Disc => LightGizmoKind::Point,
                    LightType::Spot | LightType::Tube => LightGizmoKind::Spot,
                };
                Some(LightGizmoDesc {
                    kind,
                    position: transform.translation,
                    direction: transform.rotation.mul_vec3(glam::Vec3::NEG_Z),
                    color: light.color,
                    range: light.range,
                    inner_angle: light.inner_angle,
                    outer_angle: light.outer_angle,
                    selected: selected_idx == Some(e.index()),
                })
            })
            .collect();

        if let Some(r) = self.renderer.as_mut() {
            for g in gizmos {
                r.submit_light_gizmo(g);
            }
        }
    }

    /// Queue every terrain entity for rendering this frame.
    fn submit_terrains(&mut self) {
        let terrains: Vec<(u32, glam::Mat4)> = self
            .world
            .entities()
            .filter_map(|e| {
                let tc = self.world.get::<TerrainComponent>(e)?;
                let model = self
                    .world
                    .get::<Transform>(e)
                    .map_or(glam::Mat4::IDENTITY, Transform::to_matrix);
                Some((tc.terrain_id, model))
            })
            .collect();
        if let Some(r) = self.renderer.as_mut() {
            for (id, model) in terrains {
                r.submit_terrain(id, model);
            }
        }
    }

    fn apply_inspector_color(
        &mut self,
        field: ColorField,
        rgba: [f32; 4],
        live: bool,
        cancel: bool,
    ) {
        let Some(entity) = self.selected_entity else {
            return;
        };
        match field {
            ColorField::Light => {
                if cancel {
                    if let Some((_, old)) = self.scrub_light.take() {
                        if let Some(l) = self.world.get_mut::<LightComponent>(entity) {
                            *l = old;
                        }
                    }
                    return;
                }
                if let Some(&old_light) = self.world.get::<LightComponent>(entity) {
                    let mut new_light = old_light;
                    new_light.color = glam::Vec3::new(rgba[0], rgba[1], rgba[2]);
                    new_light.color_temperature_k = 0.0;
                    if live {
                        if self.scrub_light.is_none() {
                            self.scrub_light = Some((entity.index(), old_light));
                        }
                        if let Some(l) = self.world.get_mut::<LightComponent>(entity) {
                            *l = new_light;
                        }
                        if cancel {
                            self.scrub_light = None;
                        }
                    } else {
                        let base = self
                            .scrub_light
                            .take()
                            .filter(|(idx, _)| *idx == entity.index())
                            .map(|(_, l)| l)
                            .unwrap_or(old_light);
                        if new_light != base {
                            if let Some(l) = self.world.get_mut::<LightComponent>(entity) {
                                *l = base;
                            }
                            self.undo_stack.push(
                                Box::new(SetLightCmd::new(entity.index(), base, new_light)),
                                &mut self.world,
                                &mut self.selected_entity,
                            );
                            self.scene_dirty = true;
                        }
                    }
                }
            }
            ColorField::WaterDeep => {
                if let Some(w) = self.world.get_mut::<WaterComponent>(entity) {
                    w.deep_color = rgba;
                }
                if !live {
                    self.scene_dirty = true;
                }
            }
            ColorField::WaterShallow => {
                if let Some(w) = self.world.get_mut::<WaterComponent>(entity) {
                    w.shallow_color = rgba;
                }
                if !live {
                    self.scene_dirty = true;
                }
            }
            ColorField::WaterEdge => {
                if let Some(w) = self.world.get_mut::<WaterComponent>(entity) {
                    w.edge_color = rgba;
                }
                if !live {
                    self.scene_dirty = true;
                }
            }
            ColorField::WaterAbsorption => {
                if let Some(w) = self.world.get_mut::<WaterComponent>(entity) {
                    let mag = somnium_ui::color::split_magnitude(w.absorption).1;
                    w.absorption =
                        somnium_ui::color::join_magnitude([rgba[0], rgba[1], rgba[2]], mag);
                }
                if !live {
                    self.scene_dirty = true;
                }
            }
            ColorField::WaterScattering => {
                if let Some(w) = self.world.get_mut::<WaterComponent>(entity) {
                    let mag = somnium_ui::color::split_magnitude(w.scattering).1;
                    w.scattering =
                        somnium_ui::color::join_magnitude([rgba[0], rgba[1], rgba[2]], mag);
                }
                if !live {
                    self.scene_dirty = true;
                }
            }
            ColorField::ParticleStart => {
                if let Some(p) = self.world.get_mut::<ParticleEmitter>(entity) {
                    p.color_start = rgba;
                }
                if !live {
                    self.scene_dirty = true;
                }
            }
            ColorField::ParticleEnd => {
                if let Some(p) = self.world.get_mut::<ParticleEmitter>(entity) {
                    p.color_end = rgba;
                }
                if !live {
                    self.scene_dirty = true;
                }
            }
            ColorField::MaterialBase => {
                if let Some(mat) = self.world.get::<MaterialComponent>(entity).copied() {
                    if let (Some(renderer), Some(ctx)) =
                        (self.renderer.as_mut(), self.render_ctx.as_ref())
                    {
                        if let Some(mut gpu) = renderer.materials_pool.get(mat.id) {
                            gpu.base_color = rgba;
                            renderer
                                .materials_pool
                                .set_material(&ctx.queue, mat.id, gpu);
                        }
                    }
                }
                if !live {
                    self.scene_dirty = true;
                }
            }
        }
    }

    fn handle_editor_event(&mut self, ev: EditorEvent) {
        use somnium_ui::{CreateKind, InspectorField as IF};

        match ev {
            EditorEvent::SelectEntity(opt_idx) => {
                self.selected_entity = opt_idx.and_then(|idx| self.world.find_entity_by_index(idx));
                // A new selection means the old baselines describe values that
                // are no longer on screen.
                if let Some(ui) = &mut self.ui_manager {
                    ui.reset_inspector_baseline();
                }
                // Leaving the terrain entity exits terrain edit mode.
                if self.terrain_edit_active && self.selected_terrain().is_none() {
                    self.terrain_edit_active = false;
                    self.terrain_stroke = None;
                }
                if let Some(entity) = self.selected_entity {
                    let translation = self.world.get::<Transform>(entity).map(|t| t.translation);
                    if let (Some(pos), Some(r)) = (translation, self.renderer.as_mut()) {
                        r.set_gizmo_world_pos(pos);
                    }
                } else if let Some(r) = &mut self.renderer {
                    r.clear_gizmo();
                }
            }

            EditorEvent::CreateEntity(CreateKind::VoxelTerrain) => {
                // The voxel world itself is owned by the game layer, which
                // spins its streaming driver up when it sees this component
                // (and tears it down when the entity is deleted).
                let snapshot = EntitySnapshot {
                    transform: Some(Transform::from_translation(glam::Vec3::ZERO)),
                    name: Some(Name::new("Voxel Terrain")),
                    light: None,
                    mesh: None,
                    mat: None,
                    wt: Some(WorldTransform::identity()),
                    mesh_kind: None,
                    is_particle_emitter: false,
                    terrain: None,
                    voxel_terrain: Some(crate::VoxelTerrainComponent::default()),
                    foliage: None,
                    water: None,
                    parent: None,
                    children: None,
                };
                let cmd = Box::new(CreateEntityCmd::new(snapshot));
                self.undo_stack
                    .push(cmd, &mut self.world, &mut self.selected_entity);
                info!("Created voxel terrain entity");
            }

            EditorEvent::CreateEntity(CreateKind::Terrain) => {
                // Phase 14F-2: terrain is created directly in the engine layer
                // (it needs renderer + render_ctx for GPU resources).
                let Some((renderer, render_ctx)) =
                    self.renderer.as_mut().zip(self.render_ctx.as_ref())
                else {
                    return;
                };
                match crate::create_default_landscape(renderer, render_ctx) {
                    Ok(built) => {
                        let desc = built.preset.terrain;
                        let [wx, wz] = desc.world_size();
                        let cmd = Box::new(CreateLandscapeCmd::new(built.terrain, built.water));
                        self.undo_stack
                            .push(cmd, &mut self.world, &mut self.selected_entity);
                        info!(
                            "Created landscape preset v{} ({}x{} chunks, {:.0}x{:.0} m)",
                            built.preset.version, desc.grid_size[0], desc.grid_size[1], wx, wz,
                        );
                    }
                    Err(error) => warn!("Failed to create default landscape: {error}"),
                }
            }

            EditorEvent::CreateEntity(kind) => {
                let name_str = kind.label();
                let light = match kind {
                    CreateKind::DirectionalLight => Some(LightComponent::directional(
                        crate::light_units::lux::DIRECT_SUNLIGHT,
                    )),
                    CreateKind::PointLight => Some(LightComponent::point(
                        crate::light_units::lumens::BULB_100W,
                        10.0,
                    )),
                    CreateKind::SpotLight => Some(LightComponent::spot(
                        crate::light_units::lumens::FLOODLIGHT,
                        15.0,
                        25.0_f32.to_radians(),
                        35.0_f32.to_radians(),
                    )),
                    CreateKind::RectLight => Some(LightComponent::rect(
                        crate::light_units::lumens::FLOODLIGHT,
                        15.0,
                        0.5,
                        0.25,
                    )),
                    CreateKind::DiscLight => Some(LightComponent::disc(
                        crate::light_units::lumens::FLOODLIGHT,
                        15.0,
                        0.4,
                    )),
                    CreateKind::TubeLight => Some(LightComponent::tube(
                        crate::light_units::lumens::FLOODLIGHT,
                        15.0,
                        0.75,
                        0.04,
                    )),
                    _ => None,
                };

                // Determine mesh_kind for procedural mesh entities.
                let mesh_kind = match kind {
                    CreateKind::Cube => Some(MeshKind::Cube),
                    CreateKind::Sphere => Some(MeshKind::Sphere),
                    CreateKind::Plane => Some(MeshKind::Plane),
                    CreateKind::Cylinder => Some(MeshKind::Cylinder),
                    _ => None,
                };

                // Generate and upload mesh geometry if this is a mesh primitive.
                let (mesh, mat) = if let Some(mk) = mesh_kind {
                    if let (Some(renderer), Some(render_ctx)) =
                        (&mut self.renderer, &self.render_ctx)
                    {
                        // Generate procedural geometry.
                        let (verts, idxs) = match mk {
                            MeshKind::Cube => somnium_asset::generate_cube(1.0),
                            MeshKind::Sphere => somnium_asset::generate_sphere(0.5, 32, 16),
                            MeshKind::Plane => somnium_asset::generate_plane(5.0, 1),
                            MeshKind::Cylinder => somnium_asset::generate_cylinder(0.5, 1.0, 32),
                        };

                        // Create or reuse a default material (flat white, mid-roughness).
                        let mat_id = if let Some(id) = self.default_material_id {
                            id
                        } else {
                            let id = renderer.materials_pool.add_material(
                                &render_ctx.queue,
                                somnium_renderer::material::pool::GpuMaterial {
                                    base_color: [0.8, 0.8, 0.8, 1.0],
                                    roughness: 0.5,
                                    metallic: 0.0,
                                    albedo_map: -1,
                                    normal_map: -1,
                                    metallic_roughness_map: -1,
                                    alpha_cutoff: 0.0,
                                    flags: 0,
                                    occlusion_map: -1,
                                    transmission: 0.0,
                                    emissive: [0.0; 3],
                                    emissive_map: -1,
                                    terrain_index: -1,
                                    _pad: [0.0; 2],
                                },
                            );
                            self.default_material_id = Some(id);
                            id
                        };

                        // Upload geometry to GPU.
                        let alloc =
                            renderer
                                .geometry
                                .upload_mesh(&render_ctx.queue, &verts, &idxs, mat_id);

                        (
                            Some(MeshComponent {
                                vertex_offset: alloc.vertex_offset,
                                index_offset: alloc.index_offset,
                                index_count: alloc.index_count,
                            }),
                            Some(MaterialComponent { id: mat_id }),
                        )
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                };

                let spawn_dist = if light.is_some() { 5.0 } else { 8.0 };
                let (spawn_pos, look, right) =
                    camera_spawn_basis(self.renderer.as_ref(), spawn_dist);
                let rotation = match kind {
                    CreateKind::DiscLight | CreateKind::RectLight | CreateKind::SpotLight => {
                        glam::Quat::from_rotation_arc(glam::Vec3::NEG_Z, look)
                    }
                    CreateKind::TubeLight => {
                        glam::Quat::from_rotation_arc(glam::Vec3::NEG_Z, right)
                    }
                    _ => glam::Quat::IDENTITY,
                };
                let transform = Transform {
                    translation: spawn_pos,
                    rotation,
                    scale: glam::Vec3::ONE,
                };
                let world = WorldTransform(transform.to_matrix());
                let snapshot = EntitySnapshot {
                    transform: Some(transform),
                    name: Some(Name::new(name_str)),
                    light,
                    mesh,
                    mat,
                    wt: Some(world),
                    mesh_kind,
                    is_particle_emitter: kind == CreateKind::Particle,
                    terrain: None,
                    voxel_terrain: None,
                    foliage: None,
                    water: None,
                    parent: None,
                    children: None,
                };
                let cmd = Box::new(CreateEntityCmd::new(snapshot));
                self.undo_stack
                    .push(cmd, &mut self.world, &mut self.selected_entity);
                self.scene_dirty = true;
            }

            EditorEvent::DeleteSelected => {
                if let Some(entity) = self.selected_entity {
                    let cmd = Box::new(DeleteEntityCmd::new(entity.index()));
                    self.undo_stack
                        .push(cmd, &mut self.world, &mut self.selected_entity);
                    if let Some(r) = &mut self.renderer {
                        r.clear_gizmo();
                    }
                }
            }

            EditorEvent::Undo => {
                self.undo_stack
                    .undo(&mut self.world, &mut self.selected_entity);
            }

            EditorEvent::Redo => {
                self.undo_stack
                    .redo(&mut self.world, &mut self.selected_entity);
            }

            EditorEvent::ToggleShadingMode => {
                // Drive the component so the inspector's tick stays truthful;
                // the renderer field is synced from it each frame.
                // Collected first: `entities()` borrows the world immutably and
                // `get_mut` needs it mutably.
                let target = self
                    .world
                    .entities()
                    .find(|e| self.world.get::<PostProcessComponent>(*e).is_some());
                if let Some(e) = target {
                    if let Some(pp) = self.world.get_mut::<PostProcessComponent>(e) {
                        pp.cel_shading = !pp.cel_shading;
                        info!(
                            "Shading mode: {}",
                            if pp.cel_shading { "Cel-Shading" } else { "PBR" },
                        );
                    }
                    return;
                }
                if let Some(r) = &mut self.renderer {
                    r.shading_mode ^= 1;
                    info!(
                        "Shading mode toggled to: {}",
                        if (r.shading_mode & 1) == 1 {
                            "Cel-Shading"
                        } else {
                            "PBR"
                        }
                    );
                }
            }

            EditorEvent::SetInspectorValue { field, value, live } => {
                if !live {
                    self.scene_dirty = true;
                }
                let Some(entity) = self.selected_entity else {
                    return;
                };

                if matches!(
                    field,
                    IF::WaterSurface
                        | IF::WaterMaxDepth
                        | IF::WaterClarity
                        | IF::WaterAmplitude
                        | IF::WaterRoughness
                        | IF::WaterSsrStrength
                        | IF::WaterRtReflect
                        | IF::WaterReflectDebug
                        | IF::WaterWaveLengthA
                        | IF::WaterWaveLengthB
                        | IF::WaterWaveSpeed
                        | IF::WaterWaveSteepness
                        | IF::WaterWindSpeed
                        | IF::WaterFoamDecay
                        | IF::WaterFoamThreshold
                        | IF::WaterSpectrumBlend
                        | IF::WaterEdgeScale
                        | IF::WaterAnisotropy
                        | IF::WaterCausticStrength
                        | IF::WaterWaveDirAX
                        | IF::WaterWaveDirAZ
                        | IF::WaterWaveDirBX
                        | IF::WaterWaveDirBZ
                        | IF::WaterAbsorptionMag
                        | IF::WaterScatteringMag
                ) {
                    if let Some(water) = self.world.get_mut::<WaterComponent>(entity) {
                        match field {
                            IF::WaterSurface => water.surface_level = value,
                            IF::WaterMaxDepth => water.max_depth = value.max(0.01),
                            IF::WaterClarity => water.clarity = value.clamp(0.0, 1.0),
                            IF::WaterAmplitude => water.amplitude = value.max(0.0),
                            IF::WaterRoughness => water.roughness = value.clamp(0.02, 1.0),
                            IF::WaterSsrStrength => water.ssr_strength = value.clamp(0.0, 1.0),
                            IF::WaterRtReflect => water.rt_reflect_strength = value.clamp(0.0, 1.0),
                            IF::WaterReflectDebug => {
                                water.reflect_debug = value.clamp(0.0, 2.0).round()
                            }
                            IF::WaterWaveLengthA => water.wave_length_a = value.max(0.5),
                            IF::WaterWaveLengthB => water.wave_length_b = value.max(0.5),
                            IF::WaterWaveSpeed => water.wave_speed = value.max(0.0),
                            IF::WaterWaveSteepness => water.wave_steepness = value.clamp(0.0, 0.95),
                            IF::WaterWindSpeed => water.wind_speed = value.clamp(0.1, 40.0),
                            IF::WaterFoamDecay => water.foam_decay = value.clamp(0.01, 10.0),
                            IF::WaterFoamThreshold => {
                                water.foam_threshold = value.clamp(0.05, 0.95)
                            }
                            IF::WaterSpectrumBlend => water.spectrum_blend = value.clamp(0.0, 1.0),
                            IF::WaterEdgeScale => water.edge_scale = value.max(0.05),
                            IF::WaterAnisotropy => water.anisotropy = value.clamp(-0.8, 0.8),
                            IF::WaterCausticStrength => {
                                water.caustic_strength = value.clamp(0.0, 4.0)
                            }
                            IF::WaterWaveDirAX => water.wave_dir_a[0] = value,
                            IF::WaterWaveDirAZ => water.wave_dir_a[1] = value,
                            IF::WaterWaveDirBX => water.wave_dir_b[0] = value,
                            IF::WaterWaveDirBZ => water.wave_dir_b[1] = value,
                            IF::WaterAbsorptionMag => {
                                let (tint, _) =
                                    somnium_ui::color::split_magnitude(water.absorption);
                                water.absorption =
                                    somnium_ui::color::join_magnitude(tint, value.max(0.0));
                            }
                            IF::WaterScatteringMag => {
                                let (tint, _) =
                                    somnium_ui::color::split_magnitude(water.scattering);
                                water.scattering =
                                    somnium_ui::color::join_magnitude(tint, value.max(0.0));
                            }
                            _ => unreachable!(),
                        }
                    }
                    if field == IF::WaterSurface {
                        if let Some(transform) = self.world.get_mut::<Transform>(entity) {
                            transform.translation.y = value;
                        }
                    }
                    let _ = live;
                    return;
                }

                if matches!(
                    field,
                    IF::VesselBuoyancy
                        | IF::VesselDrag
                        | IF::VesselAngularDrag
                        | IF::VesselThrust
                        | IF::VesselDraft
                        | IF::VesselRighting
                ) {
                    if let Some(vessel) = self.world.get_mut::<BuoyantVessel>(entity) {
                        match field {
                            IF::VesselBuoyancy => {
                                vessel.buoyancy_per_sample = value.clamp(0.0, 80_000.0)
                            }
                            IF::VesselDrag => vessel.linear_drag = value.clamp(0.0, 20_000.0),
                            IF::VesselAngularDrag => {
                                vessel.angular_drag = value.clamp(0.0, 40_000.0)
                            }
                            IF::VesselThrust => {
                                vessel.propulsion_force = value.clamp(0.0, 40_000.0)
                            }
                            IF::VesselDraft => vessel.draft = value.clamp(0.1, 3.0),
                            IF::VesselRighting => vessel.righting = value.clamp(0.0, 80_000.0),
                            _ => unreachable!(),
                        }
                    }
                    let _ = live;
                    return;
                }

                // Phase DOOM-E: the aerial split distance is renderer state.
                if field == IF::TerrainAerialDistance {
                    if let Some(r) = &mut self.renderer {
                        // Below ~20 m the split lands inside the detail the
                        // player is standing on; above 4 km it is past the far
                        // plane on every map that exists.
                        r.aerial_split = value.clamp(20.0, 4000.0);
                        r.classify_pass.aerial_split = r.aerial_split;
                    }
                    return;
                }

                // Phase DOOM-F: the two dynamic-resolution numbers live on the
                // Camera singleton next to Frustum Cull.
                if matches!(field, IF::CameraDynResTargetMs | IF::CameraDynResFloor) {
                    let mut settings = None;
                    if let Some(cam) = self.world.get_mut::<CameraSettingsComponent>(entity) {
                        match field {
                            // Below ~4 ms the controller would chase a budget
                            // no scale can reach and sit on its floor; above
                            // 100 ms it is not a budget.
                            IF::CameraDynResTargetMs => {
                                cam.dynamic_target_ms = value.clamp(4.0, 100.0);
                            }
                            // Entered as a percentage. 25% is a quarter of the
                            // linear resolution — about 6% of the pixels — and
                            // is already well past where FSR can reconstruct.
                            IF::CameraDynResFloor => {
                                cam.dynamic_floor = (value / 100.0).clamp(0.25, 1.0);
                            }
                            _ => unreachable!(),
                        }
                        settings = Some((
                            cam.dynamic_resolution,
                            cam.dynamic_target_ms,
                            cam.dynamic_floor,
                        ));
                    }
                    if let (Some((on, target, floor)), Some(r), Some(c)) =
                        (settings, &mut self.renderer, &self.render_ctx)
                    {
                        r.set_dynamic_resolution(c, on, target, floor);
                    }
                    return;
                }

                // Phase 15A1: post-processing fields edit PostProcessComponent.
                if matches!(
                    field,
                    IF::PostExposure
                        | IF::PostExposureCompensation
                        | IF::PostTint
                        | IF::PostLift
                        | IF::PostGamma
                        | IF::PostGain
                        | IF::PostAperture
                        | IF::PostShutter
                        | IF::PostIso
                        | IF::PostAoRadius
                        | IF::PostAoIntensity
                        | IF::PostFsrSharpness
                        | IF::PostCasSharpness
                        | IF::PostCasStrength
                        | IF::PostMotionBlurShutter
                        | IF::PostGiIntensity
                        | IF::PostFogDensity
                        | IF::PostFogHeight
                        | IF::PostFogAsymmetry
                        | IF::PostBloomIntensity
                        | IF::PostFocusDistance
                        | IF::PostTemperature
                        | IF::PostContrast
                        | IF::PostSaturation
                        | IF::PostGrain
                        | IF::PostVignetteStrength
                        | IF::PostCaStrength
                        | IF::PostIblIntensity
                        | IF::PostCacheIntensity
                        | IF::PostCacheCell
                        | IF::PostSpecRough
                        | IF::PostPathBounces
                        | IF::PostProbeIntensity
                        | IF::PostShaftIntensity
                ) {
                    if let Some(pp) = self.world.get_mut::<PostProcessComponent>(entity) {
                        match field {
                            IF::PostExposure => pp.ev100 = value,
                            IF::PostExposureCompensation => pp.exposure_compensation = value,
                            // Clamped hard: fog is an exponential, and a
                            // density a couple of orders too high is an opaque
                            // grey screen with no way back except undo.
                            IF::PostFogDensity => pp.fog_density = value.clamp(0.0, 0.05),
                            IF::PostFogHeight => pp.fog_height_falloff = value.max(0.0),
                            IF::PostFogAsymmetry => pp.fog_asymmetry = value.clamp(-0.95, 0.95),
                            IF::PostBloomIntensity => pp.bloom_intensity = value.max(0.0),
                            IF::PostFocusDistance => pp.dof_focus_distance = value.max(0.01),
                            IF::PostTemperature => pp.temperature = value.clamp(-1.0, 1.0),
                            IF::PostTint => pp.tint = value.clamp(-1.0, 1.0),
                            IF::PostLift => pp.lift = value.clamp(-1.0, 1.0),
                            // Gamma is a divisor in the grade, so zero would
                            // blow the midtones out to infinity.
                            IF::PostGamma => pp.gamma = value.clamp(0.05, 4.0),
                            IF::PostGain => pp.gain = value.max(0.0),
                            IF::PostAperture => pp.aperture_f_stops = value.clamp(0.7, 45.0),
                            // The row is the denominator, so 100 means 1/100 s.
                            IF::PostShutter => {
                                pp.shutter_speed_s = 1.0 / value.clamp(1.0, 8000.0);
                            }
                            IF::PostIso => pp.sensitivity_iso = value.clamp(25.0, 25600.0),
                            IF::PostAoRadius => pp.gtao_radius = value.clamp(0.01, 20.0),
                            IF::PostAoIntensity => pp.gtao_intensity = value.clamp(0.0, 4.0),
                            IF::PostFsrSharpness => pp.fsr_sharpness = value.clamp(0.0, 1.0),
                            IF::PostCasSharpness => pp.cas_sharpness = value.clamp(0.0, 1.0),
                            IF::PostCasStrength => pp.cas_strength = value.clamp(0.0, 1.0),
                            IF::PostMotionBlurShutter => {
                                pp.motion_blur_shutter = value.clamp(0.0, 1.0);
                            }
                            IF::PostGiIntensity => {
                                pp.restir_gi_intensity = value.clamp(0.0, 4.0);
                            }
                            IF::PostContrast => pp.contrast = value.max(0.0),
                            IF::PostSaturation => pp.saturation = value.max(0.0),
                            IF::PostGrain => pp.grain = value.max(0.0),
                            IF::PostVignetteStrength => pp.vignette_strength = value.max(0.0),
                            IF::PostCaStrength => pp.ca_strength = value.max(0.0),
                            IF::PostIblIntensity => pp.ibl_intensity = value.max(0.0),
                            IF::PostCacheIntensity => pp.cache_intensity = value.max(0.0),
                            IF::PostCacheCell => pp.cache_cell_size = value.clamp(0.25, 32.0),
                            IF::PostSpecRough => pp.spec_roughness = value.clamp(0.0, 1.0),
                            IF::PostPathBounces => {
                                pp.path_bounces = value.round().clamp(1.0, 8.0) as u32;
                            }
                            IF::PostProbeIntensity => pp.probe_intensity = value.max(0.0),
                            IF::PostShaftIntensity => pp.shaft_intensity = value.max(0.0),
                            _ => unreachable!(),
                        }
                    }
                    return;
                }

                // Phase 17C: terrain layer fields reach into renderer-side
                // TerrainData, which lives outside the ECS, so they bypass the
                // undo stack the same way sculpting already does.
                if matches!(
                    field,
                    IF::TerrainPaintLayer
                        | IF::TerrainTile0
                        | IF::TerrainRelief
                        | IF::TerrainWetness
                        | IF::TerrainMacroStrength
                        | IF::TerrainDebugView
                        | IF::TerrainMorphStart
                ) {
                    if field == IF::TerrainPaintLayer {
                        self.terrain_brush.paint_layer = (value.round().max(0.0) as usize).min(
                            somnium_renderer::terrain::textures::TERRAIN_LAYER_COUNT as usize - 1,
                        );
                        return;
                    }
                    // Phase 25H: a terrain-wide multiplier, not a per-layer
                    // value — the layers already author their own relief and
                    // this scales all of them together.
                    if field == IF::TerrainRelief {
                        let Some(tc) = self.world.get::<TerrainComponent>(entity).copied() else {
                            return;
                        };
                        if let Some(t) = self
                            .renderer
                            .as_mut()
                            .and_then(|r| r.terrain_mut(tc.terrain_id))
                        {
                            t.parallax_scale = value.clamp(0.0, 4.0);
                            if t.parallax_scale > 0.0 {
                                t.parallax_held = t.parallax_scale;
                            }
                        }
                        return;
                    }
                    if field == IF::TerrainWetness {
                        let Some(tc) = self.world.get::<TerrainComponent>(entity).copied() else {
                            return;
                        };
                        if let Some(t) = self
                            .renderer
                            .as_mut()
                            .and_then(|r| r.terrain_mut(tc.terrain_id))
                        {
                            t.wetness = value.clamp(0.0, 1.0);
                        }
                        return;
                    }
                    if field == IF::TerrainMacroStrength {
                        let Some(tc) = self.world.get::<TerrainComponent>(entity).copied() else {
                            return;
                        };
                        if let Some(t) = self
                            .renderer
                            .as_mut()
                            .and_then(|r| r.terrain_mut(tc.terrain_id))
                        {
                            t.macro_strength = value.clamp(0.0, 1.0);
                        }
                        return;
                    }
                    if field == IF::TerrainDebugView {
                        self.terrain_debug_view = value.round().clamp(0.0, 34.0);
                        if let Some(r) = self.renderer.as_mut() {
                            r.shading_debug = self.terrain_debug_view;
                        }
                        return;
                    }
                    if field == IF::TerrainMorphStart {
                        let Some(tc) = self.world.get::<TerrainComponent>(entity).copied() else {
                            return;
                        };
                        if let Some(t) = self
                            .renderer
                            .as_mut()
                            .and_then(|r| r.terrain_mut(tc.terrain_id))
                        {
                            t.lod_morph_start = value.clamp(0.0, 1.0);
                        }
                        return;
                    }
                    let Some(tc) = self.world.get::<TerrainComponent>(entity).copied() else {
                        return;
                    };
                    if let Some(t) = self
                        .renderer
                        .as_mut()
                        .and_then(|r| r.terrain_mut(tc.terrain_id))
                    {
                        let slot = self.terrain_brush.paint_layer;
                        if let Some(layer) = t.layers.get_mut(slot) {
                            // A tiling of zero collapses the texture to one
                            // texel stretched over the whole terrain.
                            layer.tiling = value.max(0.01);
                        }
                    }
                    return;
                }

                // Phase 17C: foliage settings. Re-scattering is driven by the
                // cache noticing the component changed, so nothing else to do.
                if matches!(
                    field,
                    IF::FoliageDensity
                        | IF::FoliageSeed
                        | IF::FoliageSlope
                        | IF::FoliageLayer
                        | IF::FoliageScaleMin
                        | IF::FoliageScaleMax
                        | IF::FoliageShadowDistance
                        | IF::FoliageCullDistance
                        | IF::FoliageLodDistance
                        | IF::FoliageImpostorDistance
                ) {
                    // Phase 17F: these edit the brush, not a scatter. Foliage
                    // is painted now, so the settings that matter are the ones
                    // the next stroke will use.
                    let b = &mut self.foliage_brush;
                    match field {
                        IF::FoliageDensity => b.density = value.clamp(0.0, 40.0),
                        IF::FoliageSeed => b.radius = value.clamp(0.25, 200.0),
                        IF::FoliageSlope => b.max_slope_deg = value.clamp(0.0, 90.0),
                        IF::FoliageLayer => {
                            b.kind = (value.round().max(0.0) as usize)
                                .min(FOLIAGE_PALETTE.len() - 1)
                                as u8;
                        }
                        IF::FoliageScaleMin => b.scale_min = value.max(0.01),
                        IF::FoliageScaleMax => b.scale_max = value.max(0.01),
                        // Not a brush setting: this one lives on the component,
                        // because it describes the foliage that already exists
                        // rather than the next stroke.
                        IF::FoliageShadowDistance => {
                            if let Some(e) = self.selected_entity {
                                if let Some(f) = self.world.get_mut::<FoliageComponent>(e) {
                                    f.foliage_shadow_distance = value.clamp(0.0, 2000.0);
                                }
                            }
                        }
                        IF::FoliageCullDistance => {
                            if let Some(e) = self.selected_entity {
                                if let Some(f) = self.world.get_mut::<FoliageComponent>(e) {
                                    f.cull_distance = value.clamp(0.0, 4000.0);
                                }
                            }
                        }
                        IF::FoliageLodDistance => {
                            if let Some(e) = self.selected_entity {
                                if let Some(f) = self.world.get_mut::<FoliageComponent>(e) {
                                    f.lod_distance = value.clamp(0.0, 4000.0);
                                }
                            }
                        }
                        IF::FoliageImpostorDistance => {
                            if let Some(e) = self.selected_entity {
                                if let Some(f) = self.world.get_mut::<FoliageComponent>(e) {
                                    f.impostor_distance = value.clamp(0.0, 4000.0);
                                }
                            }
                        }
                        _ => unreachable!(),
                    }
                    return;
                }

                // Phase 13E: light fields edit LightComponent, not Transform.
                if matches!(
                    field,
                    IF::LightIntensity
                        | IF::LightRange
                        | IF::LightInnerAngle
                        | IF::LightOuterAngle
                        | IF::LightColorR
                        | IF::LightColorG
                        | IF::LightColorB
                        | IF::LightColorTemperature
                        | IF::LightMoonIntensity
                        | IF::LightSourceRadius
                        | IF::LightAreaWidth
                        | IF::LightAreaHeight
                ) {
                    if let Some(&old_light) = self.world.get::<LightComponent>(entity) {
                        let mut new_light = old_light;
                        match field {
                            // Negative intensity/range would break attenuation.
                            IF::LightIntensity => new_light.intensity = value.max(0.0),
                            IF::LightRange => new_light.range = value.max(0.0),
                            // Keep inner <= outer so the spot falloff stays sane.
                            IF::LightInnerAngle => {
                                new_light.inner_angle =
                                    value.to_radians().clamp(0.0, new_light.outer_angle);
                            }
                            IF::LightOuterAngle => {
                                new_light.outer_angle = value
                                    .to_radians()
                                    .clamp(new_light.inner_angle, std::f32::consts::FRAC_PI_2);
                            }
                            // Colour is linear RGB and separate from intensity,
                            // so it is not clamped to 1 — values above white are
                            // how you get a tinted, over-bright key light.
                            IF::LightColorR => new_light.color.x = value.max(0.0),
                            IF::LightColorG => new_light.color.y = value.max(0.0),
                            IF::LightColorB => new_light.color.z = value.max(0.0),
                            IF::LightColorTemperature => {
                                new_light.color_temperature_k = value.max(0.0);
                            }
                            IF::LightMoonIntensity => {
                                new_light.moon_intensity = value.max(0.0);
                            }
                            IF::LightSourceRadius => {
                                new_light.source_radius = value.max(0.0);
                            }
                            IF::LightAreaWidth => {
                                new_light.area_width = value.max(0.05);
                            }
                            IF::LightAreaHeight => {
                                new_light.area_height = value.max(0.05);
                            }
                            _ => unreachable!(),
                        }
                        if live {
                            // Mid-drag: remember where the gesture started, then
                            // write straight through. No undo entry yet.
                            if self.scrub_light.is_none() {
                                self.scrub_light = Some((entity.index(), old_light));
                            }
                            if let Some(l) = self.world.get_mut::<LightComponent>(entity) {
                                *l = new_light;
                            }
                        } else {
                            // End of a gesture (or a typed value). Undo has to
                            // rewind to where the drag began, not to the last
                            // pixel of it.
                            let base = self
                                .scrub_light
                                .take()
                                .filter(|(idx, _)| *idx == entity.index())
                                .map(|(_, l)| l)
                                .unwrap_or(old_light);
                            if new_light != base {
                                if let Some(l) = self.world.get_mut::<LightComponent>(entity) {
                                    *l = base;
                                }
                                let cmd =
                                    Box::new(SetLightCmd::new(entity.index(), base, new_light));
                                self.undo_stack.push(
                                    cmd,
                                    &mut self.world,
                                    &mut self.selected_entity,
                                );
                            }
                        }
                    }
                    return;
                }

                if let Some(&old_t) = self.world.get::<Transform>(entity) {
                    let mut new_t = old_t;
                    let (ex, ey, ez) = old_t.rotation.to_euler(glam::EulerRot::XYZ);
                    match field {
                        IF::PosX => new_t.translation.x = value,
                        IF::PosY => new_t.translation.y = value,
                        IF::PosZ => new_t.translation.z = value,
                        IF::RotX => {
                            new_t.rotation = glam::Quat::from_euler(
                                glam::EulerRot::XYZ,
                                value.to_radians(),
                                ey,
                                ez,
                            )
                        }
                        IF::RotY => {
                            new_t.rotation = glam::Quat::from_euler(
                                glam::EulerRot::XYZ,
                                ex,
                                value.to_radians(),
                                ez,
                            )
                        }
                        IF::RotZ => {
                            new_t.rotation = glam::Quat::from_euler(
                                glam::EulerRot::XYZ,
                                ex,
                                ey,
                                value.to_radians(),
                            )
                        }
                        IF::ScaleX => new_t.scale.x = value,
                        IF::ScaleY => new_t.scale.y = value,
                        IF::ScaleZ => new_t.scale.z = value,
                        _ => unreachable!("light fields handled above"),
                    }
                    if live {
                        if self.scrub_transform.is_none() {
                            self.scrub_transform = Some((entity.index(), old_t));
                        }
                        if let Some(t) = self.world.get_mut::<Transform>(entity) {
                            *t = new_t;
                        }
                    } else {
                        let base = self
                            .scrub_transform
                            .take()
                            .filter(|(idx, _)| *idx == entity.index())
                            .map(|(_, t)| t)
                            .unwrap_or(old_t);
                        if let Some(t) = self.world.get_mut::<Transform>(entity) {
                            *t = base;
                        }
                        let cmd = Box::new(SetTransformCmd::new(entity.index(), base, new_t));
                        self.undo_stack
                            .push(cmd, &mut self.world, &mut self.selected_entity);
                    }
                    // The gizmo otherwise only re-syncs on selection or on its
                    // own drag, so typing a position left it stranded at the
                    // object's old location.
                    if let Some(r) = self.renderer.as_mut() {
                        r.set_gizmo_world_pos(new_t.translation);
                    }
                }
            }

            EditorEvent::SaveScene => {
                let path = "scene.somnium";
                // Phase 16-A: the schema-driven format (`version: 3`).
                // It writes whatever the registry describes, which is how
                // script attachments and their authored properties reach
                // the file at all — the hand-written version-1 walk has no
                // way to express them.
                match crate::scene_schema::save_scene_schema(
                    &mut self.world,
                    &self.type_registry,
                    path,
                ) {
                    Ok(()) => {
                        info!("Scene saved to {}", path);
                        self.scene_dirty = false;
                        if let Some(ui) = &mut self.ui_manager {
                            ui.push_toast("Scene saved");
                            // Saving is what "modified" was measured against.
                            ui.reset_inspector_baseline();
                        }
                    }
                    Err(e) => {
                        warn!("Failed to save scene: {}", e);
                        if let Some(ui) = &mut self.ui_manager {
                            ui.push_toast("Save failed");
                        }
                    }
                }
                // Phase 14F-3: heightmap + splatmap sidecars, one per terrain.
                if let Some(r) = &self.renderer {
                    let terrain_ids: Vec<u32> = self
                        .world
                        .entities()
                        .filter_map(|e| {
                            self.world
                                .get::<TerrainComponent>(e)
                                .map(|tc| tc.terrain_id)
                        })
                        .collect();
                    for id in terrain_ids {
                        if let Some(t) = r.terrain(id) {
                            let sidecar = format!("{path}.terrain{id}.bin");
                            match t.save_binary(&sidecar) {
                                Ok(()) => info!("Terrain {} data saved to {}", id, sidecar),
                                Err(e) => warn!("Failed to save terrain {}: {}", id, e),
                            }
                        }
                    }
                }
            }

            EditorEvent::NewScene => {
                info!("Creating new scene");
                // Clear the world
                let all_entities: Vec<somnium_ecs::Entity> = self.world.entities().collect();
                for e in all_entities {
                    self.world.despawn(e);
                }
                self.selected_entity = None;
                if let Some(ui) = &mut self.ui_manager {
                    ui.reset_inspector_baseline();
                }
                if let Some(r) = &mut self.renderer {
                    r.clear_gizmo();
                }
                // Spawn a default directional light
                let light_rot = glam::Quat::from_euler(
                    glam::EulerRot::YXZ,
                    (-30.0_f32).to_radians(),
                    (-35.0_f32).to_radians(),
                    0.0,
                );
                self.world.spawn((
                    Transform {
                        translation: glam::Vec3::ZERO,
                        rotation: light_rot,
                        scale: glam::Vec3::ONE,
                    },
                    LightComponent::directional(crate::light_units::lux::DIRECT_SUNLIGHT),
                    Name::new("SunLight"),
                    WorldTransform::identity(),
                ));
                self.world.spawn((
                    Transform {
                        translation: glam::Vec3::new(0.0, 2.0, 8.0),
                        rotation: look_rotation_neg_z(glam::Vec3::NEG_Z),
                        scale: glam::Vec3::ONE,
                    },
                    Name::new("Camera"),
                    WorldTransform::identity(),
                    CameraSettingsComponent::from_env(),
                ));
                self.world.spawn((
                    Transform::from_translation(glam::Vec3::ZERO),
                    Name::new("Post Processing"),
                    WorldTransform::identity(),
                    PostProcessComponent::default(),
                ));
                self.undo_stack = UndoStack::new(128);
                self.scene_dirty = false;
            }

            EditorEvent::DuplicateSelected => {
                if let Some(entity) = self.selected_entity {
                    let transform = self
                        .world
                        .get::<Transform>(entity)
                        .copied()
                        .unwrap_or_else(|| Transform::from_translation(glam::Vec3::ZERO));
                    let name = self
                        .world
                        .get::<Name>(entity)
                        .map(|n| Name::new(&format!("{}_copy", n.as_str())))
                        .unwrap_or_else(|| Name::new("Entity_copy"));
                    let light = self.world.get::<LightComponent>(entity).copied();
                    let mesh = self.world.get::<MeshComponent>(entity).copied();
                    let mat = self.world.get::<MaterialComponent>(entity).copied();
                    let mesh_kind = self.world.get::<MeshKind>(entity).copied();
                    let mut water = self.world.get::<WaterComponent>(entity).copied();
                    if let Some(component) = water.as_mut() {
                        if let (Some(renderer), Some(render_ctx)) =
                            (&mut self.renderer, &self.render_ctx)
                        {
                            component.water_id = renderer.allocate_water_body_id();
                            if let Err(error) =
                                renderer.ensure_water_body(render_ctx, component.descriptor())
                            {
                                warn!("Failed to duplicate water data: {error}");
                            }
                        }
                    }
                    let parent = self.world.get::<Parent>(entity).copied();
                    let is_particle_emitter =
                        self.world.get::<crate::ParticleEmitter>(entity).is_some();
                    // Offset the duplicate slightly so it's visible
                    let mut dup_transform = transform;
                    dup_transform.translation += glam::Vec3::new(1.0, 0.0, 0.0);
                    let snapshot = EntitySnapshot {
                        transform: Some(dup_transform),
                        name: Some(name),
                        light,
                        mesh,
                        mat,
                        wt: Some(WorldTransform::identity()),
                        mesh_kind,
                        is_particle_emitter,
                        // Terrains are not duplicated — two entities sharing
                        // one terrain_id would draw the same terrain twice.
                        terrain: None,
                        voxel_terrain: None,
                        foliage: None,
                        water,
                        parent,
                        children: None,
                    };
                    let cmd = Box::new(CreateEntityCmd::new(snapshot));
                    self.undo_stack
                        .push(cmd, &mut self.world, &mut self.selected_entity);
                    info!("Duplicated entity {}", entity.index());
                }
            }

            EditorEvent::LoadScene(path) => {
                let Some((renderer, render_ctx)) =
                    self.renderer.as_mut().zip(self.render_ctx.as_ref())
                else {
                    return;
                };
                for (_, (_, body)) in self.terrain_colliders.drain() {
                    if let Some(p) = self.physics.as_mut() {
                        p.destroy_body(body);
                    }
                }
                match crate::load_map(&mut self.world, renderer, render_ctx, &path) {
                    Ok(result) => {
                        info!("Loaded map {path} ({:?})", result.kind);
                        self.selected_entity = None;
                        self.undo_stack = UndoStack::new(128);
                        self.scene_dirty = false;
                        self.terrain_edit_active = false;
                        self.terrain_stroke = None;
                        self.pending_map_load = Some(result);
                    }
                    Err(error) => warn!("LoadScene failed: {error}"),
                }
            }

            EditorEvent::SetTerrainTool(tool) => {
                self.set_terrain_tool(tool);
            }

            EditorEvent::SetTerrainPaintLayer(layer) => {
                self.terrain_brush.paint_layer = (layer as usize)
                    .min(somnium_renderer::terrain::textures::TERRAIN_LAYER_COUNT as usize - 1);
                self.set_terrain_tool(5);
            }

            EditorEvent::ToggleTerrainPaint => {
                let already = self.terrain_edit_active
                    && self.terrain_brush.mode == BrushMode::Paint
                    && !self.foliage_paint_active;
                if already {
                    self.terrain_edit_active = false;
                    info!("Terrain paint: off");
                } else {
                    self.set_terrain_tool(5);
                    info!("Terrain paint: ON");
                }
            }

            EditorEvent::ToggleTerrainHex => {
                if let Some(tc) = self.selected_terrain() {
                    if let Some(r) = &mut self.renderer {
                        if let Some(t) = r.terrain_mut(tc.terrain_id) {
                            t.hex_tiling = !t.hex_tiling;
                            info!(
                                "Terrain hex tiling: {}",
                                if t.hex_tiling { "on" } else { "off" }
                            );
                        }
                    }
                }
            }

            EditorEvent::ToggleTerrainParallax => {
                if let Some(tc) = self.selected_terrain() {
                    if let Some(r) = &mut self.renderer {
                        if let Some(t) = r.terrain_mut(tc.terrain_id) {
                            t.toggle_parallax();
                            info!(
                                "Terrain parallax: {}",
                                if t.parallax_scale > 0.0 { "on" } else { "off" }
                            );
                        }
                    }
                }
            }

            EditorEvent::ToggleTerrainClipmap => {
                if let Some(tc) = self.selected_terrain() {
                    if somnium_renderer::terrain::clipmap::TerrainClipmap::env_forced_off() {
                        info!("Terrain clipmap: forced off (SOMNIUM_TERRAIN_CLIPMAP=0)");
                    } else if let Some(r) = &mut self.renderer {
                        if let Some(cm) = r.clipmaps.get_mut(tc.terrain_id as usize) {
                            cm.enabled = !cm.enabled;
                            if cm.enabled {
                                cm.invalidate();
                            }
                            info!("Terrain clipmap: {}", if cm.enabled { "on" } else { "off" });
                        }
                    }
                }
            }

            EditorEvent::SetCpuFrustum(on) => {
                if SomniumRenderer::cpu_frustum_env_off() {
                    info!("CPU frustum cull: forced off (SOMNIUM_CPU_FRUSTUM=0)");
                    if let Some(r) = &mut self.renderer {
                        r.set_cpu_frustum(false);
                    }
                } else {
                    let target = self
                        .world
                        .entities()
                        .find(|e| self.world.get::<CameraSettingsComponent>(*e).is_some());
                    if let Some(e) = target {
                        if let Some(cam) = self.world.get_mut::<CameraSettingsComponent>(e) {
                            cam.frustum_cull = on;
                        }
                    }
                    if let Some(r) = &mut self.renderer {
                        r.set_cpu_frustum(on);
                    }
                    info!("CPU frustum cull: {}", if on { "on" } else { "off" });
                }
            }

            // Phase DOOM-F. The component is the source of truth so the setting
            // survives a scene save; the renderer is told the same frame so the
            // checkbox has a visible effect rather than waiting for a resize.
            EditorEvent::SetDynamicResolution(on) => {
                let target = self
                    .world
                    .entities()
                    .find(|e| self.world.get::<CameraSettingsComponent>(*e).is_some());
                let mut settings = (on, 1000.0 / 60.0, 0.67);
                if let Some(e) = target
                    && let Some(cam) = self.world.get_mut::<CameraSettingsComponent>(e)
                {
                    cam.dynamic_resolution = on;
                    settings = (on, cam.dynamic_target_ms, cam.dynamic_floor);
                }
                if let (Some(r), Some(c)) = (&mut self.renderer, &self.render_ctx) {
                    r.set_dynamic_resolution(c, settings.0, settings.1, settings.2);
                }
                info!(
                    "Dynamic resolution: {} (target {:.2} ms, floor {:.0}%)",
                    if on { "on" } else { "off" },
                    settings.1,
                    settings.2 * 100.0
                );
            }

            // Phase DOOM-E/B/C. All four are renderer-side switches with no
            // scene state behind them, so unlike Dynamic Resolution there is no
            // component to keep in step — the renderer is the source of truth.
            EditorEvent::SetTerrainAerial(on) => {
                if let Some(r) = &mut self.renderer {
                    r.aerial_split_enabled = on;
                    info!(
                        "Aerial terrain LOD: {} (past {:.0} m)",
                        if on { "on" } else { "off" },
                        r.aerial_split
                    );
                }
            }
            EditorEvent::SetTerrainAerialHeroBank(on) => {
                if let Some(r) = &mut self.renderer {
                    r.aerial_hero_bank = on;
                    info!(
                        "Aerial terrain layer scan: {}",
                        if on { "16 (hero bank)" } else { "full" }
                    );
                }
            }
            EditorEvent::SetPixelCensus(on) => {
                if let Some(r) = &mut self.renderer {
                    r.census_pass.enabled = on;
                    info!("Pixel census: {}", if on { "on" } else { "off" });
                }
            }
            EditorEvent::SetShadeBins(on) => {
                if let Some(r) = &mut self.renderer {
                    r.classify_pass.enabled = on;
                    info!("Tile-binned shading: {}", if on { "on" } else { "off" });
                }
            }

            EditorEvent::ToggleTerrainMorph => {
                if let Some(tc) = self.selected_terrain() {
                    if let Some(r) = &mut self.renderer {
                        if let Some(t) = r.terrain_mut(tc.terrain_id) {
                            t.lod_morph = !t.lod_morph;
                            info!(
                                "Terrain LOD morph: {}",
                                if t.lod_morph { "on" } else { "off" }
                            );
                        }
                    }
                }
            }

            EditorEvent::SetCameraSpeed(normalized) => {
                self.camera_speed_norm = normalized.clamp(0.0, 1.0);
                let speed = crate::camera_speed_from_normalized(self.camera_speed_norm);
                if let Some(ui) = &mut self.ui_manager {
                    ui.update_camera_speed(self.camera_speed_norm, speed);
                }
            }

            EditorEvent::SetViewportResolution(idx) => {
                self.viewport_resolution = idx as usize;
                let w = self.viewport_size.0.max(1.0) as u32;
                let h = self.viewport_size.1.max(1.0) as u32;
                let (sw, sh) =
                    somnium_renderer::scene_size_for_preset(w, h, self.viewport_resolution);
                if let (Some(r), Some(c)) = (&mut self.renderer, &self.render_ctx)
                    && r.scene_extent() != (sw, sh)
                {
                    r.resize(c, sw, sh);
                }
                let label = somnium_renderer::VIEWPORT_RESOLUTION_LABELS
                    .get(self.viewport_resolution)
                    .copied()
                    .unwrap_or("Native");
                info!("Viewport 3D {label} ({sw}×{sh})");
            }

            // ── Phase 16-D: scripting ────────────────────────────────────
            EditorEvent::AttachScript(path) => self.attach_script(&std::path::PathBuf::from(path)),

            EditorEvent::CreateScript => self.create_script(),

            EditorEvent::DetachScript(index) => {
                self.push_script_command(|entity| {
                    Box::new(crate::editor_commands::DetachScriptCmd::new(entity, index))
                });
            }

            EditorEvent::ReorderScript { index, delta } => {
                let Some(count) = self.selected_script_count() else {
                    return;
                };
                let target = i64::try_from(index).unwrap_or(0) + i64::from(delta);
                if target < 0 || target >= i64::try_from(count).unwrap_or(0) {
                    // Already at the end of the list. Silently doing
                    // nothing is right here — the arrow is visible on every
                    // row and clamping is what a list is expected to do.
                    return;
                }
                let to = usize::try_from(target).unwrap_or(0);
                self.push_script_command(|entity| {
                    Box::new(crate::editor_commands::ReorderScriptCmd::new(
                        entity, index, to,
                    ))
                });
            }

            EditorEvent::SetScriptEnabled { index, enabled } => {
                self.push_script_command(|entity| {
                    Box::new(crate::editor_commands::SetScriptEnabledCmd::new(
                        entity, index, enabled,
                    ))
                });
            }

            EditorEvent::SetScriptNumber {
                index,
                field,
                value,
                live,
            } => {
                self.set_script_property(
                    index,
                    field,
                    somnium_script::value::ScriptValue::F64(f64::from(value)),
                    live,
                );
            }

            EditorEvent::SetScriptBool {
                index,
                field,
                value,
            } => {
                self.set_script_property(
                    index,
                    field,
                    somnium_script::value::ScriptValue::Bool(value),
                    false,
                );
            }

            EditorEvent::CreateContentFolder { parent, name } => {
                let Some(path) = self.content_target(&parent, &name, None) else {
                    return;
                };
                match std::fs::create_dir(&path) {
                    Ok(()) => {
                        info!("Created {}", path.display());
                        self.after_content_change(&format!(
                            "Created folder {}",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ));
                    }
                    Err(error) => self.report_content_error(&path, &error.to_string()),
                }
            }

            EditorEvent::CreateContentScript { parent, name } => {
                let Some(path) = self.content_target(&parent, &name, Some("luau")) else {
                    return;
                };
                match std::fs::write(&path, crate::script_host::NEW_SCRIPT_TEMPLATE) {
                    Ok(()) => {
                        info!("Created {}", path.display());
                        // Import it straight away and attach it if
                        // something is selected. Creating a script and
                        // then having to find it in the drawer to attach
                        // it is two steps where one will do.
                        self.attach_script(&path);
                        self.after_content_change(&format!(
                            "Created {}",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ));
                    }
                    Err(error) => self.report_content_error(&path, &error.to_string()),
                }
            }

            EditorEvent::RenameContentItem { path, name } => {
                let from = std::path::PathBuf::from(path);
                let leaf = name.trim();
                if leaf.is_empty() || leaf.contains(['/', '\\']) {
                    self.report_content_error(&from, "a name cannot contain a path separator");
                    return;
                }
                // Keep the extension if the author did not retype it, so
                // renaming `Player.luau` to `Character` does not quietly
                // produce a file the importer no longer recognises.
                let mut target = from.with_file_name(leaf);
                if let (Some(old), None) = (from.extension(), target.extension()) {
                    target.set_extension(old);
                }
                if target == from {
                    return;
                }
                if target.exists() {
                    self.report_content_error(&target, "something with that name is already here");
                    return;
                }
                match std::fs::rename(&from, &target) {
                    Ok(()) => {
                        info!("Renamed {} to {}", from.display(), target.display());
                        // A script's asset id is derived from its path, so
                        // a rename makes a *different* asset. Attachments
                        // that named the old one now reference something
                        // that is not there, and say so in the panel —
                        // which is honest, and better than silently
                        // re-pointing them at a file the author may have
                        // meant to fork.
                        if from.extension().is_some_and(|e| e.eq_ignore_ascii_case("luau")) {
                            let _ = self.scripts.import_script_file(&target);
                            self.drain_script_output();
                        }
                        self.after_content_change(&format!(
                            "Renamed to {}",
                            target.file_name().unwrap_or_default().to_string_lossy()
                        ));
                    }
                    Err(error) => self.report_content_error(&from, &error.to_string()),
                }
            }

            EditorEvent::ShowContentItemInFolder(path) => {
                let path = std::path::PathBuf::from(path);
                if let Err(error) = reveal_in_file_browser(&path) {
                    self.report_content_error(&path, &error);
                }
            }

            EditorEvent::ReloadScripts => {
                let (ok, failed) = self.scripts.reload_all_from_disk();
                self.drain_script_output();
                if let Some(ui) = &mut self.ui_manager {
                    if failed == 0 {
                        ui.clear_script_errors();
                        ui.push_toast(&format!("Reloaded {ok} script(s)"));
                    } else {
                        ui.push_toast(&format!(
                            "Reloaded {ok} script(s); {failed} still failing — see the Output Log"
                        ));
                    }
                }
            }

            EditorEvent::PlaySimulation => {
                // Phase 16-D: the authored world is captured on the way in,
                // once per session. Resuming from Pause must not recapture
                // it — that would silently make everything a script has
                // done so far the new "authored" state.
                if !self.play_session_active {
                    self.begin_play_session();
                }
                self.simulation_clock.state = SimulationState::Playing;
                self.play_session_active = true;
                self.gizmo_drag = None;
                self.terrain_stroke = None;
                if let Some(r) = &mut self.renderer {
                    r.set_editor_overlays_enabled(false);
                }
                if let Some(ui) = &mut self.ui_manager {
                    ui.update_simulation_controls(1);
                    ui.set_play_overlays_hidden(true);
                }
                info!("Simulation playing");
            }

            EditorEvent::PauseSimulation => {
                self.simulation_clock.state = SimulationState::Paused;
                if self.play_session_active {
                    if let Some(r) = &mut self.renderer {
                        r.set_editor_overlays_enabled(false);
                    }
                }
                if let Some(ui) = &mut self.ui_manager {
                    ui.update_simulation_controls(2);
                }
                info!("Simulation paused");
            }

            EditorEvent::StopSimulation => {
                self.simulation_clock.state = SimulationState::Editing;
                self.simulation_clock.elapsed_seconds = 0.0;
                self.simulation_accumulator = 0.0;
                // Phase 16-D: tear the scripts down and put the authored
                // world back before anything else observes it, so Stop is
                // exact rather than approximately exact.
                if self.play_session_active {
                    self.end_play_session();
                }
                self.play_session_active = false;
                if let Some(r) = &mut self.renderer {
                    r.set_editor_overlays_enabled(true);
                }
                if let Some(ui) = &mut self.ui_manager {
                    ui.set_immersive(false);
                    ui.update_simulation_controls(0);
                    ui.set_play_overlays_hidden(false);
                }
                info!("Simulation stopped");
            }

            EditorEvent::SetGizmoMode(mode) => {
                if let Some(r) = &mut self.renderer {
                    r.gizmo_mode = match mode {
                        1 => somnium_renderer::pass::gizmo::GizmoMode::Rotate,
                        2 => somnium_renderer::pass::gizmo::GizmoMode::Scale,
                        _ => somnium_renderer::pass::gizmo::GizmoMode::Translate,
                    };
                }
            }

            EditorEvent::ToggleTerrainEdit => {
                if self.selected_terrain().is_some() {
                    self.terrain_edit_active = !self.terrain_edit_active;
                    info!(
                        "Terrain edit mode: {}",
                        if self.terrain_edit_active {
                            "ON"
                        } else {
                            "off"
                        }
                    );
                } else {
                    info!("Select a terrain entity before pressing F6");
                }
            }

            EditorEvent::ToggleImmersiveViewport => {
                let entering = self
                    .ui_manager
                    .as_ref()
                    .is_some_and(|ui| !ui.is_immersive());
                if let Some(ui) = &mut self.ui_manager {
                    ui.set_immersive(entering);
                }
                if entering {
                    self.simulation_clock.state = SimulationState::Playing;
                    self.play_session_active = true;
                    self.gizmo_drag = None;
                    self.terrain_stroke = None;
                    if let Some(r) = &mut self.renderer {
                        r.set_editor_overlays_enabled(false);
                    }
                    if let Some(ui) = &mut self.ui_manager {
                        ui.update_simulation_controls(1);
                        ui.set_play_overlays_hidden(true);
                    }
                    info!("Immersive viewport");
                } else {
                    info!("Immersive viewport exited");
                }
            }

            EditorEvent::ImportModel => {
                self.import_model();
                self.scene_dirty = true;
                if let Some(ui) = &mut self.ui_manager {
                    ui.push_toast("Import finished");
                }
            }

            EditorEvent::ToggleWaterUnderwater => {
                if let Some(entity) = self.selected_entity {
                    if let Some(water) = self.world.get_mut::<WaterComponent>(entity) {
                        water.underwater_enabled = !water.underwater_enabled;
                        self.scene_dirty = true;
                    }
                }
            }

            EditorEvent::SetInspectorColor { field, rgba, live } => {
                self.apply_inspector_color(field, rgba, live, false);
            }

            EditorEvent::CancelInspectorColor { field, rgba } => {
                self.apply_inspector_color(field, rgba, true, true);
            }

            EditorEvent::CloseWindow => {
                self.ui_wants_exit = true;
            }

            // Phase 29. The toggle drives collection as well as visibility: a
            // hidden profiler that keeps writing timestamps is paying for a
            // measurement nobody reads.
            EditorEvent::ToggleProfiler => {
                if let Some(r) = self.renderer.as_mut() {
                    if r.profiler.available() {
                        r.profiler.toggle();
                    } else {
                        tracing::warn!("profiler: GPU timestamps unavailable on this adapter");
                    }
                }
            }

            EditorEvent::ToggleFoliage => {
                if let Some(entity) = self.selected_entity {
                    if let Some(f) = self.world.get_mut::<FoliageComponent>(entity) {
                        f.enabled = !f.enabled;
                    }
                }
            }

            EditorEvent::ToggleFoliagePaint => {
                self.foliage_paint_active = !self.foliage_paint_active;
                // Sculpting and foliage painting both claim the left button.
                if self.foliage_paint_active {
                    self.terrain_edit_active = false;
                }
                info!(
                    "Foliage paint: {}",
                    if self.foliage_paint_active {
                        "ON"
                    } else {
                        "off"
                    }
                );
            }
            EditorEvent::ToggleFoliageErase => {
                self.foliage_erase = !self.foliage_erase;
            }
            EditorEvent::ToggleFoliageSingle => {
                self.foliage_brush.single = !self.foliage_brush.single;
            }
            EditorEvent::SelectFoliageKind(kind) => {
                self.foliage_brush.kind = (kind as usize).min(FOLIAGE_PALETTE.len() - 1) as u8;
                let (name, _) = FOLIAGE_PALETTE[self.foliage_brush.kind as usize];
                // Trees want one-per-click; ground cover wants a spread. Setting
                // the obvious default saves a second click almost every time.
                self.foliage_brush.single = self.foliage_brush.kind >= 2;
                info!("Foliage brush: {name}");
            }

            EditorEvent::CycleTonemapper => {
                let Some(entity) = self.selected_entity else {
                    return;
                };
                if let Some(pp) = self.world.get_mut::<PostProcessComponent>(entity) {
                    pp.tonemapper = pp.tonemapper.next();
                    info!("Tonemapper: {}", pp.tonemapper.label());
                }
            }
            EditorEvent::SetTonemapper(idx) => {
                let Some(entity) = self.selected_entity else {
                    return;
                };
                if let Some(pp) = self.world.get_mut::<PostProcessComponent>(entity) {
                    pp.tonemapper = match idx {
                        1 => crate::Tonemapper::Aces,
                        2 => crate::Tonemapper::Reinhard,
                        _ => crate::Tonemapper::AgX,
                    };
                    info!("Tonemapper: {}", pp.tonemapper.label());
                }
            }
            EditorEvent::SetPostFx(which, on) => {
                use somnium_ui::PostFxToggle;
                let Some(entity) = self.selected_entity else {
                    return;
                };
                if let Some(pp) = self.world.get_mut::<PostProcessComponent>(entity) {
                    let on = match which {
                        PostFxToggle::Vignette => {
                            pp.vignette_enabled = on;
                            pp.vignette_enabled
                        }
                        PostFxToggle::ChromaticAberration => {
                            pp.ca_enabled = on;
                            pp.ca_enabled
                        }
                        PostFxToggle::Fxaa => {
                            pp.fxaa_enabled = on;
                            pp.fxaa_enabled
                        }
                        PostFxToggle::AutoExposure => {
                            pp.auto_exposure = on;
                            pp.auto_exposure
                        }
                        PostFxToggle::CelShading => {
                            pp.cel_shading = on;
                            pp.cel_shading
                        }
                        PostFxToggle::Taa => {
                            pp.set_taa_enabled(on);
                            pp.taa_enabled
                        }
                        PostFxToggle::Gtao => {
                            pp.gtao_enabled = on;
                            pp.gtao_enabled
                        }
                        PostFxToggle::PhysicalCamera => {
                            pp.use_physical_camera = on;
                            pp.use_physical_camera
                        }
                        PostFxToggle::Volumetrics => {
                            pp.set_volumetrics_enabled(on);
                            pp.volumetrics_enabled
                        }
                        PostFxToggle::LightShafts => {
                            pp.set_light_shafts_enabled(on);
                            pp.light_shafts
                        }
                        PostFxToggle::MotionBlur => {
                            pp.motion_blur_enabled = on;
                            pp.motion_blur_enabled
                        }
                        PostFxToggle::Cas => {
                            pp.set_cas_enabled(on);
                            pp.cas_enabled
                        }
                        PostFxToggle::RestirGi => {
                            pp.restir_gi_enabled = on;
                            pp.restir_gi_enabled
                        }
                        PostFxToggle::RtReflect => {
                            pp.rt_reflect_enabled = on;
                            pp.rt_reflect_enabled
                        }
                        PostFxToggle::RtRefract => {
                            pp.rt_refract_enabled = on;
                            pp.rt_refract_enabled
                        }
                        PostFxToggle::Restir => {
                            pp.restir_enabled = on;
                            pp.restir_enabled
                        }
                        PostFxToggle::Bloom => {
                            pp.bloom_enabled = on;
                            pp.bloom_enabled
                        }
                        PostFxToggle::DepthOfField => {
                            pp.dof_enabled = on;
                            pp.dof_enabled
                        }
                        PostFxToggle::Pcss => {
                            pp.pcss_enabled = on;
                            pp.pcss_enabled
                        }
                        PostFxToggle::ContactShadows => {
                            pp.contact_shadows_enabled = on;
                            pp.contact_shadows_enabled
                        }
                        PostFxToggle::WorldCache => {
                            pp.set_world_cache_enabled(on);
                            pp.world_cache
                        }
                        PostFxToggle::SpecularGi => {
                            pp.specular_gi = on;
                            pp.specular_gi
                        }
                        PostFxToggle::PathTracer => {
                            pp.path_tracer = on;
                            pp.path_tracer
                        }
                        PostFxToggle::MeshSdf => {
                            pp.set_mesh_sdf_enabled(on);
                            pp.mesh_sdf
                        }
                        PostFxToggle::Probes => {
                            pp.probes = on;
                            pp.probes
                        }
                        PostFxToggle::AnalyticGrad => {
                            pp.analytic_grad = on;
                            pp.analytic_grad
                        }
                        PostFxToggle::Fsr => {
                            pp.set_fsr_enabled(on);
                            pp.fsr_enabled
                        }
                    };
                    info!("Post FX {:?}: {}", which, if on { "on" } else { "off" });
                }
            }
        }
    }
}

// ─── Gizmo picking / drag math ────────────────────────────────────────────────

/// Place a newly created entity a few metres in front of the camera.
///
/// Origin is usually underground on the default landscape, so Create → Disc /
/// Tube / Cube at (0,0,0) looks like the feature never spawned. `look` is the
/// camera forward (the direction a disc/spot/rect should emit); `right` is the
/// camera's horizontal axis (a tube's length so it reads as a line in view).
fn camera_spawn_basis(
    renderer: Option<&SomniumRenderer>,
    distance: f32,
) -> (glam::Vec3, glam::Vec3, glam::Vec3) {
    let Some(r) = renderer else {
        return (glam::Vec3::ZERO, glam::Vec3::NEG_Z, glam::Vec3::X);
    };
    let look_pt = r
        .view_proj
        .inverse()
        .project_point3(glam::Vec3::new(0.0, 0.0, 0.35));
    let mut look = (look_pt - r.camera_pos).normalize_or_zero();
    if look == glam::Vec3::ZERO {
        look = glam::Vec3::NEG_Z;
    }
    let mut right = look.cross(glam::Vec3::Y).normalize_or_zero();
    if right == glam::Vec3::ZERO {
        right = glam::Vec3::X;
    }
    (r.camera_pos + look * distance, look, right)
}

/// Unproject a screen position to a world-space point (at mid-depth).
fn ndc_to_world(cx: f32, cy: f32, vw: f32, vh: f32, inv_vp: &glam::Mat4) -> glam::Vec3 {
    let ndc_x = 2.0 * cx / vw - 1.0;
    let ndc_y = 1.0 - 2.0 * cy / vh;
    let clip = glam::Vec4::new(ndc_x, ndc_y, 0.5, 1.0);
    let world = *inv_vp * clip;
    glam::Vec3::new(world.x, world.y, world.z) / world.w
}

/// Parameter along an axis line at which it is closest to a world-space ray.
///
/// Returns `None` if ray and axis are nearly parallel.
fn ray_axis_param(
    ray_origin: glam::Vec3,
    ray_dir: glam::Vec3,
    axis_origin: glam::Vec3,
    axis_dir: glam::Vec3,
) -> Option<f32> {
    let b = ray_dir.dot(axis_dir);
    let denom = 1.0 - b * b;
    if denom.abs() < 1e-8 {
        return None;
    }
    let w = ray_origin - axis_origin;
    let d = -w.dot(ray_dir);
    let e = -w.dot(axis_dir);
    Some((b * d - e) / denom)
}

/// Signed angle (radians) of where a ray pierces a ring plane.
///
/// Returns `None` if the ray is nearly parallel to the plane or hits behind
/// the camera.
fn ring_angle(
    ray_origin: glam::Vec3,
    ray_dir: glam::Vec3,
    ring_center: glam::Vec3,
    ring_normal: glam::Vec3,
    ring_tangent: glam::Vec3,
    ring_bitangent: glam::Vec3,
) -> Option<f32> {
    let denom = ray_dir.dot(ring_normal);
    if denom.abs() < 1e-8 {
        return None;
    }
    let t = (ring_center - ray_origin).dot(ring_normal) / denom;
    if t < 0.0 {
        return None;
    }
    let v = ray_origin + t * ray_dir - ring_center;
    Some(f32::atan2(v.dot(ring_bitangent), v.dot(ring_tangent)))
}

/// Orthonormal basis vectors for the plane perpendicular to `axis`.
fn ring_plane_basis(axis: GizmoAxis) -> (glam::Vec3, glam::Vec3) {
    match axis {
        GizmoAxis::X => (glam::Vec3::Y, glam::Vec3::Z),
        GizmoAxis::Y => (glam::Vec3::Z, glam::Vec3::X),
        GizmoAxis::Z => (glam::Vec3::X, glam::Vec3::Y),
    }
}

/// Try to begin a gizmo drag.  Returns `Some(state)` if the cursor ray hits
/// an axis handle; `None` if no gizmo is visible or no axis was hit.
fn try_start_gizmo_drag(
    renderer: Option<&SomniumRenderer>,
    world: &somnium_ecs::World,
    selected_entity: &Option<somnium_ecs::entity::Entity>,
    cursor_pos: (f32, f32),
    viewport_size: (f32, f32),
) -> Option<GizmoDragState> {
    let renderer = renderer?;
    if !renderer.editor_overlays_enabled() {
        return None;
    }
    let entity = (*selected_entity)?;
    let gizmo_pos = renderer.gizmo_world_pos?;

    let (vw, vh) = viewport_size;
    if vw < 1.0 || vh < 1.0 {
        return None;
    }

    let camera_pos = renderer.camera_pos;
    let inv_vp = renderer.view_proj.inverse();

    let world_pt = ndc_to_world(cursor_pos.0, cursor_pos.1, vw, vh, &inv_vp);
    let ray_dir = (world_pt - camera_pos).normalize();

    // Transform ray to gizmo-local space.
    let dist = (camera_pos - gizmo_pos).length().max(0.5);
    let scale = dist * 0.15;
    let model =
        glam::Mat4::from_translation(gizmo_pos) * glam::Mat4::from_scale(glam::Vec3::splat(scale));
    let inv_model = model.inverse();
    let local_origin = inv_model.transform_point3(camera_pos);
    let local_dir = inv_model.transform_vector3(ray_dir).normalize();

    let mode = renderer.gizmo_mode;
    let axis = gizmo_hit_test(local_origin, local_dir, mode)?;

    let start_transform = world
        .get::<Transform>(entity)
        .copied()
        .unwrap_or_else(|| Transform::from_translation(glam::Vec3::ZERO));

    let (start_axis_param, start_angle, ring_tangent, ring_bitangent) = match mode {
        GizmoMode::Translate | GizmoMode::Scale => {
            let s = ray_axis_param(camera_pos, ray_dir, gizmo_pos, axis.world_dir()).unwrap_or(0.0);
            (s, 0.0, glam::Vec3::ZERO, glam::Vec3::ZERO)
        }
        GizmoMode::Rotate => {
            let (tan, bitan) = ring_plane_basis(axis);
            let a = ring_angle(camera_pos, ray_dir, gizmo_pos, axis.world_dir(), tan, bitan)
                .unwrap_or(0.0);
            (0.0, a, tan, bitan)
        }
    };

    Some(GizmoDragState {
        axis,
        mode,
        entity_index: entity.index(),
        start_transform,
        start_axis_param,
        start_angle,
        ring_tangent,
        ring_bitangent,
        gizmo_pos,
    })
}

/// Compute the new entity transform given the current cursor ray.
fn apply_gizmo_drag(
    drag: &GizmoDragState,
    camera_pos: glam::Vec3,
    inv_vp: glam::Mat4,
    cursor_pos: (f32, f32),
    viewport_size: (f32, f32),
) -> Transform {
    let mut result = drag.start_transform;
    let (vw, vh) = viewport_size;
    if vw < 1.0 || vh < 1.0 {
        return result;
    }

    let world_pt = ndc_to_world(cursor_pos.0, cursor_pos.1, vw, vh, &inv_vp);
    let ray_dir = (world_pt - camera_pos).normalize();
    let axis_dir = drag.axis.world_dir();

    match drag.mode {
        GizmoMode::Translate => {
            if let Some(s) = ray_axis_param(camera_pos, ray_dir, drag.gizmo_pos, axis_dir) {
                result.translation =
                    drag.start_transform.translation + (s - drag.start_axis_param) * axis_dir;
            }
        }
        GizmoMode::Scale => {
            if let Some(s) = ray_axis_param(camera_pos, ray_dir, drag.gizmo_pos, axis_dir) {
                if drag.start_axis_param.abs() > 0.01 {
                    let factor = (s / drag.start_axis_param).abs().max(0.01);
                    let mut sc = drag.start_transform.scale;
                    match drag.axis {
                        GizmoAxis::X => sc.x *= factor,
                        GizmoAxis::Y => sc.y *= factor,
                        GizmoAxis::Z => sc.z *= factor,
                    }
                    result.scale = sc;
                }
            }
        }
        GizmoMode::Rotate => {
            if let Some(angle) = ring_angle(
                camera_pos,
                ray_dir,
                drag.gizmo_pos,
                axis_dir,
                drag.ring_tangent,
                drag.ring_bitangent,
            ) {
                let delta = angle - drag.start_angle;
                result.rotation =
                    glam::Quat::from_axis_angle(axis_dir, delta) * drag.start_transform.rotation;
            }
        }
    }
    result
}
