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

use somnium_ecs::World;
use somnium_renderer::{RenderContext, SomniumRenderer};
use somnium_physics::world::PhysicsWorld;
use somnium_audio::engine::AudioEngine;
use somnium_ui::UiManager;

use crate::config::EngineConfig;
use crate::time::TimeState;

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

    /// The UI manager for sending messages to the HTML frontend.
    pub ui: &'a mut UiManager,

    /// Set to `true` to request a graceful engine shutdown at the end
    /// of the current frame.
    ///
    /// The engine will call [`GameApp::on_shutdown`](crate::app::GameApp::on_shutdown)
    /// before exiting.
    pub should_exit: bool,
}

impl<'a> EngineContext<'a> {
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
            ui,
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
