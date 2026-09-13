//! Play owns physical input only while its scene window owns the cursor.
use super::*;
use winit::{
    event::ElementState,
    keyboard::{KeyCode, PhysicalKey},
    window::CursorGrabMode,
};

pub(super) struct PlayCursor {
    pub requested: bool,
    pub window: Option<WindowId>,
    pub mode: &'static str,
}

impl Default for PlayCursor {
    fn default() -> Self {
        Self {
            requested: false,
            window: None,
            mode: "released",
        }
    }
}

fn is_game_input(event: &WindowEvent) -> bool {
    matches!(
        event,
        WindowEvent::KeyboardInput { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::CursorMoved { .. }
            | WindowEvent::Touch(_)
            | WindowEvent::Ime(_)
    )
}

fn capture_allowed(requested: bool, state: SimulationState, focused: bool) -> bool {
    requested && state == SimulationState::Playing && focused
}

impl<G: GameApp> Engine<G> {
    fn play_window(&self) -> Option<Arc<Window>> {
        let floating = self
            .ui_manager
            .as_ref()
            .is_some_and(|ui| ui.is_panel_floating(somnium_ui::floating::FloatingKind::Viewport));
        if floating {
            self.floating
                .iter()
                .find(|w| w.kind.hosts_scene())
                .map(|w| w.window.clone())
        } else {
            self.window.clone()
        }
    }

    pub(super) fn release_play_cursor(&mut self) {
        self.play_cursor.requested = false;
        self.sync_play_cursor();
    }

    fn notify_play_focus(&mut self, focused: bool) {
        if !focused {
            self.input.handle_window_event(&WindowEvent::Focused(false));
        }
        let (Some(physics), Some(audio), Some(ui)) = (
            self.physics.as_mut(),
            self.audio.as_mut(),
            self.ui_manager.as_mut(),
        ) else {
            return;
        };
        let mut ctx = EngineContext::new(
            &self.time,
            &self.config,
            &mut self.world,
            physics,
            audio,
            &mut self.jobs,
            &mut self.navigation_editor,
            self.render_ctx.as_ref(),
            self.renderer.as_mut(),
            &mut self.selection.primary,
            ui,
            crate::camera_speed_from_normalized(self.camera_speed_norm),
            self.simulation_clock,
            &mut self.scripts,
        );
        self.game
            .on_event(&mut ctx, &EngineEvent::WindowFocused(focused));
    }

    pub(super) fn sync_play_cursor(&mut self) {
        let target = self.play_window().filter(|w| {
            capture_allowed(
                self.play_cursor.requested,
                self.simulation_clock.state,
                w.has_focus(),
            )
        });
        if self.play_cursor.window == target.as_ref().map(|w| w.id()) {
            return;
        }
        if let Some(id) = self.play_cursor.window.take() {
            let previous = self
                .window
                .iter()
                .chain(self.floating.iter().map(|w| &w.window))
                .find(|w| w.id() == id);
            if let Some(window) = previous {
                let _ = window.set_cursor_grab(CursorGrabMode::None);
                window.set_cursor_visible(true);
            }
            self.play_cursor.mode = "released";
            self.notify_play_focus(false);
        }
        if let Some(window) = target {
            // Locked is supported on Wayland/macOS, Confined on Windows. Never
            // hide the pointer after a failed grab: Esc must remain reachable.
            let mode = if window.set_cursor_grab(CursorGrabMode::Locked).is_ok() {
                "locked"
            } else if window.set_cursor_grab(CursorGrabMode::Confined).is_ok() {
                "confined"
            } else {
                window.set_cursor_visible(true);
                self.play_cursor.requested = false;
                warn!("Play could not capture the cursor; click the viewport to retry");
                return;
            };
            window.set_cursor_visible(false);
            self.play_cursor.window = Some(window.id());
            self.play_cursor.mode = mode;
            self.notify_play_focus(true);
        }
    }

    /// Runs before either docked or floating editor widgets can claim gameplay.
    pub(super) fn route_play_input(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        event: &WindowEvent,
    ) -> bool {
        if self.play_cursor.window == Some(id)
            && matches!(
                event,
                WindowEvent::Focused(false) | WindowEvent::CloseRequested | WindowEvent::Destroyed
            )
        {
            self.release_play_cursor();
        }
        let scene_window = self.play_window().is_some_and(|w| w.id() == id);
        if !self.play_session_active || !scene_window {
            return false;
        }
        if matches!(event, WindowEvent::Focused(true)) {
            self.sync_play_cursor();
        }
        if let WindowEvent::KeyboardInput { event: key, .. } = event {
            if key.physical_key == PhysicalKey::Code(KeyCode::Escape)
                && key.state == ElementState::Pressed
                && (self.play_cursor.requested
                    || self.play_cursor.window.is_some()
                    || self.ui_manager.as_ref().is_some_and(|ui| ui.is_immersive()))
            {
                self.release_play_cursor();
                if let Some(ui) = &mut self.ui_manager {
                    ui.set_immersive(false);
                }
                return true;
            }
        }
        if self.play_cursor.window == Some(id) && is_game_input(event) {
            self.input.handle_window_event(event);
            if !self.dispatch_game_os_event(event) {
                if let Some(event) = translate_window_event(event) {
                    self.forward_engine_event(event_loop, event);
                }
            }
            return true;
        }
        false
    }

    /// Called only after the editor declined a click, so inspector/toolbar clicks
    /// cannot unexpectedly recapture the pointer.
    pub(super) fn capture_on_viewport_click(&mut self, id: WindowId, event: &WindowEvent) -> bool {
        if self.simulation_clock.state != SimulationState::Playing
            || self.play_cursor.window.is_some()
            || !matches!(
                event,
                WindowEvent::MouseInput {
                    button: winit::event::MouseButton::Left,
                    state: ElementState::Pressed,
                    ..
                }
            )
        {
            return false;
        }
        let Some(window) = self.play_window().filter(|w| w.id() == id) else {
            return false;
        };
        let inside = self.ui_manager.as_ref().is_some_and(|ui| {
            let (x, y, width, height) = ui.viewport_physical_rect(window.scale_factor() as f32);
            let (cx, cy) = self.cursor_pos;
            cx >= x as f32 && cy >= y as f32 && cx < (x + width) as f32 && cy < (y + height) as f32
        });
        if !inside {
            return false;
        }
        self.play_cursor.requested = true;
        self.sync_play_cursor();
        self.play_cursor.window.is_some()
    }

    pub(super) fn suppress_released_play_input(&self, event: &WindowEvent) -> bool {
        self.play_session_active && self.play_cursor.window.is_none() && is_game_input(event)
    }

    pub(super) fn dispatch_game_os_event(&mut self, event: &WindowEvent) -> bool {
        let mut ctx = EngineContext::new(
            &self.time,
            &self.config,
            &mut self.world,
            self.physics.as_mut().unwrap(),
            self.audio.as_mut().unwrap(),
            &mut self.jobs,
            &mut self.navigation_editor,
            self.render_ctx.as_ref(),
            self.renderer.as_mut(),
            &mut self.selection.primary,
            self.ui_manager.as_mut().unwrap(),
            crate::camera_speed_from_normalized(self.camera_speed_norm),
            self.simulation_clock,
            &mut self.scripts,
        );
        self.game.on_os_event(&mut ctx, event)
    }
}
