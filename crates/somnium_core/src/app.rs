use std::sync::Arc;

use tracing::{debug, error, info, warn};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use somnium_renderer::{GizmoAxis, GizmoMode, SomniumRenderer, RenderContext, gizmo_hit_test};
use somnium_physics::{world::PhysicsWorld, config::PhysicsConfig};
use somnium_physics::body::{BodyId, MotionType, RigidBodyDescriptor};
use somnium_physics::shape::ColliderShape;
use somnium_audio::engine::AudioEngine;
use somnium_ui::{EditorEvent, UiManager};

use crate::config::EngineConfig;
use crate::context::EngineContext;
use crate::editor_commands::{
    CreateEntityCmd, DeleteEntityCmd, EntitySnapshot, SetLightCmd, SetTransformCmd,
    TerrainEditCmd, TerrainRestoreOp, TerrainRestoreQueue, UndoStack,
};
use crate::error::EngineError;
use crate::event::{translate_window_event, EngineEvent};
use crate::time::TimeState;
use crate::{FoliageComponent, LightComponent, LightType, MeshComponent, MaterialComponent, MeshKind, Name, PostProcessComponent, TerrainComponent, Transform, WorldTransform, simulate_particles};
use somnium_ecs::World;
use somnium_renderer::terrain::brush::{apply_paint, apply_sculpt, BrushMode, TerrainBrush};

/// State captured when the user begins dragging a gizmo axis handle.
#[derive(Clone)]
struct GizmoDragState {
    axis:             GizmoAxis,
    mode:             GizmoMode,
    entity_index:     u32,
    start_transform:  Transform,
    /// Scalar along the drag axis from gizmo origin at drag start (translate/scale).
    start_axis_param: f32,
    /// Angle in the ring plane at drag start, in radians (rotate).
    start_angle:      f32,
    /// Ring-plane tangent vector (rotate).
    ring_tangent:     glam::Vec3,
    /// Ring-plane bitangent vector (rotate).
    ring_bitangent:   glam::Vec3,
    /// Gizmo world position at drag start.
    gizmo_pos:        glam::Vec3,
}

/// State captured while a terrain brush stroke is in progress (Phase 14D).
///
/// On stroke start, the full heightmap (or splatmap) is snapshotted; the
/// affected region accumulates as the stroke moves. On release, the old/new
/// data of just that region is pushed as a [`TerrainEditCmd`].
struct TerrainStroke {
    terrain_id:    u32,
    is_paint:      bool,
    start_heights: Vec<f32>,
    start_texels:  Vec<[u8; 4]>,
    /// Union of all touched (vertex or texel) regions, inclusive.
    region:        Option<(u32, u32, u32, u32)>,
}

/// Trait to be implemented by the user's game.
pub trait GameApp {
    /// Called once when the engine starts.
    fn on_init(&mut self, _ctx: &mut EngineContext) {}

    /// Called for every window event.
    fn on_event(&mut self, _ctx: &mut EngineContext, _event: &EngineEvent) {}

    /// Called every frame for game logic.
    fn on_update(&mut self, _ctx: &mut EngineContext) {}

    /// Called every frame for UI and debug rendering.
    fn on_render(&mut self, _ctx: &mut EngineContext) {}

    /// Called just before the engine shuts down.
    fn on_shutdown(&mut self) {}
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
    ("Grass Medium",  "assets/foliage/grass_medium_01/grass_medium_01_2k.gltf"),
    ("Grass Bermuda", "assets/foliage/grass_bermuda_01/grass_bermuda_01_2k.gltf"),
    ("Fir Sapling",   "assets/foliage/fir_sapling/fir_sapling_2k.gltf"),
    ("Island Tree",   "assets/foliage/island_tree_02/island_tree_02_2k.gltf"),
];

/// A palette entry's geometry once it has been uploaded.
#[derive(Clone, Copy)]
struct FoliageMesh {
    vertex_offset: u32,
    index_offset: u32,
    index_count: u32,
    material_id: u32,
}

/// The central engine controller that manages the lifecycle and orchestration of all subsystems.
pub struct Engine<G: GameApp> {
    game: Box<G>,
    config: EngineConfig,
    time: TimeState,
    world: World,
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
    foliage_meshes: [Option<FoliageMesh>; FOLIAGE_PALETTE.len()],
    /// Phase 17F: the foliage brush.
    foliage_brush: somnium_renderer::terrain::foliage_paint::FoliageBrush,
    /// When true, dragging in the viewport paints foliage instead of sculpting.
    pub foliage_paint_active: bool,
    /// Erase instead of add.
    pub foliage_erase: bool,
    /// Scratch list for this frame's visible foliage, reused so a field of
    /// instances does not allocate a fresh vector every frame.
    foliage_batch: Vec<(FoliageMesh, glam::Mat4)>,
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
    /// Tracks whether Ctrl is currently held (updated via ModifiersChanged).
    ctrl_held: bool,
    /// Cached default material ID for editor-created mesh entities.
    default_material_id: Option<u32>,
    /// Phase 14F: terrain edit mode (F6 or terrain tool button activates).
    terrain_edit_active: bool,
    /// Phase 14D: current terrain brush settings.
    terrain_brush: TerrainBrush,
    /// Active brush stroke (Some while LMB is held in terrain edit mode).
    terrain_stroke: Option<TerrainStroke>,
    /// Restore ops produced by `TerrainEditCmd` undo/redo, applied before render.
    terrain_restore_queue: TerrainRestoreQueue,
    /// Phase 20B: editor camera speed as a normalized 0..1 slider position.
    /// Game code reads the mapped speed via `EngineContext::camera_speed`.
    camera_speed_norm: f32,
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
            foliage_meshes: [None; FOLIAGE_PALETTE.len()],
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
            ctrl_held: false,
            default_material_id: None,
            terrain_edit_active: false,
            terrain_brush: TerrainBrush::default(),
            terrain_stroke: None,
            terrain_restore_queue: TerrainRestoreQueue::default(),
            camera_speed_norm: crate::DEFAULT_CAMERA_SPEED_NORM,
        };

        event_loop
            .run_app(&mut engine)
            .map_err(|e| EngineError::EventLoop(e.to_string()))?;

        info!("Somnium Engine shut down cleanly");
        Ok(())
    }
}

impl<G: GameApp> ApplicationHandler for Engine<G> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
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
            );
            self.game.on_event(&mut ctx, &EngineEvent::Resumed);
            return;
        }

        info!("Creating window");

        let size = LogicalSize::new(self.config.window_size.0, self.config.window_size.1);
        let attrs = WindowAttributes::default()
            .with_title(&self.config.window_title)
            .with_inner_size(size)
            .with_resizable(self.config.resizable);

        match event_loop.create_window(attrs) {
            Ok(window) => {
                let window = Arc::new(window);
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

        // Track modifier key state for shortcut detection.
        if let WindowEvent::ModifiersChanged(m) = &event {
            self.ctrl_held = m.state().control_key();
        }

        // Handle Resizing
        if let WindowEvent::Resized(size) = &event {
            self.viewport_size = (size.width as f32, size.height as f32);
            if let Some(r_ctx) = &mut self.render_ctx {
                r_ctx.resize(size.width, size.height);
            }
            if let (Some(r), Some(c)) = (&mut self.renderer, &self.render_ctx) {
                r.resize(c, size.width, size.height);
            }
            if let (Some(ui), Some(window)) = (&mut self.ui_manager, &self.window) {
                ui.reposition_panels(window);
            }
        }

        // ── 1. Handle global Ctrl+ shortcuts FIRST (never for text widgets) ────
        if let WindowEvent::KeyboardInput { event: key_ev, .. } = &event {
            if key_ev.state == winit::event::ElementState::Pressed && !key_ev.repeat && self.ctrl_held {
                use winit::keyboard::{KeyCode as WKC, PhysicalKey};
                if let PhysicalKey::Code(code) = key_ev.physical_key {
                    match code {
                        WKC::KeyZ => {
                            self.handle_editor_event(EditorEvent::Undo);
                            return;
                        }
                        WKC::KeyY => {
                            self.handle_editor_event(EditorEvent::Redo);
                            return;
                        }
                        WKC::KeyS => {
                            self.handle_editor_event(EditorEvent::SaveScene);
                            return;
                        }
                        WKC::KeyN => {
                            self.handle_editor_event(EditorEvent::NewScene);
                            return;
                        }
                        WKC::KeyD => {
                            self.handle_editor_event(EditorEvent::DuplicateSelected);
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
                        WKC::Delete => {
                            self.handle_editor_event(EditorEvent::DeleteSelected);
                            return;
                        }
                        WKC::KeyT => {
                            if let Some(r) = &mut self.renderer {
                                r.gizmo_mode = somnium_renderer::pass::gizmo::GizmoMode::Translate;
                            }
                        }
                        WKC::KeyR => {
                            if let Some(r) = &mut self.renderer {
                                r.gizmo_mode = somnium_renderer::pass::gizmo::GizmoMode::Rotate;
                            }
                        }
                        WKC::KeyS => {
                            if let Some(r) = &mut self.renderer {
                                r.gizmo_mode = somnium_renderer::pass::gizmo::GizmoMode::Scale;
                            }
                        }
                        WKC::F5 => {
                            self.handle_editor_event(EditorEvent::ToggleShadingMode);
                        }
                        // ── Phase 14F: terrain edit mode + brush shortcuts ──
                        WKC::F6 => {
                            if self.selected_terrain().is_some() {
                                self.terrain_edit_active = !self.terrain_edit_active;
                                info!(
                                    "Terrain edit mode: {}",
                                    if self.terrain_edit_active { "ON" } else { "off" }
                                );
                            } else {
                                info!("Select a terrain entity before pressing F6");
                            }
                        }
                        WKC::F8 => {
                            // Phase 17A: toggle scattered foliage on the
                            // selected terrain. Inspector controls come with
                            // the layer UI; until then this is how it is
                            // switched on.
                            if let Some(entity) = self.selected_entity {
                                if let Some(f) = self.world.get_mut::<FoliageComponent>(entity) {
                                    f.enabled = !f.enabled;
                                    let on = f.enabled;
                                    info!("Foliage: {}", if on { "ON" } else { "off" });
                                } else {
                                    info!("Select a terrain entity before pressing F8");
                                }
                            } else {
                                info!("Select a terrain entity before pressing F8");
                            }
                        }
                        WKC::BracketLeft if self.terrain_edit_active => {
                            self.terrain_brush.radius = (self.terrain_brush.radius / 1.25).max(0.5);
                            info!("Brush radius: {:.1} m", self.terrain_brush.radius);
                        }
                        WKC::BracketRight if self.terrain_edit_active => {
                            self.terrain_brush.radius = (self.terrain_brush.radius * 1.25).min(128.0);
                            info!("Brush radius: {:.1} m", self.terrain_brush.radius);
                        }
                        WKC::Minus if self.terrain_edit_active => {
                            self.terrain_brush.strength = (self.terrain_brush.strength - 0.1).max(0.05);
                            info!("Brush strength: {:.2}", self.terrain_brush.strength);
                        }
                        WKC::Equal if self.terrain_edit_active => {
                            self.terrain_brush.strength = (self.terrain_brush.strength + 0.1).min(1.0);
                            info!("Brush strength: {:.2}", self.terrain_brush.strength);
                        }
                        WKC::Comma if self.terrain_edit_active => {
                            self.terrain_brush.paint_layer =
                                self.terrain_brush.paint_layer.checked_sub(1).unwrap_or(3);
                            info!("Paint layer: {}", self.terrain_brush.paint_layer);
                        }
                        WKC::Period if self.terrain_edit_active => {
                            self.terrain_brush.paint_layer = (self.terrain_brush.paint_layer + 1) % 4;
                            info!("Paint layer: {}", self.terrain_brush.paint_layer);
                        }
                        WKC::F7 => {
                            // Phase 14E-3: procedural splat by slope/height.
                            if let Some(tc) = self.selected_terrain() {
                                if let Some(t) = self
                                    .renderer
                                    .as_mut()
                                    .and_then(|r| r.terrain_mut(tc.terrain_id))
                                {
                                    somnium_renderer::terrain::brush::auto_splat(t, 10.0);
                                    info!("Auto-splatted terrain {} by slope/height", tc.terrain_id);
                                }
                            }
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
        if ui_consumed { return; }

        // ── 3.4 Foliage brush (Phase 17F) — takes priority over sculpting ────
        if self.foliage_paint_active {
            if let WindowEvent::MouseInput {
                state: winit::event::ElementState::Pressed,
                button: winit::event::MouseButton::Left, ..
            } = &event
            {
                self.foliage_painting = self.paint_foliage_dab();
                if self.foliage_painting {
                    return;
                }
            }
            if let WindowEvent::MouseInput {
                state: winit::event::ElementState::Released,
                button: winit::event::MouseButton::Left, ..
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
        if self.terrain_edit_active {
            if let WindowEvent::MouseInput {
                state: winit::event::ElementState::Pressed,
                button: winit::event::MouseButton::Left, ..
            } = &event {
                if self.begin_terrain_stroke() {
                    return;
                }
            }
            if let WindowEvent::MouseInput {
                state: winit::event::ElementState::Released,
                button: winit::event::MouseButton::Left, ..
            } = &event {
                if self.end_terrain_stroke() {
                    return;
                }
            }
        }

        // ── 4. Gizmo LMB pick / drag-end ────────────────────────────────────
        let mut gizmo_consumed = false;

        if let WindowEvent::MouseInput {
            state: winit::event::ElementState::Pressed,
            button: winit::event::MouseButton::Left, ..
        } = &event {
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
            button: winit::event::MouseButton::Left, ..
        } = &event {
            if let Some(drag) = self.gizmo_drag.take() {
                if let Some(entity) = self.world.find_entity_by_index(drag.entity_index) {
                    let final_t = self.world.get::<Transform>(entity).copied()
                        .unwrap_or(drag.start_transform);
                    let cmd = Box::new(SetTransformCmd::new(
                        drag.entity_index, drag.start_transform, final_t,
                    ));
                    self.undo_stack.push_silent(cmd);
                }
                gizmo_consumed = true;
            }
        }

        if gizmo_consumed {
            return;
        }

        // ── 5. Forward remaining events to game ──────────────────────────────
        if let Some(engine_event) = translate_window_event(&event) {
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
            winit::event::DeviceEvent::MouseMotion { delta } => {
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
                self.render_ctx.as_ref(),
                self.renderer.as_mut(),
                &mut self.selected_entity,
                self.ui_manager.as_mut().unwrap(),
                crate::camera_speed_from_normalized(self.camera_speed_norm),
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

        if let Some(physics) = self.physics.as_mut() {
            physics.step(dt);
        }

        // ── Gizmo drag: update entity transform each frame while dragging ────
        let drag_result: Option<(u32, Transform)> = self.gizmo_drag.as_ref().and_then(|drag| {
            let (cam, inv_vp) = self.renderer.as_ref()
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
            );
            self.game.on_update(&mut ctx);
        }

        // ── Update native UI panels with current frame state ─────────────────
        {
            let all_entities: Vec<somnium_ecs::Entity> = self.world.entities().collect();
            let entity_list: Vec<(u32, String)> = all_entities.iter()
                .map(|&e| {
                    let name = self.world.get::<Name>(e)
                        .map(|n| n.as_str().to_owned())
                        .unwrap_or_else(|| format!("Entity {}", e.index()));
                    (e.index(), name)
                })
                .collect();
            let selected_idx = self.selected_entity.map(|e| e.index());
            let sel_t = self.selected_entity
                .and_then(|e| self.world.get::<Transform>(e).copied());
            // Phase 15A1: post-processing settings for the inspector.
            let sel_post = self.selected_entity
                .and_then(|e| self.world.get::<PostProcessComponent>(e).copied())
                .map(|pp| (
                    [pp.exposure, pp.vignette_strength, pp.ca_strength, pp.ibl_intensity],
                    pp.vignette_enabled,
                    pp.ca_enabled,
                    pp.fxaa_enabled,
                ));
            // Phase 17C: terrain layer + foliage settings for the inspector.
            let sel_terrain = self.selected_entity.and_then(|e| {
                let tc = self.world.get::<TerrainComponent>(e)?;
                let r = self.renderer.as_ref()?;
                let t = r.terrain(tc.terrain_id)?;
                let tile = |i: usize| t.layers.get(i).map_or(1.0, |l| l.tiling);
                Some([
                    self.terrain_brush.paint_layer as f32,
                    tile(0), tile(1), tile(2), tile(3),
                ])
            });
            let brush = self.foliage_brush;
            let paint_on = self.foliage_paint_active;
            let erase_on = self.foliage_erase;
            let single_on = brush.single;
            let sel_foliage = self
                .selected_entity
                .and_then(|e| self.world.get::<FoliageComponent>(e).copied())
                .map(|f| (
                    [
                        brush.density,
                        brush.radius,
                        brush.max_slope_deg,
                        f32::from(brush.kind),
                        brush.scale_min,
                        brush.scale_max,
                    ],
                    [f.enabled, paint_on, erase_on, single_on],
                ));

            // Phase 13E: light properties for the inspector (angles in degrees).
            let sel_light = self.selected_entity
                .and_then(|e| self.world.get::<LightComponent>(e).copied())
                .map(|lc| ([
                    lc.intensity,
                    lc.range,
                    lc.inner_angle.to_degrees(),
                    lc.outer_angle.to_degrees(),
                    lc.color.x,
                    lc.color.y,
                    lc.color.z,
                ], lc.light_type == LightType::Directional));
            if let Some(ui) = &mut self.ui_manager {
                ui.update_outliner(&entity_list, selected_idx);
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
                ui.update_post_inspector(sel_post);
                ui.update_terrain_inspector(sel_terrain);
                ui.update_foliage_inspector(sel_foliage);
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
            let gpu_particles = simulate_particles(&mut self.world, dt, frame);
            if let Some(r) = &mut self.renderer {
                r.set_particles(gpu_particles);
            }
        }

        // ── Terrain editing + submission (Phase 14) ──────────────────────────
        self.apply_terrain_restores();
        self.update_terrain_editing(dt);
        self.submit_terrains();
        self.submit_foliage();
        self.sync_terrain_colliders();

        // ── Light gizmos (Phase 13E) ─────────────────────────────────────────
        self.submit_light_gizmos();

        // ── Post-processing settings (Phase 15A1) ────────────────────────────
        self.apply_post_process();

        if let (Some(r), Some(c), Some(ui), Some(window)) = (
            &mut self.renderer,
            &self.render_ctx,
            &mut self.ui_manager,
            &self.window,
        ) {
            r.time = self.time.elapsed().as_secs_f32();
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
            self.cursor_pos.0, self.cursor_pos.1,
            self.viewport_size.0, self.viewport_size.1,
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
        info!("Terrain tool: {}", self.terrain_brush.mode.label());
    }

    /// Begin a brush stroke under the cursor. Returns true if a stroke started.
    fn begin_terrain_stroke(&mut self) -> bool {
        let Some(tc) = self.selected_terrain() else { return false };
        let Some(model) = self.selected_terrain_model() else { return false };
        let Some((origin, dir)) = self.cursor_ray() else { return false };
        let Some(renderer) = self.renderer.as_mut() else { return false };
        let Some(terrain) = renderer.terrain_mut(tc.terrain_id) else { return false };

        terrain.model = model; // keep raycast in sync with the entity transform
        let Some(hit) = terrain.raycast(origin, dir) else { return false };

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
            start_heights: if is_paint { Vec::new() } else { terrain.heightmap.clone() },
            start_texels: if is_paint { terrain.splatmap.data.clone() } else { Vec::new() },
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
            let Some(tc) = self.selected_terrain() else { return };
            let Some(model) = self.selected_terrain_model() else { return };
            let ray = self.cursor_ray();
            let radius = self.foliage_brush.radius;
            let Some(renderer) = self.renderer.as_mut() else { return };
            renderer.clear_gizmo();
            let Some(terrain) = renderer.terrain_mut(tc.terrain_id) else { return };
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
        let Some(tc) = self.selected_terrain() else { return };
        let Some(model) = self.selected_terrain_model() else { return };
        let ray = self.cursor_ray();
        let brush = self.terrain_brush;
        let stroking = self.terrain_stroke.is_some();
        let Some(renderer) = self.renderer.as_mut() else { return };
        // Phase 14F-1: regular transform gizmos are hidden in terrain mode.
        renderer.clear_gizmo();
        let Some(terrain) = renderer.terrain_mut(tc.terrain_id) else { return };
        terrain.model = model;

        let hit = ray.and_then(|(o, d)| terrain.raycast(o, d));
        let Some(hit) = hit else {
            terrain.brush_cursor = [0.0; 4];
            return;
        };

        // Cursor ring: green for sculpt modes, blue for paint (Phase 14D-3).
        let mode_flag = if brush.mode == BrushMode::Paint { 2.0 } else { 1.0 };
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
                        acc.0.min(rg.0), acc.1.min(rg.1),
                        acc.2.max(rg.2), acc.3.max(rg.3),
                    ),
                });
            }
        }
    }

    /// Finish the active stroke and push an undo command. Returns true if a
    /// stroke was finished.
    fn end_terrain_stroke(&mut self) -> bool {
        let Some(stroke) = self.terrain_stroke.take() else { return false };
        let Some(region) = stroke.region else { return true };
        let Some(renderer) = self.renderer.as_ref() else { return true };
        let Some(terrain) = renderer.terrain(stroke.terrain_id) else { return true };

        let (x0, z0, x1, z1) = region;
        let cmd: Box<dyn crate::editor_commands::EditorCommand> = if stroke.is_paint {
            let row_w = terrain.splatmap.width;
            let extract = |data: &[[u8; 4]]| -> Vec<[u8; 4]> {
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
        true
    }

    /// Apply queued terrain restores produced by `TerrainEditCmd` undo/redo.
    fn apply_terrain_restores(&mut self) {
        let ops: Vec<TerrainRestoreOp> = match self.terrain_restore_queue.lock() {
            Ok(mut q) => q.drain(..).collect(),
            Err(_) => return,
        };
        let Some(renderer) = self.renderer.as_mut() else { return };
        for op in ops {
            match op {
                TerrainRestoreOp::Heights { terrain_id, region, heights } => {
                    let Some(terrain) = renderer.terrain_mut(terrain_id) else { continue };
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
                TerrainRestoreOp::Splat { terrain_id, region, texels } => {
                    let Some(terrain) = renderer.terrain_mut(terrain_id) else { continue };
                    let (x0, z0, x1, z1) = region;
                    let row_w = terrain.splatmap.width;
                    let w = (x1 - x0 + 1) as usize;
                    for (i, z) in (z0..=z1).enumerate() {
                        let dst = (z * row_w + x0) as usize;
                        terrain.splatmap.data[dst..dst + w]
                            .copy_from_slice(&texels[i * w..(i + 1) * w]);
                    }
                    terrain.splatmap.mark_dirty(x0, z0, x1, z1);
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

        let Some((renderer, render_ctx)) =
            self.renderer.as_mut().zip(self.render_ctx.as_ref())
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
            let (scale, rotation, translation) =
                node.transform.to_scale_rotation_translation();
            let name = if node.entity_name.is_empty() {
                Name::new("Imported Mesh")
            } else {
                Name::new(&node.entity_name)
            };
            let entity = self.world.spawn((
                Transform { translation, rotation, scale },
                name,
                WorldTransform::identity(),
                MeshComponent {
                    vertex_offset: node.vertex_offset,
                    index_offset: node.index_offset,
                    index_count: node.index_count,
                },
                MaterialComponent { id: node.material_id },
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
    /// such entity the renderer keeps its defaults, which are all-off — an
    /// editor viewport shows the raw image unless a look is asked for.
    fn apply_post_process(&mut self) {
        let settings = self
            .world
            .entities()
            .find_map(|e| self.world.get::<PostProcessComponent>(e).copied());
        if let (Some(pp), Some(r)) = (settings, self.renderer.as_mut()) {
            r.exposure = pp.exposure.max(0.0);
            r.vignette_strength = pp.effective_vignette();
            r.chromatic_aberration = pp.effective_ca();
            r.fxaa_enabled = pp.fxaa_enabled;
            // Phase 22C: rides along with the sun in the directional-light
            // buffer, so every pass that lights anything picks it up.
            r.set_ibl_intensity(pp.ibl_intensity);
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
                    p.remove_body(body);
                }
            }
        }

        for (terrain_id, position) in terrains {
            let revision = self
                .renderer
                .as_ref()
                .and_then(|r| r.terrain(terrain_id))
                .map_or(0, |t| t.edit_revision);
            if self.terrain_colliders.get(&terrain_id).is_some_and(|(rev, _)| *rev == revision) {
                continue;
            }

            let Some(renderer) = self.renderer.as_ref() else { continue };
            let Some(terrain) = renderer.terrain(terrain_id) else { continue };
            let (samples, sample_count, scale) = terrain.heightfield();

            // Drop the old body first: two overlapping static surfaces would
            // fight over every contact.
            if let Some((_, old)) = self.terrain_colliders.remove(&terrain_id) {
                if let Some(p) = self.physics.as_mut() {
                    p.remove_body(old);
                }
            }

            let Some(physics) = self.physics.as_mut() else { continue };
            let body = physics.create_body(RigidBodyDescriptor {
                shape: ColliderShape::HeightField { samples, sample_count, scale },
                position,
                motion_type: MotionType::Static,
                object_layer: somnium_physics::layer::LAYER_NON_MOVING,
                ..Default::default()
            });
            self.terrain_colliders.insert(terrain_id, (revision, body));
            info!(
                "Terrain {terrain_id}: collider rebuilt ({sample_count}x{sample_count} samples)",
            );
        }
    }

    /// Submit every painted foliage instance (Phase 17F).
    ///
    /// Instances are ordinary draw commands, so they inherit the Phase 15
    /// pipeline — indirect draws, frustum, Hi-Z and per-cluster culling —
    /// without foliage needing to know any of it exists.
    fn submit_foliage(&mut self) {
        let camera_ws = self.renderer.as_ref().map_or(glam::Vec3::ZERO, |r| r.camera_pos);
        let terrains: Vec<(u32, glam::Mat4, f32)> = self
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
                Some((tc.terrain_id, model, fc.cull_distance))
            })
            .collect();

        for (terrain_id, model, cull_distance) in terrains {
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
            let cull_sq = if cull_distance > 0.0 { cull_distance * cull_distance } else { f32::MAX };
            self.foliage_batch.clear();
            self.foliage_batch.extend(t
                .painted_foliage
                .iter()
                .filter_map(|inst| {
                    let d = inst.position - camera_local;
                    // Horizontal distance: flying up should not make ground
                    // cover vanish out from under you.
                    if d.x * d.x + d.z * d.z > cull_sq {
                        return None;
                    }
                    let mesh = (*self.foliage_meshes.get(inst.kind as usize)?)?;
                    // Terrain-local placement composed with the terrain's own
                    // transform, so moving the terrain carries its foliage.
                    let local = glam::Mat4::from_scale_rotation_translation(
                        glam::Vec3::splat(inst.scale),
                        glam::Quat::from_rotation_y(inst.yaw),
                        inst.position,
                    );
                    Some((mesh, model * local))
                }));

            if let Some(r) = self.renderer.as_mut() {
                for (mesh, transform) in self.foliage_batch.drain(..) {
                    r.submit(somnium_renderer::command::DrawCommand {
                        sort_key: somnium_renderer::command::SortKey::new(0, 0, 0),
                        vertex_offset: mesh.vertex_offset,
                        index_offset: mesh.index_offset,
                        index_count: mesh.index_count,
                        material_id: mesh.material_id,
                        transform,
                    });
                }
            }
        }
    }

    /// Load and upload one palette entry, the first time it is painted.
    fn ensure_palette_mesh(&mut self, kind: u8) {
        let idx = kind as usize;
        if idx >= FOLIAGE_PALETTE.len() || self.foliage_meshes[idx].is_some() {
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
            }
        }

        let uploaded = renderer.upload_scene(ctx, &scene);
        // These models are a clump of separate parts; the largest is the one
        // worth instancing. Scattering every part separately would multiply the
        // instance count for no visual gain.
        let Some(node) = uploaded.iter().max_by_key(|n| n.index_count) else {
            return;
        };
        info!("Foliage: loaded {name} ({} triangles)", node.index_count / 3);
        self.foliage_meshes[idx] = Some(FoliageMesh {
            vertex_offset: node.vertex_offset,
            index_offset: node.index_offset,
            index_count: node.index_count,
            material_id: node.material_id,
        });
    }

    /// Apply one dab of the foliage brush under the cursor (Phase 17F).
    ///
    /// Returns true when a terrain was hit, so the caller knows the click was
    /// consumed by painting rather than falling through to selection.
    fn paint_foliage_dab(&mut self) -> bool {
        let Some(tc) = self.selected_terrain() else { return false };
        let Some(model) = self.selected_terrain_model() else { return false };
        let Some((origin, dir)) = self.cursor_ray() else { return false };

        let brush = self.foliage_brush;
        let erase = self.foliage_erase;
        let seed = self.foliage_stroke_seed;
        self.foliage_stroke_seed = seed.wrapping_add(1);

        let Some(renderer) = self.renderer.as_mut() else { return false };
        let Some(terrain) = renderer.terrain_mut(tc.terrain_id) else { return false };
        terrain.model = model; // keep the raycast in sync with the entity
        let Some(hit) = terrain.raycast(origin, dir) else { return false };
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
                    LightType::Point => LightGizmoKind::Point,
                    LightType::Spot => LightGizmoKind::Spot,
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

    fn handle_editor_event(&mut self, ev: EditorEvent) {
        use somnium_ui::{CreateKind, InspectorField as IF};

        match ev {
            EditorEvent::SelectEntity(opt_idx) => {
                self.selected_entity = opt_idx.and_then(|idx| self.world.find_entity_by_index(idx));
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
                };
                let cmd = Box::new(CreateEntityCmd::new(snapshot));
                self.undo_stack.push(cmd, &mut self.world, &mut self.selected_entity);
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
                let desc = somnium_renderer::terrain::TerrainDescriptor::default();
                let terrain_id = renderer.create_terrain(render_ctx, desc);
                let [wx, wz] = desc.world_size();

                let snapshot = EntitySnapshot {
                    // Center the terrain footprint on the world origin.
                    transform: Some(Transform::from_translation(glam::Vec3::new(
                        -wx * 0.5, 0.0, -wz * 0.5,
                    ))),
                    name: Some(Name::new("Terrain")),
                    light: None,
                    mesh: None,
                    mat: None,
                    wt: Some(WorldTransform::identity()),
                    mesh_kind: None,
                    is_particle_emitter: false,
                    voxel_terrain: None,
                    terrain: Some(TerrainComponent {
                        terrain_id,
                        chunk_cells: desc.chunk_cells,
                        grid_x: desc.grid_size[0],
                        grid_z: desc.grid_size[1],
                        cell_size: desc.cell_size,
                        height_scale: desc.height_scale,
                    }),
                    // Phase 17A: present but disabled, so a new terrain is bare
                    // until foliage is deliberately switched on.
                    foliage: Some(crate::FoliageComponent::default()),
                };
                let cmd = Box::new(CreateEntityCmd::new(snapshot));
                self.undo_stack.push(cmd, &mut self.world, &mut self.selected_entity);
                info!(
                    "Created terrain {} ({}x{} chunks, {:.0}x{:.0} m) — press F6 to edit",
                    terrain_id, desc.grid_size[0], desc.grid_size[1], wx, wz,
                );
            }

            EditorEvent::CreateEntity(kind) => {
                let name_str = kind.label();
                let light = match kind {
                    CreateKind::DirectionalLight => Some(LightComponent::directional(3.0)),
                    CreateKind::PointLight => Some(LightComponent::point(3.0, 10.0)),
                    CreateKind::SpotLight => Some(LightComponent::spot(
                        3.0, 15.0, 25.0_f32.to_radians(), 35.0_f32.to_radians(),
                    )),
                    _ => None,
                };

                // Determine mesh_kind for procedural mesh entities.
                let mesh_kind = match kind {
                    CreateKind::Cube     => Some(MeshKind::Cube),
                    CreateKind::Sphere   => Some(MeshKind::Sphere),
                    CreateKind::Plane    => Some(MeshKind::Plane),
                    CreateKind::Cylinder => Some(MeshKind::Cylinder),
                    _ => None,
                };

                // Generate and upload mesh geometry if this is a mesh primitive.
                let (mesh, mat) = if let Some(mk) = mesh_kind {
                    if let (Some(renderer), Some(render_ctx)) = (&mut self.renderer, &self.render_ctx) {
                        // Generate procedural geometry.
                        let (verts, idxs) = match mk {
                            MeshKind::Cube     => somnium_asset::generate_cube(1.0),
                            MeshKind::Sphere   => somnium_asset::generate_sphere(0.5, 32, 16),
                            MeshKind::Plane    => somnium_asset::generate_plane(5.0, 1),
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
                _padding: 0,
                                },
                            );
                            self.default_material_id = Some(id);
                            id
                        };

                        // Upload geometry to GPU.
                        let alloc = renderer.geometry.upload_mesh(
                            &render_ctx.queue, &verts, &idxs, mat_id,
                        );

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

                let snapshot = EntitySnapshot {
                    transform: Some(Transform::from_translation(glam::Vec3::ZERO)),
                    name: Some(Name::new(name_str)),
                    light,
                    mesh,
                    mat,
                    wt: Some(WorldTransform::identity()),
                    mesh_kind,
                    is_particle_emitter: kind == CreateKind::Particle,
                    terrain: None,
                    voxel_terrain: None,
                    foliage: None,
                };
                let cmd = Box::new(CreateEntityCmd::new(snapshot));
                self.undo_stack.push(cmd, &mut self.world, &mut self.selected_entity);
            }

            EditorEvent::DeleteSelected => {
                if let Some(entity) = self.selected_entity {
                    let cmd = Box::new(DeleteEntityCmd::new(entity.index()));
                    self.undo_stack.push(cmd, &mut self.world, &mut self.selected_entity);
                    if let Some(r) = &mut self.renderer {
                        r.clear_gizmo();
                    }
                }
            }

            EditorEvent::Undo => {
                self.undo_stack.undo(&mut self.world, &mut self.selected_entity);
            }

            EditorEvent::Redo => {
                self.undo_stack.redo(&mut self.world, &mut self.selected_entity);
            }

            EditorEvent::ToggleShadingMode => {
                if let Some(r) = &mut self.renderer {
                    r.shading_mode = if r.shading_mode == 0 { 1 } else { 0 };
                    info!("Shading mode toggled to: {}", if r.shading_mode == 1 { "Cel-Shading" } else { "PBR" });
                }
            }

            EditorEvent::SetInspectorValue { field, value, live } => {
                let Some(entity) = self.selected_entity else { return };

                // Phase 15A1: post-processing fields edit PostProcessComponent.
                if matches!(
                    field,
                    IF::PostExposure
                        | IF::PostVignetteStrength
                        | IF::PostCaStrength
                        | IF::PostIblIntensity
                ) {
                    if let Some(pp) = self.world.get_mut::<PostProcessComponent>(entity) {
                        match field {
                            IF::PostExposure => pp.exposure = value.max(0.0),
                            IF::PostVignetteStrength => pp.vignette_strength = value.max(0.0),
                            IF::PostCaStrength => pp.ca_strength = value.max(0.0),
                            IF::PostIblIntensity => pp.ibl_intensity = value.max(0.0),
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
                        | IF::TerrainTile1
                        | IF::TerrainTile2
                        | IF::TerrainTile3
                ) {
                    if field == IF::TerrainPaintLayer {
                        self.terrain_brush.paint_layer =
                            (value.round().max(0.0) as usize).min(3);
                        return;
                    }
                    let slot = match field {
                        IF::TerrainTile0 => 0,
                        IF::TerrainTile1 => 1,
                        IF::TerrainTile2 => 2,
                        _ => 3,
                    };
                    let Some(tc) = self.world.get::<TerrainComponent>(entity).copied() else {
                        return;
                    };
                    if let Some(t) = self
                        .renderer
                        .as_mut()
                        .and_then(|r| r.terrain_mut(tc.terrain_id))
                    {
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
                                .min(FOLIAGE_PALETTE.len() - 1) as u8;
                        }
                        IF::FoliageScaleMin => b.scale_min = value.max(0.01),
                        IF::FoliageScaleMax => b.scale_max = value.max(0.01),
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
                                self.undo_stack.push(cmd, &mut self.world, &mut self.selected_entity);
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
                        IF::RotX => new_t.rotation = glam::Quat::from_euler(glam::EulerRot::XYZ, value.to_radians(), ey, ez),
                        IF::RotY => new_t.rotation = glam::Quat::from_euler(glam::EulerRot::XYZ, ex, value.to_radians(), ez),
                        IF::RotZ => new_t.rotation = glam::Quat::from_euler(glam::EulerRot::XYZ, ex, ey, value.to_radians()),
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
                        self.undo_stack.push(cmd, &mut self.world, &mut self.selected_entity);
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
                match crate::scene_serial::save_scene(&self.world, path) {
                    Ok(()) => info!("Scene saved to {}", path),
                    Err(e) => warn!("Failed to save scene: {}", e),
                }
                // Phase 14F-3: heightmap + splatmap sidecars, one per terrain.
                if let Some(r) = &self.renderer {
                    let terrain_ids: Vec<u32> = self
                        .world
                        .entities()
                        .filter_map(|e| self.world.get::<TerrainComponent>(e).map(|tc| tc.terrain_id))
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
                    Transform { translation: glam::Vec3::ZERO, rotation: light_rot, scale: glam::Vec3::ONE },
                    LightComponent::directional(5.0),
                    Name::new("SunLight"),
                    WorldTransform::identity(),
                ));
                self.undo_stack = UndoStack::new(128);
            }

            EditorEvent::DuplicateSelected => {
                if let Some(entity) = self.selected_entity {
                    let transform = self.world.get::<Transform>(entity).copied()
                        .unwrap_or_else(|| Transform::from_translation(glam::Vec3::ZERO));
                    let name = self.world.get::<Name>(entity)
                        .map(|n| Name::new(&format!("{}_copy", n.as_str())))
                        .unwrap_or_else(|| Name::new("Entity_copy"));
                    let light = self.world.get::<LightComponent>(entity).copied();
                    let mesh = self.world.get::<MeshComponent>(entity).copied();
                    let mat = self.world.get::<MaterialComponent>(entity).copied();
                    let mesh_kind = self.world.get::<MeshKind>(entity).copied();
                    let is_particle_emitter = self.world.get::<crate::ParticleEmitter>(entity).is_some();
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
                    };
                    let cmd = Box::new(CreateEntityCmd::new(snapshot));
                    self.undo_stack.push(cmd, &mut self.world, &mut self.selected_entity);
                    info!("Duplicated entity {}", entity.index());
                }
            }

            EditorEvent::LoadScene(_path) => {
                // TODO: Load scene from file (requires GPU mesh reconstruction)
                info!("LoadScene not yet fully implemented");
            }

            EditorEvent::SetTerrainTool(tool) => {
                self.set_terrain_tool(tool);
            }

            EditorEvent::SetCameraSpeed(normalized) => {
                self.camera_speed_norm = normalized.clamp(0.0, 1.0);
                let speed = crate::camera_speed_from_normalized(self.camera_speed_norm);
                if let Some(ui) = &mut self.ui_manager {
                    ui.update_camera_speed(self.camera_speed_norm, speed);
                }
            }

            EditorEvent::ImportModel => {
                self.import_model();
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
                    if self.foliage_paint_active { "ON" } else { "off" }
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

            EditorEvent::TogglePostFx(which) => {
                use somnium_ui::PostFxToggle;
                let Some(entity) = self.selected_entity else { return };
                if let Some(pp) = self.world.get_mut::<PostProcessComponent>(entity) {
                    let on = match which {
                        PostFxToggle::Vignette => {
                            pp.vignette_enabled = !pp.vignette_enabled;
                            pp.vignette_enabled
                        }
                        PostFxToggle::ChromaticAberration => {
                            pp.ca_enabled = !pp.ca_enabled;
                            pp.ca_enabled
                        }
                        PostFxToggle::Fxaa => {
                            pp.fxaa_enabled = !pp.fxaa_enabled;
                            pp.fxaa_enabled
                        }
                    };
                    info!("Post FX {:?}: {}", which, if on { "on" } else { "off" });
                }
            }
        }
    }
}

// ─── Gizmo picking / drag math ────────────────────────────────────────────────

/// Unproject a screen position to a world-space point (at mid-depth).
fn ndc_to_world(cx: f32, cy: f32, vw: f32, vh: f32, inv_vp: &glam::Mat4) -> glam::Vec3 {
    let ndc_x = 2.0 * cx / vw - 1.0;
    let ndc_y = 1.0 - 2.0 * cy / vh;
    let clip  = glam::Vec4::new(ndc_x, ndc_y, 0.5, 1.0);
    let world = *inv_vp * clip;
    glam::Vec3::new(world.x, world.y, world.z) / world.w
}

/// Parameter along an axis line at which it is closest to a world-space ray.
///
/// Returns `None` if ray and axis are nearly parallel.
fn ray_axis_param(
    ray_origin: glam::Vec3,
    ray_dir:    glam::Vec3,
    axis_origin: glam::Vec3,
    axis_dir:    glam::Vec3,
) -> Option<f32> {
    let b     = ray_dir.dot(axis_dir);
    let denom = 1.0 - b * b;
    if denom.abs() < 1e-8 { return None; }
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
    ray_origin:    glam::Vec3,
    ray_dir:       glam::Vec3,
    ring_center:   glam::Vec3,
    ring_normal:   glam::Vec3,
    ring_tangent:  glam::Vec3,
    ring_bitangent: glam::Vec3,
) -> Option<f32> {
    let denom = ray_dir.dot(ring_normal);
    if denom.abs() < 1e-8 { return None; }
    let t = (ring_center - ray_origin).dot(ring_normal) / denom;
    if t < 0.0 { return None; }
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
    renderer:        Option<&SomniumRenderer>,
    world:           &somnium_ecs::World,
    selected_entity: &Option<somnium_ecs::entity::Entity>,
    cursor_pos:      (f32, f32),
    viewport_size:   (f32, f32),
) -> Option<GizmoDragState> {
    let renderer  = renderer?;
    let entity    = (*selected_entity)?;
    let gizmo_pos = renderer.gizmo_world_pos?;

    let (vw, vh) = viewport_size;
    if vw < 1.0 || vh < 1.0 { return None; }

    let camera_pos = renderer.camera_pos;
    let inv_vp     = renderer.view_proj.inverse();

    let world_pt  = ndc_to_world(cursor_pos.0, cursor_pos.1, vw, vh, &inv_vp);
    let ray_dir   = (world_pt - camera_pos).normalize();

    // Transform ray to gizmo-local space.
    let dist      = (camera_pos - gizmo_pos).length().max(0.5);
    let scale     = dist * 0.15;
    let model     = glam::Mat4::from_translation(gizmo_pos)
        * glam::Mat4::from_scale(glam::Vec3::splat(scale));
    let inv_model = model.inverse();
    let local_origin = inv_model.transform_point3(camera_pos);
    let local_dir    = inv_model.transform_vector3(ray_dir).normalize();

    let mode = renderer.gizmo_mode;
    let axis = gizmo_hit_test(local_origin, local_dir, mode)?;

    let start_transform = world.get::<Transform>(entity).copied()
        .unwrap_or_else(|| Transform::from_translation(glam::Vec3::ZERO));

    let (start_axis_param, start_angle, ring_tangent, ring_bitangent) = match mode {
        GizmoMode::Translate | GizmoMode::Scale => {
            let s = ray_axis_param(camera_pos, ray_dir, gizmo_pos, axis.world_dir())
                .unwrap_or(0.0);
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
    drag:         &GizmoDragState,
    camera_pos:   glam::Vec3,
    inv_vp:       glam::Mat4,
    cursor_pos:   (f32, f32),
    viewport_size: (f32, f32),
) -> Transform {
    let mut result = drag.start_transform;
    let (vw, vh) = viewport_size;
    if vw < 1.0 || vh < 1.0 { return result; }

    let world_pt = ndc_to_world(cursor_pos.0, cursor_pos.1, vw, vh, &inv_vp);
    let ray_dir  = (world_pt - camera_pos).normalize();
    let axis_dir = drag.axis.world_dir();

    match drag.mode {
        GizmoMode::Translate => {
            if let Some(s) = ray_axis_param(camera_pos, ray_dir, drag.gizmo_pos, axis_dir) {
                result.translation = drag.start_transform.translation
                    + (s - drag.start_axis_param) * axis_dir;
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
                camera_pos, ray_dir, drag.gizmo_pos,
                axis_dir, drag.ring_tangent, drag.ring_bitangent,
            ) {
                let delta = angle - drag.start_angle;
                result.rotation =
                    glam::Quat::from_axis_angle(axis_dir, delta) * drag.start_transform.rotation;
            }
        }
    }
    result
}
