//! Application lifecycle and editor/runtime orchestration.

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
use somnium_ui::{EditorEvent, GestureId, TerrainInspectorState, UiManager};

use crate::config::EngineConfig;
use crate::context::{EngineContext, SimulationClock, SimulationState};
use crate::editor_commands::{
    AssignMaterialCmd, CreateEntityCmd, CreateLandscapeCmd, DeleteEntityCmd, EntitySnapshot,
    FieldUndoSnapshot, SetFieldCmd, SetTransformCmd, TerrainEditCmd, TerrainRestoreOp,
    TerrainRestoreQueue, UndoStack,
};
use crate::editor_gizmo::{
    GizmoFollower, apply_followers, capture_followers, entity_world_matrix, invert_affine,
    parent_world_matrix, world_to_local_translation,
};
use crate::error::EngineError;
use crate::event::{EngineEvent, translate_window_event};
use crate::jobs::{JobHandle, JobPriority, JobSystem};
use crate::time::TimeState;
use crate::{
    AudioEmitterComponent, CameraSettingsComponent, EditorFlags, FoliageComponent, LightComponent,
    LightType, MaterialComponent, MeshComponent, MeshKind, Name, Parent, PostProcessComponent,
    TerrainComponent, Transform, UiCanvasComponent, VoxelTerrainComponent, WaterComponent,
    WorldPartitionComponent, WorldTransform, look_rotation_neg_z, simulate_particles,
};
use somnium_ecs::{Entity, World};
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

/// Some editor shortcuts reuse keys that remain meaningful to the viewport.
///
/// The command is still consumed by the editor, but its physical key transition
/// must also reach the game input state. Otherwise Ctrl+S followed by RMB leaves
/// the fly camera unaware that `S` is held until the user releases and presses it
/// again.
/// The editor action a key press should run, if any.
///
/// Extracted so the rule is testable without a window. The engine's dispatcher
/// runs **before** the UI is consulted and every arm of it returns, so a key it
/// claims never reaches the game — which is why `game_owns_keyboard` has to be
/// part of the decision rather than a check somewhere downstream.
fn shortcut_action_for(
    code: winit::keyboard::KeyCode,
    modifiers: somnium_ui::message::Modifiers,
    game_owns_keyboard: bool,
) -> Option<somnium_ui::commands::CommandAction> {
    somnium_ui::commands::Chord::from_winit(
        code,
        modifiers.command(),
        modifiers.shift,
        modifiers.alt,
        false,
    )
    // While the game owns the keyboard — flying the viewport, or in a play
    // session — an unmodified shortcut stands down and the key falls through.
    // Modified chords are unaffected: `Ctrl+S` is unambiguous and should still
    // save mid-flight and mid-play.
    .filter(|chord| !game_owns_keyboard || chord.has_modifier())
    .and_then(|chord| somnium_ui::commands::registry().binding(chord))
    .map(|command| command.action)
}

fn shortcut_preserves_game_key(action: somnium_ui::commands::CommandAction) -> bool {
    matches!(action, somnium_ui::commands::CommandAction::SaveScene)
}

#[cfg(test)]
mod shortcut_input_tests {
    use super::{shortcut_action_for, shortcut_preserves_game_key};
    use somnium_ui::commands::CommandAction;
    use somnium_ui::message::Modifiers;
    use winit::keyboard::KeyCode;

    #[test]
    fn save_keeps_the_physical_s_transition_for_viewport_flight() {
        assert!(shortcut_preserves_game_key(CommandAction::SaveScene));
        assert!(!shortcut_preserves_game_key(CommandAction::Undo));
        assert!(!shortcut_preserves_game_key(CommandAction::Redo));
    }

    /// The reported bug, as a test.
    ///
    /// `S` is bound to the Scale tool *and* is the fly-cam's "move backward".
    /// This dispatcher runs before the UI and returns, so when it claimed the
    /// key the camera never saw it — and the two-or-three-second delay before
    /// movement started was OS auto-repeat slipping past the `!repeat` guard,
    /// not a stall.
    #[test]
    fn an_unmodified_shortcut_stands_down_while_the_viewport_is_flying() {
        let none = Modifiers::default();

        // Grounded: `S` is the Scale tool, which is what makes this a conflict
        // at all. If this assertion ever fails the binding moved and the rest
        // of the test is measuring nothing.
        assert_eq!(
            shortcut_action_for(KeyCode::KeyS, none, false),
            Some(CommandAction::SetGizmoMode(2)),
        );

        // Flying: nothing is claimed, so the key reaches the camera.
        assert_eq!(shortcut_action_for(KeyCode::KeyS, none, true), None);
    }

    /// The rule is "unmodified", not "all". A modified chord is unambiguous and
    /// must keep working mid-flight, or flying would silently disable Save.
    #[test]
    fn a_modified_shortcut_still_fires_while_flying() {
        let ctrl = Modifiers {
            ctrl: true,
            ..Modifiers::default()
        };
        assert_eq!(
            shortcut_action_for(KeyCode::KeyS, ctrl, true),
            Some(CommandAction::SaveScene),
        );
    }

    /// The same rule, reached the other way: a play session.
    ///
    /// Reported after the fly-cam fix landed — walking a character backwards
    /// with `S` had the identical two-or-three-second delay, because the
    /// character is not flying the viewport and the latch was never set. Both
    /// contexts end at this dispatcher, so both belong in the same predicate.
    #[test]
    fn an_unmodified_shortcut_stands_down_during_a_play_session() {
        let none = Modifiers::default();
        assert_eq!(
            shortcut_action_for(KeyCode::KeyS, none, false),
            Some(CommandAction::SetGizmoMode(2))
        );
        assert_eq!(shortcut_action_for(KeyCode::KeyS, none, true), None);
    }

    /// Stopping a play session must not depend on a suppressed key.
    ///
    /// Play, Pause and Stop are declared with no chord at all — they are
    /// toolbar commands — so suppressing unmodified keys during a session
    /// cannot trap anybody in one. If somebody ever binds a bare key to Stop,
    /// this fails and says why.
    #[test]
    fn the_transport_is_not_reachable_by_an_unmodified_key() {
        for command in somnium_ui::commands::registry().commands() {
            if matches!(
                command.action,
                CommandAction::Play | CommandAction::Pause | CommandAction::Stop
            ) {
                assert!(
                    command
                        .default_binding
                        .is_none_or(somnium_ui::commands::Chord::has_modifier),
                    "{} has an unmodified binding, which play-session suppression would eat",
                    command.id
                );
            }
        }
    }

    /// Every fly-cam key must be free of an unmodified binding while the game
    /// owns the keyboard — `S` was the one that showed, but the rule has to
    /// hold for all of them or the next report is `Q`.
    #[test]
    fn no_fly_cam_key_is_claimed_while_flying() {
        let none = Modifiers::default();
        for key in [
            KeyCode::KeyW,
            KeyCode::KeyA,
            KeyCode::KeyS,
            KeyCode::KeyD,
            KeyCode::KeyQ,
            KeyCode::KeyE,
        ] {
            assert_eq!(
                shortcut_action_for(key, none, true),
                None,
                "{key:?} is a fly-cam key and must fall through while flying"
            );
        }
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
                !path
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .contains(".luau.luau"),
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
    /// Parent model transform at drag start. Identity for a root entity.
    ///
    /// The gizmo solves its gesture in world space; authored `Transform` is
    /// local. Keeping both directions here is what lets the seam convert once
    /// instead of making every drag caller understand hierarchy algebra.
    parent_world: glam::Mat4,
    parent_world_inverse: glam::Mat4,
    /// The selected entity's world rotation at drag start, for local axes.
    start_world_rotation: glam::Quat,
    /// The rest of the selection, with the transform each started at.
    ///
    /// CONTROL-F made the selection a set; a gizmo that still moved one thing
    /// would have made multi-select a lie the moment anyone dragged. The
    /// *deltas* are applied to these, not the primary's final value, so twelve
    /// objects keep their relative positions.
    followers: Vec<GizmoFollower>,
    /// Whether the drag axes follow the object's own rotation.
    local_space: bool,
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
    ///
    /// This is where a game **builds** its widget trees, because it is the last
    /// callback that carries the whole [`EngineContext`]. Drawing them is
    /// [`Self::on_render_ui`], which runs later in the frame with the GPU open.
    fn on_render(&mut self, _ctx: &mut EngineContext) {}

    /// Called inside the frame, after the world and before the editor shell,
    /// with the encoder open. **Draw here; build in [`Self::on_render`].**
    ///
    /// MORROWIND-E2, and the sub-phase exists because this method did not:
    /// MORROWIND-D through -G built a paint layer, canvases, anchors,
    /// directional navigation and styled text, and a `GameApp` had no way to
    /// put any of it on screen. `examples/vvardenfell` printed its HUD layout
    /// to stdout for want of these six lines.
    ///
    /// ```ignore
    /// fn on_render_ui(&mut self, frame: &mut GameUiFrame) {
    ///     frame.draw(&mut self.hud);
    ///     frame.draw(&mut self.name_plate);
    /// }
    /// ```
    ///
    /// Deliberately carries no world, no physics and no time. A game that finds
    /// it needs one of those here has found that it is building rather than
    /// drawing, and the build belongs one callback earlier.
    fn on_render_ui(&mut self, _frame: &mut somnium_ui::GameUiFrame) {}

    /// Called for every raw OS window event the editor shell did not consume.
    /// Return `true` to consume it before the editor's own viewport handling.
    ///
    /// MORROWIND-E2's other half. [`EngineEvent`](crate::EngineEvent) is a
    /// translated, engine-shaped event and stays the thing most games want;
    /// this is the untranslated one, because
    /// [`UiCanvas::process_os_event`](somnium_ui::UiCanvas::process_os_event)
    /// takes a `winit::event::WindowEvent` and a translation layer in front of
    /// it would be a second event vocabulary for the runtime UI to disagree
    /// with the editor UI in.
    ///
    /// **Ordering is the editor's, and that is a temporary answer.** The editor
    /// shell gets first refusal today, which is right while the game is a thing
    /// inside a viewport and wrong once there is a play mode with input focus
    /// of its own. That is MORROWIND-N's call and it is named here so N does
    /// not have to rediscover it.
    fn on_os_event(
        &mut self,
        _ctx: &mut EngineContext,
        _event: &winit::event::WindowEvent,
    ) -> bool {
        false
    }

    /// Called just before the engine shuts down.
    fn on_shutdown(&mut self) {}

    /// Called after a version-2 map factory finishes (drawer double-click or tests).
    fn on_map_loaded(&mut self, _ctx: &mut EngineContext, _result: &crate::MapLoadResult) {}

    /// The game's authored `.somui` documents, for `ctx:setUiProperty`.
    ///
    /// MORROWIND-M2. Defaulted to `None` so no existing game changes; a game
    /// that returns `None` and whose scripts write UI gets one rejection line
    /// naming this method, rather than a HUD that quietly never updates.
    ///
    /// The engine does not own these because a HUD is part of the game: how
    /// many documents there are and what they are called is not something an
    /// engine can decide for it.
    fn ui_documents(&mut self) -> Option<&mut dyn crate::script_host::UiDocumentSink> {
        None
    }
}

/// Joins a boxed [`GameApp`] to the renderer's [`somnium_ui::GameUi`] seam.
///
/// MORROWIND-E2. `somnium_renderer` cannot name a `GameApp` — `somnium_core`
/// depends on the renderer and not the other way round — so the renderer takes
/// a trait object it *can* name and this is the six lines that satisfy it.
struct GameUiAdapter<'g, G: GameApp + ?Sized> {
    game: &'g mut G,
}

impl<G: GameApp + ?Sized> somnium_ui::GameUi for GameUiAdapter<'_, G> {
    fn draw_ui(&mut self, frame: &mut somnium_ui::GameUiFrame) {
        self.game.on_render_ui(frame);
    }
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

#[derive(Clone)]
struct MaterialDocument {
    path: std::path::PathBuf,
    asset: somnium_asset::material::MaterialAsset,
    dirty: bool,
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
    audio_scene: crate::audio_scene::AudioScene,
    window: Option<Arc<Window>>,
    render_ctx: Option<RenderContext>,
    renderer: Option<SomniumRenderer>,
    ui_manager: Option<UiManager>,
    /// MORROWIND-I. The platform screen-reader adapter, attached to the window
    /// before it is shown. `None` in a headless run.
    a11y: Option<crate::a11y_bridge::A11yBridge>,
    /// Shared bounded workers for imports, inventory scans, bakes and previews.
    jobs: JobSystem,
    /// MORROWIND-S production coordinator driven by the reflected component
    /// attached to a terrain entity.
    world_partition: Option<crate::world_partition::WorldPartition>,
    world_partition_cell_size: f64,
    world_partition_pin: Option<crate::world_partition::CellCoord>,
    /// The inventory and the reference graph travel together, because they
    /// are two readings of the same disk and a panel showing one against the
    /// other's assets is worse than no panel (MORROWIND-M item 3).
    asset_scan: Option<
        JobHandle<(
            somnium_asset::database::AssetDbSnapshot,
            somnium_asset::depend::DependencyIndex,
        )>,
    >,
    asset_gate: somnium_asset::database::DebouncedAssetDb,
    /// The project's string tables, as loaded. The editor holds the *table*;
    /// this is the catalogue it was projected from, kept because a save needs
    /// the display names and font lists a table cannot carry.
    locale_catalog: Option<somnium_i18n::Catalog>,
    /// Float requests waiting for an `ActiveEventLoop`.
    ///
    /// `handle_editor_event` has no event loop and a window cannot be created
    /// without one, so the request is parked here and serviced a few lines
    /// later in `about_to_wait` — the nearest point that has both.
    pending_float: Vec<somnium_ui::floating::FloatingKind>,
    /// Panels the user has pulled out into their own OS windows.
    ///
    /// A `Vec` rather than a map keyed by `WindowId`: there are at most a
    /// handful, they are iterated far more often than they are looked up, and a
    /// linear scan of four is not the thing to optimise.
    floating: Vec<FloatingWindow>,
    next_asset_scan: std::time::Instant,
    /// When shader files were last polled for hot reload (MORROWIND-C).
    last_shader_poll: std::time::Instant,
    /// This frame's background-work telemetry, folded by job name.
    ///
    /// MORROWIND-B. Refreshed by `pump_jobs` once per frame and read by the
    /// profiler panel; `phase_MORROWIND.md` §8 makes job visibility a
    /// requirement rather than a nicety, because a job system without it turns
    /// one mystery (a stall) into a harder one (a stall inside a thread pool).
    job_profile: Vec<crate::jobs::JobProfileRow>,
    /// Zones the telemetry ring had to drop for capacity since the last frame.
    job_zones_dropped: usize,
    preview_jobs: std::collections::HashMap<
        std::path::PathBuf,
        JobHandle<Option<somnium_asset::preview::PreparedPreview>>,
    >,
    preview_ready: std::collections::VecDeque<(std::path::PathBuf, Vec<u8>)>,
    /// Loaded authored material documents, keyed by durable content identity.
    material_documents:
        std::collections::HashMap<somnium_asset::database::AssetId, MaterialDocument>,
    /// Runtime pool slots reconstructed from material asset ids.
    material_runtime: std::collections::HashMap<somnium_asset::database::AssetId, u32>,
    material_textures: std::collections::HashMap<somnium_asset::database::AssetId, i32>,
    material_texture_jobs: std::collections::HashMap<
        somnium_asset::database::AssetId,
        JobHandle<somnium_asset::LoadedTexture>,
    >,
    /// Entity edit sessions and their last observed reflected value. The actual
    /// editable value is the `MaterialAsset` component temporarily attached to
    /// the entity and therefore uses the normal generated Details undo path.
    material_sessions: std::collections::HashMap<
        somnium_ecs::Entity,
        (
            somnium_asset::database::AssetId,
            somnium_asset::material::MaterialAsset,
        ),
    >,
    import_job: Option<
        JobHandle<(
            String,
            somnium_asset::LoadedScene,
            Vec<somnium_asset::database::AssetId>,
        )>,
    >,
    import_spawn_at: [f32; 3],
    external_import_job: Option<JobHandle<Vec<(std::path::PathBuf, std::path::PathBuf)>>>,
    /// CONTROL-F: an ordered selection with a primary. Single-selection
    /// call sites read and write `selection.primary`; everything that means
    /// "all of them" reads `selection.as_slice()`.
    selection: crate::selection::Selection,
    /// The Outliner's flattened row order, refreshed each frame. `Shift`-range
    /// selection means "the rows between these two", so the range has to be
    /// resolved against the order the user can actually see.
    outliner_order: Vec<somnium_ecs::entity::Entity>,
    /// Seam 4: preferences, project settings, and which of them the
    /// environment has taken out of the author's hands.
    settings: crate::settings::SettingsStore,
    /// Camera poses stored in slots 1..=9: `(position, yaw, pitch)`.
    ///
    /// Stored here rather than in the game so a bookmark survives whatever the
    /// camera implementation is; the recall is a focus request like `F`.
    camera_bookmarks: [Option<(glam::Vec3, f32, f32)>; 9],
    /// Orbit the camera around the selection rather than around itself.
    orbit_selection: bool,
    /// A pending exact camera pose: `(position, yaw degrees, pitch degrees)`.
    /// Distinct from `camera_focus_request`, which asks the game to frame
    /// something and leaves the heading to it.
    camera_pose_request: Option<(glam::Vec3, f32, f32)>,
    /// Entities under the cursor, newest piercing menu first.
    piercing_candidates: Vec<somnium_ecs::entity::Entity>,
    /// When the next interval autosave is due.
    autosave: crate::autosave::AutosaveClock,
    /// A recoverable autosave found at launch, until the person answers.
    pending_recovery: Option<crate::autosave::Recovery>,
    /// Most-recently-opened scenes, newest first.
    recent_scenes: Vec<std::path::PathBuf>,
    /// The entity clipboard. Values, never handles — see `clipboard.rs`.
    entity_clipboard: crate::clipboard::EntityClipboard,
    /// Pending "frame this" request for the game-owned editor camera:
    /// world-space centre and the radius that should fit the view.
    camera_focus_request: Option<(glam::Vec3, f32)>,
    state: LifecycleState,
    /// Bounded command history for editor undo/redo (128-command capacity).
    undo_stack: UndoStack,
    /// Gesture baselines captured before the first live reflected write.
    ///
    /// Keyed per `(gesture, entity)` since CONTROL-F: one drag of one slider
    /// can now be editing twelve entities, and each needs its own baseline for
    /// the single undo entry to restore all twelve.
    field_gestures:
        std::collections::HashMap<(GestureId, somnium_ecs::entity::Entity), FieldUndoSnapshot>,
    /// Current cursor position in physical pixels.
    cursor_pos: (f32, f32),
    /// Current window dimensions in physical pixels (updated on resize).
    viewport_size_hint: (f32, f32),
    /// Active gizmo drag state (Some while LMB is held on a gizmo axis).
    gizmo_drag: Option<GizmoDragState>,
    /// Viewport rubber-band, live only while the left button is down over
    /// empty space with no gizmo axis under the pointer.
    marquee: Option<crate::selection::Marquee>,

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
    /// CONTROL-L: this frame's day-cycle result, or `None` when no cycle is
    /// enabled. Recomputed every frame and never persisted — it is a cache of
    /// one evaluation, not state.
    day_state: Option<crate::time_of_day::DayState>,
    /// Fingerprint of the content root at the last inventory scan. `None`
    /// forces one, which is what every explicit invalidation point sets.
    asset_scan_stamp: Option<(u64, u64)>,
    /// CONTROL-N: the live weather. Unlike [`Self::day_state`] this genuinely
    /// *is* state — the two wetness scalars integrate over time — but it is
    /// still never saved: a scene reloads dry and wets again, which is the
    /// only honest answer when the file records causes rather than history.
    weather_state: crate::weather::WeatherState,
    /// The entity carrying the rain emitter, spawned on demand and despawned
    /// when the weather stops.
    precipitation_entity: Option<somnium_ecs::Entity>,
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
    /// Hardware-independent action maps shared by game code and scripts.
    input: somnium_input::InputSystem,
    /// The authored world as it was when Play was pressed. Stop restores
    /// it, which is what stops a script dirtying the edit-time scene.
    play_checkpoint: Option<crate::script_input::WorldCheckpoint>,
    /// Fixed steps elapsed in this play session. Part of the deterministic
    /// clock a script sees, and reset by Stop.
    script_step: u64,
    /// MORROWIND-N: fixed steps owed to a paused simulation.
    ///
    /// A counter rather than a flag so holding the Step control queues steps
    /// instead of dropping every one that arrives inside a single frame — at
    /// 60 Hz a key repeat is faster than the frame that would consume it.
    pending_steps: u32,
    /// Whether *this frame* is a hand-driven step.
    ///
    /// Separate from `pending_steps` because the counter is spent before the
    /// step it pays for actually runs: deriving the flag from it made
    /// `stepping` false during the very step it describes.
    stepping_now: bool,
}

/// One floating window's swapchain image, acquired before the frame that fills
/// it.
///
/// MORROWIND-J step 2. The viewport's window is the only one that needs this.
/// Its picture is recorded by the renderer, in the editor's own encoder, so the
/// surface has to be acquired before that encoder is built — and then carried
/// past the whole frame to the UI pass that draws the context bar over it and
/// the present that ends it.
struct AcquiredFrame {
    output: somnium_ui::wgpu::SurfaceTexture,
    view: somnium_ui::wgpu::TextureView,
    /// That surface's size, which the renderer's upscale target has to match.
    size: (u32, u32),
}

/// Where a window event belongs, once a second window exists.
///
/// MORROWIND-J step 2. The interesting arm is the third one. A floating
/// Outliner is finished the moment its widgets have seen the event; a floating
/// viewport is not, because flying the camera, dragging a gizmo and picking an
/// entity are not widget behaviour — they are the editor's, and the editor's
/// input path has to run for that window too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FloatingRoute {
    /// Not a floating window's event.
    Main,
    /// A floating window's, and dealt with.
    Handled,
    /// The floating viewport's: the widgets have had it and declined, and the
    /// editor's viewport tools still have to see it.
    Viewport,
}

/// A panel in its own OS window: the parts `somnium_ui` deliberately does not
/// own.
///
/// MORROWIND-J step 2. The widgets are not here and are not a copy of anything:
/// they are the panel's own nodes, detached from the dock and laid out against
/// this window instead (see [`somnium_ui::floating`]). Neither is the pass,
/// which the editor owns and every window shares. What this owns is the
/// platform: a window, a surface, and its configuration.
struct FloatingWindow {
    kind: somnium_ui::floating::FloatingKind,
    window: std::sync::Arc<Window>,
    surface: somnium_ui::wgpu::Surface<'static>,
    config: somnium_ui::wgpu::SurfaceConfiguration,
    /// How many frames this window has drawn, for `SOMNIUM_FLOAT_PNG`.
    frames: u64,
    captured: bool,
    /// Whether this window has yet reported what it drew.
    ///
    /// One line, once, naming the rectangle and the instance count. A floating
    /// window cannot be captured the way the editor's own frame can — the
    /// capture path reads the editor's swapchain — so without this the only
    /// evidence that a detached panel drew anything is somebody looking at it.
    reported: bool,
}

impl FloatingWindow {
    /// The window's size in the units the widget tree lays out in.
    ///
    /// The *interface's* scale, not this window's, and read live rather than
    /// remembered. The tree lays out in one scale and converts pointer
    /// positions with the same one, so a window sized by its own monitor while
    /// its clicks were converted by the editor's would put every control a
    /// little away from where it responds. On a second monitor this window is
    /// therefore the wrong apparent size, which is the deferral
    /// `FontAtlas::render_scale` already names, and not the wrong *shape*.
    fn logical(&self, ui: &somnium_ui::UiManager) -> (f32, f32) {
        let scale = if ui.ui_scale() > 0.0 {
            ui.ui_scale()
        } else {
            1.0
        };
        (
            self.config.width as f32 / scale,
            self.config.height as f32 / scale,
        )
    }

    /// Reconfigure for a new physical size.
    ///
    /// A zero-sized surface is a validation error rather than a small one, and
    /// minimising a window is exactly how it happens.
    fn resize(
        &mut self,
        device: &somnium_ui::wgpu::Device,
        ui: &mut somnium_ui::UiManager,
        width: u32,
        height: u32,
    ) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(device, &self.config);
        let logical = self.logical(ui);
        ui.resize_floating(self.kind, logical);
    }

    /// Draw this window's panel into this window's swapchain.
    ///
    /// The panel is drawn from the editor's own interface, after the editor's
    /// frame has been recorded *and submitted*: one drawing context and one
    /// `UiPass` serve both windows, because both are one interface.
    ///
    /// `frame` is present for the viewport and nothing else. That window's
    /// surface was acquired before the editor's frame began, because the scene
    /// was recorded straight into it — so what arrives here is a swapchain
    /// image that already holds the picture, and the UI pass loads over it
    /// rather than clearing, exactly as it does in the editor's own window.
    fn render(
        &mut self,
        ui: &mut somnium_ui::UiManager,
        ctx: &RenderContext,
        frame: Option<AcquiredFrame>,
    ) {
        use somnium_ui::wgpu;
        if !ui.is_panel_floating(self.kind) {
            // Nothing of this panel is in this window, so there is nothing to
            // draw. Acquiring a frame to leave it blank would make the window
            // flicker on its way back to the dock.
            return;
        }
        let (output, view) = match frame {
            Some(frame) => (frame.output, frame.view),
            None => {
                let output = match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(tex)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(tex) => tex,
                    // A window being resized or occluded fails to acquire, and
                    // skipping the frame is the whole of the correct response.
                    _ => return,
                };
                let view = output
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                (output, view)
            }
        };
        let physical = (self.config.width, self.config.height);
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Floating Window Encoder"),
            });
        let surface = somnium_ui::pass::UiSurface::new(self.logical(ui), physical);
        let instances = ui.render_floating(
            self.kind,
            &ctx.device,
            &ctx.queue,
            &mut encoder,
            &view,
            surface,
        );
        ctx.queue.submit(std::iter::once(encoder.finish()));
        self.frames += 1;
        // Before `present`, because presenting hands the image back to the
        // swapchain and the texture is no longer ours to read.
        // At or past, and once: this window opens a frame or two after the
        // editor and would otherwise miss an exact target the editor quits on.
        if !self.captured && self.frames >= Self::capture_frame() {
            self.captured = true;
            if let Ok(path) = std::env::var("SOMNIUM_FLOAT_PNG") {
                // Named per panel, because a run can open four of them and four
                // windows writing one path leaves whichever finished last.
                let named = match path.rsplit_once('.') {
                    Some((stem, ext)) => format!("{stem}-{}.{ext}", self.kind.slug()),
                    None => format!("{path}-{}", self.kind.slug()),
                };
                somnium_renderer::capture::write_surface_png(
                    &ctx.device,
                    &ctx.queue,
                    &output.texture,
                    self.config.format,
                    &named,
                );
            }
        }
        ctx.queue.present(output);
        if !self.reported {
            self.reported = true;
            let bounds = ui.panel_bounds(self.kind);
            info!(
                kind = ?self.kind,
                window = ?physical,
                panel = ?(bounds.w, bounds.h),
                instances,
                "floating window drew its panel"
            );
        }
    }

    /// Which of this window's frames `SOMNIUM_FLOAT_PNG` writes.
    ///
    /// Shares `SOMNIUM_CAPTURE_FRAME` with the editor's own capture, so one
    /// run can write both and they are the same moment.
    fn capture_frame() -> u64 {
        std::env::var("SOMNIUM_CAPTURE_FRAME")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(240)
    }
}

impl<G: GameApp + 'static> Engine<G> {
    /// Start the engine loop. This will take control of the current thread.
    pub fn run(mut config: EngineConfig, game: G) -> Result<(), EngineError> {
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

        if config.content_root.is_relative() {
            config.content_root = std::env::current_dir()
                .unwrap_or_default()
                .join(&config.content_root);
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
        // Seam 4 resolves before the engine exists, because the content root
        // it produces is what the asset scan and the window title read.
        let settings_store = crate::settings::SettingsStore::load(
            crate::settings::default_global_path(),
            config_project_path(&config),
        );
        let autosave_interval = settings_store.project().autosave_interval_s;
        let resolved_root = std::path::PathBuf::from(&settings_store.project().content_root);
        if resolved_root.as_os_str().is_empty() {
            // An empty root would silently point the drawer at the working
            // directory; the declared default is the honest fallback.
            config.content_root = std::path::PathBuf::from("assets");
        } else {
            config.content_root = resolved_root;
        }
        let mut engine = Self {
            game: Box::new(game),
            time: TimeState::new(config.target_fps),
            config,
            world: World::new(),
            type_registry: crate::reflect_registry::component_registry(),
            physics: None,
            audio: None,
            audio_scene: crate::audio_scene::AudioScene::default(),
            window: None,
            render_ctx: None,
            renderer: None,
            ui_manager: None,
            a11y: None,
            jobs: JobSystem::default(),
            world_partition: None,
            world_partition_cell_size: 0.0,
            world_partition_pin: None,
            asset_scan: None,
            asset_gate: somnium_asset::database::DebouncedAssetDb::default(),
            locale_catalog: None,
            pending_float: Vec::new(),
            floating: Vec::new(),
            next_asset_scan: std::time::Instant::now(),
            last_shader_poll: std::time::Instant::now(),
            job_profile: Vec::new(),
            job_zones_dropped: 0,
            preview_jobs: std::collections::HashMap::new(),
            preview_ready: std::collections::VecDeque::new(),
            material_documents: std::collections::HashMap::new(),
            material_runtime: std::collections::HashMap::new(),
            material_textures: std::collections::HashMap::new(),
            material_texture_jobs: std::collections::HashMap::new(),
            material_sessions: std::collections::HashMap::new(),
            import_job: None,
            import_spawn_at: [0.0; 3],
            external_import_job: None,
            selection: crate::selection::Selection::default(),
            outliner_order: Vec::new(),
            settings: settings_store,
            camera_bookmarks: [None; 9],
            orbit_selection: false,
            camera_pose_request: None,
            piercing_candidates: Vec::new(),
            autosave: crate::autosave::AutosaveClock::new(autosave_interval),
            pending_recovery: None,
            recent_scenes: crate::settings::load_recent_scenes(),
            entity_clipboard: crate::clipboard::EntityClipboard::default(),
            camera_focus_request: None,
            state: LifecycleState::Uninitialized,
            undo_stack: UndoStack::new(128),
            field_gestures: std::collections::HashMap::new(),
            cursor_pos: (0.0, 0.0),
            viewport_size_hint: initial_vp,
            gizmo_drag: None,
            marquee: None,
            foliage_meshes: std::array::from_fn(|_| None),
            foliage_failed: [false; FOLIAGE_PALETTE.len()],
            foliage_brush: somnium_renderer::terrain::foliage_paint::FoliageBrush::default(),
            foliage_paint_active: false,
            foliage_erase: false,
            foliage_batch: Vec::new(),
            foliage_stroke_seed: 0,
            foliage_painting: false,
            terrain_colliders: std::collections::HashMap::new(),
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
            day_state: None,
            asset_scan_stamp: None,
            weather_state: crate::weather::WeatherState::default(),
            precipitation_entity: None,
            ui_wants_exit: false,
            pending_map_load: None,
            scripts: crate::script_host::ScriptHost::default(),
            script_input: crate::script_input::ScriptInputTracker::new(),
            input: somnium_input::InputSystem::with_default_maps(),
            play_checkpoint: None,
            pending_steps: 0,
            stepping_now: false,
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
    /// Drive the production partition from the terrain attachment and the
    /// scene camera. This is intentionally engine-owned: it needs both the ECS
    /// and the one shared job system, neither of which a UI panel should own.
    fn update_world_partition(&mut self) {
        use crate::world_partition::{
            CellCoord, CellLoadState, PartitionStore, StreamingSource, StreamingSourceKind,
            WorldPartition,
        };

        let owner = self.world.entities().find(|entity| {
            self.world.get::<TerrainComponent>(*entity).is_some()
                && self.world.get::<WorldPartitionComponent>(*entity).is_some()
        });
        let Some(owner) = owner else {
            // Deleting/reloading the terrain must not strand streamed actors
            // in the ECS. Drain the coordinator before dropping it; empty
            // partitions disappear immediately.
            if let Some(partition) = self.world_partition.as_mut() {
                partition.remove_source(1);
                if let Some(pin) = self.world_partition_pin.take() {
                    partition.unpin(pin);
                }
                let _ = partition.update(
                    &mut self.world,
                    &mut self.jobs,
                    std::time::Instant::now() + std::time::Duration::from_millis(100),
                );
                let drained = partition.diagnostics().iter().all(|cell| {
                    cell.actor_count == 0
                        && !matches!(
                            cell.state,
                            CellLoadState::Loading | CellLoadState::Unloading
                        )
                });
                if drained {
                    self.world_partition = None;
                }
            }
            return;
        };
        let settings = self
            .world
            .get::<WorldPartitionComponent>(owner)
            .cloned()
            .expect("owner query required component");
        let requested_cell_size = f64::from(settings.cell_size);
        if !requested_cell_size.is_finite() || requested_cell_size <= 0.0 {
            if let Some(component) = self.world.get_mut::<WorldPartitionComponent>(owner) {
                component.status = "Cell size must be finite and positive".into();
            }
            return;
        }

        let can_rebuild = self.world_partition.as_ref().is_none_or(|partition| {
            partition
                .diagnostics()
                .iter()
                .all(|cell| cell.actor_count == 0)
        });
        if (self.world_partition.is_none()
            || (self.world_partition_cell_size - requested_cell_size).abs() > f64::EPSILON)
            && can_rebuild
        {
            self.world_partition = Some(WorldPartition::new(
                PartitionStore::new(self.config.content_root.join("world_partition")),
                requested_cell_size,
            ));
            self.world_partition_cell_size = requested_cell_size;
            self.world_partition_pin = None;
        }
        let Some(partition) = self.world_partition.as_mut() else {
            return;
        };
        let cell_size_change_pending =
            (self.world_partition_cell_size - requested_cell_size).abs() > f64::EPSILON;

        // A cell-size edit changes every coordinate. Drain the old grid before
        // rebuilding it, otherwise an enabled source keeps the old actors
        // resident forever and the Details control appears to do nothing.
        let desired_pin = (!cell_size_change_pending && settings.pin_cell).then_some(CellCoord {
            x: settings.pin_x,
            y: settings.pin_y,
            z: settings.pin_z,
        });
        if self.world_partition_pin != desired_pin {
            if let Some(old) = self.world_partition_pin.take() {
                partition.unpin(old);
            }
            if let Some(coord) = desired_pin {
                partition.pin(coord);
                self.world_partition_pin = Some(coord);
            }
        }

        // The view is game-owned. In Hello Engine the Camera entity is an
        // authored settings object and its Transform is not the moving editor
        // camera (and during Play it is not the player camera either). The
        // renderer is the one production handoff shared by all games: after
        // `GameApp::on_render` its camera is the exact view this frame draws.
        let camera_position = self
            .renderer
            .as_ref()
            .map(|renderer| renderer.camera_pos.as_dvec3().to_array());
        if settings.enabled && !cell_size_change_pending {
            if let Some(position) = camera_position {
                partition.set_source(StreamingSource {
                    id: 1,
                    position,
                    radius: f64::from(settings.load_radius.max(0.0)),
                    priority: settings.source_priority.min(u32::from(u8::MAX)) as u8,
                    kind: StreamingSourceKind::Camera,
                });
            }
        } else {
            partition.remove_source(1);
        }

        let update_error = partition
            .update(
                &mut self.world,
                &mut self.jobs,
                std::time::Instant::now() + std::time::Duration::from_millis(100),
            )
            .err();
        let diagnostics = partition.diagnostics();
        let loaded = diagnostics
            .iter()
            .filter(|cell| cell.state == CellLoadState::Loaded)
            .count();
        let pending = diagnostics
            .iter()
            .filter(|cell| {
                matches!(
                    cell.state,
                    CellLoadState::Loading | CellLoadState::Unloading
                )
            })
            .count();
        let wanted = diagnostics
            .iter()
            .filter(|cell| cell.priority.is_some())
            .count();
        let actors = diagnostics
            .iter()
            .map(|cell| cell.actor_count)
            .sum::<usize>();
        if let Some(component) = self.world.get_mut::<WorldPartitionComponent>(owner) {
            component.wanted_cells = u32::try_from(wanted).unwrap_or(u32::MAX);
            component.loaded_cells = u32::try_from(loaded).unwrap_or(u32::MAX);
            component.pending_cells = u32::try_from(pending).unwrap_or(u32::MAX);
            component.resident_actors = u32::try_from(actors).unwrap_or(u32::MAX);
            component.status = update_error.map_or_else(
                || {
                    if cell_size_change_pending {
                        format!(
                            "Changing cell size to {requested_cell_size:.1} m; unloading old grid"
                        )
                    } else if !settings.enabled && settings.pin_cell {
                        "Camera streaming disabled; manual pin remains active".into()
                    } else if !settings.enabled {
                        "Camera streaming disabled; unloading cells".into()
                    } else if camera_position.is_none() {
                        "Waiting for the active renderer camera".into()
                    } else {
                        format!("{loaded} loaded, {pending} pending, {wanted} wanted")
                    }
                },
                |error| format!("Streaming job rejected: {error:?}"),
            );
        }
    }

    fn load_material_document(&mut self, asset_id: somnium_asset::database::AssetId) -> bool {
        if self.material_documents.contains_key(&asset_id) {
            return true;
        }
        let record = self
            .asset_gate
            .published()
            .and_then(|snapshot| snapshot.get(asset_id))
            .cloned();
        let Some(record) = record else {
            return false;
        };
        match somnium_asset::material::load_material(&record.absolute_path) {
            Ok(asset) => {
                self.material_documents.insert(
                    asset_id,
                    MaterialDocument {
                        path: record.absolute_path,
                        asset,
                        dirty: false,
                    },
                );
                true
            }
            Err(error) => {
                warn!(%error, "material asset could not be opened");
                false
            }
        }
    }

    /// Reconstruct every authored material reference after scene load, without
    /// requiring the entity to be selected first.
    fn sync_authored_material_components(&mut self) {
        let assets: Vec<_> = self
            .world
            .entities()
            .filter_map(|entity| self.world.get::<MaterialComponent>(entity))
            .map(|material| material.asset)
            .filter(|asset| *asset != somnium_asset::database::AssetId::NONE)
            .collect();
        for asset in assets {
            if self.load_material_document(asset) {
                self.queue_material_textures(asset);
                self.ensure_material_runtime(asset);
            }
        }
    }

    // ── Phase 16-C: driving scripts from the frame loop ──────────────────

    /// The clock a script sees. Fixed-step callbacks are handed
    /// `fixed_delta` and simulation time and nothing else, because those
    /// are the only two values that are the same on a replay.
    fn script_time(&self, fixed_dt: f32, dt: f32) -> somnium_script::snapshot::TimeSnapshot {
        somnium_script::snapshot::TimeSnapshot {
            fixed_delta: fixed_dt,
            delta: dt,
            simulation_time: f64::from(self.simulation_clock.elapsed_seconds),
            step: self.script_step,
            // MORROWIND-N. True only on a frame the step control drove; while
            // Playing it is always false, which is what makes the flag mean
            // "held" rather than "in the editor".
            stepping: self.stepping_now,
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
            ui: self.game.ui_documents(),
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
            ui: self.game.ui_documents(),
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
            ui: self.game.ui_documents(),
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

    /// Finish a rubber-band: everything whose origin projects inside it is
    /// selected, and `command()` adds to the selection rather than replacing
    /// it — the same modifier that adds a single row in the Outliner.
    fn apply_marquee(&mut self, band: crate::selection::Marquee) {
        let Some(renderer) = self.renderer.as_ref() else {
            return;
        };
        let view_proj = renderer.picking_view_proj();
        let viewport = self.viewport_size();
        let caught: Vec<_> = self
            .world
            .entities()
            .filter(|entity| {
                let flags = self
                    .world
                    .get::<EditorFlags>(*entity)
                    .copied()
                    .unwrap_or_default();
                !flags.locked && !flags.hidden
            })
            .filter_map(|entity| {
                let position = self
                    .world
                    .get::<WorldTransform>(entity)
                    .map(|world| world.0.to_scale_rotation_translation().2)
                    .or_else(|| {
                        self.world
                            .get::<Transform>(entity)
                            .map(|transform| transform.translation)
                    })?;
                let clip = view_proj * position.extend(1.0);
                let ndc = glam::Vec3::new(clip.x, clip.y, clip.z) / clip.w;
                band.contains_ndc(ndc, clip.w, viewport).then_some(entity)
            })
            .collect();

        let additive = self
            .ui_manager
            .as_ref()
            .is_some_and(somnium_ui::UiManager::command_modifier_held);
        if additive {
            for entity in caught {
                if !self.selection.contains(entity) {
                    self.selection.toggle(entity);
                }
            }
        } else if caught.is_empty() {
            self.selection.clear();
        } else {
            self.selection.set_many(caught);
        }
        self.after_selection_change();
    }

    /// The generated Preferences panels and the overrides in force.
    fn settings_panels(
        &self,
    ) -> (
        Vec<somnium_ui::editor::inspector_gen::GeneratedComponentPanel>,
        Vec<(
            somnium_ecs::reflect::StableId,
            somnium_ecs::reflect::FieldId,
            String,
        )>,
    ) {
        let editors = somnium_ui::editor::property_editors::PropertyEditorRegistry::standard();
        let rules = somnium_ui::editor::editing_rules::EditingRulesRegistry::default();
        let (world, entity) = self.settings.world();
        let panels = crate::settings::SettingsStore::schemas()
            .into_iter()
            .filter_map(|schema| {
                let values = (schema.snapshot)(world, entity)?;
                Some(somnium_ui::editor::inspector_gen::generate_component_panel(
                    &schema, &values, &editors, &rules,
                ))
            })
            .collect();
        let overrides = self
            .settings
            .overrides()
            .into_iter()
            .map(|(component, field, name)| (component, field, format!("overridden by {name}")))
            .collect();
        (panels, overrides)
    }

    /// Write an autosave, without disturbing the scene's dirty state.
    ///
    /// An autosave is a copy, not a save: it must not mark the scene clean,
    /// because the person has not saved anything and the title bar would be
    /// lying to them.
    fn write_autosave(&mut self, reason: crate::autosave::AutosaveReason) {
        let path = crate::autosave::autosave_path(&self.config.content_root, reason);
        if let Some(parent) = path.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            warn!(%error, "could not create the autosave folder");
            return;
        }
        match crate::scene_schema::save_scene_schema(
            &mut self.world,
            &self.type_registry,
            &path.to_string_lossy(),
        ) {
            Ok(()) => info!(?reason, "autosaved"),
            Err(error) => warn!(%error, "autosave failed"),
        }
    }

    /// Per-frame autosave tick, and the interval setting tracking its control.
    fn tick_autosave(&mut self, dt: f32) {
        self.autosave
            .set_interval(self.settings.project().autosave_interval_s);
        if self.autosave.tick(dt, self.scene_dirty) {
            self.write_autosave(crate::autosave::AutosaveReason::Interval);
        }
    }

    /// Offer a recoverable autosave, once, at launch.
    ///
    /// Offered rather than applied: silently replacing the scene somebody
    /// opened with a file they have never seen is a worse failure than losing
    /// the autosave, and they are the only one who knows which they want.
    fn check_crash_recovery(&mut self) {
        let scene = std::path::PathBuf::from("scene.somnium");
        self.pending_recovery = crate::autosave::find_recovery(&self.config.content_root, &scene);
        if let Some(recovery) = &self.pending_recovery {
            let reason = match recovery.reason {
                crate::autosave::AutosaveReason::BeforePlay => "before Play",
                crate::autosave::AutosaveReason::Interval => "autosave",
            };
            let message = format!("Unsaved work was recovered ({reason}) — File > Open it");
            if let Some(ui) = self.ui_manager.as_mut() {
                ui.append_log(&format!(
                    "[scene] recovered {} ({reason})",
                    recovery.path.display()
                ));
                ui.push_toast(&message);
            }
        }
    }

    /// Open a `.somnium` file, routing on what it actually is.
    ///
    /// CONTROL-J's first bullet, and the `NEXT:` line at the top of
    /// `context.md`. Until now every `LoadScene` went to `map::load_map`,
    /// which only accepts version-2 map recipes — so a scene the editor had
    /// just saved could not be opened by the editor that saved it.
    ///
    /// Three formats, three routes, chosen by name rather than by a number:
    /// a map recipe rebuilds through the map factory, a schema scene rebuilds
    /// through the registry plus GPU reconstruction, and anything else is
    /// refused with a reason rather than half-read.
    fn load_scene_file(&mut self, path: &str) {
        use crate::scene_file::SceneKind;

        let (header, document) = match crate::scene_file::read(std::path::Path::new(path)) {
            Ok(parts) => parts,
            Err(error) => {
                warn!(%error, "could not read scene");
                if let Some(ui) = self.ui_manager.as_mut() {
                    ui.push_toast("Could not read that scene");
                }
                return;
            }
        };
        let kind = SceneKind::of(&document);
        let _ = header;

        // The renderer's scene-side state is torn down once, for every route,
        // so a half-loaded scene cannot inherit the previous one's colliders.
        for (_, (_, body)) in self.terrain_colliders.drain() {
            if let Some(p) = self.physics.as_mut() {
                p.destroy_body(body);
            }
        }

        match kind {
            SceneKind::MapRecipe | SceneKind::LegacyDump => {
                let Some((renderer, render_ctx)) =
                    self.renderer.as_mut().zip(self.render_ctx.as_ref())
                else {
                    return;
                };
                match crate::load_map(&mut self.world, renderer, render_ctx, path) {
                    Ok(result) => {
                        info!("Loaded map {path} ({:?})", result.kind);
                        self.after_scene_load(path);
                        self.pending_map_load = Some(result);
                    }
                    Err(error) => {
                        warn!("LoadScene failed: {error}");
                        if let Some(ui) = self.ui_manager.as_mut() {
                            ui.push_toast("Scene failed to load");
                        }
                    }
                }
            }
            SceneKind::Schema => self.load_schema_scene(path, &document),
            SceneKind::Unsupported(version) => {
                warn!(version, "scene version this build does not read");
                if let Some(ui) = self.ui_manager.as_mut() {
                    ui.push_toast(&format!("Scene version {version} is not supported"));
                }
            }
        }
    }

    /// Rebuild the world from a version-3 schema scene, then rebuild the GPU
    /// state the file deliberately does not contain.
    ///
    /// A scene stores *authored* facts. Vertex offsets, renderer material
    /// slots and terrain GPU buffers are session state — CONTROL-D was
    /// explicit that runtime pool ids must never reach disk — so loading one
    /// means reconstructing all three from what the file does say.
    fn load_schema_scene(&mut self, path: &str, document: &serde_json::Value) {
        if let Some((renderer, render_ctx)) = self.renderer.as_mut().zip(self.render_ctx.as_ref()) {
            renderer.wait_gpu(render_ctx);
            renderer.reset_scene_gpu();
        }
        for entity in self.world.entities().collect::<Vec<_>>() {
            self.world.despawn(entity);
        }

        let report = match crate::scene_schema::scene_from_json(
            &mut self.world,
            &self.type_registry,
            document,
        ) {
            Ok(report) => report,
            Err(error) => {
                warn!(%error, "scene failed to load");
                if let Some(ui) = self.ui_manager.as_mut() {
                    ui.push_toast("Scene failed to load");
                }
                return;
            }
        };
        for warning in &report.warnings {
            if let Some(ui) = self.ui_manager.as_mut() {
                ui.append_log(&format!("[scene] {} — {}", warning.entity, warning.message));
            }
        }

        self.reconstruct_scene_gpu(path);
        crate::propagate_transforms(&mut self.world);
        info!(
            "Loaded scene {path} ({} entities, {} warnings)",
            report.entities.len(),
            report.warnings.len()
        );
        self.after_scene_load(path);
    }

    /// Re-upload the GPU state a schema scene does not carry.
    fn reconstruct_scene_gpu(&mut self, path: &str) {
        // ── primitives ──────────────────────────────────────────────────────
        // A `MeshKind` says what the geometry *is*; the `MeshComponent` beside
        // it holds offsets into a buffer that no longer exists. Regenerating
        // and overwriting in place keeps the entity's identity, its parent and
        // its authored material — which is why this is not the game layer's
        // despawn-and-respawn auto-attach.
        let primitives: Vec<_> = self
            .world
            .entities()
            .filter_map(|entity| Some((entity, *self.world.get::<MeshKind>(entity)?)))
            .collect();
        if !primitives.is_empty()
            && let Some((renderer, render_ctx)) =
                self.renderer.as_mut().zip(self.render_ctx.as_ref())
        {
            for (entity, kind) in primitives {
                let (vertices, indices) = match kind {
                    MeshKind::Cube => somnium_asset::generate_cube(1.0),
                    MeshKind::Plane => somnium_asset::generate_plane(1.0, 1),
                    MeshKind::Sphere => somnium_asset::generate_sphere(0.5, 16, 16),
                    MeshKind::Cylinder => somnium_asset::generate_cylinder(0.5, 1.0, 16),
                };
                let material = self
                    .world
                    .get::<MaterialComponent>(entity)
                    .map_or(0, |material| material.runtime_id);
                let alloc =
                    renderer
                        .geometry
                        .upload_mesh(&render_ctx.queue, &vertices, &indices, material);
                let mesh = MeshComponent {
                    vertex_offset: alloc.vertex_offset,
                    index_offset: alloc.index_offset,
                    index_count: alloc.index_count,
                };
                if let Some(existing) = self.world.get_mut::<MeshComponent>(entity) {
                    *existing = mesh;
                } else {
                    let _ = self.world.insert_component(entity, mesh);
                }
            }
        }

        // ── terrain sidecars ────────────────────────────────────────────────
        // Heightmaps and splatmaps are megabytes of painted data and live
        // beside the scene rather than inside it. Each is named after the
        // scene, so moving a scene moves its terrain with it.
        let terrains: Vec<_> = self
            .world
            .entities()
            .filter_map(|entity| self.world.get::<TerrainComponent>(entity).copied())
            .collect();
        for component in terrains {
            let sidecar = format!("{path}.terrain{}.bin", component.terrain_id);
            if !std::path::Path::new(&sidecar).exists() {
                continue;
            }
            let Some(renderer) = self.renderer.as_mut() else {
                break;
            };
            let Some(terrain) = renderer.terrain_mut(component.terrain_id) else {
                warn!(sidecar, "no renderer terrain to restore into");
                continue;
            };
            match terrain.load_binary(&sidecar) {
                Ok(()) => info!("Terrain {} restored from {sidecar}", component.terrain_id),
                Err(error) => warn!(%error, "terrain sidecar failed to load"),
            }
        }

        // ── materials ───────────────────────────────────────────────────────
        // Authored `AssetId`s resolve back to renderer pool slots by the same
        // path an ordinary edit uses, so there is one implementation of
        // "what does this material mean right now".
        self.sync_authored_material_components();
    }

    /// The state every route resets after a scene arrives.
    ///
    /// One function because "undo is cleared" and "the scene is not dirty" are
    /// facts about *having loaded*, not about which format was loaded, and
    /// three copies of them would eventually disagree.
    fn after_scene_load(&mut self, path: &str) {
        self.remember_recent_scene(std::path::Path::new(path));
        let name = std::path::Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        if let Some(ui) = self.ui_manager.as_mut() {
            ui.set_scene_name(name);
        }
        self.selection.clear();
        self.material_sessions.clear();
        // Scene load is not undoable: the stack describes a world that no
        // longer exists. CONTROL-J states this rather than leaving entries
        // that would corrupt the new scene if replayed.
        self.undo_stack = UndoStack::new(128);
        self.scene_dirty = false;
        self.terrain_edit_active = false;
        self.terrain_stroke = None;
        self.after_selection_change();
    }

    /// Show a file in the OS file browser.
    ///
    /// The fallback when no external editor is configured. It cannot open at
    /// the line — no file browser can — which is exactly why the setting
    /// exists and why the Preferences row for it says so.
    fn reveal_in_file_browser(&mut self, file: &str) {
        let path = std::path::Path::new(file);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.config.content_root.join(path)
        };
        let target = if path.exists() {
            path
        } else {
            let Some(parent) = path.parent().map(std::path::Path::to_path_buf) else {
                return;
            };
            parent
        };
        let launched = if cfg!(target_os = "windows") {
            std::process::Command::new("explorer").arg(&target).spawn()
        } else if cfg!(target_os = "macos") {
            std::process::Command::new("open").arg(&target).spawn()
        } else {
            std::process::Command::new("xdg-open").arg(&target).spawn()
        };
        if let Err(error) = launched {
            warn!(%error, "could not reveal the file");
            if let Some(ui) = self.ui_manager.as_mut() {
                ui.push_toast("Could not reveal the file");
            }
        }
    }

    /// Resolve a settings field by name. `None` for a stale address, so a
    /// renamed setting makes a control inert rather than panicking.
    fn settings_field_id(
        &self,
        component: somnium_ecs::reflect::StableId,
        field_name: &str,
    ) -> Option<somnium_ecs::reflect::FieldId> {
        self.settings
            .registry()
            .by_stable_id(component)?
            .fields
            .iter()
            .find(|field| field.name == field_name)
            .map(|field| field.id)
    }

    /// Record a scene in the recent list, newest first, without duplicates.
    fn remember_recent_scene(&mut self, path: &std::path::Path) {
        let path = path.to_path_buf();
        self.recent_scenes.retain(|existing| existing != &path);
        self.recent_scenes.insert(0, path);
        self.recent_scenes
            .truncate(crate::settings::RECENT_SCENE_LIMIT);
        crate::settings::save_recent_scenes(&self.recent_scenes);
    }

    /// Push the resolved settings into the systems that consume them.
    ///
    /// Called after every write, so the effect of a preference is immediate
    /// rather than waiting for a restart. Snapping, the gizmo pivot and the
    /// select-only mode all read the store directly each frame; what needs
    /// pushing is the state that lives somewhere else.
    fn apply_settings(&mut self) {
        let root = std::path::PathBuf::from(&self.settings.project().content_root);
        if root != self.config.content_root {
            self.config.content_root = root;
            self.next_asset_scan = std::time::Instant::now();
            self.asset_scan_stamp = None;
        }
        if let Some(ui) = self.ui_manager.as_mut() {
            ui.set_tooltip_delay_ms(self.settings.editor().tooltip_delay_ms);
        }
    }

    /// Ground height under a world position, if any terrain is beneath it.
    ///
    /// A downward ray from well above rather than a heightfield lookup,
    /// because the terrain the editor draws is the terrain the renderer holds
    /// and its raycast is the one function that already accounts for the
    /// terrain's own model matrix.
    fn surface_height_at(&mut self, position: glam::Vec3) -> Option<f32> {
        let terrains: Vec<_> = self
            .world
            .entities()
            .filter_map(|entity| {
                let component = self.world.get::<TerrainComponent>(entity)?;
                let model = self
                    .world
                    .get::<Transform>(entity)
                    .map_or(glam::Mat4::IDENTITY, Transform::to_matrix);
                Some((component.terrain_id, model))
            })
            .collect();
        let renderer = self.renderer.as_mut()?;
        let origin = glam::Vec3::new(position.x, position.y + 10_000.0, position.z);
        let mut best: Option<f32> = None;
        for (id, model) in terrains {
            let Some(terrain) = renderer.terrain_mut(id) else {
                continue;
            };
            terrain.model = model;
            if let Some(hit) = terrain.raycast(origin, glam::Vec3::NEG_Y) {
                best = Some(best.map_or(hit.y, |current: f32| current.max(hit.y)));
            }
        }
        best
    }

    /// Every pickable entity under the cursor, nearest first.
    ///
    /// The same ray and the same AABB test the drag-and-drop probe uses, kept
    /// as one function so the piercing menu can never disagree with what a
    /// plain click would have hit.
    fn entities_under_cursor(&self) -> Vec<somnium_ecs::entity::Entity> {
        let Some((origin, direction)) = self.cursor_ray() else {
            return Vec::new();
        };
        let Some(renderer) = self.renderer.as_ref() else {
            return Vec::new();
        };
        let mut hits: Vec<_> = self
            .world
            .entities()
            .filter(|entity| {
                let flags = self
                    .world
                    .get::<EditorFlags>(*entity)
                    .copied()
                    .unwrap_or_default();
                !flags.locked && !flags.hidden
            })
            .filter_map(|entity| {
                entity_ray_hit_distance(&self.world, renderer, entity, origin, direction)
                    .map(|distance| (distance, entity))
            })
            .collect();
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        hits.into_iter().map(|(_, entity)| entity).collect()
    }

    /// The centre and radius the camera should frame: the selection when
    /// there is one, and nothing otherwise.
    fn focus_target(&self) -> Option<(glam::Vec3, f32)> {
        let points: Vec<glam::Vec3> = self
            .selection
            .as_slice()
            .iter()
            .filter_map(|entity| {
                self.world
                    .get::<WorldTransform>(*entity)
                    .map(|world| world.0.to_scale_rotation_translation().2)
                    .or_else(|| {
                        self.world
                            .get::<Transform>(*entity)
                            .map(|transform| transform.translation)
                    })
            })
            .collect();
        let first = points.first().copied()?;
        let mut min = first;
        let mut max = first;
        for point in &points {
            min = min.min(*point);
            max = max.max(*point);
        }
        Some(((min + max) * 0.5, ((max - min).length() * 0.5).max(2.0)))
    }

    /// Frame the selection with the editor camera.
    ///
    /// The pivot is the centroid of the selected transforms, and the distance
    /// comes from how far apart they are, so `F` on one object and `F` on
    /// twelve both end up looking at the thing the user meant.
    fn focus_camera_on_selection(&mut self) {
        // The minimum radius inside `focus_target` keeps a single point from
        // putting the camera inside the object it was asked to look at.
        let Some((centre, radius)) = self.focus_target() else {
            return;
        };
        // The editor camera belongs to the game layer, so this is a request
        // rather than a write — the same shape `camera_speed_request` already
        // uses. CONTROL-G builds bookmarks, orbit and view presets on this
        // channel rather than opening a second one.
        self.camera_focus_request = Some((centre, radius));
    }

    /// Everything that must follow a selection change, whatever caused it.
    ///
    /// Extracted in CONTROL-F because there are now four callers — plain
    /// click, modifier click, marquee/paste and the legacy single-selection
    /// event — and a gizmo left on a deselected entity is exactly the class of
    /// bug that only shows up on the fourth one.
    fn after_selection_change(&mut self) {
        // A new selection means the old baselines describe values that are no
        // longer on screen.
        if let Some(ui) = &mut self.ui_manager {
            ui.reset_inspector_baseline();
        }
        // Leaving the terrain entity exits terrain edit mode.
        if self.terrain_edit_active && self.selected_terrain().is_none() {
            self.terrain_edit_active = false;
            self.terrain_stroke = None;
        }
        self.refresh_gizmo_anchor();
    }

    /// Put the transform gizmo where the primary selection actually is.
    ///
    /// Called once per frame as well as on selection change, and that is the
    /// point. The anchor used to be pushed to the renderer only when the
    /// selection changed, which quietly meant the gizmo tracked *selection
    /// events* rather than the selected entity:
    ///
    /// * `Create` sets `selection.primary` through the undo stack without
    ///   raising a selection event, so a newly created Audio Emitter — or
    ///   light, or particle emitter — arrived with the gizmo still parked on
    ///   whatever was selected before it, or with no gizmo at all. The
    ///   Details panel edited it perfectly; the viewport had no handle on it.
    ///   That is the bug this function exists for.
    /// * Undo, Redo and a typed Details translation all move an entity
    ///   without changing the selection, and all left the gizmo behind.
    ///
    /// A value recomputed from the world every frame cannot drift out of
    /// step with it, which is why this replaced the push rather than gaining
    /// another caller.
    fn refresh_gizmo_anchor(&mut self) {
        // Terrain sculpting, foliage painting and Play each own the viewport
        // and have already cleared the gizmo; re-placing it here would undo
        // that every frame.
        if self.play_session_active || self.terrain_edit_active || self.foliage_paint_active {
            return;
        }
        let anchor = gizmo_anchor(&self.world, self.selection.primary);
        if let Some(renderer) = self.renderer.as_mut() {
            match anchor {
                Some(pos) => {
                    let rotation = if self.settings.editor().gizmo_local_space {
                        self.selection
                            .primary
                            .and_then(|entity| self.world.get::<WorldTransform>(entity))
                            .map_or(glam::Quat::IDENTITY, |world| {
                                world.0.to_scale_rotation_translation().1
                            })
                    } else {
                        glam::Quat::IDENTITY
                    };
                    renderer.set_gizmo_world_transform(pos, rotation);
                }
                None => renderer.clear_gizmo(),
            }
        }
    }

    /// How many attachments the selection carries, if it carries a
    /// `ScriptSet` at all.
    fn selected_script_count(&self) -> Option<usize> {
        let entity = self.selection.primary?;
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
        let Some(entity) = self.selection.primary else {
            return;
        };
        let command = build(entity.index());
        self.undo_stack
            .push(command, &mut self.world, &mut self.selection.primary);
        self.scene_dirty = true;
    }

    /// Import a `.luau` file and attach it to the selection.
    fn attach_script(&mut self, path: &std::path::Path) {
        match self.scripts.import_script_file(path) {
            Ok(asset) => {
                if self.selection.primary.is_none() {
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
        let root = self.config.content_root.clone();
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

    /// Where the project keeps its string tables.
    ///
    /// A directory rather than a file, one table per locale, because that is
    /// what a translator is handed and what a git diff is readable in.
    fn locale_dir(&self) -> std::path::PathBuf {
        self.config.content_root.join("locale")
    }

    /// Load the project's catalogue and hand the editor the table for it.
    ///
    /// MORROWIND-M item 2. The projection happens here, in the one crate that
    /// knows both `somnium_i18n` and `somnium_ui` — the editor is given a
    /// `DataTable` and never learns what a catalogue is.
    fn load_localisation(&mut self) {
        let dir = self.locale_dir();
        if !dir.is_dir() {
            // A project with no translations is the ordinary case, not an
            // error. The panel opens empty and says nothing alarming.
            return;
        }
        match crate::i18n::load_catalog(&dir, "en") {
            Ok(catalog) => {
                let table = crate::i18n::catalog_to_table(&catalog);
                if let Some(ui) = self.ui_manager.as_mut() {
                    ui.set_localisation_table(table);
                }
                self.locale_catalog = Some(catalog);
            }
            Err(error) => {
                error!("{error}");
                if let Some(ui) = self.ui_manager.as_mut() {
                    ui.append_log(&format!("[locale] {error}"));
                }
            }
        }
    }

    /// Write the edited table back, one file per locale.
    fn save_localisation(&mut self) {
        let dir = self.locale_dir();
        let Some(table) = self
            .ui_manager
            .as_ref()
            .and_then(UiManager::localisation_table)
            .cloned()
        else {
            return;
        };
        // The loaded catalogue is the template: it carries the display name and
        // the font list, which a grid of strings cannot hold and which a save
        // that dropped them would cost a language its typeface.
        let template = self
            .locale_catalog
            .clone()
            .unwrap_or_else(|| somnium_i18n::Catalog::new("en"));
        let catalog = crate::i18n::table_to_catalog(&table, &template);
        match crate::i18n::save_catalog(&dir, &catalog) {
            Ok(()) => {
                let locales = catalog.locales().len();
                self.locale_catalog = Some(catalog);
                // Rescan: the files just changed on disk, and the drawer is
                // showing them.
                self.next_asset_scan = std::time::Instant::now();
                if let Some(ui) = self.ui_manager.as_mut() {
                    ui.append_log(&format!(
                        "[locale] saved {locales} locale(s) to {}",
                        dir.display()
                    ));
                    ui.push_toast("Localisation saved");
                }
            }
            Err(error) => self.report_content_error(&dir, &error),
        }
    }

    /// Hand the table to a translator as one CSV.
    fn export_localisation_csv(&mut self) {
        let Some(table) = self
            .ui_manager
            .as_ref()
            .and_then(UiManager::localisation_table)
            .cloned()
        else {
            return;
        };
        let path = self.locale_dir().join("localisation.csv");
        match std::fs::create_dir_all(self.locale_dir())
            .and_then(|()| std::fs::write(&path, table.to_csv()))
        {
            Ok(()) => {
                self.next_asset_scan = std::time::Instant::now();
                if let Some(ui) = self.ui_manager.as_mut() {
                    ui.append_log(&format!("[locale] exported {}", path.display()));
                    ui.push_toast("Exported localisation.csv");
                }
            }
            Err(error) => self.report_content_error(&path, &format!("{error}")),
        }
    }

    /// Write a new `.luau` file from the template and attach it.
    ///
    /// Never overwrites: a name that exists gets a numeric suffix. Losing
    /// someone's script to a menu click is not a recoverable mistake.
    fn create_script(&mut self) {
        let folder = self.config.content_root.join("scripts");
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
        let Some(entity) = self.selection.primary else {
            return;
        };
        if live {
            // Mid-drag: apply it, do not record it. The gesture's final
            // value arrives once with `live == false` and becomes the one
            // undo step — the same convention property scrubs use.
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
        let entity = self.selection.primary?;
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
        // A snapshot before the risky operation. `WorldCheckpoint` restores
        // the ECS but explicitly not the renderer's terrain and map state, so
        // a file on disk is the only thing that survives a play session that
        // ends badly enough to need it.
        self.write_autosave(crate::autosave::AutosaveReason::BeforePlay);
        self.audio_scene.stop_all();
        self.play_checkpoint = Some(crate::script_input::WorldCheckpoint::capture(
            &mut self.world,
            &self.type_registry,
        ));
        self.script_step = 0;
        self.scripts.runtime_mut().set_world_seed(SCRIPT_WORLD_SEED);
    }

    /// Tear every script down and restore the world exactly as it was.
    fn end_play_session(&mut self) {
        self.audio_scene.stop_all();
        let mut services = crate::script_host::HostServices {
            physics: self.physics.as_mut(),
            audio: self.audio.as_mut(),
            ui: self.game.ui_documents(),
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
    let result = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn();
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

fn edit_content_asset(path: &std::path::Path) -> Result<(), String> {
    if !path.is_file() {
        return Err("that asset is no longer there".to_string());
    }
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(path)
        .spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(path).spawn();
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
                &mut self.jobs,
                self.render_ctx.as_ref(),
                self.renderer.as_mut(),
                &mut self.selection.primary,
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
            // MORROWIND-I. Created invisible and shown at the end of this
            // block. `accesskit_winit` *panics* if its adapter is attached to a
            // window that has already been shown, so the accessibility adapter
            // has to be built in the gap. It is a better startup regardless:
            // the window appears painted rather than appearing and then
            // painting.
            .with_visible(false)
            .with_decorations(false);
        #[cfg(target_os = "windows")]
        {
            attrs = attrs.with_undecorated_shadow(true);
        }

        match event_loop.create_window(attrs) {
            Ok(window) => {
                // MORROWIND-I, in the gap before the window is shown.
                self.a11y = Some(crate::a11y_bridge::A11yBridge::new(event_loop, &window));

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

                // MORROWIND-M item 2. Read once at startup rather than on the
                // asset inventory's job: a catalogue is a handful of files, and
                // rebuilding the table under a translator every time anything
                // in the project changed would throw away their scroll
                // position and their in-flight edit.
                self.load_localisation();

                // MORROWIND-J step 2, opened from the environment so a
                // headless run can exercise the second surface.
                for kind in somnium_ui::floating::FloatingKind::from_env() {
                    self.pending_float.push(kind);
                    if let Some(ui) = self.ui_manager.as_mut() {
                        ui.set_panel_floating(kind, true);
                    }
                }

                // MORROWIND-I. Everything is initialised; show the window.
                window.set_visible(true);

                self.state = LifecycleState::Running;
                // Craft defect C11's other half: work that survived a crash is
                // offered on the next launch, once, rather than sitting in a
                // folder nobody looks in.
                self.check_crash_recovery();

                let mut ctx = EngineContext::new(
                    &self.time,
                    &self.config,
                    &mut self.world,
                    self.physics.as_mut().unwrap(),
                    self.audio.as_mut().unwrap(),
                    &mut self.jobs,
                    self.render_ctx.as_ref(),
                    self.renderer.as_mut(),
                    &mut self.selection.primary,
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
                &mut self.jobs,
                self.render_ctx.as_ref(),
                self.renderer.as_mut(),
                &mut self.selection.primary,
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
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.state != LifecycleState::Running {
            return;
        }

        // MORROWIND-J step 2. The id was ignored while there was one window,
        // and that is precisely the assumption a second window breaks. Without
        // this line the floating log's `Resized` reaches the main render
        // context, which then scissors the editor's 1920x1032 frame against a
        // 900x420 target — a validation error rather than a wrong picture, and
        // the first thing that happened when this was tried.
        let route = self.floating_window_event(window_id, &event);
        if route == FloatingRoute::Handled {
            return;
        }
        // The floating viewport's widgets have already declined this event, so
        // feeding it to the main window's tree as well would hover and click
        // whatever the editor happens to have at the same coordinates.
        let from_viewport_window = route == FloatingRoute::Viewport;

        // Always track cursor position (needed for gizmo picking every frame).
        if let WindowEvent::CursorMoved { position, .. } = &event {
            self.cursor_pos = (position.x as f32, position.y as f32);
            if let Some(band) = self.marquee.as_mut() {
                band.current = self.cursor_pos;
            }
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

        // Moved to a display with a different DPI.
        //
        // Windows follows this with a `Resized` carrying the new physical size,
        // and the shell lays out in physical pixels, so the layout does follow
        // the window across monitors. What it does *not* do is change apparent
        // size: 13 px of text is 13 device pixels on both displays, so the
        // editor reads smaller on the denser one. Fixing that means moving
        // layout to logical units, which `FontAtlas::render_scale` names as its
        // own piece of work rather than a constant to flip here.
        if let WindowEvent::ScaleFactorChanged { scale_factor, .. } = &event {
            info!(scale_factor, "display scale changed");
            if let (Some(ui), Some(window)) = (&mut self.ui_manager, &self.window) {
                ui.reposition_panels(window);
            }
        }

        // Handle Resizing
        if let WindowEvent::Resized(size) = &event {
            self.viewport_size_hint = (size.width as f32, size.height as f32);
            if let Some(r_ctx) = &mut self.render_ctx {
                r_ctx.resize(size.width, size.height);
            }
            self.resize_scene_targets();
            if let (Some(ui), Some(window)) = (&mut self.ui_manager, &self.window) {
                ui.reposition_panels(window);
            }
        }

        // ── 1. Registered editor shortcuts FIRST (never array-position dispatch) ──
        //
        // **This dispatcher runs before the UI is consulted**, and every arm
        // below `return`s — so a key it claims never reaches the game at all.
        // That is what made bare `S` unusable for viewport flight: `S` is bound
        // to the Scale tool, `SetGizmoMode` returned, and `move_backward` never
        // latched. It appeared to work a couple of seconds later only because
        // the `!key_ev.repeat` guard lets OS auto-repeat through, so the camera
        // started moving exactly when the keyboard began repeating.
        //
        // While the fly-cam is driving, an *unmodified* shortcut stands down
        // and the key falls through. Modified chords are unaffected: `Ctrl+S`
        // is unambiguous and should still save mid-flight, which is also why
        // `shortcut_preserves_game_key` continues to forward its key
        // transition.
        // Two ways the game owns the keyboard: the fly-cam is driving, or a
        // play session is running. Both were reported as the same symptom —
        // `S` moving backward only after two or three seconds — because both
        // end at the same place: this dispatcher claims the key and returns.
        //
        // `play_session_active` is read from the engine's own flag rather than
        // the UI's, so the rule still holds in a headless or UI-less run.
        let game_owns_keyboard = self.play_session_active
            || self
                .ui_manager
                .as_ref()
                .is_some_and(somnium_ui::UiManager::viewport_camera_active);
        if let WindowEvent::KeyboardInput { event: key_ev, .. } = &event {
            if key_ev.state == winit::event::ElementState::Pressed && !key_ev.repeat {
                use winit::keyboard::PhysicalKey;
                if let PhysicalKey::Code(code) = key_ev.physical_key {
                    let action =
                        shortcut_action_for(code, self.shortcut_modifiers, game_owns_keyboard);
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
                        Some(A::SaveScene) => {
                            self.handle_editor_event(EditorEvent::SaveScene);
                            if shortcut_preserves_game_key(A::SaveScene) {
                                if let Some(engine_event) = translate_window_event(&event) {
                                    self.forward_engine_event(event_loop, engine_event);
                                }
                            }
                            return;
                        }
                        Some(A::Undo) => {
                            self.handle_editor_event(EditorEvent::Undo);
                            return;
                        }
                        Some(A::Redo) => {
                            self.handle_editor_event(EditorEvent::Redo);
                            return;
                        }
                        Some(A::DeleteSelected) => {
                            self.handle_editor_event(EditorEvent::DeleteSelected);
                            return;
                        }
                        Some(A::DuplicateSelected) => {
                            self.handle_editor_event(EditorEvent::DuplicateSelected);
                            return;
                        }
                        Some(A::SetGizmoMode(mode)) => {
                            self.handle_editor_event(EditorEvent::SetGizmoMode(mode));
                            return;
                        }
                        Some(A::ToggleTerrainEdit) => {
                            self.handle_editor_event(EditorEvent::ToggleTerrainEdit);
                            return;
                        }
                        Some(A::ToggleFoliage) => {
                            self.handle_editor_event(EditorEvent::ToggleFoliage);
                            return;
                        }
                        Some(A::ReloadScripts) => {
                            self.handle_editor_event(EditorEvent::ReloadScripts);
                            return;
                        }
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
                            self.announce_brush();
                        }
                        WKC::BracketRight if self.terrain_edit_active => {
                            self.terrain_brush.radius =
                                (self.terrain_brush.radius * 1.25).min(128.0);
                            self.announce_brush();
                        }
                        WKC::Minus if self.terrain_edit_active => {
                            self.terrain_brush.strength =
                                (self.terrain_brush.strength - 0.1).max(0.05);
                            self.announce_brush();
                        }
                        WKC::Equal if self.terrain_edit_active => {
                            self.terrain_brush.strength =
                                (self.terrain_brush.strength + 0.1).min(1.0);
                            self.announce_brush();
                        }
                        WKC::Comma if self.terrain_edit_active => {
                            self.terrain_brush.paint_layer =
                                self.terrain_brush.paint_layer.checked_sub(1).unwrap_or(
                                    somnium_renderer::terrain::textures::TERRAIN_LAYER_COUNT
                                        as usize
                                        - 1,
                                );
                            self.announce_brush();
                        }
                        WKC::Period if self.terrain_edit_active => {
                            self.terrain_brush.paint_layer = (self.terrain_brush.paint_layer + 1)
                                % somnium_renderer::terrain::textures::TERRAIN_LAYER_COUNT as usize;
                            self.announce_brush();
                        }
                        // Cycle the dab mask. Next to the layer keys because
                        // it is a painting decision, not a sculpting one.
                        WKC::Slash if self.terrain_edit_active => {
                            self.terrain_brush.alpha = self.terrain_brush.alpha.next();
                            self.announce_brush();
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

        // ── 2.9 Accessibility (MORROWIND-I) ──────────────────────────────────
        //
        // Every event, not only the ones that look relevant: the adapter tracks
        // window focus and geometry, and a reader whose idea of where the
        // window is has gone stale points at the wrong place on screen. Before
        // the early returns below for the same reason.
        if let (Some(a11y), Some(window)) = (self.a11y.as_mut(), self.window.as_ref()) {
            // Not another window's events: the adapter tracks *this* window's
            // geometry, and telling it about a click that happened somewhere
            // else moves a reader's cursor to a place nothing was clicked.
            if !from_viewport_window {
                a11y.process_event(window, &event);
            }
        }

        // ── 3. Route to native UI; return early if consumed ──────────────────
        let ui_consumed = match (&mut self.ui_manager, from_viewport_window) {
            (Some(ui), false) => ui.process_os_event(&event),
            // Already offered to the widget tree, rooted at the floating
            // viewport, and declined there.
            _ => false,
        };
        if ui_consumed {
            return;
        }

        // Once the editor has declined the event, feed the same physical
        // transition to the action system. Scripts never see this hardware
        // event; they sample the named actions evaluated from it.
        if self.play_session_active {
            self.input.handle_window_event(&event);
        }

        // ── 3.2 Route to the game (MORROWIND-E2) ─────────────────────────────
        //
        // After the editor shell and before the editor's own viewport tools, so
        // a game's HUD can take a click the shell did not want but the sculpt
        // brush would have. See `GameApp::on_os_event` for why the order is
        // MORROWIND-N's to revisit.
        {
            let mut ctx = EngineContext::new(
                &self.time,
                &self.config,
                &mut self.world,
                self.physics.as_mut().unwrap(),
                self.audio.as_mut().unwrap(),
                &mut self.jobs,
                self.render_ctx.as_ref(),
                self.renderer.as_mut(),
                &mut self.selection.primary,
                self.ui_manager.as_mut().unwrap(),
                crate::camera_speed_from_normalized(self.camera_speed_norm),
                self.simulation_clock,
                &mut self.scripts,
            );
            if self.game.on_os_event(&mut ctx, &event) {
                return;
            }
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
                // Godot 4.6's select-only mode. A real bug class rather than a
                // preference: without it a click that happens to land on a
                // gizmo axis moves the object you were only trying to select,
                // and the damage is silent until you notice it later.
                //
                // The mode had the opposite failure, and a worse one. It is
                // persisted in `editor.toml`, so once it is on it stays on
                // across every session — and it used to swallow the press with
                // no word of explanation. Every gizmo in the editor was inert,
                // for translate and rotate and scale alike, and nothing
                // anywhere said why. A mode that refuses a gesture has to
                // admit it at the moment it refuses, or it is indistinguishable
                // from a broken feature.
                let started = try_start_gizmo_drag(
                    self.renderer.as_ref(),
                    &self.world,
                    &self.selection.primary,
                    self.cursor_pos,
                    self.viewport_size(),
                    self.settings.editor().gizmo_local_space,
                );
                let blocked = started.is_some() && self.settings.editor().select_only;
                if blocked {
                    let message =
                        "Select Only is on — click it in the toolbar to move, rotate and scale";
                    info!("{message}");
                    if let Some(ui) = self.ui_manager.as_mut() {
                        ui.push_toast(message);
                    }
                }
                let drag = if blocked {
                    None
                } else {
                    started.and_then(|mut drag| {
                        drag.followers = capture_followers(
                            &self.world,
                            self.selection.as_slice(),
                            drag.entity_index,
                        )?;
                        Some(drag)
                    })
                };
                if drag.is_some() {
                    self.gizmo_drag = drag;
                    gizmo_consumed = true;
                } else {
                    // No gizmo axis under the pointer, so a left press in the
                    // viewport is the start of a marquee. It only becomes one
                    // once it is dragged; a plain click still selects.
                    self.marquee = Some(crate::selection::Marquee::new(self.cursor_pos));
                }
            }

            if let WindowEvent::MouseInput {
                state: winit::event::ElementState::Released,
                button: winit::event::MouseButton::Left,
                ..
            } = &event
            {
                if let Some(band) = self.marquee.take() {
                    if band.is_dragged() {
                        self.apply_marquee(band);
                    } else {
                        let hit = self.entities_under_cursor().into_iter().next();
                        self.selection.set_single(hit);
                        self.after_selection_change();
                    }
                    gizmo_consumed = true;
                }
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
            self.forward_engine_event(event_loop, engine_event);
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
            winit::event::DeviceEvent::MouseMotion { delta } => {
                if self.play_session_active {
                    self.input
                        .add_mouse_delta(glam::Vec2::new(delta.0 as f32, delta.1 as f32));
                }
                Some(EngineEvent::MouseMotion {
                    delta_x: delta.0 as f32,
                    delta_y: delta.1 as f32,
                })
            }
            _ => None,
        };

        if let Some(ev) = engine_event {
            let mut ctx = EngineContext::new(
                &self.time,
                &self.config,
                &mut self.world,
                self.physics.as_mut().unwrap(),
                self.audio.as_mut().unwrap(),
                &mut self.jobs,
                self.render_ctx.as_ref(),
                self.renderer.as_mut(),
                &mut self.selection.primary,
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

        // PORTAL-0-B: the frame body starts here and ends at
        // `wait_for_frame_budget` below. Timed with a plain `Instant` rather
        // than a profiler CPU scope because a scope spanning the render call
        // would still be open when `GpuProfiler::end_frame` harvests them, and
        // that path warns about unclosed scopes for good reason. The value is
        // handed to the profiler at the end of the frame and read by the timing
        // harness during the *next* one; see `GpuProfiler::frame_cpu_ms`.
        let frame_body_started = std::time::Instant::now();

        self.time.tick();
        let dt = self.time.delta_time().as_secs_f32();
        if self.play_session_active {
            self.input.update(dt);
            self.script_input.capture(&self.input);
        }
        self.tick_autosave(dt);
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
        // MORROWIND-N: a step is a play frame that happens to be paused, so
        // scripts have to be reconciled for it too — otherwise stepping never
        // initialises an attachment added while the simulation was held.
        let stepping = self.simulation_clock.state == SimulationState::Paused
            && self.pending_steps > 0
            && self.play_session_active;
        self.stepping_now = stepping;
        if self.simulation_clock.state == SimulationState::Playing || stepping {
            self.sync_scripts(dt);
        }

        if self.simulation_clock.state != SimulationState::Paused || stepping {
            let fixed_dt = self.simulation_clock.fixed_delta_seconds;
            if stepping {
                // Exactly one step, and never the wall clock: the point of a
                // step is that it is the same size every time, so a slow frame
                // cannot turn one press into three.
                self.pending_steps -= 1;
                self.simulation_accumulator += fixed_dt;
            } else {
                self.simulation_accumulator += dt.min(0.1);
            }
            while self.simulation_accumulator >= fixed_dt {
                {
                    let mut ctx = EngineContext::new(
                        &self.time,
                        &self.config,
                        &mut self.world,
                        self.physics.as_mut().unwrap(),
                        self.audio.as_mut().unwrap(),
                        &mut self.jobs,
                        self.render_ctx.as_ref(),
                        self.renderer.as_mut(),
                        &mut self.selection.primary,
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
                        crate::character::read_physics_into_world(
                            &mut self.world,
                            physics,
                            fixed_dt,
                        );
                    }
                    self.script_fixed_update(fixed_dt, dt);
                    // Components → Jolt, after the command apply, so a
                    // script's write is the last word before integration.
                    if let Some(physics) = self.physics.as_mut() {
                        crate::character::write_world_into_physics(&mut self.world, physics);
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
        // Snapping is settings, not constants (Seam 4), and `command()` held
        // during the drag inverts it.
        let snap = SnapSettings {
            translate_m: self.settings.editor().snap_translate_m,
            rotate_deg: self.settings.editor().snap_rotate_deg,
            scale: self.settings.editor().snap_scale,
        }
        .inverted(
            self.ui_manager
                .as_ref()
                .is_some_and(somnium_ui::UiManager::command_modifier_held),
        );
        let viewport = self.viewport_size();
        let drag_result: Option<(u32, Transform)> = self.gizmo_drag.as_ref().and_then(|drag| {
            let (cam, inv_vp) = self
                .renderer
                .as_ref()
                .map(|r| (r.camera_pos, r.picking_view_proj().inverse()))
                .unwrap_or((glam::Vec3::ZERO, glam::Mat4::IDENTITY));
            let new_t = apply_gizmo_drag(drag, cam, inv_vp, self.cursor_pos, viewport, snap);
            Some((drag.entity_index, new_t))
        });
        if let Some((idx, mut new_t)) = drag_result {
            // Snap-to-surface, applied after grid snapping: the grid decides
            // where in the plane the object lands, and the ground decides how
            // high. Doing it the other way round would have the grid quantise
            // the terrain height, which is meaningless.
            if self.settings.editor().snap_to_surface
                && self
                    .gizmo_drag
                    .as_ref()
                    .is_some_and(|drag| drag.mode == GizmoMode::Translate)
                && let Some(mut world_position) = self
                    .gizmo_drag
                    .as_ref()
                    .map(|drag| drag.parent_world.transform_point3(new_t.translation))
                && let Some(height) = self.surface_height_at(world_position)
            {
                world_position.y = height;
                if let Some(drag) = self.gizmo_drag.as_ref() {
                    new_t.translation = drag.parent_world_inverse.transform_point3(world_position);
                }
            }
            let start = self
                .gizmo_drag
                .as_ref()
                .map_or(new_t, |drag| drag.start_transform);
            if let Some(entity) = self.world.find_entity_by_index(idx)
                && let Some(t) = self.world.get_mut::<Transform>(entity)
            {
                *t = new_t;
            }
            // The same delta, applied to every follower's own starting
            // transform. Rotation composes and scale multiplies, because that
            // is what those operations mean; translation adds.
            let world_offset = self.gizmo_drag.as_ref().map_or_else(
                || new_t.translation - start.translation,
                |drag| drag.parent_world.transform_point3(new_t.translation) - drag.gizmo_pos,
            );
            let spin = new_t.rotation * start.rotation.inverse();
            let growth = glam::Vec3::new(
                safe_ratio(new_t.scale.x, start.scale.x),
                safe_ratio(new_t.scale.y, start.scale.y),
                safe_ratio(new_t.scale.z, start.scale.z),
            );
            let pivot = self
                .settings
                .editor()
                .gizmo_pivot_centre
                .then(|| self.focus_target().map(|(centre, _)| centre))
                .flatten();
            let followers = self
                .gizmo_drag
                .as_ref()
                .map(|drag| drag.followers.clone())
                .unwrap_or_default();
            apply_followers(
                &mut self.world,
                &followers,
                world_offset,
                spin,
                growth,
                pivot,
            );
            if let Some(r) = &mut self.renderer {
                let world_position = self.gizmo_drag.as_ref().map_or(new_t.translation, |drag| {
                    drag.parent_world.transform_point3(new_t.translation)
                });
                r.set_gizmo_world_pos(world_position);
            }
        }

        {
            // Computed before the context borrows the world, because the
            // orbit pivot reads the selection's transforms.
            let camera_focus = self.camera_focus_request.take();
            let camera_pose = self.camera_pose_request.take();
            let orbit_selection = self.orbit_selection;
            let orbit_pivot = self.focus_target().map(|(centre, _)| centre);
            let mut ctx = EngineContext::new(
                &self.time,
                &self.config,
                &mut self.world,
                self.physics.as_mut().unwrap(),
                self.audio.as_mut().unwrap(),
                &mut self.jobs,
                self.render_ctx.as_ref(),
                self.renderer.as_mut(),
                &mut self.selection.primary,
                self.ui_manager.as_mut().unwrap(),
                crate::camera_speed_from_normalized(self.camera_speed_norm),
                self.simulation_clock,
                &mut self.scripts,
            );
            // Handed over on the update tick only: the game's camera moves in
            // `on_update`, so delivering it anywhere else would leave the
            // request sitting for a frame or be honoured twice.
            ctx.camera_focus = camera_focus;
            ctx.camera_pose = camera_pose;
            ctx.orbit_selection = orbit_selection;
            ctx.orbit_pivot = orbit_pivot;
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
            .filter(|entity| !crate::is_hidden(&self.world, *entity))
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
        // PORTAL-0-B: the editor's per-frame panel rebuild had no zone at
        // all, which is why `Frame wall` was the only number anyone could
        // quote about editor cost.
        if let Some(r) = &mut self.renderer {
            r.profiler.cpu_begin("Editor panels");
        }
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
                out: &mut Vec<somnium_ui::OutlinerRow>,
            ) {
                let has = children.get(&id).map(|c| !c.is_empty()).unwrap_or(false);
                out.push(somnium_ui::OutlinerRow {
                    id,
                    name: name_of.get(&id).cloned().unwrap_or_default(),
                    depth,
                    has_children: has,
                    hidden: false,
                    locked: false,
                    script_error: false,
                    tags: Vec::new(),
                });
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
            // One reconciliation point per frame. Commands, undo, redo, game
            // code and the drag routes all write `selection.primary` through
            // the `&mut Option<Entity>` shim; this is where the ordered set is
            // brought back into agreement with it and with the world, so no
            // stale or orphaned handle can reach a multi-entity command.
            // Row facts: the badges and the typed filters both read them, and
            // they are gathered here rather than in `walk` because they need
            // the world and `walk` is a pure tree flatten.
            for row in &mut tree {
                let Some(entity) = self.world.find_entity_by_index(row.id) else {
                    continue;
                };
                if let Some(flags) = self.world.get::<EditorFlags>(entity) {
                    row.hidden = flags.hidden;
                    row.locked = flags.locked;
                }
                row.tags = entity_tags(&self.world, entity);
            }
            self.selection.retain_alive(&self.world);
            self.selection.reconcile();
            self.outliner_order = tree
                .iter()
                .filter_map(|row| self.world.find_entity_by_index(row.id))
                .collect();
            let selected_idx = self.selection.primary.map(|e| e.index());
            let selected_ids: Vec<u32> = self
                .selection
                .as_slice()
                .iter()
                .map(|e| e.index())
                .collect();
            let sel_t = self
                .selection
                .primary
                .and_then(|e| self.world.get::<Transform>(e).copied());
            // Phase 17C: terrain layer + foliage settings for the inspector.
            let sel_terrain = self.selection.primary.and_then(|e| {
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
                .selection
                .primary
                .and_then(|e| self.world.get::<FoliageComponent>(e).cloned())
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
            // Phase 16-D: the Scripts section, built from what each
            // attached script declared. Computed before the `ui` borrow
            // because it reads the world and the script host.
            let sel_scripts = self.script_inspector_state();
            self.sync_material_sessions();
            self.sync_authored_material_components();
            self.ensure_material_session();
            let generated_panels = self
                .selection.primary
                .map(|entity| {
                    let editors =
                        somnium_ui::editor::property_editors::PropertyEditorRegistry::standard();
                    let rules = somnium_ui::editor::editing_rules::standard_editing_rules(
                        &self.type_registry,
                    );
                    let mut panels = self
                        .type_registry
                        .schemas_on(&self.world, entity)
                        .into_iter()
                        .filter_map(|schema| {
                            let values = (schema.snapshot)(&self.world, entity)?;
                            // CONTROL-F: with more than one entity selected the
                            // panel is the *intersection*. Building it here
                            // rather than in the widget layer is the whole
                            // point — Details receives the same model it always
                            // did, with `mixed` set on the rows that disagree.
                            if self.selection.len() > 1 {
                                let others: Vec<_> = self
                                    .selection
                                    .as_slice()
                                    .iter()
                                    .filter(|other| **other != entity)
                                    .map(|other| (schema.snapshot)(&self.world, *other))
                                    .collect();
                                if others.iter().any(Option::is_none) {
                                    // A member without this component drops the
                                    // whole section, not just its rows.
                                    return None;
                                }
                                let others: Vec<_> =
                                    others.into_iter().flatten().collect();
                                return Some(
                                    somnium_ui::editor::inspector_gen::generate_multi_component_panel(
                                        schema, &values, &others, &editors, &rules,
                                    ),
                                );
                            }
                            Some(somnium_ui::editor::inspector_gen::generate_component_panel(
                                schema, &values, &editors, &rules,
                            ))
                        })
                        .collect::<Vec<_>>();
                    // Asset contents are transient edit-session components,
                    // deliberately absent from the scene registry. They still
                    // use the exact same schema generator and widget tree.
                    let editor_registry = crate::reflect_registry::editor_registry();
                    if let Some(schema) = editor_registry.by_name("somnium.asset.Material")
                        && let Some(values) = (schema.snapshot)(&self.world, entity)
                    {
                        let mut panel = somnium_ui::editor::inspector_gen::generate_component_panel(
                            schema, &values, &editors, &rules,
                        );
                        panel.preview_path = self
                            .world
                            .get::<MaterialComponent>(entity)
                            .and_then(|component| self.material_documents.get(&component.asset))
                            .map(|document| document.path.clone());
                        panels.push(panel);
                    }
                    panels
                })
                .unwrap_or_default();
            // Settings are properties, so their panel is generated by exactly
            // the same call the entity inspector uses — against the settings
            // store's private world instead of the scene's.
            let (settings_panels, settings_overrides) = self.settings_panels();
            // Named after what changed, so a history of twenty rows is not
            // twenty rows reading "Change".
            let (history_entries, history_position) = {
                let (names, position) = self.undo_stack.history();
                (
                    names.into_iter().map(str::to_owned).collect::<Vec<_>>(),
                    position,
                )
            };
            // The corner axis widget's three directions, projected with the
            // live view matrix so it turns with the camera. The rotation is
            // taken without translation: the widget shows orientation, not
            // where the axes happen to be in the world.
            let axis_directions =
                self.renderer
                    .as_ref()
                    .map_or([(1.0, 0.0), (0.0, -1.0), (0.0, 1.0)], |renderer| {
                        let view = renderer.view_proj;
                        std::array::from_fn(|index| {
                            let world = match index {
                                0 => glam::Vec3::X,
                                1 => glam::Vec3::Y,
                                _ => glam::Vec3::Z,
                            };
                            let projected = view.transform_vector3(world);
                            let flat = glam::Vec2::new(projected.x, -projected.y);
                            let flat = flat.normalize_or_zero();
                            (flat.x, flat.y)
                        })
                    });
            let snap_state = (
                self.settings.editor().snap_translate_m,
                self.settings.editor().snap_rotate_deg,
                self.settings.editor().snap_to_surface,
                self.settings.editor().gizmo_local_space,
                self.settings.editor().select_only,
            );
            // The statistics overlay reports the frame the renderer actually
            // submitted, so it is read here, after the frame's draws exist.
            let viewport_statistics = self.settings.editor().show_statistics.then(|| {
                self.renderer.as_ref().map_or_else(
                    somnium_ui::debug::ViewportStats::default,
                    |renderer| {
                        let counters = renderer.profiler.counters;
                        let (width, height) = renderer.scene_extent();
                        somnium_ui::debug::ViewportStats {
                            draw_calls: counters.draw_calls,
                            instances: counters.instances,
                            triangles: counters.triangles,
                            terrain_chunks: counters.terrain_chunks,
                            shadow_casters: counters.shadow_casters,
                            resolution: (width, height),
                            resolution_scale: renderer.dynamic_resolution.scale(),
                            vram_bytes: 0,
                        }
                    },
                )
            });
            let drop_probe_entity = self.viewport_entity_drop_pick();
            let drop_probe_hit = self.viewport_terrain_drop_hit();
            if let Some(ui) = &mut self.ui_manager {
                ui.update_outliner_tree(&tree, selected_idx);
                ui.set_outliner_entity_handles(all_entities.iter().copied());
                ui.set_outliner_selection(selected_ids);
                ui.set_clipboard_filled(!self.entity_clipboard.is_empty());
                ui.set_history(history_entries, history_position);
                ui.set_viewport_statistics(viewport_statistics);
                ui.set_axis_widget(axis_directions);
                ui.set_snap_state(
                    snap_state.0,
                    snap_state.1,
                    snap_state.2,
                    snap_state.3,
                    snap_state.4,
                );
                if ui.preferences_open() {
                    ui.update_settings_panels(settings_panels, &settings_overrides);
                }
                ui.set_recent_scenes(
                    self.recent_scenes
                        .iter()
                        .map(|path| (path.to_string_lossy().into_owned(), path.exists()))
                        .collect(),
                );
                ui.set_marquee(
                    self.marquee
                        .filter(crate::selection::Marquee::is_dragged)
                        .map(|band| band.rect()),
                );
                ui.set_viewport_drop_probe(drop_probe_entity, drop_probe_hit);
                ui.set_fps(self.time.fps());
                // Phase 26-Zeta: the status bar is an instrument panel, so it
                // gets the same per-frame facts the Outliner does.
                ui.set_status_stats(tree.len(), self.time.fps());
                ui.set_status_selection(
                    selected_idx
                        .and_then(|idx| tree.iter().find(|row| row.id == idx))
                        .map(|row| row.name.as_str()),
                );
                if let Some(t) = sel_t {
                    let (rx, ry, rz) = t.rotation.to_euler(glam::EulerRot::XYZ);
                    ui.update_inspector(
                        self.selection.primary,
                        Some(t.translation.to_array()),
                        Some([rx.to_degrees(), ry.to_degrees(), rz.to_degrees()]),
                        Some(t.scale.to_array()),
                    );
                } else {
                    ui.update_inspector(None, None, None, None);
                }
                ui.update_generated_details(self.selection.primary, generated_panels);
                ui.update_terrain_inspector(sel_terrain);
                ui.update_foliage_inspector(sel_foliage);
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
                    // MORROWIND-B: background work, folded by name and
                    // sorted worst-wait first. Only shown when something ran,
                    // so an idle editor does not carry an empty heading.
                    if !self.job_profile.is_empty() {
                        rows.push(somnium_ui::ProfilerRow {
                            label: "— Jobs (CPU, background) —".to_string(),
                            value: String::new(),
                            depth: 0,
                        });
                        for job in &self.job_profile {
                            rows.push(somnium_ui::ProfilerRow {
                                label: format!("{} x{}", job.name, job.count),
                                // Queue wait first, because it is the number
                                // that explains a stall: run time says the work
                                // was slow, queue wait says the pool was busy,
                                // and those have different fixes.
                                value: format!(
                                    "wait {:.1} ms · ran {:.1} ms · {:?}{}",
                                    job.worst_queued_ms,
                                    job.ran_ms,
                                    job.priority,
                                    if job.expired > 0 {
                                        format!(" · {} expired", job.expired)
                                    } else {
                                        String::new()
                                    }
                                ),
                                depth: 1,
                            });
                        }
                        if self.job_zones_dropped > 0 {
                            rows.push(somnium_ui::ProfilerRow {
                                label: format!("and {} more", self.job_zones_dropped),
                                value: String::new(),
                                depth: 1,
                            });
                        }
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

        if let Some(r) = &mut self.renderer {
            {
                r.profiler.cpu_end();
            }
        }

        if let Some(r) = &mut self.renderer {
            r.profiler.cpu_begin("Jobs & assets");
        }
        self.pump_shader_reload();
        self.pump_jobs();
        self.update_asset_pipeline();
        if let Some(r) = &mut self.renderer {
            {
                r.profiler.cpu_end();
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
                &mut self.jobs,
                self.render_ctx.as_ref(),
                self.renderer.as_mut(),
                &mut self.selection.primary,
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

        // `on_render` is where a game publishes its active editor/player view
        // through `renderer.set_view`. Stream from that same-frame position,
        // not from a stale ECS settings transform or last frame's renderer.
        self.update_world_partition();

        // The active camera is the listener. Reconcile authored emitters only
        // after `on_render` publishes that same-frame view.
        if self.play_session_active
            && let (Some(audio), Some(renderer)) = (self.audio.as_mut(), self.renderer.as_ref())
        {
            let (_, orientation, _) = renderer
                .view_matrix
                .inverse()
                .to_scale_rotation_translation();
            self.audio_scene.update(
                &self.world,
                self.asset_gate.published(),
                audio,
                renderer.camera_pos,
                orientation,
                dt,
            );
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
        if let Some(r) = &mut self.renderer {
            r.profiler.cpu_begin("Scene submit");
        }
        self.submit_terrains();
        self.submit_foliage();
        self.submit_decals();
        self.sync_terrain_colliders();
        if let Some(r) = &mut self.renderer {
            {
                r.profiler.cpu_end();
            }
        }

        // The gizmo follows the entity, not the last selection event. After
        // the game layer has propagated transforms, so a child is anchored
        // where it is drawn rather than where its parent's origin is.
        self.refresh_gizmo_anchor();

        // ── Light gizmos (Phase 13E) ─────────────────────────────────────────
        if !self.play_session_active {
            self.submit_light_gizmos();
            self.submit_audio_gizmos();
            self.submit_spline_gizmos();
        }

        // ── Day cycle (CONTROL-L), then post-processing (Phase 15A1) ─────────
        if let Some(r) = &mut self.renderer {
            r.profiler.cpu_begin("Environment");
        }
        self.apply_time_of_day(dt);
        self.apply_sky(dt);
        self.apply_weather(dt);
        self.publish_time_of_day();
        self.apply_post_process();
        self.apply_camera_settings();
        if let Some(r) = &mut self.renderer {
            {
                r.profiler.cpu_end();
            }
        }

        // MORROWIND-J step 2. Acquired before the editor's encoder exists,
        // because the scene is recorded straight into it rather than into the
        // editor's swapchain. Held across the whole frame and handed to the
        // window it came from below, which draws the context bar over it and
        // presents it.
        let scene_frame = self.acquire_floating_viewport();
        let scene_target = scene_frame.as_ref().map(|frame| somnium_renderer::SceneTarget {
            view: &frame.view,
            size: frame.size,
        });

        if let (Some(r), Some(c), Some(ui), Some(window)) = (
            &mut self.renderer,
            &self.render_ctx,
            &mut self.ui_manager,
            &self.window,
        ) {
            r.set_editor_overlays_enabled(!self.play_session_active);
            r.time = self.simulation_clock.elapsed_seconds;

            // MORROWIND-J step 3. After the game has set its camera and before
            // the frame is recorded, because the tiles are the editor's and the
            // camera is the game's, and this is the one place that holds both.
            //
            // Single stays *empty* rather than becoming a one-element list: an
            // empty list is the path the renderer took before views existed,
            // and a one-viewport editor should not be paying for a blit to
            // prove a feature it is not using.
            let layout = ui.viewport_layout();
            // The whole of the redirected surface, not a rectangle of this
            // window's. `viewport_physical_rect` reports the *other* window
            // once the viewport is floated, so tiling against it would put four
            // views wherever those numbers happen to land in this swapchain.
            if layout == somnium_ui::viewport_layout::ViewportLayout::Single
                || self.play_session_active
                || scene_target.is_some()
            {
                r.set_scene_views(&[]);
            } else {
                let scale = window.scale_factor() as f32;
                let tiles = layout.tiles(ui.viewport_physical_rect(scale));
                let views = somnium_renderer::view::standard_views(&tiles, r.primary_scene_view());
                r.set_scene_views(&views);
            }
            // MORROWIND-E2. `self.game` is a disjoint field from the four
            // borrowed above, so the game can be handed to the renderer as a
            // callback without any of this becoming a `RefCell`.
            let mut adapter = GameUiAdapter {
                game: self.game.as_mut(),
            };
            r.render_with_game_ui(c, ui, window, Some(&mut adapter), scene_target);
        }

        // MORROWIND-J step 2. After the editor's own frame, and on the same
        // device: each floating window submits its own encoder, so a slow one
        // costs its own frame rather than the editor's.
        self.render_floating(scene_frame);

        // ── Accessibility preferences (MORROWIND-I) ──────────────────────────
        //
        // The platform first, the preference over it. Applied every frame
        // rather than on change because `set_a11y_settings` is two stores and a
        // bool compare, and a change-detection path would be more code than the
        // thing it avoids.
        if let Some(ui) = self.ui_manager.as_mut() {
            let editor = self.settings.editor();
            let platform = somnium_ui::A11ySettings::from_platform();
            let wanted = somnium_ui::A11ySettings {
                reduced_motion: platform.reduced_motion || editor.reduced_motion,
                high_contrast: platform.high_contrast || editor.high_contrast,
            };
            if ui.a11y_settings() != wanted {
                ui.set_a11y_settings(wanted);
            }
        }

        // ── Accessibility tree (MORROWIND-I) ─────────────────────────────────
        //
        // After the render call, because that is what ran layout: a tree
        // published before layout carries last frame's bounds, and a reader
        // pointing one frame behind is a reader pointing at the wrong control
        // during exactly the interactions that move things.
        //
        // Gated on `is_active`, so a run with no screen reader attached — which
        // is almost every run — pays one lock acquisition per frame and does
        // not walk the widget tree at all.
        if let Some(a11y) = self.a11y.as_mut() {
            if a11y.is_active() {
                if let Some(ui) = self.ui_manager.as_ref() {
                    a11y.publish(ui.a11y_tree());
                }
            }
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
            // Serviced here rather than in the handler: a window needs the
            // event loop, and this is the first point after the drain that has
            // one.
            for kind in std::mem::take(&mut self.pending_float) {
                self.float_panel(event_loop, kind);
            }
            if let Some(result) = self.pending_map_load.take() {
                let mut ctx = EngineContext::new(
                    &self.time,
                    &self.config,
                    &mut self.world,
                    self.physics.as_mut().unwrap(),
                    self.audio.as_mut().unwrap(),
                    &mut self.jobs,
                    self.render_ctx.as_ref(),
                    self.renderer.as_mut(),
                    &mut self.selection.primary,
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

        // PORTAL-0-B: before the limiter, so this is engine work and not sleep.
        if let Some(r) = &mut self.renderer {
            r.profiler.frame_cpu_ms = frame_body_started.elapsed().as_secs_f32() * 1000.0;
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
    /// Attach the selected material document as a transient ECS component so
    /// generated Details and `SetFieldCmd` can edit it without a bespoke path.
    fn ensure_material_session(&mut self) {
        let Some(entity) = self.selection.primary else {
            return;
        };
        let Some(component) = self.world.get::<MaterialComponent>(entity).copied() else {
            return;
        };
        if component.asset == somnium_asset::database::AssetId::NONE {
            self.material_sessions.remove(&entity);
            let _ = self
                .world
                .remove_component::<somnium_asset::material::MaterialAsset>(entity);
            return;
        }

        if !self.load_material_document(component.asset) {
            return;
        }

        let needs_session = self
            .material_sessions
            .get(&entity)
            .is_none_or(|(asset, _)| *asset != component.asset)
            || self
                .world
                .get::<somnium_asset::material::MaterialAsset>(entity)
                .is_none();
        if needs_session {
            let asset = self.material_documents[&component.asset].asset.clone();
            if let Some(existing) = self
                .world
                .get_mut::<somnium_asset::material::MaterialAsset>(entity)
            {
                *existing = asset.clone();
            } else if let Err(error) = self.world.insert_component(entity, asset.clone()) {
                warn!(%error, "could not attach material edit session");
                return;
            }
            self.material_sessions
                .insert(entity, (component.asset, asset));
        }

        self.queue_material_textures(component.asset);
        self.ensure_material_runtime(component.asset);
    }

    fn queue_material_textures(&mut self, asset_id: somnium_asset::database::AssetId) {
        let Some(document) = self.material_documents.get(&asset_id) else {
            return;
        };
        let slots = [
            document.asset.albedo_map,
            document.asset.normal_map,
            document.asset.metallic_roughness_map,
            document.asset.occlusion_map,
            document.asset.emissive_map,
        ];
        for texture_id in slots {
            if texture_id == somnium_asset::database::AssetId::NONE
                || self.material_textures.contains_key(&texture_id)
                || self.material_texture_jobs.contains_key(&texture_id)
            {
                continue;
            }
            let record = self
                .asset_gate
                .published()
                .and_then(|snapshot| snapshot.get(texture_id))
                .cloned();
            let Some(record) = record else {
                continue;
            };
            let path = record.absolute_path;
            match self
                .jobs
                .submit("Material texture", JobPriority::Visible, move |ctx| {
                    ctx.check_cancelled()
                        .map_err(|error| format!("{error:?}"))?;
                    let texture = somnium_asset::material::load_material_texture(path)?;
                    ctx.set_progress(1.0);
                    Ok(texture)
                }) {
                Ok(job) => {
                    self.material_texture_jobs.insert(texture_id, job);
                }
                Err(error) => warn!(?error, "material texture queue is full"),
            }
        }
    }

    fn refresh_material_gpu(&mut self, asset_id: somnium_asset::database::AssetId) {
        let (Some(document), Some(runtime_id)) = (
            self.material_documents.get(&asset_id),
            self.material_runtime.get(&asset_id).copied(),
        ) else {
            return;
        };
        let gpu =
            somnium_renderer::material::pool::GpuMaterial::from_asset(&document.asset, |texture| {
                self.material_textures.get(&texture).copied().unwrap_or(-1)
            });
        if let (Some(renderer), Some(ctx)) = (self.renderer.as_mut(), self.render_ctx.as_ref()) {
            renderer
                .materials_pool
                .set_material(&ctx.queue, runtime_id, gpu);
            renderer.set_material_double_sided(runtime_id, document.asset.double_sided);
            renderer.set_material_blend(
                runtime_id,
                document.asset.alpha_mode == somnium_asset::AlphaMode::Blend,
            );
        }
    }

    fn ensure_material_runtime(&mut self, asset_id: somnium_asset::database::AssetId) {
        let Some(document) = self.material_documents.get(&asset_id) else {
            return;
        };
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let Some(ctx) = self.render_ctx.as_ref() else {
            return;
        };
        let runtime_id = if let Some(runtime_id) = self.material_runtime.get(&asset_id).copied() {
            runtime_id
        } else {
            let gpu = somnium_renderer::material::pool::GpuMaterial::from_asset(
                &document.asset,
                |texture| self.material_textures.get(&texture).copied().unwrap_or(-1),
            );
            let runtime_id = renderer.materials_pool.add_material(&ctx.queue, gpu);
            renderer.set_material_double_sided(runtime_id, document.asset.double_sided);
            renderer.set_material_blend(
                runtime_id,
                document.asset.alpha_mode == somnium_asset::AlphaMode::Blend,
            );
            self.material_runtime.insert(asset_id, runtime_id);
            runtime_id
        };

        let entities: Vec<_> = self.world.entities().collect();
        for entity in entities {
            if let Some(material) = self.world.get_mut::<MaterialComponent>(entity)
                && material.asset == asset_id
            {
                material.runtime_id = runtime_id;
            }
        }
    }

    /// Detect reflected edits (including undo/redo), update the shared runtime
    /// slot once, and mirror the same asset value into every open session.
    fn sync_material_sessions(&mut self) {
        let changes: Vec<_> = self
            .material_sessions
            .iter()
            .filter_map(|(entity, (asset_id, observed))| {
                let current = self
                    .world
                    .get::<somnium_asset::material::MaterialAsset>(*entity)?;
                (current != observed).then(|| (*asset_id, current.clone()))
            })
            .collect();

        for (asset_id, current) in changes {
            let live_preview = somnium_asset::preview::render_material_sphere(&current);
            let live_path = self
                .material_documents
                .get(&asset_id)
                .map(|document| document.path.clone());
            if let Some(document) = self.material_documents.get_mut(&asset_id) {
                document.asset = current.clone();
                document.dirty = true;
            }
            for (entity, (session_asset, observed)) in &mut self.material_sessions {
                if *session_asset == asset_id {
                    if let Some(open) = self
                        .world
                        .get_mut::<somnium_asset::material::MaterialAsset>(*entity)
                    {
                        *open = current.clone();
                    }
                    *observed = current.clone();
                }
            }
            self.queue_material_textures(asset_id);
            self.refresh_material_gpu(asset_id);
            if let (Some(path), Some(ui)) = (live_path, self.ui_manager.as_mut()) {
                ui.invalidate_thumbnail(&path);
                let _ = ui.deliver_thumbnail(&path, &live_preview);
            }
        }
    }

    /// Persist dirty material documents and refresh both the embedded header
    /// preview and live thumbnail cell. Scene save is the asset save boundary.
    fn flush_material_assets(&mut self) {
        self.sync_material_sessions();
        let dirty: Vec<_> = self
            .material_documents
            .iter()
            .filter(|(_, document)| document.dirty)
            .map(|(id, document)| (*id, document.path.clone()))
            .collect();
        for (asset_id, path) in dirty {
            let Some(document) = self.material_documents.get_mut(&asset_id) else {
                continue;
            };
            let preview = somnium_asset::preview::render_material_sphere(&document.asset);
            match somnium_asset::material::save_material(&path, &mut document.asset, &preview) {
                Ok(()) => {
                    document.dirty = false;
                    if let Some(ui) = self.ui_manager.as_mut() {
                        ui.invalidate_thumbnail(&path);
                        let _ = ui.deliver_thumbnail(&path, &preview);
                    }
                    self.next_asset_scan = std::time::Instant::now();
                    self.asset_scan_stamp = None;
                }
                Err(error) => self.report_content_error(&path, &error),
            }
        }
    }

    /// Poll immutable asset scans and worker previews without doing file IO in
    /// the frame loop. A periodic 350 ms scan is the portable watcher fallback;
    /// the two-sample gate debounces partial external writes.
    /// Poll for edited shader files and toast the result (MORROWIND-C).
    ///
    /// Throttled to four times a second. A shader edit is a human action and a
    /// quarter-second is imperceptible against the time it takes to alt-tab;
    /// polling fifty-odd modification times every frame would be a cost paid
    /// 240 times per second for a benefit nobody could see.
    ///
    /// Release builds return immediately — `Shaders::poll_reload` is a no-op
    /// there, and a shipped build needs no `.wgsl` files on disk at all.
    fn pump_shader_reload(&mut self) {
        if !cfg!(debug_assertions) {
            return;
        }
        const INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
        let now = std::time::Instant::now();
        if now.duration_since(self.last_shader_poll) < INTERVAL {
            return;
        }
        self.last_shader_poll = now;

        let (Some(renderer), Some(ctx)) = (self.renderer.as_mut(), self.render_ctx.as_ref()) else {
            return;
        };
        // A toast, not a log line: the whole point of hot reload is that the
        // author is looking at the viewport, not at a terminal. A failed edit
        // that only writes to stderr is indistinguishable from an edit that
        // did nothing.
        if let Some(message) = renderer.reload_shaders(ctx)
            && let Some(ui) = self.ui_manager.as_mut()
        {
            ui.push_toast(&message);
        }
    }

    /// Apply finished background work and collect its telemetry. Once a frame.
    ///
    /// MORROWIND-B, Seam 1's third property: the worker produces data, the main
    /// thread installs it, and the installation is budgeted. Two milliseconds
    /// out of a 16.6 ms frame is the starting number — enough to install a
    /// handful of decodes, small enough that a burst of sixty cannot become a
    /// hitch. `drain_completions` returning with work outstanding is the
    /// mechanism working, not a fault, and the remainder lands next frame.
    fn pump_jobs(&mut self) {
        const COMPLETION_BUDGET: std::time::Duration = std::time::Duration::from_millis(2);
        self.jobs.drain_completions(COMPLETION_BUDGET);
        let (zones, dropped) = self.jobs.take_zones();
        if !zones.is_empty() {
            self.job_profile = crate::jobs::profile_rows(&zones);
            self.job_zones_dropped = dropped;
        }
    }

    fn update_asset_pipeline(&mut self) {
        let finished_material_textures: Vec<_> = self
            .material_texture_jobs
            .iter()
            .filter_map(|(asset, job)| job.try_take().map(|result| (*asset, result)))
            .collect();
        for (texture_id, result) in finished_material_textures {
            self.material_texture_jobs.remove(&texture_id);
            match result {
                Ok(texture) => {
                    if let (Some(renderer), Some(ctx)) =
                        (self.renderer.as_mut(), self.render_ctx.as_ref())
                    {
                        let slot = renderer.upload_material_texture(
                            ctx,
                            &texture.data,
                            texture.width,
                            texture.height,
                        );
                        self.material_textures.insert(texture_id, slot);
                    }
                    let affected: Vec<_> = self
                        .material_documents
                        .iter()
                        .filter(|(_, document)| {
                            let material = &document.asset;
                            [
                                material.albedo_map,
                                material.normal_map,
                                material.metallic_roughness_map,
                                material.occlusion_map,
                                material.emissive_map,
                            ]
                            .contains(&texture_id)
                        })
                        .map(|(asset, _)| *asset)
                        .collect();
                    for asset in affected {
                        self.refresh_material_gpu(asset);
                    }
                }
                Err(error) => warn!(?error, "material texture decode failed"),
            }
        }

        let completed_scan = self.asset_scan.as_ref().and_then(JobHandle::try_take);
        if let Some(result) = completed_scan {
            self.asset_scan = None;
            match result {
                Ok((snapshot, index)) => {
                    if let Some(published) = self.asset_gate.stage(snapshot) {
                        if let Some(ui) = self.ui_manager.as_mut() {
                            ui.set_asset_snapshot(published);
                            ui.set_dependency_index(index);
                        }
                    }
                }
                Err(error) => warn!(?error, "asset inventory scan failed"),
            }
        }

        let now = std::time::Instant::now();
        // The poll is 350 ms, and it used to submit a scan on every tick
        // whether or not anything on disk had moved. Three background jobs a
        // second is invisible on the CPU and very visible in the chrome: the
        // status bar's Cancel chip blinked on and off at ~3 Hz, the status text
        // was overwritten with "Asset inventory — 0%" and never restored, and
        // the Jobs panel tore down and rebuilt its rows each cycle because the
        // job id changes every time.
        //
        // A cheap stamp over the content root's directory mtimes settles that:
        // an idle project scans once and then stops. Every explicit
        // invalidation point already sets `next_asset_scan = now`, and those
        // now clear the stamp too, so an edit the stamp cannot see still
        // rescans immediately.
        // A guard, not an early return: everything below this block —
        // thumbnail requests, preview jobs, external imports — has to keep
        // running on a frame where the inventory has nothing to do.
        let due = self.asset_scan.is_none() && now >= self.next_asset_scan;
        let changed = due && {
            let stamp = content_root_stamp(&self.config.content_root);
            let moved = self.asset_scan_stamp != Some(stamp);
            self.asset_scan_stamp = Some(stamp);
            self.next_asset_scan = now + std::time::Duration::from_millis(350);
            moved
        };
        if changed {
            let root = self.config.content_root.clone();
            match self
                .jobs
                .submit("Asset inventory", JobPriority::Background, move |ctx| {
                    ctx.check_cancelled()
                        .map_err(|error| format!("{error:?}"))?;
                    let snapshot = somnium_asset::database::AssetDb::scan(root)?;
                    ctx.set_progress(0.8);
                    // On the job, not on the frame: this opens every scene,
                    // prefab, material and document in the project.
                    let index = somnium_asset::depend::DependencyIndex::build(&snapshot);
                    ctx.set_progress(1.0);
                    Ok((snapshot, index))
                }) {
                Ok(handle) => self.asset_scan = Some(handle),
                Err(error) => warn!(?error, "asset scan queue is full"),
            }
        }

        let requests = self
            .ui_manager
            .as_mut()
            .map(UiManager::take_thumbnail_requests)
            .unwrap_or_default();
        for request in requests {
            if self.preview_jobs.contains_key(&request.path) {
                continue;
            }
            let record = self
                .asset_gate
                .published()
                .and_then(|snapshot| {
                    snapshot
                        .records()
                        .iter()
                        .find(|r| r.absolute_path == request.path)
                })
                .cloned();
            let Some(record) = record else {
                if let Some(ui) = self.ui_manager.as_mut() {
                    ui.fail_thumbnail(&request.path);
                }
                continue;
            };
            let cache_root = self.config.content_root.join(".somnium/thumbnails");
            let priority = if request.visible {
                JobPriority::Visible
            } else {
                JobPriority::Background
            };
            let key = request.path.clone();
            // MORROWIND-B, the deadline's first real customer.
            //
            // An off-screen preview is speculative prefetch: the drawer might
            // scroll to it, and might not. One that is still queued five
            // seconds after it was asked for is almost certainly for a tile
            // nobody is looking at any more, and running it makes the queue
            // *further* behind for the tiles somebody is looking at. Dropping
            // it is what lets a fast scroll through a large folder settle.
            //
            // Visible previews get no deadline: somebody is waiting at a
            // spinner, and late is better than never. Cancellation, not
            // expiry, is the right tool when a visible tile scrolls away.
            const SPECULATIVE_PREVIEW_BUDGET: std::time::Duration =
                std::time::Duration::from_secs(5);
            let mut desc = crate::jobs::JobDesc::new("Asset preview").priority(priority);
            if !request.visible {
                desc = desc.within(SPECULATIVE_PREVIEW_BUDGET);
            }
            match self.jobs.submit_with(desc, move |ctx| {
                ctx.check_cancelled()
                    .map_err(|error| format!("{error:?}"))?;
                let result = somnium_asset::preview::prepare_preview(&record, &cache_root)?;
                ctx.set_progress(1.0);
                Ok(result)
            }) {
                Ok(handle) => {
                    self.preview_jobs.insert(key, handle);
                }
                Err(error) => warn!(?error, "preview queue is full"),
            }
        }

        let finished: Vec<_> = self
            .preview_jobs
            .iter()
            .filter_map(|(path, handle)| handle.try_take().map(|result| (path.clone(), result)))
            .collect();
        for (path, result) in finished {
            self.preview_jobs.remove(&path);
            match result {
                Ok(Some(preview)) => self.preview_ready.push_back((path, preview.rgba)),
                Ok(None) | Err(_) => {
                    if let Some(ui) = self.ui_manager.as_mut() {
                        ui.fail_thumbnail(&path);
                    }
                }
            }
        }
        if let Some(ui) = self.ui_manager.as_mut() {
            ui.deliver_thumbnails_budgeted(
                &mut self.preview_ready,
                somnium_ui::thumbnail::DEFAULT_APPLY_BUDGET,
            );
        }

        let completed_import = self.import_job.as_ref().and_then(JobHandle::try_take);
        if let Some(result) = completed_import {
            self.import_job = None;
            match result {
                Ok((path, scene, materials)) => self.finish_import_model(path, scene, materials),
                Err(error) => {
                    warn!(?error, "model import failed");
                    if let Some(ui) = self.ui_manager.as_mut() {
                        ui.push_toast("Import failed — see the Output Log");
                    }
                }
            }
        }
        let completed_external = self
            .external_import_job
            .as_ref()
            .and_then(JobHandle::try_take);
        if let Some(result) = completed_external {
            self.external_import_job = None;
            match result {
                Ok(files) => {
                    let destinations = files
                        .into_iter()
                        .map(|(_, destination)| destination)
                        .collect();
                    self.undo_stack.push_silent(Box::new(
                        crate::editor_commands::FileImportCmd::new(destinations),
                    ));
                    self.next_asset_scan = std::time::Instant::now();
                    self.asset_scan_stamp = None;
                    if let Some(ui) = self.ui_manager.as_mut() {
                        ui.push_toast("Files imported");
                    }
                }
                Err(error) => warn!(?error, "external file import failed"),
            }
        }
        let active = self.jobs.active();
        if let Some(ui) = self.ui_manager.as_mut() {
            // Housekeeping does not get the status bar. The chip is a
            // *cancellation* affordance for work a person started and might
            // want to stop; a background inventory sweep is neither, and
            // flashing it there taught people to ignore the one place an
            // import or a bake reports itself. The Jobs panel below still
            // lists everything, which is where CONTROL-I said it belonged.
            //
            // DOOM-H: the test used to be `priority != Background`, which only
            // worked while every continuous system sat at that class. Voxel
            // chunk meshing is `Visible` and correctly so, and it would have
            // parked "voxel.chunk_mesh — 0%" and a meaningless Cancel button on
            // the status line for as long as the camera kept moving.
            let status: Vec<_> = active
                .iter()
                .rev()
                .filter(|job| !job.housekeeping)
                .map(|job| somnium_ui::UiJobStatus {
                    id: job.id,
                    name: job.name,
                    progress: job.progress,
                })
                .collect();
            ui.update_jobs(&status);
            // CONTROL-I: the same jobs, in a panel rather than a chip, so a
            // cancelled or failed import is inspectable after the chip has
            // gone. `progress >= 1.0` reads as done; a failed job reports
            // itself rather than simply disappearing.
            ui.set_job_rows(
                active
                    .iter()
                    .rev()
                    .map(|job| {
                        (
                            job.id,
                            job.name.to_string(),
                            job.progress,
                            matches!(
                                job.status,
                                crate::jobs::JobStatus::Failed | crate::jobs::JobStatus::Cancelled
                            ),
                        )
                    })
                    .collect(),
            );
        }
        self.jobs.prune_finished();
    }

    /// Deliver an already-translated input event through the same game/script
    /// path whether it came from the ordinary window route or from an editor
    /// shortcut that deliberately preserves its physical key transition.
    fn forward_engine_event(&mut self, event_loop: &ActiveEventLoop, engine_event: EngineEvent) {
        let mut ctx = EngineContext::new(
            &self.time,
            &self.config,
            &mut self.world,
            self.physics.as_mut().unwrap(),
            self.audio.as_mut().unwrap(),
            &mut self.jobs,
            self.render_ctx.as_ref(),
            self.renderer.as_mut(),
            &mut self.selection.primary,
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
        let entity = self.selection.primary?;
        self.world.get::<TerrainComponent>(entity).copied()
    }

    /// Model matrix of the selected terrain entity.
    fn selected_terrain_model(&self) -> Option<glam::Mat4> {
        let entity = self.selection.primary?;
        self.world.get::<Transform>(entity).map(|t| t.to_matrix())
    }

    /// World-space cursor ray (origin, direction) from the camera through
    /// the current cursor position.
    /// CONTROL-O: the terrain's surface normal at a world position.
    ///
    /// Four vertical raycasts around the point, because the terrain exposes a
    /// raycast and not a normal query and adding one would mean deciding what
    /// a normal means at a chunk seam. Called **once**, when a decal is
    /// created — not from the per-frame drop probe, which stays a single ray.
    ///
    /// Falls back to straight up, which is the right answer for the common
    /// case and a harmless one for the rest: a decal can be rotated.
    fn terrain_normal_at(&mut self, position: glam::Vec3) -> glam::Vec3 {
        const STEP: f32 = 0.5;
        let sample = |engine: &mut Self, x: f32, z: f32| -> Option<f32> {
            let origin = glam::Vec3::new(x, position.y + 500.0, z);
            let terrains: Vec<_> = engine
                .world
                .entities()
                .filter_map(|entity| {
                    let component = engine.world.get::<TerrainComponent>(entity)?;
                    let model = engine
                        .world
                        .get::<Transform>(entity)
                        .map_or(glam::Mat4::IDENTITY, Transform::to_matrix);
                    Some((component.terrain_id, model))
                })
                .collect();
            let renderer = engine.renderer.as_mut()?;
            for (id, model) in terrains {
                let Some(terrain) = renderer.terrain_mut(id) else {
                    continue;
                };
                terrain.model = model;
                if let Some(hit) = terrain.raycast(origin, glam::Vec3::NEG_Y) {
                    return Some(hit.y);
                }
            }
            None
        };

        let (Some(east), Some(west), Some(north), Some(south)) = (
            sample(self, position.x + STEP, position.z),
            sample(self, position.x - STEP, position.z),
            sample(self, position.x, position.z + STEP),
            sample(self, position.x, position.z - STEP),
        ) else {
            return glam::Vec3::Y;
        };
        // Central differences. The cross product of the two tangents is the
        // normal; written out rather than crossed so the sign is visible.
        glam::Vec3::new(west - east, 2.0 * STEP, south - north).normalize_or(glam::Vec3::Y)
    }

    /// The renderer pool slot a material asset currently occupies, or zero.
    ///
    /// Zero is the default material, which is what a decal referencing an
    /// asset the pool has not loaded yet should show: a flat tint rather than
    /// whatever happened to be in slot `n`.
    fn material_runtime_id(&self, asset: somnium_asset::database::AssetId) -> u32 {
        self.world
            .entities()
            .filter_map(|entity| self.world.get::<MaterialComponent>(entity))
            .find(|component| component.asset == asset)
            .map_or(0, |component| component.runtime_id)
    }

    fn viewport_terrain_drop_hit(&mut self) -> Option<[f32; 3]> {
        let (origin, direction) = self.cursor_ray()?;
        let terrains: Vec<_> = self
            .world
            .entities()
            .filter_map(|entity| {
                let component = self.world.get::<TerrainComponent>(entity)?;
                let model = self
                    .world
                    .get::<Transform>(entity)
                    .map_or(glam::Mat4::IDENTITY, Transform::to_matrix);
                Some((component.terrain_id, model))
            })
            .collect();
        let renderer = self.renderer.as_mut()?;
        let mut nearest: Option<(f32, glam::Vec3)> = None;
        for (id, model) in terrains {
            let Some(terrain) = renderer.terrain_mut(id) else {
                continue;
            };
            terrain.model = model;
            let Some(hit) = terrain.raycast(origin, direction) else {
                continue;
            };
            let distance = origin.distance_squared(hit);
            if nearest.is_none_or(|(best, _)| distance < best) {
                nearest = Some((distance, hit));
            }
        }
        nearest.map(|(_, hit)| hit.to_array())
    }

    fn viewport_entity_drop_pick(&self) -> Option<somnium_ecs::Entity> {
        let (origin, direction) = self.cursor_ray()?;
        let renderer = self.renderer.as_ref()?;
        self.world
            .entities()
            .filter_map(|entity| {
                // Locked and hidden entities are deliberately unpickable: that is
                // what the two flags are for, and a drop that landed on an
                // invisible object would be the exact bug they prevent.
                let flags = self
                    .world
                    .get::<EditorFlags>(entity)
                    .copied()
                    .unwrap_or_default();
                if flags.locked || flags.hidden {
                    return None;
                }
                entity_ray_hit_distance(&self.world, renderer, entity, origin, direction)
                    .map(|distance| (distance, entity))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, entity)| entity)
    }

    /// The size, in physical pixels, of the surface the cursor is over.
    ///
    /// Read from the live surface configuration rather than from a cached
    /// `Resized`, because the cache was wrong in two ways at once and both
    /// were invisible until something aimed a ray. `window_event` drops every
    /// event that arrives before the lifecycle reaches `Running`, and on
    /// Windows the window's first `Resized` is one of them — so the cache kept
    /// the *requested* size for the whole session unless the user happened to
    /// drag the window edge. That requested size is also **logical**, while
    /// the surface and the cursor are both physical, so on any display with a
    /// scale factor other than 1.0 the two disagreed by the scale even after a
    /// resize did land.
    ///
    /// Everything that turns a cursor position into a world ray goes through
    /// here: the transform gizmo, the terrain and foliage brushes, the
    /// rubber band, the drop probe. All of them were aiming somewhere the user
    /// was not pointing.
    fn viewport_size(&self) -> (f32, f32) {
        self.render_ctx
            .as_ref()
            .map(|ctx| (ctx.config.width as f32, ctx.config.height as f32))
            .filter(|(w, h)| *w >= 1.0 && *h >= 1.0)
            .unwrap_or(self.viewport_size_hint)
    }

    fn cursor_ray(&self) -> Option<(glam::Vec3, glam::Vec3)> {
        let r = self.renderer.as_ref()?;
        let inv_vp = r.picking_view_proj().inverse();
        let (vw, vh) = self.viewport_size();
        let world_pt = ndc_to_world(self.cursor_pos.0, self.cursor_pos.1, vw, vh, &inv_vp);
        let dir = (world_pt - r.camera_pos).normalize_or_zero();
        (dir != glam::Vec3::ZERO).then_some((r.camera_pos, dir))
    }

    /// Put `path`'s asset into the first field of the selection that accepts
    /// its kind.
    ///
    /// The menu-and-keyboard route to where a drop lands. It exists because a
    /// drag is a gesture with a dozen ways to not quite happen — the pointer a
    /// few pixels off the row, a threshold not crossed, a window that lost
    /// focus mid-drag — and an author who cannot make the drag work needs a
    /// path that is a single click and cannot miss.
    ///
    /// The kind comes from the extension rather than from the asset database,
    /// so this works during the window after a file appears and before the
    /// scan that indexes it has finished.
    fn assign_asset_to_selection(&mut self, path: &std::path::Path) {
        use somnium_ecs::reflect::ReflectValue;

        let Some(entity) = self.selection.primary else {
            self.toast("Select something first — an asset has to be assigned to an entity");
            return;
        };
        let kind = somnium_asset::database::classify(path, false);
        // Ids are minted from the path *relative to the content root*, which
        // is the whole point of `AssetId`: the same file names the same asset
        // whatever the project happens to be checked out at.
        let relative = path.strip_prefix(&self.config.content_root).unwrap_or(path);
        let asset = somnium_asset::database::AssetId::from_relative_path(relative);
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();

        let target = asset_field_for(&self.type_registry, &self.world, entity, kind);

        let Some((component, field, component_name, field_name)) = target else {
            self.toast(&format!(
                "Nothing on this entity takes a {} asset",
                kind.label()
            ));
            return;
        };

        self.handle_editor_event(EditorEvent::SetComponentField {
            entity,
            component,
            field,
            value: ReflectValue::Asset(Some(somnium_ecs::reflect::AssetRef::from_raw(asset.raw()))),
            gesture: GestureId(u64::MAX - 2),
            live: false,
        });
        self.toast(&format!("{name} → {component_name} · {field_name}"));
    }

    /// Say something in the viewport, and in the log.
    ///
    /// Both, always: the log is the record and the toast is the one the author
    /// is actually looking at. Several routes had only the log, which is the
    /// same as saying nothing while someone is working in the viewport.
    fn toast(&mut self, message: &str) {
        info!("{message}");
        if let Some(ui) = self.ui_manager.as_mut() {
            ui.push_toast(message);
        }
    }

    /// Put the whole brush state in front of the author, in the viewport.
    ///
    /// It used to go to `info!`, which means the Output Log, which means a
    /// panel nobody has open while they are painting. The reported symptom was
    /// having to click and look to find out whether the brush was doing
    /// anything — a settings readout that lives somewhere else is the same as
    /// no readout.
    fn announce_brush(&mut self) {
        let brush = self.terrain_brush;
        let message = if brush.mode == BrushMode::Paint {
            format!(
                "{}  ·  layer {}  ·  {:.1} m  ·  strength {:.0}%  ·  {}",
                brush.mode.label(),
                brush.paint_layer,
                brush.radius,
                brush.strength * 100.0,
                brush.alpha.label(),
            )
        } else {
            format!(
                "{}  ·  {:.1} m  ·  strength {:.0}%  ·  {}",
                brush.mode.label(),
                brush.radius,
                brush.strength * 100.0,
                brush.alpha.label(),
            )
        };
        info!("{message}");
        if let Some(ui) = self.ui_manager.as_mut() {
            ui.push_toast(&message);
        }
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
        self.foliage_paint_active = false;
        if self.selected_terrain().is_none() {
            // The reported symptom: clicking Raise with nothing selected
            // changed the mode, armed nothing, and said nothing, so the whole
            // toolbar read as decorative. A tool that cannot run has to say
            // why — silence is indistinguishable from a broken button.
            self.terrain_edit_active = false;
            let message = format!(
                "{} needs a terrain — select a Landscape in the Outliner first",
                self.terrain_brush.mode.label()
            );
            info!("{message}");
            if let Some(ui) = self.ui_manager.as_mut() {
                ui.push_toast(&message);
            }
            return;
        }
        self.terrain_edit_active = true;
        self.announce_brush();
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
        self.queue_import_model(path, [0.0; 3]);
    }

    fn queue_import_model(&mut self, path: std::path::PathBuf, at: [f32; 3]) {
        if self.import_job.is_some() {
            if let Some(ui) = self.ui_manager.as_mut() {
                ui.push_toast("An import is already running");
            }
            return;
        }
        self.import_spawn_at = at;
        let path_str = path.to_string_lossy().to_string();
        let worker_path = path_str.clone();
        let content_root = self.config.content_root.clone();
        match self
            .jobs
            .submit("glTF import", JobPriority::User, move |ctx| {
                ctx.set_progress(0.05);
                ctx.check_cancelled()
                    .map_err(|error| format!("{error:?}"))?;
                let scene = somnium_asset::load_gltf(&worker_path)?;
                ctx.set_progress(0.7);
                let materials = somnium_asset::material::materialize_gltf_assets(
                    &scene,
                    &worker_path,
                    &content_root,
                )?;
                ctx.set_progress(1.0);
                Ok((worker_path, scene, materials))
            }) {
            Ok(handle) => {
                self.import_job = Some(handle);
                if let Some(ui) = self.ui_manager.as_mut() {
                    ui.push_toast("Import queued");
                }
            }
            Err(error) => warn!(?error, "model import queue is full"),
        }
    }

    fn queue_external_import(
        &mut self,
        files: Vec<std::path::PathBuf>,
        folder: std::path::PathBuf,
    ) {
        if files.is_empty() || self.external_import_job.is_some() {
            return;
        }
        if folder.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        }) {
            if let Some(ui) = self.ui_manager.as_mut() {
                ui.push_toast("Import folder must be inside Content");
            }
            return;
        }
        let destination = self.config.content_root.join(folder);
        match self
            .jobs
            .submit("File import", JobPriority::User, move |ctx| {
                std::fs::create_dir_all(&destination).map_err(|e| e.to_string())?;
                let mut imported = Vec::new();
                for (index, source) in files.into_iter().enumerate() {
                    ctx.check_cancelled().map_err(|e| format!("{e:?}"))?;
                    let Some(name) = source.file_name() else {
                        continue;
                    };
                    let stem = std::path::Path::new(name)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("asset");
                    let ext = std::path::Path::new(name)
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or("");
                    let mut target = destination.join(name);
                    let mut suffix = 1u32;
                    while target.exists() {
                        let leaf = if ext.is_empty() {
                            format!("{stem}_{suffix}")
                        } else {
                            format!("{stem}_{suffix}.{ext}")
                        };
                        target = destination.join(leaf);
                        suffix += 1;
                    }
                    std::fs::copy(&source, &target).map_err(|e| e.to_string())?;
                    imported.push((source, target));
                    ctx.set_progress((index + 1) as f32 / imported.len().max(index + 1) as f32);
                }
                Ok(imported)
            }) {
            Ok(handle) => self.external_import_job = Some(handle),
            Err(error) => warn!(?error, "file import queue is full"),
        }
    }

    /// Upload a worker-decoded glTF and spawn its renderable nodes.
    fn finish_import_model(
        &mut self,
        path_str: String,
        scene: somnium_asset::LoadedScene,
        material_assets: Vec<somnium_asset::database::AssetId>,
    ) {
        let Some((renderer, render_ctx)) = self.renderer.as_mut().zip(self.render_ctx.as_ref())
        else {
            warn!("Cannot import before the renderer is ready");
            return;
        };

        let uploaded = renderer.upload_scene(render_ctx, &scene);
        if uploaded.is_empty() {
            warn!("{} contained no renderable meshes", path_str);
            return;
        }

        let count = uploaded.len();
        let offset = glam::Vec3::from_array(self.import_spawn_at);
        let mut commands: Vec<Box<dyn crate::editor_commands::EditorCommand>> =
            Vec::with_capacity(count);
        for node in uploaded {
            let (scale, rotation, translation) = node.transform.to_scale_rotation_translation();
            let name = if node.entity_name.is_empty() {
                Name::new("Imported Mesh")
            } else {
                Name::new(&node.entity_name)
            };
            let snapshot = EntitySnapshot {
                spline: None,
                transform: Some(Transform {
                    translation: translation + offset,
                    rotation,
                    scale,
                }),
                name: Some(name),
                wt: Some(WorldTransform::identity()),
                mesh: Some(MeshComponent {
                    vertex_offset: node.vertex_offset,
                    index_offset: node.index_offset,
                    index_count: node.index_count,
                }),
                mat: Some(MaterialComponent {
                    asset: material_assets
                        .get(node.material_index)
                        .copied()
                        .unwrap_or(somnium_asset::database::AssetId::NONE),
                    runtime_id: node.material_id,
                }),
                light: None,
                audio: None,
                mesh_kind: None,
                is_particle_emitter: false,
                environment: false,
                decal: None,
                terrain: None,
                world_partition: None,
                ui_canvas: None,
                voxel_terrain: None,
                foliage: None,
                water: None,
                parent: None,
                children: None,
            };
            commands.push(Box::new(CreateEntityCmd::new(snapshot)));
        }
        self.undo_stack.push(
            Box::new(crate::editor_commands::CommandGroup::new(
                "Import Model",
                commands,
            )),
            &mut self.world,
            &mut self.selection.primary,
        );

        info!("Imported {} ({} mesh nodes)", path_str, count);
        self.scene_dirty = true;
        self.next_asset_scan = std::time::Instant::now();
        self.asset_scan_stamp = None;
        if let Some(ui) = self.ui_manager.as_mut() {
            ui.push_toast("Import finished");
        }
    }

    /// CONTROL-L: run the day cycle, once per frame, before anything reads a
    /// light or a post-process value.
    ///
    /// Order matters and is the reason this is its own method called from
    /// exactly one place: the sun's rotation must be final before
    /// `submit_light_gizmos` draws it and before the game layer reads it into
    /// the light buffer, and the fog/exposure overrides must be recorded
    /// before `apply_post_process` pushes the authored values they replace.
    fn apply_time_of_day(&mut self, dt: f32) {
        self.day_state = None;
        let Some(entity) = self.world.entities().find(|e| {
            self.world
                .get::<crate::time_of_day::TimeOfDayComponent>(*e)
                .is_some()
        }) else {
            return;
        };
        // The clock only runs during a play session. An editor that advanced
        // time while nobody was playing would make every scene dirty on its
        // own and make a capture unrepeatable.
        if self.play_session_active {
            if let Some(tod) = self
                .world
                .get_mut::<crate::time_of_day::TimeOfDayComponent>(entity)
            {
                tod.advance(dt);
            }
        }
        let Some(tod) = self
            .world
            .get::<crate::time_of_day::TimeOfDayComponent>(entity)
            .cloned()
        else {
            return;
        };
        if !tod.enabled {
            return;
        }
        let mut state = tod.evaluate();

        // Seam 4: the environment variable is an override of a real system now,
        // not the only way to place the sun. It still wins, because every
        // recorded repro in `dev records/` sets it.
        let env_elevation = std::env::var("SOMNIUM_SUN_ELEVATION")
            .ok()
            .and_then(|v| v.parse::<f32>().ok());
        let env_azimuth = std::env::var("SOMNIUM_SUN_AZIMUTH")
            .ok()
            .and_then(|v| v.parse::<f32>().ok());
        if env_elevation.is_some() || env_azimuth.is_some() {
            state.elevation_deg = env_elevation.unwrap_or(state.elevation_deg);
            state.azimuth_deg = env_azimuth.unwrap_or(state.azimuth_deg);
            state.rotation =
                crate::time_of_day::sun_rotation(state.azimuth_deg, state.elevation_deg);
        }

        // The sun is the first directional light. Not a named entity: a scene
        // that renamed "SunLight" would silently stop having a day cycle, and
        // a name is not a type.
        let sun = self.world.entities().find(|e| {
            self.world
                .get::<LightComponent>(*e)
                .is_some_and(|light| light.light_type == LightType::Directional)
        });
        if let Some(sun) = sun {
            if let Some(transform) = self.world.get_mut::<Transform>(sun) {
                transform.rotation = state.rotation;
            }
            if let Some(light) = self.world.get_mut::<LightComponent>(sun) {
                light.color = state.color;
                // A colour temperature would fight the authored tint ramp, and
                // the ramp is the more specific statement, so the driver wins
                // by clearing the temperature it would otherwise be overridden
                // by. Documented in the component, not discovered here.
                light.color_temperature_k = 0.0;
                if state.intensity.is_finite() {
                    light.intensity = state.intensity;
                }
            }
        }
        // Deliberately **not** setting `scene_dirty`: every value written above
        // is derived from fields that are themselves saved, so reopening the
        // scene reproduces them exactly. A day cycle that made a scene dirty by
        // existing would make "unsaved changes" meaningless.
        self.day_state = Some(state);
    }

    /// CONTROL-M: push the authored sky to the renderer.
    ///
    /// Runs after [`Self::apply_time_of_day`] so the day cycle's cloud-coverage
    /// track can override the authored coverage — which is the chain §6.3 asks
    /// for, and the reason coverage is a track on the clock rather than a
    /// second slider on the sky.
    fn apply_sky(&mut self, dt: f32) {
        let sky = self
            .world
            .entities()
            .find_map(|e| self.world.get::<crate::sky::SkyComponent>(e).copied());
        let coverage_override = self.day_state.and_then(|state| state.cloud_coverage);
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let Some(sky) = sky else {
            // No Environment: the pass stays exactly as the environment
            // variable left it, which is how a scene with no sky component
            // still honours `SOMNIUM_CLOUDS=1` for a capture.
            return;
        };
        renderer.cloud_pass.enabled = renderer.cloud_pass.env_override.unwrap_or(sky.enabled);
        let mut settings = sky.to_settings();
        if let Some(coverage) = coverage_override {
            settings.coverage = coverage;
        }
        renderer.cloud_pass.settings = settings;
        // The march target's size is part of the authored quality, so pushing
        // settings has to be followed by a resize. `resize` compares extents
        // and returns immediately when nothing changed, so this is free on
        // every frame that is not a quality change.
        if let Some(ctx) = self.render_ctx.as_ref() {
            let (rw, rh) = renderer.scene_extent();
            renderer.cloud_pass.resize(&ctx.device, rw, rh);
        }
        // The wind advances on wall-clock time rather than on the day cycle's
        // timescale: a paused editor should still let you watch the sky move
        // while you author it, and a 60× timescale should not turn the clouds
        // into a blur.
        renderer.cloud_pass.advance_wind(dt);
    }

    /// CONTROL-O: collect this frame's decals and bin them.
    ///
    /// Runs beside `submit_foliage` and for the same reason: the renderer
    /// should be handed a flat list, not asked to walk an ECS. The texture
    /// indices come out of the material pool, so a decal and the mesh beside
    /// it resolve the same material through the same slot — there is no second
    /// path from an `AssetId` to a bindless index.
    fn submit_decals(&mut self) {
        let collected: Vec<(glam::Mat4, crate::decal::DecalComponent, Option<u32>)> = self
            .world
            .entities()
            .filter_map(|entity| {
                if crate::is_hidden(&self.world, entity) {
                    return None;
                }
                let decal = self.world.get::<crate::decal::DecalComponent>(entity)?;
                if !decal.enabled || decal.opacity <= 0.001 {
                    return None;
                }
                let transform = entity_world_matrix(&self.world, entity)?;
                let material = self
                    .world
                    .get::<MaterialComponent>(entity)
                    .map(|m| m.runtime_id);
                Some((transform, *decal, material))
            })
            .collect();

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        renderer.decals.clear();
        for (transform, decal, material) in collected {
            let source = material.and_then(|id| renderer.materials_pool.get(id));
            let (base, albedo, normal, orm) =
                source.map_or(([1.0, 1.0, 1.0, 1.0], -1, -1, -1), |m| {
                    (
                        m.base_color,
                        m.albedo_map,
                        m.normal_map,
                        m.metallic_roughness_map,
                    )
                });
            renderer
                .decals
                .push(somnium_renderer::pass::decal::GpuDecal::new(
                    transform,
                    somnium_renderer::pass::decal::DecalLook {
                        base_color: [
                            base[0],
                            base[1],
                            base[2],
                            base[3] * decal.opacity.clamp(0.0, 1.0),
                        ],
                        albedo_map: albedo,
                        normal_map: normal,
                        orm_map: orm,
                        priority: decal.priority,
                        angle_fade_degrees: decal.angle_fade_degrees,
                        normal_strength: decal.normal_strength,
                        roughness: decal.roughness,
                    },
                ));
        }
    }

    /// CONTROL-N: step the weather and push it everywhere it is read.
    ///
    /// Runs after [`Self::apply_sky`] so the sky's wind is the one this
    /// overwrites rather than the other way round — §6.3's "wind becomes one
    /// global vector" only means anything if there is a single last writer.
    ///
    /// Nothing here sets `scene_dirty`. Every value written is derived from
    /// fields that are themselves saved, and a world that got dirty by raining
    /// would make "unsaved changes" meaningless.
    fn apply_weather(&mut self, dt: f32) {
        let weather = self.world.entities().find_map(|e| {
            self.world
                .get::<crate::weather::WeatherComponent>(e)
                .copied()
        });
        let Some(weather) = weather else {
            self.weather_state = crate::weather::WeatherState::default();
            return;
        };
        self.weather_state = weather.step(self.weather_state, dt);
        let state = self.weather_state;

        // ── The one wind ─────────────────────────────────────────────────────
        //
        // Clouds, the ocean spectrum and precipitation shear all read this.
        // Written only while the weather is enabled, so a scene with weather
        // off keeps every authored wind exactly as it was.
        if weather.enabled {
            let speed = (state.wind[0] * state.wind[0] + state.wind[1] * state.wind[1]).sqrt();
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.cloud_pass.settings.wind = state.wind;
                renderer.water_pass.rain_ripple = state.ripples;
            }
            // The sea roughens because the wind does, through the spectrum the
            // water already had — not through a second "storminess" knob.
            let bodies: Vec<somnium_ecs::Entity> = self
                .world
                .entities()
                .filter(|e| self.world.get::<WaterComponent>(*e).is_some())
                .collect();
            for entity in bodies {
                if let Some(water) = self.world.get_mut::<WaterComponent>(entity) {
                    water.wind_speed = speed;
                }
            }
        } else if let Some(renderer) = self.renderer.as_mut() {
            renderer.water_pass.rain_ripple = 0.0;
        }

        // ── Wetness ──────────────────────────────────────────────────────────
        if let (Some(renderer), Some(ctx)) = (self.renderer.as_mut(), self.render_ctx.as_ref()) {
            // Meshes, through the one uniform `shading.wgsl` reads. Written
            // every frame including when the weather is off, so the uniform
            // can never hold a stale wetness from weather that has stopped.
            renderer.shading_pass.set_weather(
                &ctx.queue,
                state.wet_diffuse,
                state.wet_specular,
                state.puddles,
            );
            // Terrain, through XV-H's existing uniform. Driven rather than
            // replaced: `SOMNIUM_TERRAIN_WETNESS` and the Terrain tool's own
            // slider still author the value when the weather is off.
            if weather.enabled {
                for terrain in &mut renderer.terrains {
                    terrain.wetness = state.wet_diffuse;
                }
            }
        }

        self.apply_precipitation(weather, state);
    }

    /// CONTROL-N: keep the rain emitter in step with the weather.
    ///
    /// The existing `ParticleEmitter`, camera-anchored and wind-sheared, rather
    /// than a second particle system — which is what CONTROL-K's
    /// `velocity_bias` and `spawn_extents` were added for.
    fn apply_precipitation(
        &mut self,
        weather: crate::weather::WeatherComponent,
        state: crate::weather::WeatherState,
    ) {
        use crate::weather::Precipitation;

        let falling = weather.enabled && state.rate > 0.01;
        if !falling {
            if let Some(entity) = self.precipitation_entity.take() {
                // Despawned rather than left with a zero rate: an emitter with
                // no particles is still an outliner row somebody has to
                // wonder about.
                let _ = self.world.despawn(entity);
            }
            return;
        }

        let camera = self
            .renderer
            .as_ref()
            .map_or(glam::Vec3::ZERO, |r| r.camera_pos);
        let snow = state.precipitation == Precipitation::Snow;

        // Snow falls at about 1 m/s and drifts; rain at about 9 and shears
        // hard. The same emitter, two sets of numbers.
        let fall_speed = if snow { -1.2 } else { -9.0 };
        let shear = if snow { 0.35 } else { 0.12 };
        let colour = if snow {
            somnium_ecs::curve::Gradient::ramp([0.9, 0.93, 1.0, 0.85], [0.9, 0.93, 1.0, 0.0])
        } else {
            somnium_ecs::curve::Gradient::ramp([0.62, 0.70, 0.82, 0.55], [0.62, 0.70, 0.82, 0.0])
        };
        // A box overhead, wide enough that the far edge is out of frame and
        // tall enough that a particle lives long enough to be seen falling.
        let extents = [26.0_f32, 12.0, 26.0];
        let emitter = crate::ParticleEmitter {
            max_particles: 20_000,
            spawn_rate: weather.particle_rate.max(0.0) * state.rate,
            lifetime: 3.0,
            initial_speed: 0.0,
            spread_angle: 0.0,
            size_start: if snow { 0.09 } else { 0.035 },
            size_end: if snow { 0.09 } else { 0.035 },
            color_over_life: colour,
            gravity: 0.0,
            velocity_bias: [state.wind[0] * shear, fall_speed, state.wind[1] * shear],
            spawn_extents: extents,
            ..crate::ParticleEmitter::default()
        };
        let origin = Transform::from_translation(camera + glam::Vec3::new(0.0, 14.0, 0.0));

        match self.precipitation_entity {
            Some(entity) if self.world.is_alive(entity) => {
                if let Some(transform) = self.world.get_mut::<Transform>(entity) {
                    *transform = origin;
                }
                if let Some(existing) = self.world.get_mut::<crate::ParticleEmitter>(entity) {
                    // The live particle list is kept: rewriting the whole
                    // component every frame would restart the rain sixty times
                    // a second.
                    let particles = std::mem::take(&mut existing.particles);
                    let accum = existing.spawn_accum;
                    *existing = emitter;
                    existing.particles = particles;
                    existing.spawn_accum = accum;
                }
            }
            _ => {
                let entity = self.world.spawn((
                    origin,
                    Name::new("Precipitation"),
                    WorldTransform::identity(),
                    emitter,
                ));
                self.precipitation_entity = Some(entity);
            }
        }
    }

    /// Snapshot a component value through its schema, without a live entity.
    ///
    /// A scratch world rather than a hand-built `ReflectObject`: the record has
    /// to come from the schema, or a preset would be a second description of
    /// the component's field order — exactly what Seam 1 exists to prevent.
    fn stage_component<C: somnium_ecs::Component + Clone>(
        value: C,
    ) -> Option<somnium_ecs::reflect::ReflectObject> {
        let mut scratch = somnium_ecs::World::new();
        let entity = scratch.spawn((value,));
        let registry = crate::reflect_registry::editor_registry();
        let schema = registry
            .iter()
            .find(|schema| schema.component_id == somnium_ecs::ComponentId::of::<C>())?;
        (schema.snapshot)(&scratch, entity)
    }

    /// Build the command that applies a named sky preset, or `None`.
    fn sky_preset_command(
        &self,
        id: &str,
    ) -> Option<Box<dyn crate::editor_commands::EditorCommand>> {
        let entity = self
            .world
            .entities()
            .find(|e| self.world.get::<crate::sky::SkyComponent>(*e).is_some())?;
        let mut next = self
            .world
            .get::<crate::sky::SkyComponent>(entity)
            .copied()?;
        if !next.apply_preset(id) {
            return None;
        }
        let values = Self::stage_component(next)?;
        crate::editor_commands::SetComponentCmd::new(
            &self.world,
            entity,
            somnium_ecs::reflect::StableId::new("somnium.Sky"),
            values,
            "Sky preset",
        )
        .ok()
        .map(|command| Box::new(command) as Box<dyn crate::editor_commands::EditorCommand>)
    }

    /// Push one or more component writes as a single named undo entry.
    fn push_environment_preset(
        &mut self,
        commands: Vec<Box<dyn crate::editor_commands::EditorCommand>>,
        description: String,
    ) {
        if commands.is_empty() {
            return;
        }
        // `CommandGroup` wants a `&'static str`; a preset's label is built at
        // runtime from the registry's table, so it is leaked once per click.
        // The alternative is a history that reads "Change" — Stride's rule
        // again, and the leak is a handful of bytes per authoring action.
        let description: &'static str = Box::leak(description.into_boxed_str());
        let group = crate::editor_commands::CommandGroup::new(description, commands);
        self.undo_stack.push(
            Box::new(group),
            &mut self.world,
            &mut self.selection.primary,
        );
        self.scene_dirty = true;
    }

    /// CONTROL-L: publish the clock to the viewport context bar.
    ///
    /// Separate from [`Self::apply_time_of_day`] because the bar must show the
    /// hour whether or not the cycle is *enabled* — a disabled cycle is still
    /// a cycle you are about to scrub — while the driver must do nothing at
    /// all when it is off.
    fn publish_time_of_day(&mut self) {
        let hour = self.world.entities().find_map(|e| {
            self.world
                .get::<crate::time_of_day::TimeOfDayComponent>(e)
                .map(|tod| tod.hour.rem_euclid(24.0))
        });
        if let Some(ui) = self.ui_manager.as_mut() {
            ui.update_time_of_day(hour);
        }
    }

    /// Push the scene's post-processing settings to the renderer (Phase 15A1).
    ///
    /// Driven by the first entity carrying a `PostProcessComponent`. With no
    /// Legacy scenes with none or several are normalized to one before the
    /// selected settings are copied into the renderer.
    fn apply_post_process(&mut self) {
        normalize_post_process_singleton(&mut self.world, &mut self.selection.primary);
        // Prefer the selected Post Processing entity. This makes the details
        // panel authoritative even if an imported legacy scene accidentally
        // contains a duplicate; falling back to the first keeps old scenes
        // working when another entity is selected.
        let settings = self
            .selection
            .primary
            .and_then(|e| self.world.get::<PostProcessComponent>(e).cloned())
            .or_else(|| {
                self.world
                    .entities()
                    .find_map(|e| self.world.get::<PostProcessComponent>(e).cloned())
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
            r.exposure_compensation = self
                .day_state
                .and_then(|state| state.exposure_compensation)
                .unwrap_or(pp.exposure_compensation);
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
                .set_enabled(pp.fsr_enabled() && !path_active && fsr_safe_for_lighting);
            r.fsr_pass.sharpness = pp.fsr_sharpness;
            // Use the pass's effective state, not the authored request: on a
            // device without FSR features, `aa` may be `Fsr` while the pass
            // correctly declined it. TAA/CAS must still be allowed.
            let fsr_active = r.fsr_pass.enabled;
            let fsr_fallback = pp.fsr_enabled() && !path_active && !fsr_active;
            r.taa_pass
                .set_enabled((pp.taa_enabled() || fsr_fallback) && !fsr_active && !path_active);
            r.gtao_pass.enabled = pp.gtao_enabled && !path_active;
            r.bloom_pass.enabled = pp.bloom_enabled;
            r.bloom_pass.intensity = pp.bloom_intensity;
            r.dof_pass.enabled = pp.dof_enabled && !path_active;
            r.dof_pass.focus_distance = pp.dof_focus_distance;
            r.dof_pass.f_stop = pp.aperture_f_stops;
            r.restir_pass.enabled = pp.restir_enabled && !path_active;
            let restir_gi_active =
                pp.restir_gi_enabled && r.restir_gi_pass.supported() && !path_active;
            r.restir_gi_pass.enabled = restir_gi_active;
            // `probes` is the pre-AB scene field. Treat it as a compatibility
            // request for the portable tier; ReSTIR remains the explicit
            // higher-quality winner if both old/new fields are authored.
            let ddgi_active = (pp.ddgi_enabled || pp.probes) && !restir_gi_active && !path_active;
            r.ddgi_pass.configure(
                ddgi_active,
                somnium_renderer::pass::ddgi::DdgiConfig {
                    spacing: pp.ddgi_probe_spacing_m,
                    update_budget: pp.ddgi_update_budget,
                    hysteresis: pp.ddgi_hysteresis,
                    intensity: if pp.ddgi_enabled {
                        pp.ddgi_intensity
                    } else {
                        pp.probe_intensity
                    },
                },
            );
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
            r.volumetric_pass.fog.density = self
                .day_state
                .and_then(|state| state.fog_density)
                .unwrap_or(pp.fog_density);
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
                    if pp.world_cache && rt && !ddgi_active {
                        flags |= FLAG_CACHE;
                    }
                    if pp.specular_gi && rt {
                        flags |= FLAG_SPECULAR;
                    }
                    if ddgi_active || (pp.mesh_sdf && !pp.world_cache) {
                        flags |= FLAG_SDF;
                    }
                    if ddgi_active {
                        flags |= FLAG_PROBES;
                    }
                }
                r.lighting_extra_pass.flags = flags;
                r.lighting_extra_pass.intensity = pp.cache_intensity;
                r.lighting_extra_pass.cell_size = if ddgi_active {
                    pp.ddgi_probe_spacing_m
                } else {
                    pp.cache_cell_size
                };
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
                // CONTROL-K: the authored curve is sampled here, once per
                // frame, and the renderer never sees a keyframe. This is the
                // whole "no refresh button" story: editing the curve changes
                // the table on the very next frame because the table is
                // rebuilt on every frame regardless.
                response: (!pp.response_curve.is_empty()).then(|| {
                    let mut table = [0.0_f32; 32];
                    pp.response_curve.sample_into(0.0, 1.0, &mut table);
                    table
                }),
            };
            r.vignette_strength = pp.effective_vignette();
            r.chromatic_aberration = pp.effective_ca();
            // MORROWIND-AC. One authored value in, two effective flags out, and
            // the renderer no longer re-derives precedence from three booleans.
            r.fxaa_enabled = pp.fxaa_enabled();
            r.oit_pass.enabled = pp.oit_enabled && !path_active;
            r.smaa_pass.set_mode(
                pp.smaa_enabled(),
                pp.smaa_preset.threshold(),
                pp.smaa_preset.max_search_steps(),
            );
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
        let terrains: Vec<(
            u32,
            glam::Mat4,
            f32,
            f32,
            f32,
            f32,
            somnium_ecs::curve::Curve,
        )> = self
            .world
            .entities()
            .filter_map(|e| {
                if crate::is_hidden(&self.world, e) {
                    return None;
                }
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
                    fc.lod_falloff.clone(),
                ))
            })
            .collect();

        if let Some(r) = self.renderer.as_mut() {
            r.profiler.cpu_begin("Foliage");
        }

        for (
            terrain_id,
            model,
            cull_distance,
            shadow_distance,
            lod_distance,
            impostor_distance,
            lod_falloff,
        ) in terrains
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
                // CONTROL-K: the authored falloff curve, evaluated against
                // normalised horizontal distance. An empty curve is the
                // pre-CONTROL-K behaviour exactly — no multiply at all, not a
                // multiply by one, so an unauthored foliage component cannot
                // change even by a rounding error.
                let horizontal_sq = d.x * d.x + d.z * d.z;
                let scale = if lod_falloff.is_empty() || cull_distance <= 0.0 {
                    inst.scale
                } else {
                    inst.scale * lod_falloff.evaluate(horizontal_sq.sqrt() / cull_distance)
                };
                if scale <= 0.0 {
                    continue;
                }
                let placement = glam::Mat4::from_scale_rotation_translation(
                    glam::Vec3::splat(scale),
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

        let selected_idx = self.selection.primary.map(|e| e.index());
        let gizmos: Vec<LightGizmoDesc> = self
            .world
            .entities()
            .filter_map(|e| {
                if crate::is_hidden(&self.world, e) {
                    return None;
                }
                let light = self.world.get::<LightComponent>(e)?;
                let transform = entity_world_matrix(&self.world, e)?;
                let (_, rotation, position) = transform.to_scale_rotation_translation();
                let kind = match light.light_type {
                    LightType::Directional => LightGizmoKind::Directional,
                    LightType::Point | LightType::Rect | LightType::Disc => LightGizmoKind::Point,
                    LightType::Spot | LightType::Tube => LightGizmoKind::Spot,
                };
                Some(LightGizmoDesc {
                    kind,
                    position,
                    direction: rotation.mul_vec3(glam::Vec3::NEG_Z),
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

    /// Queue attenuation and directivity shapes for authored audio emitters.
    /// Draw every spline as a polyline, with a tick at each control point.
    ///
    /// The curve the author is editing has to be visible before it can be
    /// edited, and the polyline drawn here is the *same* sampling the
    /// nearest-point query uses — so what you see is exactly what the audio
    /// hears, rather than a smooth curve drawn beside a coarse one that is
    /// actually in effect.
    fn submit_spline_gizmos(&mut self) {
        use somnium_renderer::pass::light_gizmo::LineVertex;

        // Selected splines are drawn bright; the rest are dimmed, the same
        // rule the light gizmos already use.
        const BRIGHT: [f32; 3] = [0.30, 0.90, 0.85];
        const DIM: [f32; 3] = [0.13, 0.38, 0.36];

        let mut lines: Vec<LineVertex> = Vec::new();
        let entities: Vec<_> = self.world.entities().collect();
        for entity in entities {
            let Some(spline) = self.world.get::<crate::SplineComponent>(entity) else {
                continue;
            };
            let flags = self
                .world
                .get::<EditorFlags>(entity)
                .copied()
                .unwrap_or_default();
            if flags.hidden {
                continue;
            }
            let model = self
                .world
                .get::<WorldTransform>(entity)
                .map(|world| world.0)
                .or_else(|| {
                    self.world
                        .get::<Transform>(entity)
                        .map(Transform::to_matrix)
                })
                .unwrap_or(glam::Mat4::IDENTITY);
            let colour = if self.selection.contains(entity) {
                BRIGHT
            } else {
                DIM
            };

            let path: Vec<glam::Vec3> = spline
                .polyline()
                .iter()
                .map(|point| model.transform_point3(*point))
                .collect();
            for pair in path.windows(2) {
                lines.push(LineVertex {
                    position: pair[0].to_array(),
                    color: colour,
                });
                lines.push(LineVertex {
                    position: pair[1].to_array(),
                    color: colour,
                });
            }

            // A cross at each control point, sized in metres rather than in
            // screen space: these mark authored data, and an author needs to
            // see which bend belongs to a point they can move.
            const TICK: f32 = 0.6;
            for point in &spline.points {
                let centre = model.transform_point3(*point);
                for axis in [glam::Vec3::X, glam::Vec3::Y, glam::Vec3::Z] {
                    lines.push(LineVertex {
                        position: (centre - axis * TICK).to_array(),
                        color: colour,
                    });
                    lines.push(LineVertex {
                        position: (centre + axis * TICK).to_array(),
                        color: colour,
                    });
                }
            }
        }

        if !lines.is_empty()
            && let Some(renderer) = self.renderer.as_mut()
        {
            renderer.submit_gizmo_lines(lines);
        }
    }

    fn submit_audio_gizmos(&mut self) {
        use somnium_renderer::pass::light_gizmo::{LightGizmoDesc, LightGizmoKind};

        let selected_idx = self.selection.primary.map(|entity| entity.index());
        let gizmos: Vec<_> = self
            .world
            .entities()
            .filter_map(|entity| {
                if crate::is_hidden(&self.world, entity) {
                    return None;
                }
                let audio = self.world.get::<crate::AudioEmitterComponent>(entity)?;
                let transform = entity_world_matrix(&self.world, entity)?;
                let (_, rotation, position) = transform.to_scale_rotation_translation();
                Some(LightGizmoDesc {
                    kind: if audio.cone_enabled {
                        LightGizmoKind::AudioCone
                    } else {
                        LightGizmoKind::AudioOmni
                    },
                    position,
                    direction: rotation * glam::Vec3::NEG_Z,
                    color: glam::Vec3::new(0.15, 0.85, 1.0),
                    range: audio.max_distance,
                    inner_angle: audio.cone_inner_degrees.to_radians(),
                    outer_angle: audio.cone_outer_degrees.to_radians(),
                    selected: selected_idx == Some(entity.index()),
                })
            })
            .collect();
        if let Some(renderer) = self.renderer.as_mut() {
            for gizmo in gizmos {
                renderer.submit_light_gizmo(gizmo);
            }
        }
    }

    /// Queue every terrain entity for rendering this frame.
    fn submit_terrains(&mut self) {
        let terrains: Vec<(Entity, TerrainComponent, glam::Mat4)> = self
            .world
            .entities()
            .filter_map(|e| {
                // The Outliner's eye means "not drawn", for a terrain exactly
                // as for a mesh.
                if crate::is_hidden(&self.world, e) {
                    return None;
                }
                let tc = self.world.get::<TerrainComponent>(e).copied()?;
                let model = self
                    .world
                    .get::<Transform>(e)
                    .map_or(glam::Mat4::IDENTITY, Transform::to_matrix);
                Some((e, tc, model))
            })
            .collect();
        let mut diagnostics = Vec::with_capacity(terrains.len());
        if let Some(r) = self.renderer.as_mut() {
            for (entity, component, model) in terrains {
                let mut virtual_texturing = false;
                if let Some(terrain) = r.terrain_mut(component.terrain_id) {
                    terrain.configure_virtual_texture(
                        component.virtual_texturing,
                        component.virtual_texture_cache_mib,
                        component.virtual_texture_uploads_per_frame,
                    );
                    let stats = *terrain.virtual_texture.stats();
                    virtual_texturing = terrain.virtual_texture_enabled;
                    diagnostics.push((
                        entity,
                        stats,
                        terrain.virtual_texture_enabled,
                        terrain.virtual_texture_cache_mib,
                    ));
                }
                if virtual_texturing
                    && !somnium_renderer::terrain::clipmap::TerrainClipmap::env_forced_off()
                    && let Some(clipmap) = r.clipmaps.get_mut(component.terrain_id as usize)
                    && !clipmap.enabled
                {
                    clipmap.enabled = true;
                    clipmap.invalidate();
                }
                r.submit_terrain(component.terrain_id, model);
            }
        }
        for (entity, stats, enabled, cache_mib) in diagnostics {
            if let Some(component) = self.world.get_mut::<TerrainComponent>(entity) {
                component.virtual_texturing = enabled;
                if enabled {
                    component.virtual_texture_cache_mib = cache_mib;
                }
                component.virtual_texture_resident_pages = stats.resident_pages;
                component.virtual_texture_pending_pages = stats.pending_pages;
                component.virtual_texture_hits = stats.hits.min(u64::from(u32::MAX)) as u32;
                component.virtual_texture_misses = stats.misses.min(u64::from(u32::MAX)) as u32;
                component.virtual_texture_evictions =
                    stats.evictions.min(u64::from(u32::MAX)) as u32;
            }
        }
    }

    /// Open a panel in its own OS window, or focus the one already showing it.
    ///
    /// MORROWIND-J step 2. Created here rather than anywhere else because
    /// `create_window` needs an `ActiveEventLoop`, and this is the point in the
    /// frame that has one *and* has just drained the editor's events.
    fn float_panel(
        &mut self,
        event_loop: &ActiveEventLoop,
        kind: somnium_ui::floating::FloatingKind,
    ) {
        if let Some(existing) = self.floating.iter().find(|w| w.kind == kind) {
            // Already open. Raising it is what a user means by asking twice.
            existing.window.focus_window();
            return;
        }
        let Some(ctx) = self.render_ctx.as_ref() else {
            return;
        };
        let (w, h) = kind.default_size();
        let attrs = WindowAttributes::default()
            .with_title(kind.title())
            .with_inner_size(LogicalSize::new(w, h));
        // The panel is already detached by the time this runs — the manager
        // does that when the command fires. So every failure below has to put
        // it back, or it is in no window at all and there is no control left
        // anywhere to bring it home.
        let window = match event_loop.create_window(attrs) {
            Ok(window) => std::sync::Arc::new(window),
            Err(error) => {
                error!(?error, "could not open a floating window");
                self.dock_panel(kind);
                return;
            }
        };
        let surface = match ctx.instance.create_surface(std::sync::Arc::clone(&window)) {
            Ok(surface) => surface,
            Err(error) => {
                error!(?error, "floating window has no drawable surface");
                self.dock_panel(kind);
                return;
            }
        };
        let size = window.inner_size();
        let mut config = ctx.config.clone();
        config.width = size.width.max(1);
        config.height = size.height.max(1);
        // So `SOMNIUM_FLOAT_PNG` can read this window back. The editor's own
        // surface asks for the same thing for the same reason, and a floating
        // window that could not be looked at without a camera is a floating
        // window nobody can review.
        config.usage |= somnium_ui::wgpu::TextureUsages::COPY_SRC;
        surface.configure(&ctx.device, &config);

        let floating = FloatingWindow {
            kind,
            window,
            surface,
            config,
            frames: 0,
            captured: false,
            reported: false,
        };
        // The panel is told its window's size before its first layout, so the
        // first frame is drawn for this window rather than for the default the
        // detach used.
        if let Some(ui) = self.ui_manager.as_mut() {
            let logical = floating.logical(ui);
            ui.resize_floating(kind, logical);
        }
        floating.window.request_redraw();
        info!(?kind, "floating window opened");
        if std::env::var("SOMNIUM_FLOAT_PNG").is_ok() {
            // Holds `SOMNIUM_CAPTURE_QUIT` open until this window has written
            // its frame, which is always a little after the editor writes its
            // own because this window opened later.
            somnium_renderer::capture::expect_surface_capture();
        }
        self.floating.push(floating);
        if kind.hosts_scene() {
            self.resize_scene_targets();
        }
    }

    /// Return a panel to the dock, wherever the decision was made.
    fn dock_panel(&mut self, kind: somnium_ui::floating::FloatingKind) {
        if let Some(ui) = self.ui_manager.as_mut() {
            ui.set_panel_floating(kind, false);
        }
        if kind.hosts_scene() {
            // The scene comes back to this window, and it is a different size.
            self.resize_scene_targets();
        }
    }

    /// Route a window event to a floating window, if it belongs to one.
    ///
    /// The main window's handler must not act on a resize that was not its own
    /// — that is the bug a single-window event loop grows the moment it gains a
    /// second window, and it presents as a scissor rectangle validated against
    /// the wrong target. See [`FloatingRoute`] for the case that is not simply
    /// "mine" or "not mine".
    fn floating_window_event(&mut self, id: WindowId, event: &WindowEvent) -> FloatingRoute {
        let Some(index) = self.floating.iter().position(|w| w.window.id() == id) else {
            return FloatingRoute::Main;
        };
        match event {
            WindowEvent::CloseRequested => {
                // Closing returns the panel to the dock rather than losing it.
                let closed = self.floating.remove(index);
                info!(kind = ?closed.kind, "floating window closed");
                self.dock_panel(closed.kind);
            }
            WindowEvent::Resized(size) => {
                if let (Some(ctx), Some(ui)) =
                    (self.render_ctx.as_ref(), self.ui_manager.as_mut())
                {
                    self.floating[index].resize(&ctx.device, ui, size.width, size.height);
                }
                if self.floating[index].kind.hosts_scene() {
                    self.resize_scene_targets();
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                // Deliberately nothing. See `FloatingWindow::scale`: the tree
                // has one scale, and taking this window's would split layout
                // from input. Windows follows this with a `Resized`, which is
                // handled above.
            }
            // Everything else is the panel's: pointer, wheel, keys. It reaches
            // the editor's own interface with hit-testing rooted at this
            // panel, so a click here runs exactly the handler a click on the
            // docked panel would have run.
            other => {
                let kind = self.floating[index].kind;
                let consumed = match self.ui_manager.as_mut() {
                    Some(ui) => ui.process_floating_event(kind, other),
                    None => false,
                };
                if kind.hosts_scene() && !consumed {
                    // The pointer was over the render rather than over the
                    // bar. A detached viewport is laid out at its window's
                    // origin, so this window's cursor position and
                    // `viewport_physical_rect` are already in the same space
                    // and the editor's tools need no translation to work here.
                    return FloatingRoute::Viewport;
                }
            }
        }
        FloatingRoute::Handled
    }

    /// Size the renderer's internal targets to whichever window holds the scene.
    ///
    /// MORROWIND-J step 2. The scene is recorded at `render_width` x
    /// `render_height` and lands on a surface; when the viewport floats, that
    /// surface is another window, and targets still sized to the editor's would
    /// render the scene at the wrong resolution and upscale it into a window
    /// that had the pixels all along.
    fn resize_scene_targets(&mut self) {
        let floated = self.ui_manager.as_ref().is_some_and(|ui| {
            ui.is_panel_floating(somnium_ui::floating::FloatingKind::Viewport)
        });
        let size = if floated {
            self.floating
                .iter()
                .find(|w| w.kind.hosts_scene())
                .map(|w| (w.config.width, w.config.height))
        } else {
            self.render_ctx.as_ref().map(|c| (c.config.width, c.config.height))
        };
        // Floating but with no window yet, which is the frame between the
        // command firing and the event loop opening one. The next call catches
        // it, and until then the editor's own targets are the right ones.
        let Some((width, height)) = size else {
            return;
        };
        if let (Some(r), Some(c)) = (&mut self.renderer, &self.render_ctx) {
            let (sw, sh) =
                somnium_renderer::scene_size_for_preset(width, height, self.viewport_resolution);
            r.resize(c, sw, sh);
        }
    }

    /// Take the floating viewport's next swapchain image, if there is one.
    ///
    /// Two conditions, and both matter. The window has to exist, which it does
    /// not for the frame between the command firing and the event loop opening
    /// it; and the panel has to still be floating, which it is not for the
    /// frame between a close and the window being dropped. In either gap the
    /// editor renders the scene into its own window, which is correct — the
    /// dock is holding the viewport in exactly those frames.
    fn acquire_floating_viewport(&mut self) -> Option<AcquiredFrame> {
        use somnium_ui::wgpu;
        let floating = self
            .ui_manager
            .as_ref()?
            .is_panel_floating(somnium_ui::floating::FloatingKind::Viewport);
        if !floating {
            return None;
        }
        let window = self.floating.iter().find(|w| w.kind.hosts_scene())?;
        let output = match window.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(tex)
            | wgpu::CurrentSurfaceTexture::Suboptimal(tex) => tex,
            // Resizing or occluded. The editor draws its own frame without a
            // scene for one frame, which the expanded panels cover.
            _ => return None,
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        Some(AcquiredFrame {
            output,
            view,
            size: (window.config.width, window.config.height),
        })
    }

    /// Draw every floating window, after the editor's own frame.
    ///
    /// Nothing is synchronised here and nothing is copied. Each window renders
    /// the panel's own nodes out of the editor's own interface, so a floating
    /// panel cannot fall behind the docked one: there is no second copy of it
    /// to fall behind.
    fn render_floating(&mut self, mut scene_frame: Option<AcquiredFrame>) {
        let Some(ctx) = self.render_ctx.as_ref() else {
            return;
        };
        let Some(ui) = self.ui_manager.as_mut() else {
            return;
        };
        for window in &mut self.floating {
            let frame = if window.kind.hosts_scene() {
                scene_frame.take()
            } else {
                None
            };
            window.render(ui, ctx, frame);
        }
        // Acquired for a viewport window that has closed since. Dropping the
        // image without presenting is how a surface frame is abandoned; leaking
        // it would stall the swapchain within a few frames.
        drop(scene_frame);
    }

    fn handle_editor_event(&mut self, ev: EditorEvent) {
        use somnium_ui::{CreateKind, FoliageBrushField as FB, TerrainToolField as TT};

        match ev {
            EditorEvent::CompleteDrop(request) => {
                use somnium_ui::DropRequest;
                match request {
                    DropRequest::CreateDecal { asset, at } => {
                        let position = glam::Vec3::from(at);
                        let normal = self.terrain_normal_at(position);
                        let transform = crate::decal::placement(position, normal);
                        let runtime_id = self.material_runtime_id(asset);
                        let snapshot = EntitySnapshot {
                            transform: Some(transform),
                            name: Some(Name::new("Decal")),
                            wt: Some(WorldTransform::identity()),
                            decal: Some(crate::decal::DecalComponent::default()),
                            mat: Some(MaterialComponent { asset, runtime_id }),
                            ..EntitySnapshot::default()
                        };
                        // One drop, one undo step — CONTROL-E's rule, and this
                        // is the eighth route through it.
                        let cmd = Box::new(CreateEntityCmd::new(snapshot));
                        self.undo_stack
                            .push(cmd, &mut self.world, &mut self.selection.primary);
                        self.scene_dirty = true;
                        info!("Created decal");
                    }
                    DropRequest::AssignMaterial { asset, entities } => {
                        self.handle_editor_event(EditorEvent::AssignMaterial { entities, asset });
                    }
                    DropRequest::SetAssetField {
                        asset,
                        entity,
                        component,
                        field,
                    } => {
                        self.handle_editor_event(EditorEvent::SetComponentField {
                            entity,
                            component,
                            field,
                            value: somnium_ecs::reflect::ReflectValue::Asset(Some(
                                somnium_ecs::reflect::AssetRef::from_raw(asset.raw()),
                            )),
                            gesture: GestureId(u64::MAX - 1),
                            live: false,
                        });
                    }
                    DropRequest::Reparent { entities, parent } => {
                        match crate::editor_commands::ReparentBatchCmd::new(
                            &mut self.world,
                            entities,
                            parent,
                        ) {
                            Ok(command) => {
                                self.undo_stack.push(
                                    Box::new(command),
                                    &mut self.world,
                                    &mut self.selection.primary,
                                );
                                self.scene_dirty = true;
                            }
                            Err(reason) => {
                                if let Some(ui) = self.ui_manager.as_mut() {
                                    ui.push_toast(&reason);
                                }
                            }
                        }
                    }
                    DropRequest::AttachScripts { assets, entity } => {
                        let paths: Vec<_> = assets
                            .into_iter()
                            .filter_map(|id| {
                                self.asset_gate
                                    .published()
                                    .and_then(|db| db.get(id))
                                    .map(|r| r.absolute_path.clone())
                            })
                            .collect();
                        let mut commands: Vec<Box<dyn crate::editor_commands::EditorCommand>> =
                            Vec::new();
                        for path in paths {
                            match self.scripts.import_script_file(&path) {
                                Ok(asset) => commands.push(Box::new(
                                    crate::editor_commands::AttachScriptCmd::new(
                                        entity.index(),
                                        asset,
                                    ),
                                )),
                                Err(diagnostics) => {
                                    for message in diagnostics.messages {
                                        warn!(%message, "script drop rejected");
                                    }
                                }
                            }
                        }
                        if !commands.is_empty() {
                            self.undo_stack.push(
                                Box::new(crate::editor_commands::CommandGroup::new(
                                    "Attach Scripts",
                                    commands,
                                )),
                                &mut self.world,
                                &mut self.selection.primary,
                            );
                            self.scene_dirty = true;
                        }
                    }
                    DropRequest::LoadScene { asset } => {
                        if let Some(path) = self
                            .asset_gate
                            .published()
                            .and_then(|db| db.get(asset))
                            .map(|r| r.absolute_path.to_string_lossy().into_owned())
                        {
                            self.handle_editor_event(EditorEvent::LoadScene(path));
                        }
                    }
                    DropRequest::SpawnModels { assets, at } => {
                        if let Some(path) = assets
                            .first()
                            .and_then(|id| self.asset_gate.published().and_then(|db| db.get(*id)))
                            .map(|r| r.absolute_path.clone())
                        {
                            self.queue_import_model(path, at);
                        }
                    }
                    DropRequest::ImportExternal { files, folder } => {
                        self.queue_external_import(files, folder)
                    }
                }
            }
            EditorEvent::ModifySelection { id, mode } => {
                let Some(entity) = self.world.find_entity_by_index(id) else {
                    return;
                };
                match mode {
                    somnium_ui::SelectionMode::Replace => {
                        self.selection.set_single(Some(entity));
                    }
                    somnium_ui::SelectionMode::Toggle => self.selection.toggle(entity),
                    somnium_ui::SelectionMode::Range => {
                        let order = std::mem::take(&mut self.outliner_order);
                        self.selection.extend_range(&order, entity);
                        self.outliner_order = order;
                    }
                }
                self.after_selection_change();
            }

            EditorEvent::SelectEntities(ids) => {
                let entities: Vec<_> = ids
                    .into_iter()
                    .filter_map(|idx| self.world.find_entity_by_index(idx))
                    .collect();
                if entities.is_empty() {
                    self.selection.clear();
                } else {
                    self.selection.set_many(entities);
                }
                self.after_selection_change();
            }

            EditorEvent::CopySelected => {
                if self.selection.is_empty() {
                    return;
                }
                self.entity_clipboard =
                    crate::clipboard::EntityClipboard::copy(&self.world, self.selection.as_slice());
                let count = self.entity_clipboard.root_count();
                if let Some(ui) = self.ui_manager.as_mut() {
                    ui.push_toast(&format!("Copied {count}"));
                }
            }

            EditorEvent::PasteClipboard => {
                if self.entity_clipboard.is_empty() {
                    return;
                }
                // Pasting with something selected pastes *into* it, which is
                // what makes Copy/Paste usable for building a hierarchy rather
                // than only for cloning one.
                let parent = self.selection.primary;
                use crate::editor_commands::EditorCommand as _;
                let mut command =
                    crate::clipboard::PasteEntitiesCmd::new(self.entity_clipboard.clone(), parent);
                command.execute(&mut self.world, &mut self.selection.primary);
                let roots = command.roots().to_vec();
                self.undo_stack.push_silent(Box::new(command));
                self.selection.set_many(roots);
                self.after_selection_change();
                self.scene_dirty = true;
            }

            EditorEvent::ToggleEntityFlag {
                entity,
                lock,
                value,
            } => {
                let Some(entity) = self.world.find_entity_by_index(entity) else {
                    return;
                };
                let current = self
                    .world
                    .get::<EditorFlags>(entity)
                    .copied()
                    .unwrap_or_default();
                let mut next = current;
                if lock {
                    next.locked = value.unwrap_or(!current.locked);
                } else {
                    next.hidden = value.unwrap_or(!current.hidden);
                }
                if next == current {
                    return;
                }
                self.undo_stack.push(
                    Box::new(crate::editor_commands::SetEditorFlagsCmd::new(
                        entity, current, next,
                    )),
                    &mut self.world,
                    &mut self.selection.primary,
                );
                // A locked entity keeps its Outliner selection but loses the
                // gizmo, so the viewport stops offering a transform it would
                // refuse to perform.
                self.after_selection_change();
                self.scene_dirty = true;
            }

            EditorEvent::SetSetting {
                component,
                field,
                value,
            } => {
                if let Err(reason) = self.settings.set(component, field, value)
                    && let Some(ui) = self.ui_manager.as_mut()
                {
                    // The "overridden by ..." reason reaches the person, rather than
                    // the control appearing to accept a value it discarded.
                    ui.push_toast(&reason);
                }
                self.apply_settings();
            }

            EditorEvent::SetSettingByName {
                component,
                field_name,
                value,
            } => {
                let Some(field) = self.settings_field_id(component, field_name) else {
                    return;
                };
                self.handle_editor_event(EditorEvent::SetSetting {
                    component,
                    field,
                    value,
                });
            }

            EditorEvent::ToggleSetting {
                component,
                field_name,
            } => {
                let Some(field) = self.settings_field_id(component, field_name) else {
                    return;
                };
                let Some(schema) = self.settings.registry().by_stable_id(component) else {
                    return;
                };
                let (world, entity) = self.settings.world();
                let current =
                    (schema.snapshot)(world, entity).and_then(|record| record.get(&field).cloned());
                let Some(somnium_ecs::reflect::ReflectValue::Bool(value)) = current else {
                    return;
                };
                self.handle_editor_event(EditorEvent::SetSetting {
                    component,
                    field,
                    value: somnium_ecs::reflect::ReflectValue::Bool(!value),
                });
            }

            EditorEvent::ResetAllSettings => {
                for schema in crate::settings::SettingsStore::schemas() {
                    for field in &schema.fields {
                        let _ = self.settings.revert(schema.stable_id, field.id);
                    }
                }
                self.apply_settings();
            }

            EditorEvent::OpenProjectPicker => {
                // 27-G's picker, unblocked: that phase deferred it because it
                // needed an `EditorEvent` addition it had forbidden itself.
                let Some(folder) = rfd::FileDialog::new()
                    .set_title("Open Project")
                    .pick_folder()
                else {
                    return;
                };
                let (component, field) =
                    crate::settings::field_address("somnium.ProjectSettings", "content_root");
                let value =
                    somnium_ecs::reflect::ReflectValue::Str(folder.to_string_lossy().into_owned());
                match self.settings.set(component, field, value) {
                    Ok(()) => {
                        self.config.content_root = folder;
                        self.next_asset_scan = std::time::Instant::now();
                        self.asset_scan_stamp = None;
                        if let Some(ui) = self.ui_manager.as_mut() {
                            ui.push_toast("Project opened");
                        }
                    }
                    Err(reason) => {
                        if let Some(ui) = self.ui_manager.as_mut() {
                            ui.push_toast(&reason);
                        }
                    }
                }
            }

            EditorEvent::SetDebugView(id) => {
                let Some(view) = somnium_ui::debug::debug_view(id) else {
                    return;
                };
                self.terrain_debug_view = view.code;
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.shading_debug = view.code;
                }
                if let Some(ui) = self.ui_manager.as_mut() {
                    ui.set_active_debug_view(id);
                    ui.push_toast(view.label);
                }
            }

            EditorEvent::ToggleRenderSwitch(id) => {
                let Some(renderer) = self.renderer.as_mut() else {
                    return;
                };
                let next = !renderer.debug_toggles.is_on(id);
                match renderer.debug_toggles.set(id, next) {
                    Ok(()) => {
                        renderer.apply_debug_toggles();
                        let states = renderer.debug_toggles.clone();
                        if let Some(ui) = self.ui_manager.as_mut() {
                            ui.set_render_toggles(states);
                        }
                    }
                    Err(reason) => {
                        if let Some(ui) = self.ui_manager.as_mut() {
                            ui.push_toast(&reason);
                        }
                    }
                }
            }

            EditorEvent::ViewPreset(index) => {
                // A preset frames the selection when there is one and the
                // origin otherwise, so "Top" always looks at something.
                let (centre, radius) = self.focus_target().unwrap_or((glam::Vec3::ZERO, 20.0));
                // The offset direction, from the subject toward the camera.
                let direction = match index {
                    // Straight down is a degenerate yaw, so Top is nudged a
                    // hair off the pole: an exactly vertical look has no
                    // defined heading and the camera would spin on recall.
                    0 => glam::Vec3::new(0.0, 1.0, 0.001).normalize(),
                    1 => glam::Vec3::Z,
                    2 => glam::Vec3::X,
                    _ => glam::Vec3::new(0.6, 0.5, 0.6).normalize(),
                };
                let position = centre + direction * (radius * 3.0);
                let look = (centre - position).normalize_or_zero();
                self.camera_pose_request = Some((
                    position,
                    look.z.atan2(look.x).to_degrees(),
                    look.y.clamp(-1.0, 1.0).asin().to_degrees(),
                ));
            }

            EditorEvent::SetCameraBookmark(slot) => {
                let Some(index) = slot.checked_sub(1).map(usize::from) else {
                    return;
                };
                let Some(renderer) = self.renderer.as_ref() else {
                    return;
                };
                let forward = renderer
                    .view_proj
                    .inverse()
                    .transform_vector3(glam::Vec3::NEG_Z);
                let forward = forward.normalize_or_zero();
                let yaw = forward.z.atan2(forward.x).to_degrees();
                let pitch = forward.y.clamp(-1.0, 1.0).asin().to_degrees();
                self.camera_bookmarks[index.min(8)] = Some((renderer.camera_pos, yaw, pitch));
                if let Some(ui) = self.ui_manager.as_mut() {
                    ui.push_toast(&format!("Bookmark {slot} set"));
                }
            }

            EditorEvent::RecallCameraBookmark(slot) => {
                let Some(index) = slot.checked_sub(1).map(usize::from) else {
                    return;
                };
                let Some((position, yaw, pitch)) = self.camera_bookmarks[index.min(8)] else {
                    if let Some(ui) = self.ui_manager.as_mut() {
                        ui.push_toast(&format!("Bookmark {slot} is empty"));
                    }
                    return;
                };
                self.camera_pose_request = Some((position, yaw, pitch));
            }

            EditorEvent::ToggleOrbitSelection => {
                self.orbit_selection = !self.orbit_selection;
                if let Some(ui) = self.ui_manager.as_mut() {
                    ui.push_toast(if self.orbit_selection {
                        "Orbit around selection"
                    } else {
                        "Orbit around camera"
                    });
                }
            }

            EditorEvent::OpenPiercingMenu => {
                // Unity 6's affordance, and craft defect C9: in a foliage
                // cluster the thing you want is behind three things you do
                // not, and clicking repeatedly is not a way to reach it.
                self.piercing_candidates = self.entities_under_cursor();
                let rows: Vec<_> = self
                    .piercing_candidates
                    .iter()
                    .map(|entity| {
                        (
                            entity.index(),
                            self.world.get::<Name>(*entity).map_or_else(
                                || format!("Entity {}", entity.index()),
                                |name| name.as_str().to_owned(),
                            ),
                        )
                    })
                    .collect();
                if let Some(ui) = self.ui_manager.as_mut() {
                    ui.open_piercing_menu(rows);
                }
            }

            EditorEvent::PickPierced(index) => {
                if let Some(entity) = self.world.find_entity_by_index(index) {
                    self.selection.set_single(Some(entity));
                    self.after_selection_change();
                }
            }

            EditorEvent::OpenSource { file, line, column } => {
                let source = somnium_ui::log::SourceRef {
                    file: file.clone(),
                    line,
                    column: Some(column),
                    span: (0, 0),
                };
                let template = self.settings.project().external_editor.clone();
                // A configured editor opens at the line — the detail §17.18.6
                // says is the part that matters. Without one the file is
                // revealed instead, because silently doing nothing is how a
                // clickable link becomes a thing people stop clicking.
                match somnium_ui::log::external_editor_command(&template, &source) {
                    Some(parts) => {
                        let (program, arguments) = parts.split_first().expect("non-empty");
                        match std::process::Command::new(program).args(arguments).spawn() {
                            Ok(_) => {}
                            Err(error) => {
                                warn!(%error, "external editor failed to start");
                                if let Some(ui) = self.ui_manager.as_mut() {
                                    ui.push_toast("External editor failed to start");
                                }
                                self.reveal_in_file_browser(&file);
                            }
                        }
                    }
                    None => self.reveal_in_file_browser(&file),
                }
            }

            EditorEvent::CopyText(text) => match copy_to_clipboard(&text) {
                Ok(()) => {
                    if let Some(ui) = self.ui_manager.as_mut() {
                        ui.push_toast("Copied");
                    }
                }
                Err(error) => {
                    warn!(%error, "clipboard write failed");
                    if let Some(ui) = self.ui_manager.as_mut() {
                        ui.push_toast("Could not reach the clipboard");
                    }
                }
            },

            EditorEvent::JumpToHistory(target) => {
                let steps =
                    self.undo_stack
                        .jump_to(target, &mut self.world, &mut self.selection.primary);
                if steps > 0 {
                    self.scene_dirty = true;
                    self.after_selection_change();
                }
            }

            EditorEvent::RequestHistory => {}

            EditorEvent::CancelMarquee => {
                self.marquee = None;
                if let Some(ui) = self.ui_manager.as_mut() {
                    ui.set_marquee(None);
                }
            }

            EditorEvent::SelectAll => {
                let all: Vec<_> = self.world.entities().collect();
                if all.is_empty() {
                    self.selection.clear();
                } else {
                    self.selection.set_many(all);
                }
                self.after_selection_change();
            }

            EditorEvent::FocusSelection => self.focus_camera_on_selection(),

            EditorEvent::RenameSelected => {}

            EditorEvent::RenameEntity { entity, name } => {
                let Some(entity) = self.world.find_entity_by_index(entity) else {
                    return;
                };
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    return;
                }
                let before = self
                    .world
                    .get::<Name>(entity)
                    .map(|name| name.as_str().to_owned())
                    .unwrap_or_default();
                if before == trimmed {
                    return;
                }
                self.undo_stack.push(
                    Box::new(crate::editor_commands::SetNameCmd::new(
                        entity.index(),
                        Name::new(&before),
                        Name::new(trimmed),
                    )),
                    &mut self.world,
                    &mut self.selection.primary,
                );
                self.scene_dirty = true;
            }

            EditorEvent::SelectEntity(opt_idx) => {
                self.selection
                    .set_single(opt_idx.and_then(|idx| self.world.find_entity_by_index(idx)));
                self.after_selection_change();
            }

            EditorEvent::CreateEntity(CreateKind::UiCanvas) => {
                let snapshot = EntitySnapshot {
                    transform: Some(Transform::from_translation(glam::Vec3::ZERO)),
                    name: Some(Name::new("UI Canvas")),
                    wt: Some(WorldTransform::identity()),
                    ui_canvas: Some(UiCanvasComponent::default()),
                    ..EntitySnapshot::default()
                };
                self.undo_stack.push(
                    Box::new(CreateEntityCmd::new(snapshot)),
                    &mut self.world,
                    &mut self.selection.primary,
                );
                self.scene_dirty = true;
                info!("Created runtime UI canvas entity");
            }

            EditorEvent::CreateEntity(CreateKind::VoxelTerrain) => {
                // The voxel world itself is owned by the game layer, which
                // spins its streaming driver up when it sees this component
                // (and tears it down when the entity is deleted).
                let snapshot = EntitySnapshot {
                    spline: None,
                    transform: Some(Transform::from_translation(glam::Vec3::ZERO)),
                    name: Some(Name::new("Voxel Terrain")),
                    light: None,
                    audio: None,
                    mesh: None,
                    mat: None,
                    wt: Some(WorldTransform::identity()),
                    environment: false,
                    decal: None,
                    mesh_kind: None,
                    is_particle_emitter: false,
                    terrain: None,
                    world_partition: None,
                    ui_canvas: None,
                    voxel_terrain: Some(crate::VoxelTerrainComponent::default()),
                    foliage: None,
                    water: None,
                    parent: None,
                    children: None,
                };
                let cmd = Box::new(CreateEntityCmd::new(snapshot));
                self.undo_stack
                    .push(cmd, &mut self.world, &mut self.selection.primary);
                info!("Created voxel terrain entity");
            }

            EditorEvent::CreateEntity(CreateKind::Environment) => {
                if self.world.entities().any(|e| {
                    self.world
                        .get::<crate::time_of_day::TimeOfDayComponent>(e)
                        .is_some()
                }) {
                    warn!("this scene already has an Environment");
                    return;
                }
                let snapshot = EntitySnapshot {
                    transform: Some(Transform::from_translation(glam::Vec3::ZERO)),
                    name: Some(Name::new("Environment")),
                    wt: Some(WorldTransform::identity()),
                    environment: true,
                    ..EntitySnapshot::default()
                };
                let cmd = Box::new(CreateEntityCmd::new(snapshot));
                self.undo_stack
                    .push(cmd, &mut self.world, &mut self.selection.primary);
                info!("Created environment entity");
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
                            .push(cmd, &mut self.world, &mut self.selection.primary);
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
                // A shoreline emitter is an ordinary emitter that happens to
                // carry a path. There is no second component and no second
                // code path: the audio runtime asks where the sound is, and a
                // spline answers "at your nearest point" while everything else
                // answers "at my origin".
                let audio = matches!(kind, CreateKind::AudioEmitter | CreateKind::ShorelineAudio)
                    .then(|| {
                        let mut emitter = crate::AudioEmitterComponent::default();
                        if kind == CreateKind::ShorelineAudio {
                            // A shoreline is heard from far further than a point
                            // source, and it loops: surf does not stop.
                            emitter.max_distance = 120.0;
                            emitter.min_distance = 6.0;
                            emitter.looping = true;
                        }
                        emitter
                    });
                let spline = matches!(kind, CreateKind::Spline | CreateKind::ShorelineAudio)
                    .then(|| crate::SplineComponent::straight(4, 12.0));

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
                                    porosity: 0.5,
                                    _pad: 0.0,
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
                            Some(MaterialComponent {
                                asset: somnium_asset::database::AssetId::NONE,
                                runtime_id: mat_id,
                            }),
                        )
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                };

                let spawn_dist = if light.is_some() || audio.is_some() {
                    5.0
                } else {
                    8.0
                };
                let spawn_dist = if spline.is_some() { 18.0 } else { spawn_dist };
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
                    spline,
                    transform: Some(transform),
                    name: Some(Name::new(name_str)),
                    light,
                    audio,
                    mesh,
                    mat,
                    wt: Some(world),
                    environment: false,
                    decal: None,
                    mesh_kind,
                    is_particle_emitter: kind == CreateKind::Particle,
                    terrain: None,
                    world_partition: None,
                    ui_canvas: None,
                    voxel_terrain: None,
                    foliage: None,
                    water: None,
                    parent: None,
                    children: None,
                };
                let cmd = Box::new(CreateEntityCmd::new(snapshot));
                self.undo_stack
                    .push(cmd, &mut self.world, &mut self.selection.primary);
                self.scene_dirty = true;
            }

            EditorEvent::DeleteSelected => {
                if let Some(entity) = self.selection.primary {
                    let cmd = Box::new(DeleteEntityCmd::new(entity.index()));
                    self.undo_stack
                        .push(cmd, &mut self.world, &mut self.selection.primary);
                    if let Some(r) = &mut self.renderer {
                        r.clear_gizmo();
                    }
                }
            }

            EditorEvent::Undo => {
                self.undo_stack
                    .undo(&mut self.world, &mut self.selection.primary);
            }

            EditorEvent::Redo => {
                self.undo_stack
                    .redo(&mut self.world, &mut self.selection.primary);
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

            EditorEvent::OpenScenePicker => {
                let Some(path) = rfd::FileDialog::new()
                    .set_title("Open Scene")
                    .add_filter("Somnium scene", &["somnium"])
                    .set_directory(&self.config.content_root)
                    .pick_file()
                else {
                    return;
                };
                self.handle_editor_event(EditorEvent::LoadScene(
                    path.to_string_lossy().into_owned(),
                ));
            }

            EditorEvent::AssignAssetToSelection(path) => {
                self.assign_asset_to_selection(std::path::Path::new(&path));
            }

            EditorEvent::SetComponentField {
                entity,
                component,
                field,
                value,
                gesture,
                live,
            } => {
                if !self.world.is_alive(entity) {
                    self.field_gestures.retain(|(g, _), _| *g != gesture);
                    return;
                }
                // CONTROL-F: an edit addressed at the primary is an edit on
                // the whole selection. Details never learns this; it addresses
                // the primary exactly as it always has, and the fan-out
                // happens here, where the selection lives.
                let targets: Vec<somnium_ecs::Entity> =
                    if self.selection.len() > 1 && self.selection.primary == Some(entity) {
                        self.selection
                            .as_slice()
                            .iter()
                            .copied()
                            .filter(|target| self.world.is_alive(*target))
                            .collect()
                    } else {
                        vec![entity]
                    };

                if live {
                    let Some(scope) = SetFieldCmd::scope(component, field) else {
                        warn!("reflected edit addressed an unknown field");
                        return;
                    };
                    for target in &targets {
                        let key = (gesture, *target);
                        if !self.field_gestures.contains_key(&key) {
                            let Some(snapshot) = FieldUndoSnapshot::capture(
                                &self.world,
                                *target,
                                component,
                                field,
                                scope,
                            ) else {
                                warn!("could not capture reflected edit baseline");
                                continue;
                            };
                            self.field_gestures.insert(key, snapshot);
                        }
                        if let Err(error) = SetFieldCmd::apply_live(
                            &mut self.world,
                            *target,
                            component,
                            field,
                            value.clone(),
                        ) {
                            warn!(%error, "rejected reflected live edit");
                            self.field_gestures.remove(&key);
                        }
                    }
                    return;
                }

                let mut baselines: std::collections::HashMap<_, _> = targets
                    .iter()
                    .filter_map(|target| {
                        self.field_gestures
                            .remove(&(gesture, *target))
                            .map(|snapshot| (*target, snapshot))
                    })
                    .collect();
                match crate::editor_commands::SetFieldMultiCmd::new(
                    &self.world,
                    &targets,
                    component,
                    field,
                    value,
                    gesture,
                    |target| baselines.remove(&target),
                ) {
                    Ok(command) => {
                        self.undo_stack.push(
                            Box::new(command),
                            &mut self.world,
                            &mut self.selection.primary,
                        );
                        self.scene_dirty = true;
                    }
                    Err(error) => warn!(%error, "rejected reflected property edit"),
                }
            }

            // CONTROL-L. The context bar and the preset commands both land
            // here. Routed through the one generic field write rather than
            // poking the component, so a scrub from the context bar and a drag
            // in Details produce the same undo entry with the same label.
            EditorEvent::SetTimeOfDayHour { hour, live } => {
                let Some(entity) = self.world.entities().find(|e| {
                    self.world
                        .get::<crate::time_of_day::TimeOfDayComponent>(*e)
                        .is_some()
                }) else {
                    warn!("no Time of Day component in the scene");
                    return;
                };
                let Some(field) = crate::reflect_registry::component_registry()
                    .by_name("somnium.TimeOfDay")
                    .and_then(|schema| schema.fields.iter().find(|f| f.name == "hour"))
                    .map(|f| f.id)
                else {
                    return;
                };
                // One id for the whole scrub, so the drag coalesces into a
                // single undo entry exactly as a Details drag does. `u64::MAX - 2`
                // joins the two sentinels already above; the day scrub is a
                // singleton gesture and cannot overlap another of its own kind.
                self.handle_editor_event(EditorEvent::SetComponentField {
                    entity,
                    component: somnium_ecs::reflect::StableId::new("somnium.TimeOfDay"),
                    field,
                    value: somnium_ecs::reflect::ReflectValue::F64(f64::from(
                        hour.rem_euclid(24.0),
                    )),
                    gesture: GestureId(u64::MAX - 2),
                    live,
                });
            }

            // CONTROL-M / CONTROL-N. Both presets take the same route: mutate
            // a copy, snapshot it through the schema, and push one named
            // `SetComponentCmd`. A weather preset carries its sky with it,
            // because CONTROL-N's exit criterion is *one* preset closing the
            // clouds and starting the rain.
            EditorEvent::SetSkyPreset(id) => {
                let label = somnium_ui::commands::SKY_PRESETS
                    .iter()
                    .find(|(preset, _)| *preset == id)
                    .map_or(id, |(_, label)| *label);
                match self.sky_preset_command(id) {
                    Some(command) => {
                        self.push_environment_preset(vec![command], format!("Sky preset: {label}"))
                    }
                    None => warn!(%id, "no Sky component, or unknown sky preset"),
                }
            }

            EditorEvent::SetWeatherPreset(id) => {
                let label = somnium_ui::commands::WEATHER_PRESETS
                    .iter()
                    .find(|(preset, _)| *preset == id)
                    .map_or(id, |(_, label)| *label);
                let mut commands: Vec<Box<dyn crate::editor_commands::EditorCommand>> = Vec::new();
                let Some(entity) = self.world.entities().find(|e| {
                    self.world
                        .get::<crate::weather::WeatherComponent>(*e)
                        .is_some()
                }) else {
                    warn!("no Weather component in the scene");
                    return;
                };
                let Some(current) = self
                    .world
                    .get::<crate::weather::WeatherComponent>(entity)
                    .copied()
                else {
                    return;
                };
                let mut next = current;
                if !next.apply_preset(id) {
                    warn!(%id, "unknown weather preset");
                    return;
                }
                let Some(values) = Self::stage_component(next) else {
                    return;
                };
                match crate::editor_commands::SetComponentCmd::new(
                    &self.world,
                    entity,
                    somnium_ecs::reflect::StableId::new("somnium.Weather"),
                    values,
                    format!("Weather preset: {label}"),
                ) {
                    Ok(command) => commands.push(Box::new(command)),
                    Err(error) => {
                        warn!(%error, "rejected weather preset");
                        return;
                    }
                }
                if let Some(command) =
                    crate::weather::sky_preset_for(id).and_then(|sky| self.sky_preset_command(sky))
                {
                    commands.push(command);
                }
                self.push_environment_preset(commands, format!("Weather preset: {label}"));
            }

            EditorEvent::SetTerrainToolValue {
                field,
                value,
                live: _,
            } => {
                if matches!(field, TT::AerialDistance) {
                    if let Some(renderer) = &mut self.renderer {
                        renderer.aerial_split = value.clamp(20.0, 4000.0);
                        renderer.classify_pass.aerial_split = renderer.aerial_split;
                    }
                    return;
                }
                let Some(entity) = self.selection.primary else {
                    return;
                };
                match field {
                    TT::PaintLayer => {
                        self.terrain_brush.paint_layer = (value.round().max(0.0) as usize).min(
                            somnium_renderer::terrain::textures::TERRAIN_LAYER_COUNT as usize - 1,
                        );
                    }
                    TT::DebugView => {
                        self.terrain_debug_view = value.round().clamp(0.0, 34.0);
                        if let Some(renderer) = self.renderer.as_mut() {
                            renderer.shading_debug = self.terrain_debug_view;
                        }
                    }
                    TT::TileScale
                    | TT::Relief
                    | TT::Wetness
                    | TT::MacroStrength
                    | TT::MorphStart => {
                        let Some(component) = self.world.get::<TerrainComponent>(entity).copied()
                        else {
                            return;
                        };
                        let Some(terrain) = self
                            .renderer
                            .as_mut()
                            .and_then(|renderer| renderer.terrain_mut(component.terrain_id))
                        else {
                            return;
                        };
                        match field {
                            TT::TileScale => {
                                if let Some(layer) =
                                    terrain.layers.get_mut(self.terrain_brush.paint_layer)
                                {
                                    layer.tiling = value.max(0.01);
                                }
                            }
                            TT::Relief => {
                                terrain.parallax_scale = value.clamp(0.0, 4.0);
                                if terrain.parallax_scale > 0.0 {
                                    terrain.parallax_held = terrain.parallax_scale;
                                }
                            }
                            TT::Wetness => terrain.wetness = value.clamp(0.0, 1.0),
                            TT::MacroStrength => terrain.macro_strength = value.clamp(0.0, 1.0),
                            TT::MorphStart => terrain.lod_morph_start = value.clamp(0.0, 1.0),
                            _ => unreachable!(),
                        }
                    }
                    TT::AerialDistance => unreachable!(),
                }
            }

            EditorEvent::SetFoliageBrushValue {
                field,
                value,
                live: _,
            } => {
                let brush = &mut self.foliage_brush;
                match field {
                    FB::Density => brush.density = value.clamp(0.0, 40.0),
                    FB::Radius => brush.radius = value.clamp(0.25, 200.0),
                    FB::MaxSlope => brush.max_slope_deg = value.clamp(0.0, 90.0),
                    FB::Kind => {
                        brush.kind =
                            (value.round().max(0.0) as usize).min(FOLIAGE_PALETTE.len() - 1) as u8
                    }
                    FB::ScaleMin => brush.scale_min = value.max(0.01),
                    FB::ScaleMax => brush.scale_max = value.max(0.01),
                }
            }

            EditorEvent::SaveScene => {
                self.flush_material_assets();
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
                        // A clean manual save is the moment there is nothing
                        // left to recover, so the autosaves go with it. Leaving
                        // them would offer the person older work next launch.
                        self.autosave.saved();
                        crate::autosave::clear(&self.config.content_root);
                        self.remember_recent_scene(std::path::Path::new(path));
                        if let Some(ui) = self.ui_manager.as_mut() {
                            ui.set_scene_name(Some(path.to_string()));
                        }
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
                self.selection.primary = None;
                self.material_sessions.clear();
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
                if let Some(entity) = self.selection.primary {
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
                    let audio = self
                        .world
                        .get::<crate::AudioEmitterComponent>(entity)
                        .cloned();
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
                        spline: None,
                        transform: Some(dup_transform),
                        name: Some(name),
                        light,
                        audio,
                        mesh,
                        mat,
                        wt: Some(WorldTransform::identity()),
                        environment: false,
                        decal: None,
                        mesh_kind,
                        is_particle_emitter,
                        // Terrains are not duplicated — two entities sharing
                        // one terrain_id would draw the same terrain twice.
                        terrain: None,
                        world_partition: None,
                        ui_canvas: None,
                        voxel_terrain: None,
                        foliage: None,
                        water,
                        parent,
                        children: None,
                    };
                    let cmd = Box::new(CreateEntityCmd::new(snapshot));
                    self.undo_stack
                        .push(cmd, &mut self.world, &mut self.selection.primary);
                    info!("Duplicated entity {}", entity.index());
                }
            }

            EditorEvent::LoadScene(path) => self.load_scene_file(&path),

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
                let w = self.viewport_size().0.max(1.0) as u32;
                let h = self.viewport_size().1.max(1.0) as u32;
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

            EditorEvent::CreateContentMaterial { parent, name } => {
                let Some(path) = self.content_target(&parent, &name, Some("sommat")) else {
                    return;
                };
                match somnium_asset::material::create_material(&path) {
                    Ok(_) => {
                        info!("Created {}", path.display());
                        self.next_asset_scan = std::time::Instant::now();
                        self.asset_scan_stamp = None;
                        self.after_content_change(&format!(
                            "Created {}",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ));
                    }
                    Err(error) => self.report_content_error(&path, &error),
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
                        if from
                            .extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case("luau"))
                        {
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

            EditorEvent::FloatPanel(kind) => self.pending_float.push(kind),

            EditorEvent::ClosePanelWindow(kind) => {
                self.floating.retain(|w| w.kind != kind);
            }

            EditorEvent::SaveLocalisation => self.save_localisation(),

            EditorEvent::ExportLocalisationCsv => self.export_localisation_csv(),

            EditorEvent::EditContentAsset(path) => {
                let path = std::path::PathBuf::from(path);
                if let Err(error) = edit_content_asset(&path) {
                    self.report_content_error(&path, &error);
                }
            }

            EditorEvent::MakeAssetUnique {
                source,
                entity,
                component,
                field,
            } => {
                let source = std::path::PathBuf::from(source);
                let target = somnium_asset::material::unique_sibling(&source);
                match std::fs::copy(&source, &target) {
                    Ok(_) => {
                        let relative = target
                            .strip_prefix(&self.config.content_root)
                            .unwrap_or(&target);
                        let asset = somnium_ecs::reflect::AssetRef::from_raw(
                            somnium_asset::database::AssetId::from_relative_path(relative).raw(),
                        );
                        self.handle_editor_event(EditorEvent::SetComponentField {
                            entity,
                            component,
                            field,
                            value: somnium_ecs::reflect::ReflectValue::Asset(Some(asset)),
                            gesture: GestureId(u64::MAX),
                            live: false,
                        });
                        self.next_asset_scan = std::time::Instant::now();
                        self.asset_scan_stamp = None;
                        if let Some(ui) = self.ui_manager.as_mut() {
                            ui.push_toast("Made unique asset copy");
                        }
                    }
                    Err(error) => self.report_content_error(&source, &error.to_string()),
                }
            }

            EditorEvent::AssignMaterial { entities, asset } => {
                let command = AssignMaterialCmd::new(&self.world, entities, asset);
                self.undo_stack.push(
                    Box::new(command),
                    &mut self.world,
                    &mut self.selection.primary,
                );
                self.scene_dirty = true;
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
                self.audio_scene.set_paused(false);
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
                self.audio_scene.set_paused(true);
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

            EditorEvent::StepSimulation => {
                // Refused rather than reinterpreted. Stepping a *running*
                // simulation would either do nothing visible or fight the
                // accumulator, and stepping from Edit would advance a clock
                // that is not running — neither is what the control means, and
                // guessing which one the user wanted is how a debugging tool
                // stops being trustworthy.
                if self.simulation_clock.state != SimulationState::Paused {
                    info!("Step ignored: the simulation is not paused");
                } else if !self.play_session_active {
                    info!("Step ignored: no play session is running");
                } else {
                    self.pending_steps = self.pending_steps.saturating_add(1);
                }
            }

            EditorEvent::StopSimulation => {
                self.pending_steps = 0;
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
            }

            EditorEvent::CancelJob(id) => {
                if !self.jobs.cancel(id) {
                    warn!(id, "cancel requested for an unknown job");
                }
            }

            EditorEvent::ToggleWaterUnderwater => {
                if let Some(entity) = self.selection.primary {
                    if let Some(water) = self.world.get_mut::<WaterComponent>(entity) {
                        water.underwater_enabled = !water.underwater_enabled;
                        self.scene_dirty = true;
                    }
                }
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
                if let Some(entity) = self.selection.primary {
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
                let Some(entity) = self.selection.primary else {
                    return;
                };
                if let Some(pp) = self.world.get_mut::<PostProcessComponent>(entity) {
                    pp.tonemapper = pp.tonemapper.next();
                    info!("Tonemapper: {}", pp.tonemapper.label());
                }
            }
            EditorEvent::SetTonemapper(idx) => {
                let Some(entity) = self.selection.primary else {
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
/// A cheap fingerprint of the content root: entry count and newest mtime,
/// one directory level deep plus the root itself.
///
/// Not a hash of every file — that would cost more than the scan it is
/// avoiding. It catches a file added, removed or written, which is every way
/// the drawer's contents change from outside the editor. Changes the stamp
/// cannot see are covered by the explicit invalidation points, which clear it.
fn content_root_stamp(root: &std::path::Path) -> (u64, u64) {
    fn fold(dir: &std::path::Path, depth: u32, count: &mut u64, newest: &mut u64) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            *count += 1;
            if let Ok(meta) = entry.metadata() {
                if let Ok(modified) = meta.modified() {
                    if let Ok(age) = modified.duration_since(std::time::UNIX_EPOCH) {
                        *newest = (*newest).max(age.as_secs());
                    }
                }
                if meta.is_dir() && depth > 0 {
                    fold(&entry.path(), depth - 1, count, newest);
                }
            }
        }
    }
    let (mut count, mut newest) = (0_u64, 0_u64);
    fold(root, 2, &mut count, &mut newest);
    (count, newest)
}

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
/// Where a project's committed settings live, given its content root.
///
/// The file sits beside the content directory rather than inside it, so it is
/// not itself an asset and never shows up in the Content Drawer.
fn config_project_path(config: &EngineConfig) -> std::path::PathBuf {
    config.content_root.parent().map_or_else(
        || std::path::PathBuf::from("project.toml"),
        |parent| parent.join("project.toml"),
    )
}

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
    local_space: bool,
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
    let inv_vp = renderer.picking_view_proj().inverse();
    let ray_dir = (ndc_to_world(cursor_pos.0, cursor_pos.1, vw, vh, &inv_vp) - camera_pos)
        .normalize_or_zero();

    let mode = renderer.gizmo_mode;
    let gizmo_rotation = if local_space {
        renderer.gizmo_world_rotation
    } else {
        glam::Quat::IDENTITY
    };
    let axis = gizmo_axis_under_ray(camera_pos, ray_dir, gizmo_pos, mode, gizmo_rotation)?;

    let start_transform = world
        .get::<Transform>(entity)
        .copied()
        .unwrap_or_else(|| Transform::from_translation(glam::Vec3::ZERO));
    let parent_world = parent_world_matrix(world, entity);
    // A collapsed parent has no local-space answer for a world-space gesture.
    // Refuse the drag instead of letting `Mat4::inverse` manufacture NaNs that
    // poison the authored transform and the undo record.
    let parent_world_inverse = invert_affine(parent_world)?;
    let start_world_rotation = world.get::<WorldTransform>(entity).map_or_else(
        || {
            (parent_world * start_transform.to_matrix())
                .to_scale_rotation_translation()
                .1
        },
        |world| world.0.to_scale_rotation_translation().1,
    );

    let axis_dir = gizmo_rotation * axis.world_dir();
    let (start_axis_param, start_angle, ring_tangent, ring_bitangent) = match mode {
        GizmoMode::Translate | GizmoMode::Scale => {
            let s = ray_axis_param(camera_pos, ray_dir, gizmo_pos, axis_dir).unwrap_or(0.0);
            (s, 0.0, glam::Vec3::ZERO, glam::Vec3::ZERO)
        }
        GizmoMode::Rotate => {
            let (tan, bitan) = ring_plane_basis(axis);
            let (tan, bitan) = (gizmo_rotation * tan, gizmo_rotation * bitan);
            let a = ring_angle(camera_pos, ray_dir, gizmo_pos, axis_dir, tan, bitan).unwrap_or(0.0);
            (0.0, a, tan, bitan)
        }
    };

    Some(GizmoDragState {
        // Filled by the caller, which is the only place that knows the
        // selection and the settings.
        followers: Vec::new(),
        local_space,
        axis,
        mode,
        entity_index: entity.index(),
        start_transform,
        start_axis_param,
        start_angle,
        ring_tangent,
        ring_bitangent,
        gizmo_pos,
        parent_world,
        parent_world_inverse,
        start_world_rotation,
    })
}

/// Compute the new entity transform given the current cursor ray.
/// Scale ratio that never divides by zero. A zero starting scale is degenerate
/// but authored scenes contain one occasionally, and `NaN` would spread from
/// it into every follower.
fn safe_ratio(new: f32, old: f32) -> f32 {
    if old.abs() > 1e-6 { new / old } else { 1.0 }
}

/// The snap increments a gizmo drag rounds to.
///
/// Zero means "no snapping on this axis of the problem", which is why they are
/// plain numbers rather than `Option`: a settings file storing `0.0` and a
/// person meaning "off" are the same thing, and one representation for both is
/// one fewer state to get wrong.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SnapSettings {
    /// Translation grid, in metres.
    pub translate_m: f32,
    /// Rotation increment, in degrees.
    pub rotate_deg: f32,
    /// Scale increment.
    pub scale: f32,
}

impl SnapSettings {
    /// Round `value` to the nearest multiple of `step`, or leave it alone when
    /// `step` is zero or not finite.
    #[must_use]
    pub fn quantise(value: f32, step: f32) -> f32 {
        if step > 0.0 && step.is_finite() {
            (value / step).round() * step
        } else {
            value
        }
    }

    /// `command()` held during a drag inverts snapping: on becomes off and off
    /// becomes on. Blender, Unity and Unreal all do this, and it is what makes
    /// a grid usable — you want it most of the time and need to escape it
    /// occasionally, without visiting a menu either way.
    #[must_use]
    pub fn inverted(self, held: bool) -> Self {
        if !held {
            return self;
        }
        // Inverting "off" needs a value to invert *to*. These are the same
        // defaults the settings ship with, so holding the modifier in a scene
        // with snapping off gives the grid everyone expects.
        Self {
            translate_m: if self.translate_m > 0.0 { 0.0 } else { 0.25 },
            rotate_deg: if self.rotate_deg > 0.0 { 0.0 } else { 15.0 },
            scale: if self.scale > 0.0 { 0.0 } else { 0.1 },
        }
    }
}

/// The first field on `entity` that accepts an asset of `kind`.
///
/// Registration order decides, and the probe for "does this entity have that
/// component at all" is `read_field` returning `Some` — the schema's own
/// accessor, rather than a second list of component types that would drift
/// away from it.
fn asset_field_for(
    registry: &somnium_ecs::reflect::TypeRegistry,
    world: &somnium_ecs::World,
    entity: somnium_ecs::Entity,
    kind: somnium_asset::database::AssetKind,
) -> Option<(
    somnium_ecs::reflect::StableId,
    somnium_ecs::reflect::FieldId,
    &'static str,
    &'static str,
)> {
    use somnium_ecs::reflect::FieldType;
    registry.iter().find_map(|schema| {
        let field = schema.fields.iter().find(|field| {
            field.ty == FieldType::Asset
                && kind.bit() & field.asset_kind_mask != 0
                && !field.read_only
        })?;
        (schema.read_field)(world, entity, field.id)?;
        Some((schema.stable_id, field.id, schema.display_name, field.name))
    })
}

/// The gizmo axis a world-space ray hits, if any.
///
/// The gizmo is drawn at a constant size on screen, so its handles live in a
/// space scaled by the camera distance; picking has to enter that same space
/// or the arrows are nowhere near where they look. Both this and the draw in
/// `SomniumRenderer::render` build the model matrix the same way from the same
/// two numbers, which is the only reason they agree.
fn gizmo_axis_under_ray(
    camera_pos: glam::Vec3,
    ray_dir: glam::Vec3,
    gizmo_pos: glam::Vec3,
    mode: GizmoMode,
    rotation: glam::Quat,
) -> Option<GizmoAxis> {
    if ray_dir == glam::Vec3::ZERO {
        return None;
    }
    let dist = (camera_pos - gizmo_pos).length().max(0.5);
    let scale = dist * 0.15;
    let model = glam::Mat4::from_translation(gizmo_pos)
        * glam::Mat4::from_quat(rotation)
        * glam::Mat4::from_scale(glam::Vec3::splat(scale));
    let inv_model = model.inverse();
    gizmo_hit_test(
        inv_model.transform_point3(camera_pos),
        inv_model.transform_vector3(ray_dir).normalize(),
        mode,
    )
}

/// Where the transform gizmo belongs for `primary`, or `None` for no gizmo.
///
/// Split out from [`SomniumApp::refresh_gizmo_anchor`] because the method
/// needs a renderer and therefore a GPU, and this is the half that decides
/// anything. Everything the tests at the bottom of this file assert about
/// gizmo placement is asserted here.
fn gizmo_anchor(
    world: &somnium_ecs::World,
    primary: Option<somnium_ecs::entity::Entity>,
) -> Option<glam::Vec3> {
    let entity = primary?;
    // Locked and hidden are the two flags whose whole job is "do not let the
    // viewport move this". The Outliner keeps the row selected, so the
    // Details panel still reads and edits it.
    let flags = world
        .get::<EditorFlags>(entity)
        .copied()
        .unwrap_or_default();
    if flags.locked || flags.hidden {
        return None;
    }
    // A root's local translation *is* its world position and is always
    // current. A child's is an offset from its parent, and only
    // `WorldTransform` says where it is on screen — anchoring a child by its
    // local translation puts the gizmo near the world origin, nowhere near
    // the thing it is supposed to be a handle for.
    if world.get::<crate::Parent>(entity).is_none() {
        return world
            .get::<Transform>(entity)
            .map(|transform| transform.translation);
    }
    world
        .get::<WorldTransform>(entity)
        .map(|world| world.0.to_scale_rotation_translation().2)
        .or_else(|| {
            world
                .get::<Transform>(entity)
                .map(|transform| transform.translation)
        })
}

fn apply_gizmo_drag(
    drag: &GizmoDragState,
    camera_pos: glam::Vec3,
    inv_vp: glam::Mat4,
    cursor_pos: (f32, f32),
    viewport_size: (f32, f32),
    snap: SnapSettings,
) -> Transform {
    let mut result = drag.start_transform;
    let (vw, vh) = viewport_size;
    if vw < 1.0 || vh < 1.0 {
        return result;
    }

    let world_pt = ndc_to_world(cursor_pos.0, cursor_pos.1, vw, vh, &inv_vp);
    let ray_dir = (world_pt - camera_pos).normalize();
    // Local space turns the handle with the object. World space is the
    // default because a scene's grid is a world-space idea; local is what you
    // want the moment anything is rotated.
    let axis_dir = if drag.local_space {
        (drag.start_world_rotation * drag.axis.world_dir()).normalize_or_zero()
    } else {
        drag.axis.world_dir()
    };
    if axis_dir == glam::Vec3::ZERO {
        return result;
    }

    match drag.mode {
        GizmoMode::Translate => {
            if let Some(s) = ray_axis_param(camera_pos, ray_dir, drag.gizmo_pos, axis_dir) {
                let moved = drag.gizmo_pos + (s - drag.start_axis_param) * axis_dir;
                // Snap the *result*, not the delta: an object already off the
                // grid should land on it, which is what "snap to grid" means
                // to everyone who has used one.
                let snapped_world = glam::Vec3::new(
                    SnapSettings::quantise(moved.x, snap.translate_m),
                    SnapSettings::quantise(moved.y, snap.translate_m),
                    SnapSettings::quantise(moved.z, snap.translate_m),
                );
                result.translation =
                    world_to_local_translation(drag.parent_world_inverse, snapped_world);
            }
        }
        GizmoMode::Scale => {
            if let Some(s) = ray_axis_param(camera_pos, ray_dir, drag.gizmo_pos, axis_dir) {
                if drag.start_axis_param.abs() > 0.01 {
                    let factor = (s / drag.start_axis_param).abs().max(0.01);
                    let mut sc = drag.start_transform.scale;
                    match drag.axis {
                        GizmoAxis::X => sc.x = SnapSettings::quantise(sc.x * factor, snap.scale),
                        GizmoAxis::Y => sc.y = SnapSettings::quantise(sc.y * factor, snap.scale),
                        GizmoAxis::Z => sc.z = SnapSettings::quantise(sc.z * factor, snap.scale),
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
                let delta = SnapSettings::quantise(
                    (angle - drag.start_angle).to_degrees(),
                    snap.rotate_deg,
                )
                .to_radians();
                result.rotation =
                    glam::Quat::from_axis_angle(axis_dir, delta) * drag.start_transform.rotation;
            }
        }
    }
    result
}

/// Component tags for one entity, used by the Outliner's `type:` filter.
///
/// Derived from the components actually present rather than from a name
/// heuristic, which is the difference between `type:light` finding the lights
/// and `type:light` finding everything called "Lamp".
fn entity_tags(world: &World, entity: somnium_ecs::Entity) -> Vec<&'static str> {
    let mut tags = Vec::new();
    if world.get::<LightComponent>(entity).is_some() {
        tags.push("light");
    }
    if world.get::<MeshComponent>(entity).is_some() {
        tags.push("mesh");
    }
    if world.get::<TerrainComponent>(entity).is_some() {
        tags.push("terrain");
    }
    if world.get::<VoxelTerrainComponent>(entity).is_some() {
        tags.push("voxel");
    }
    if world.get::<WaterComponent>(entity).is_some() {
        tags.push("water");
    }
    if world.get::<FoliageComponent>(entity).is_some() {
        tags.push("foliage");
    }
    if world.get::<crate::ParticleEmitter>(entity).is_some() {
        tags.push("particles");
    }
    if world.get::<AudioEmitterComponent>(entity).is_some() {
        tags.push("audio");
    }
    if world.get::<crate::decal::DecalComponent>(entity).is_some() {
        tags.push("decal");
    }
    if world.get::<PostProcessComponent>(entity).is_some() {
        tags.push("postfx");
    }
    if world
        .get::<somnium_script::attachment::ScriptSet>(entity)
        .is_some_and(|set| !set.is_empty())
    {
        tags.push("script");
    }
    tags
}

/// The platform command that reads stdin onto the clipboard.
///
/// A shell-out rather than a dependency. Every desktop this editor runs on
/// ships one of these three, and adding a crate — with its own X11/Wayland
/// backends and its own thread — to move a few hundred bytes of log text
/// would be a poor trade. Returned as data so the choice is testable without
/// actually spawning anything.
fn clipboard_command() -> (&'static str, &'static [&'static str]) {
    if cfg!(target_os = "windows") {
        ("cmd", &["/C", "clip"])
    } else if cfg!(target_os = "macos") {
        ("pbcopy", &[])
    } else {
        // `-selection clipboard`, because the default is the middle-click
        // primary selection, which is not what "Copy" means to anyone.
        ("xclip", &["-selection", "clipboard"])
    }
}

/// Put `text` on the system clipboard.
fn copy_to_clipboard(text: &str) -> Result<(), String> {
    use std::io::Write as _;
    let (program, arguments) = clipboard_command();
    let mut child = std::process::Command::new(program)
        .args(arguments)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| "clipboard helper refused stdin".to_string())?
        .write_all(text.as_bytes())
        .map_err(|error| error.to_string())?;
    let status = child.wait().map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("clipboard helper exited with {status}"))
    }
}

/// Choose the component-neutral local bounds used by all viewport ray picks.
fn entity_pick_aabb(
    world: &somnium_ecs::World,
    renderer: &SomniumRenderer,
    entity: somnium_ecs::Entity,
) -> Option<(glam::Vec3, glam::Vec3)> {
    if let Some(mesh) = world.get::<MeshComponent>(entity)
        && let Some((min, max)) = renderer.geometry.mesh_aabb(mesh.vertex_offset)
    {
        return Some((glam::Vec3::from_array(min), glam::Vec3::from_array(max)));
    }

    // Non-mesh authoring volumes still need a viewport handle. Decals use the
    // transform's full scale as their projection box, so a unit local box is
    // exact. Lights and emitters use a deliberately small proxy: selecting a
    // lamp must not claim its whole 40 m illumination range and hide every
    // object behind it.
    authored_proxy_aabb(world, entity)
}

fn authored_proxy_aabb(
    world: &somnium_ecs::World,
    entity: somnium_ecs::Entity,
) -> Option<(glam::Vec3, glam::Vec3)> {
    if world.get::<crate::decal::DecalComponent>(entity).is_some() {
        Some((glam::Vec3::splat(-0.5), glam::Vec3::splat(0.5)))
    } else if world.get::<LightComponent>(entity).is_some()
        || world.get::<AudioEmitterComponent>(entity).is_some()
        || world.get::<crate::ParticleEmitter>(entity).is_some()
    {
        Some((glam::Vec3::splat(-0.35), glam::Vec3::splat(0.35)))
    } else {
        None
    }
}

fn entity_ray_hit_distance(
    world: &somnium_ecs::World,
    renderer: &SomniumRenderer,
    entity: somnium_ecs::Entity,
    origin: glam::Vec3,
    direction: glam::Vec3,
) -> Option<f32> {
    let (min, max) = entity_pick_aabb(world, renderer, entity)?;
    let model = world
        .get::<WorldTransform>(entity)
        .map(|world| world.0)
        .or_else(|| world.get::<Transform>(entity).map(Transform::to_matrix))?;
    let inverse = invert_affine(model)?;
    let local_origin = inverse.transform_point3(origin);
    let local_direction = inverse.transform_vector3(direction);
    let t = ray_aabb_distance(local_origin, local_direction, min, max)?;
    let world_hit = model.transform_point3(local_origin + local_direction * t);
    Some(origin.distance_squared(world_hit))
}

/// Slab-test a ray against a local-space AABB, returning the nearest positive
/// hit distance along `direction`. `direction` need not be normalised, so the
/// result is expressed in the same parameterisation the caller passed in.
fn ray_aabb_distance(
    origin: glam::Vec3,
    direction: glam::Vec3,
    min: glam::Vec3,
    max: glam::Vec3,
) -> Option<f32> {
    let mut near = f32::NEG_INFINITY;
    let mut far = f32::INFINITY;
    for axis in 0..3 {
        let d = direction[axis];
        let o = origin[axis];
        if d.abs() < 1e-8 {
            if o < min[axis] || o > max[axis] {
                return None;
            }
            continue;
        }
        let t0 = (min[axis] - o) / d;
        let t1 = (max[axis] - o) / d;
        near = near.max(t0.min(t1));
        far = far.min(t0.max(t1));
    }
    (far >= near.max(0.0)).then(|| near.max(0.0))
}

#[cfg(test)]
mod viewport_control_tests {
    /// Every submission path asks the same question about `hidden`.
    ///
    /// A source check rather than a render check, and deliberately so: the
    /// failure it guards is *omission*. Hiding a mesh worked while hiding a
    /// terrain, foliage, a decal or a water body did nothing, because the check
    /// was copy-pasted into the paths that had it and absent from the rest. A
    /// rule spread across six call sites holds in five, and the sixth is the
    /// one the user finds.
    #[test]
    fn every_submitter_consults_the_hidden_flag() {
        let source = include_str!("app.rs");
        for name in [
            "submit_terrains",
            "submit_foliage",
            "submit_decals",
            "submit_light_gizmos",
            "submit_audio_gizmos",
        ] {
            let start = source
                .find(&format!("fn {name}("))
                .unwrap_or_else(|| panic!("{name} has moved or been renamed"));
            // The filter sits near the top of each. A generous window keeps
            // this from breaking on an unrelated edit further down.
            let window = &source[start..(start + 1600).min(source.len())];
            assert!(
                window.contains("is_hidden"),
                "{name} does not consult `is_hidden`, so the Outliner's eye does not reach what it submits"
            );
        }
    }
    use super::{
        EditorFlags, SnapSettings, Transform, WorldTransform, authored_proxy_aabb,
        ray_aabb_distance, world_to_local_translation,
    };

    /// Snapping rounds the *result*, so an object already off the grid lands
    /// on it. Rounding the delta instead would preserve the original error
    /// forever, which is not what "snap to grid" means to anyone.
    #[test]
    fn snapping_lands_on_the_grid_rather_than_preserving_an_offset() {
        assert_eq!(SnapSettings::quantise(1.13, 0.25), 1.25);
        assert_eq!(SnapSettings::quantise(-1.13, 0.25), -1.25);
        assert_eq!(SnapSettings::quantise(1.13, 1.0), 1.0);
    }

    /// A zero step means "off", and so does a step that is not a number. Both
    /// leave the value exactly alone rather than collapsing it to zero.
    #[test]
    fn a_zero_or_invalid_step_disables_snapping() {
        assert_eq!(SnapSettings::quantise(1.13, 0.0), 1.13);
        assert_eq!(SnapSettings::quantise(1.13, -1.0), 1.13);
        assert_eq!(SnapSettings::quantise(1.13, f32::NAN), 1.13);
    }

    /// `command()` inverts snapping in both directions — that is what makes a
    /// grid usable, and testing only one direction would miss the half that
    /// gives a grid to a scene that has none configured.
    #[test]
    fn the_modifier_inverts_snapping_both_ways() {
        let on = SnapSettings {
            translate_m: 0.5,
            rotate_deg: 45.0,
            scale: 0.1,
        };
        let off = on.inverted(true);
        assert_eq!(off.translate_m, 0.0);
        assert_eq!(off.rotate_deg, 0.0);
        assert_eq!(off.scale, 0.0);

        let none = SnapSettings::default();
        let forced = none.inverted(true);
        assert!(forced.translate_m > 0.0);
        assert!(forced.rotate_deg > 0.0);
        assert!(forced.scale > 0.0);

        assert_eq!(on.inverted(false), on, "not holding it changes nothing");
    }

    /// The piercing menu and a plain click share one ray test, so this is
    /// the shared half: the nearest box wins, a box behind the camera never
    /// does, and a miss is a miss.
    #[test]
    fn the_ray_test_orders_by_distance_and_ignores_what_is_behind() {
        let origin = glam::Vec3::ZERO;
        let forward = glam::Vec3::Z;
        let near = ray_aabb_distance(
            origin,
            forward,
            glam::Vec3::new(-1.0, -1.0, 4.0),
            glam::Vec3::new(1.0, 1.0, 6.0),
        );
        let far = ray_aabb_distance(
            origin,
            forward,
            glam::Vec3::new(-1.0, -1.0, 20.0),
            glam::Vec3::new(1.0, 1.0, 22.0),
        );
        assert!(near.unwrap() < far.unwrap());

        assert_eq!(
            ray_aabb_distance(
                origin,
                forward,
                glam::Vec3::new(-1.0, -1.0, -6.0),
                glam::Vec3::new(1.0, 1.0, -4.0),
            ),
            None,
            "a box behind the camera is not under the cursor"
        );
        assert_eq!(
            ray_aabb_distance(
                origin,
                forward,
                glam::Vec3::new(5.0, 5.0, 4.0),
                glam::Vec3::new(6.0, 6.0, 6.0),
            ),
            None,
            "a box the ray misses is not under the cursor"
        );
    }

    #[test]
    fn non_mesh_authoring_entities_have_pickable_proxy_bounds() {
        let mut world = somnium_ecs::World::new();
        let light = world.spawn((Transform::default(), crate::LightComponent::default()));
        let audio = world.spawn((
            Transform::default(),
            crate::AudioEmitterComponent::default(),
        ));
        let particles = world.spawn((Transform::default(), crate::ParticleEmitter::default()));
        let decal = world.spawn((
            Transform::default(),
            crate::decal::DecalComponent::default(),
        ));
        let plain = world.spawn((Transform::default(),));

        for entity in [light, audio, particles, decal] {
            assert!(
                authored_proxy_aabb(&world, entity).is_some(),
                "an authored viewport volume had no pick proxy"
            );
        }
        assert!(authored_proxy_aabb(&world, plain).is_none());
    }

    // ── Assigning an asset without a drag ──────────────────────────

    /// A clip lands on the Audio Emitter's clip field and nowhere else.
    ///
    /// This is the menu route to the thing a drag was supposed to do. It
    /// exists because the drag has repeatedly done nothing at all in the
    /// running editor, and an author needs one path to assigning an asset
    /// that is a single click and cannot be missed by a few pixels.
    #[test]
    fn a_clip_finds_the_audio_emitters_field() {
        use somnium_asset::database::AssetKind;
        let registry = crate::reflect_registry::component_registry();
        let mut world = somnium_ecs::World::new();
        let emitter = world.spawn((
            Transform::default(),
            crate::AudioEmitterComponent::default(),
        ));

        let found = super::asset_field_for(&registry, &world, emitter, AssetKind::Audio);
        let (component, _, _, field_name) = found.expect("an emitter must accept a clip");
        assert_eq!(
            component,
            somnium_ecs::reflect::StableId::new("somnium.AudioEmitter")
        );
        assert_eq!(field_name, "audio");
    }

    /// And the same entity refuses a texture rather than dropping it into
    /// whichever asset field happens to come first in the registry.
    #[test]
    fn an_emitter_does_not_take_a_texture() {
        use somnium_asset::database::AssetKind;
        let registry = crate::reflect_registry::component_registry();
        let mut world = somnium_ecs::World::new();
        let emitter = world.spawn((
            Transform::default(),
            crate::AudioEmitterComponent::default(),
        ));
        assert!(super::asset_field_for(&registry, &world, emitter, AssetKind::Texture).is_none());
    }

    /// An entity that has no asset field at all is not a target, however
    /// many other components it carries.
    #[test]
    fn an_entity_with_no_asset_field_accepts_nothing() {
        use somnium_asset::database::AssetKind;
        let registry = crate::reflect_registry::component_registry();
        let mut world = somnium_ecs::World::new();
        let plain = world.spawn((Transform::default(), WorldTransform::identity()));
        for kind in [AssetKind::Audio, AssetKind::Texture, AssetKind::Mesh] {
            assert!(super::asset_field_for(&registry, &world, plain, kind).is_none());
        }
    }

    // ── Gizmo picking ──────────────────────────────────────────────

    /// A camera at `+Z` looking at the origin, and the matrix a click is
    /// unprojected through.
    fn look_at_origin(surface: (f32, f32)) -> (glam::Vec3, glam::Mat4) {
        let camera = glam::Vec3::new(0.0, 0.0, 12.0);
        let view = glam::Mat4::look_at_rh(camera, glam::Vec3::ZERO, glam::Vec3::Y);
        let proj =
            glam::Mat4::perspective_rh(60.0_f32.to_radians(), surface.0 / surface.1, 0.1, 1000.0);
        (camera, proj * view)
    }

    fn axis_at(
        cursor: (f32, f32),
        viewport: (f32, f32),
        surface: (f32, f32),
    ) -> Option<super::GizmoAxis> {
        let (camera, view_proj) = look_at_origin(surface);
        let inv = view_proj.inverse();
        let world = super::ndc_to_world(cursor.0, cursor.1, viewport.0, viewport.1, &inv);
        super::gizmo_axis_under_ray(
            camera,
            (world - camera).normalize_or_zero(),
            glam::Vec3::ZERO,
            super::GizmoMode::Translate,
            glam::Quat::IDENTITY,
        )
    }

    #[test]
    fn local_gizmo_hit_testing_uses_the_same_rotation_as_drawing() {
        let surface = (1920.0, 1080.0);
        let (camera, view_proj) = look_at_origin(surface);
        let cursor = (surface.0 * 0.5, surface.1 * 0.5 - 68.0);
        let world = super::ndc_to_world(
            cursor.0,
            cursor.1,
            surface.0,
            surface.1,
            &view_proj.inverse(),
        );
        let rotation = glam::Quat::from_rotation_z(90.0_f32.to_radians());
        assert_eq!(
            super::gizmo_axis_under_ray(
                camera,
                (world - camera).normalize_or_zero(),
                glam::Vec3::ZERO,
                super::GizmoMode::Translate,
                rotation,
            ),
            Some(super::GizmoAxis::X),
            "local X is drawn along world +Y after the quarter turn"
        );
    }

    /// **The bug that made every gizmo inert.** `viewport_size` was a cache
    /// filled from `Resized`, and `window_event` drops every event that
    /// arrives before the lifecycle reaches `Running` — which on Windows
    /// includes the window's first one. The cache therefore kept the
    /// *requested logical* size for the whole session, while the cursor and
    /// the surface were both in physical pixels. On a 1.5× display that is a
    /// 50% error on both axes, and a click on an arrow unprojects to a ray
    /// pointing somewhere else entirely.
    ///
    /// Nothing was visibly broken, which is the point: the gizmo drew in the
    /// right place, the click landed on it, and the drag simply never started.
    #[test]
    fn a_click_on_an_arrow_picks_it_only_when_the_viewport_size_is_the_real_one() {
        // A 1280×720 window on a 1.5× display: the surface, and the cursor,
        // are 1920×1080.
        let surface = (1920.0, 1080.0);
        let stale = (1280.0, 720.0);

        // Straight down the +X arrow from a camera on +Z: a little right of
        // centre, vertically centred.
        let on_the_x_arrow = (surface.0 * 0.5 + 68.0, surface.1 * 0.5);

        assert_eq!(
            axis_at(on_the_x_arrow, surface, surface),
            Some(super::GizmoAxis::X),
            "with the real surface size the click picks the arrow it is on"
        );
        assert_ne!(
            axis_at(on_the_x_arrow, stale, surface),
            Some(super::GizmoAxis::X),
            "and with the stale logical size it does not — this is the whole bug"
        );
    }

    /// The centre of the gizmo is inside every handle's box, so it must
    /// resolve to *an* axis rather than nothing: a click there is a drag, not
    /// a miss that falls through to a rubber band.
    #[test]
    fn the_centre_of_the_gizmo_is_always_a_hit() {
        let surface = (1600.0, 900.0);
        assert!(axis_at((surface.0 * 0.5, surface.1 * 0.5), surface, surface).is_some());
    }

    /// Empty sky is not a handle. Without this the previous test passes for a
    /// hit test that says yes to everything.
    #[test]
    fn a_click_in_the_corner_hits_nothing() {
        let surface = (1600.0, 900.0);
        assert_eq!(axis_at((12.0, 12.0), surface, surface), None);
    }

    // ── Gizmo placement ────────────────────────────────────────────

    /// The bug: `Create` sets the selection through the undo stack without
    /// raising a selection event, and the gizmo anchor used to be pushed
    /// only from selection events. A freshly created Audio Emitter was
    /// therefore fully editable in the Details panel and had no working
    /// handle in the viewport — the gizmo was still on whatever came
    /// before it. Reading the anchor out of the world is what makes that
    /// unrepresentable, so this asserts the read rather than a push.
    #[test]
    fn the_anchor_is_wherever_the_entity_currently_is() {
        let mut world = somnium_ecs::World::new();
        let entity = world.spawn((
            Transform::from_translation(glam::Vec3::new(1.0, 2.0, 3.0)),
            WorldTransform::identity(),
        ));
        assert_eq!(
            super::gizmo_anchor(&world, Some(entity)),
            Some(glam::Vec3::new(1.0, 2.0, 3.0))
        );

        // A typed Details translation, an Undo and a script write are all
        // this: the transform moved and nothing raised a selection event.
        world.get_mut::<Transform>(entity).unwrap().translation = glam::Vec3::new(-4.0, 0.5, 9.0);
        assert_eq!(
            super::gizmo_anchor(&world, Some(entity)),
            Some(glam::Vec3::new(-4.0, 0.5, 9.0)),
            "the gizmo has to follow the entity, not the last selection event"
        );
    }

    /// A child's local translation is an offset, so anchoring by it would
    /// park the gizmo near the parent's origin rather than on the child.
    #[test]
    fn a_child_is_anchored_where_it_is_drawn() {
        let mut world = somnium_ecs::World::new();
        let parent = world.spawn((
            Transform::from_translation(glam::Vec3::new(10.0, 0.0, 0.0)),
            WorldTransform(glam::Mat4::from_translation(glam::Vec3::new(
                10.0, 0.0, 0.0,
            ))),
        ));
        let child = world.spawn((
            Transform::from_translation(glam::Vec3::new(0.0, 2.0, 0.0)),
            WorldTransform(glam::Mat4::from_translation(glam::Vec3::new(
                10.0, 2.0, 0.0,
            ))),
            crate::Parent { entity: parent },
        ));
        assert_eq!(
            super::gizmo_anchor(&world, Some(child)),
            Some(glam::Vec3::new(10.0, 2.0, 0.0))
        );
    }

    /// A world-space gizmo result must cross the inverse parent transform
    /// before it is stored in the child's local `Transform`. Translation-only
    /// parents hid this bug; rotation and non-uniform scale expose it.
    #[test]
    fn a_child_gizmo_world_delta_is_written_back_in_parent_local_space() {
        let parent = glam::Mat4::from_scale_rotation_translation(
            glam::Vec3::new(2.0, 3.0, 4.0),
            glam::Quat::from_rotation_y(90.0_f32.to_radians()),
            glam::Vec3::new(10.0, -2.0, 5.0),
        );
        let start_local = glam::Vec3::new(1.0, 2.0, 3.0);
        let start_world = parent.transform_point3(start_local);
        let delta = glam::Vec3::new(4.0, -3.0, 2.0);
        let new_local = world_to_local_translation(parent.inverse(), start_world + delta);

        assert!(
            parent
                .transform_point3(new_local)
                .abs_diff_eq(start_world + delta, 1e-4),
            "the stored local point did not reproduce the gizmo's world result"
        );
        assert_ne!(
            new_local,
            start_local + delta,
            "adding the world delta directly would only be valid for an identity parent"
        );
    }

    /// Locked and hidden exist to stop the viewport moving something. The
    /// gizmo is the viewport moving something.
    #[test]
    fn a_locked_or_hidden_entity_offers_no_handle() {
        for flags in [
            EditorFlags {
                locked: true,
                ..EditorFlags::default()
            },
            EditorFlags {
                hidden: true,
                ..EditorFlags::default()
            },
        ] {
            let mut world = somnium_ecs::World::new();
            let entity = world.spawn((
                Transform::from_translation(glam::Vec3::ONE),
                WorldTransform::identity(),
                flags,
            ));
            assert_eq!(super::gizmo_anchor(&world, Some(entity)), None);
        }
        assert_eq!(super::gizmo_anchor(&somnium_ecs::World::new(), None), None);
    }

    /// A ray that starts inside a box hits it at zero, not at a negative
    /// distance — otherwise a camera inside geometry would sort it last.
    #[test]
    fn a_ray_starting_inside_a_box_hits_it_at_zero() {
        assert_eq!(
            ray_aabb_distance(
                glam::Vec3::ZERO,
                glam::Vec3::Z,
                glam::Vec3::splat(-1.0),
                glam::Vec3::splat(1.0),
            ),
            Some(0.0)
        );
    }
}
