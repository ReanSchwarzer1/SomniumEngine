//! Per-frame engine context.
//!
//! [`EngineContext`] is the primary data bundle passed to every
//! [`GameApp`](crate::app::GameApp) callback. It provides read access
//! to timing and configuration, mutable access to the ECS [`World`],
//! and a `should_exit` flag for graceful shutdown.
//!
//! # Lifetime Design
//!
//! `EngineContext` borrows `TimeState`, `EngineConfig`, and `World`
//! from the engine rather than cloning or wrapping them in `Arc`. This
//! is intentional:
//!
//! * **Zero overhead** — no reference counting, no heap allocation.
//! * **Single ownership** — the engine owns the data; game code gets a
//!   scoped view.
//! * **Borrow checker enforced** — Rust guarantees that game code
//!   cannot hold these references beyond the callback scope.

use somnium_audio::engine::AudioEngine;
use somnium_ecs::World;
use somnium_physics::world::PhysicsWorld;
use somnium_renderer::{RenderContext, SomniumRenderer};
use somnium_ui::UiManager;

use crate::config::EngineConfig;
use crate::time::TimeState;

/// Editor-controlled simulation clock shared by gameplay, physics and render
/// systems. Editing is a live environment preview; rendering may continue
/// while explicitly paused, but no simulation time or fixed physics step then
/// advances.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimulationClock {
    /// Whether the editor is live-previewing, playing, or explicitly paused.
    pub state: SimulationState,
    /// Deterministic environment/game time advanced only by fixed steps.
    pub elapsed_seconds: f32,
    /// Duration of one gameplay/physics step, in seconds.
    pub fixed_delta_seconds: f32,
}

#[cfg(test)]
mod tests {
    use super::{SimulationClock, SimulationState};

    #[test]
    fn simulation_clock_starts_in_live_editor_at_sixty_hertz() {
        let clock = SimulationClock::default();
        assert_eq!(clock.state, SimulationState::Editing);
        assert!(clock.elapsed_seconds.abs() < f32::EPSILON);
        assert!((clock.fixed_delta_seconds - 1.0 / 60.0).abs() < f32::EPSILON);
    }
}

impl Default for SimulationClock {
    fn default() -> Self {
        Self {
            state: SimulationState::Editing,
            elapsed_seconds: 0.0,
            fixed_delta_seconds: 1.0 / 60.0,
        }
    }
}

/// Editor transport state for gameplay and fixed-step physics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulationState {
    /// Authoring mode; gameplay time is reset and physics does not advance.
    Editing,
    /// Fixed-step gameplay and physics are advancing.
    Playing,
    /// The current simulated state is held while rendering continues.
    Paused,
}

/// Contextual data available to game code each frame.
///
/// See the [module-level documentation](self) for lifetime rationale.
pub struct EngineContext<'a> {
    /// Current frame timing (delta time, FPS, elapsed, etc.).
    pub time: &'a TimeState,

    /// Engine configuration snapshot (immutable after startup).
    pub config: &'a EngineConfig,

    /// The ECS World — mutable access for spawning, querying, and
    /// running systems.
    pub world: &'a mut World,

    /// Reference to the physics world.
    pub physics: &'a mut PhysicsWorld,

    /// Reference to the audio engine.
    pub audio: &'a mut AudioEngine,

    /// The renderer context containing wgpu state. Optional if headless.
    pub render_ctx: Option<&'a RenderContext>,

    /// The high-level Somnium Renderer for submitting draw commands. Optional if headless.
    pub renderer: Option<&'a mut SomniumRenderer>,

    /// The currently selected entity.
    pub selected_entity: &'a mut Option<somnium_ecs::entity::Entity>,

    /// Editor camera speed in m/s, driven by the viewport toolbar slider and
    /// RMB + scroll wheel (Phase 20B). Game code reads this each frame rather
    /// than owning its own speed.
    pub camera_speed: f32,

    /// Play/Pause/Stop state and deterministic fixed-step time.
    pub simulation: SimulationClock,

    /// Set by [`Self::set_camera_speed`]; the engine applies it after the
    /// callback returns. Same read-back pattern as `should_exit`, since the
    /// context is rebuilt per callback and cannot own the state.
    pub camera_speed_request: Option<f32>,

    /// A pending "frame this" request: the world-space centre the editor
    /// camera should look at and the radius that should fit in view.
    ///
    /// The editor camera lives in the game layer, so CONTROL-F's `F` — and
    /// CONTROL-G's bookmarks, orbit pivot and view presets after it — arrive
    /// as a request the game honours rather than as a write behind its back.
    /// Read it with [`Self::take_camera_focus`], which clears it.
    pub camera_focus: Option<(glam::Vec3, f32)>,

    /// The UI manager for sending messages to the HTML frontend.
    pub ui: &'a mut UiManager,

    /// Phase 16-C: live scripts.
    ///
    /// Game code uses this to import `.luau` assets and to install the
    /// entity-to-rigid-body mapping `applyForce` needs — the engine has no
    /// rigid-body component of its own, so it cannot route a force without
    /// being told how. The *phases* are driven by the engine, not from
    /// here; calling them again from a callback would run every script
    /// twice.
    pub scripts: &'a mut crate::script_host::ScriptHost,

    /// Set to `true` to request a graceful engine shutdown at the end
    /// of the current frame.
    ///
    /// The engine will call [`GameApp::on_shutdown`](crate::app::GameApp::on_shutdown)
    /// before exiting.
    pub should_exit: bool,
}

impl<'a> EngineContext<'a> {
    /// Consume any pending focus request. Returns `(centre, radius)` once.
    pub fn take_camera_focus(&mut self) -> Option<(glam::Vec3, f32)> {
        self.camera_focus.take()
    }

    /// Request a new editor camera speed, as a normalized `0..=1` slider
    /// position. Applied by the engine once the callback returns, which also
    /// refreshes the viewport toolbar so the slider tracks the change.
    pub fn set_camera_speed(&mut self, normalized: f32) {
        self.camera_speed_request = Some(normalized.clamp(0.0, 1.0));
    }

    /// Construct a new context.
    ///
    /// This is called internally by [`Engine`](crate::app::Engine) each
    /// frame. Game code should not need to construct this directly.
    pub(crate) fn new(
        time: &'a TimeState,
        config: &'a EngineConfig,
        world: &'a mut World,
        physics: &'a mut PhysicsWorld,
        audio: &'a mut AudioEngine,
        render_ctx: Option<&'a RenderContext>,
        renderer: Option<&'a mut SomniumRenderer>,
        selected_entity: &'a mut Option<somnium_ecs::entity::Entity>,
        ui: &'a mut UiManager,
        camera_speed: f32,
        simulation: SimulationClock,
        scripts: &'a mut crate::script_host::ScriptHost,
    ) -> Self {
        Self {
            time,
            config,
            world,
            physics,
            audio,
            render_ctx,
            renderer,
            selected_entity,
            camera_speed,
            simulation,
            camera_speed_request: None,
            camera_focus: None,
            ui,
            scripts,
            should_exit: false,
        }
    }

    /// Convenience accessor: delta time as `f32` seconds.
    ///
    /// Equivalent to `ctx.time.dt()`.
    #[inline]
    #[must_use]
    pub fn dt(&self) -> f32 {
        self.time.dt()
    }

    /// Request engine shutdown.
    ///
    /// Prefer this over setting `should_exit` directly for readability.
    #[inline]
    pub fn exit(&mut self) {
        self.should_exit = true;
    }
}
