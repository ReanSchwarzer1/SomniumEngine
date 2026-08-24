//! Screen-space game canvas (Phase 26-G).
//!
//! Wraps [`UserInterface`] + [`UiPass`] without `UiManager` editor chrome so a
//! game can build a widget tree (HUD, pause, menus) and draw it through the
//! same GPU pass the editor uses. Nine-slice lives on [`DrawingContext`].

pub mod anchor;
pub mod canvas;
pub mod nav;

pub use anchor::{Anchoring, Anchors, Offsets, Pivot};
pub use canvas::{Canvas, CanvasLayout, CanvasMode, CanvasScaler, Layer, SafeArea};
pub use nav::{Direction, InputSource, NavAction, NavActions, NavCandidate, NavLinks};

use crate::{
    message::NodeHandle,
    pass::UiPass,
    ui::UserInterface,
    widget::WidgetBuilder,
    widgets::{border::BorderBuilder, text::TextBuilder},
};
use glam::Vec2;
use winit::event::WindowEvent;
use winit::window::Window;

/// Retained screen-space UI owned by a game, not the editor shell.
pub struct UiCanvas {
    ui: UserInterface,
    pass: Option<UiPass>,
    font_id: u8,
    /// MORROWIND-E. What space this tree lives in, how it scales, what it must
    /// keep clear of, and which layer it draws on.
    canvas: Canvas,
    /// MORROWIND-H. When the last frame was, for the motion clock.
    ///
    /// The editor shell has had one of these since Phase 27-C
    /// (`UiManager::last_frame_at`) and a game canvas did not, which meant a
    /// game's tweens never advanced — `render` laid out and drew and never
    /// ticked. The same class of bug as MORROWIND-E2 itself: the capability
    /// existed, the runtime path to it did not.
    last_frame_at: Option<std::time::Instant>,
    /// Pixels per world unit for a [`CanvasMode::World`] canvas.
    ///
    /// A world canvas renders to an offscreen target of `size *
    /// world_pixels_per_unit`, so this is the knob that trades a name-plate's
    /// text crispness against its memory. It is per-canvas on purpose: the
    /// world-space decision in `canvas.rs` names raising it for one canvas as
    /// the mitigation for resampled text, and a global would make that a
    /// whole-project trade.
    world_pixels_per_unit: f32,
}

impl UiCanvas {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            ui: UserInterface::new(width, height),
            pass: None,
            font_id: 0,
            canvas: Canvas::screen(),
            world_pixels_per_unit: 100.0,
            last_frame_at: None,
        }
    }

    /// Build a canvas in an explicit mode.
    ///
    /// The size is derived from the canvas rather than passed in, because for
    /// every mode except `Screen` the canvas already knows it — and a caller
    /// that supplies both can supply two that disagree.
    #[must_use]
    pub fn with_canvas(canvas: Canvas, viewport: glam::Vec2) -> Self {
        let layout = canvas.layout(viewport, 100.0);
        let mut out = Self::new(layout.logical_size.x, layout.logical_size.y);
        out.canvas = canvas;
        out
    }

    /// The canvas root.
    #[must_use]
    pub fn canvas(&self) -> &Canvas {
        &self.canvas
    }

    /// Change the canvas root. Takes effect at the next layout.
    pub fn set_canvas(&mut self, canvas: Canvas) {
        self.canvas = canvas;
    }

    /// Pixels per world unit for a world-space canvas.
    pub fn set_world_pixels_per_unit(&mut self, pixels: f32) {
        self.world_pixels_per_unit = pixels.max(1.0);
    }

    /// Resolve this canvas against a viewport, in logical pixels.
    #[must_use]
    pub fn layout_for(&self, viewport: glam::Vec2) -> CanvasLayout {
        self.canvas.layout(viewport, self.world_pixels_per_unit)
    }

    /// Where an anchored child lands, inside the safe area.
    ///
    /// The call a game HUD makes: give it an [`Anchoring`] and it comes back
    /// with a rect that already respects the resolution policy and the notch.
    #[must_use]
    pub fn place(&self, viewport: glam::Vec2, anchoring: &Anchoring) -> crate::types::Rect {
        self.canvas.place(&self.layout_for(viewport), anchoring)
    }

    /// Apply the canvas's resolution policy to the widget tree.
    ///
    /// Called by [`Self::render`] before layout; exposed so a headless caller —
    /// a test, or a game laying out before its first frame — gets the same
    /// result without a GPU.
    pub fn apply_canvas(&mut self, viewport: glam::Vec2) -> CanvasLayout {
        let layout = self.layout_for(viewport);
        self.ui.screen_size = layout.logical_size;
        layout
    }

    /// Put a node exactly where an anchoring says it goes.
    ///
    /// MORROWIND-E2, closing a loop MORROWIND-E left open: [`Canvas::place`]
    /// resolved an [`Anchoring`] into a [`Rect`](crate::types::Rect) and
    /// *nothing consumed the result*. Anchors that compute a rectangle no
    /// widget ever reads are a layout system with no output, and the only
    /// reason it looked finished is that the sub-phase's tests asserted on the
    /// rectangles rather than on the tree.
    ///
    /// Returns the rectangle, because a caller that wants to know where its
    /// minimap landed should not have to ask twice.
    pub fn place_anchored(
        &mut self,
        handle: NodeHandle,
        anchoring: &Anchoring,
    ) -> crate::types::Rect {
        let layout = self.layout_for(self.ui.screen_size);
        let rect = self.canvas.place(&layout, anchoring);
        self.place_node(handle, rect);
        rect
    }

    /// Put a node at an explicit rectangle in canvas space.
    ///
    /// The primitive under [`Self::place_anchored`], for a game that computes
    /// its own placement — a health bar that shrinks with the value, a tooltip
    /// following the cursor. Sets the size *and* the position, because a node
    /// placed at a rectangle it does not fill is the bug this replaces.
    pub fn place_node(&mut self, handle: NodeHandle, rect: crate::types::Rect) {
        self.ui.place_node(handle, rect);
    }

    pub fn ui(&self) -> &UserInterface {
        &self.ui
    }

    pub fn ui_mut(&mut self) -> &mut UserInterface {
        &mut self.ui
    }

    /// The motion driver, for a game that builds its own transitions.
    ///
    /// MORROWIND-H. Register a CONTROL-K curve here and the [`CurveId`] it
    /// returns is usable as an `Easing` anywhere in this canvas.
    ///
    /// [`CurveId`]: crate::motion::CurveId
    pub fn motion(&self) -> &crate::motion::Animator {
        &self.ui.draw_ctx.motion
    }

    /// The motion driver, mutably.
    pub fn motion_mut(&mut self) -> &mut crate::motion::Animator {
        &mut self.ui.draw_ctx.motion
    }

    /// Advance motion by an explicit `dt_ms` instead of by the wall clock.
    ///
    /// [`UiCanvas::render`] ticks from its own `Instant`, which is right for a
    /// HUD and wrong for two cases a game actually has: a **fixed-timestep**
    /// simulation, where UI motion should advance with the simulation and not
    /// with the frame; and a **paused** game whose pause menu must still
    /// animate while nothing else does. Calling this makes `render`'s own tick
    /// the smaller of the two rather than a double advance — the clock is reset
    /// each time, so an explicit tick and the automatic one cannot both charge
    /// the same milliseconds.
    pub fn tick_motion(&mut self, dt_ms: f32) -> bool {
        self.last_frame_at = Some(std::time::Instant::now());
        self.ui.draw_ctx.motion.tick(dt_ms)
    }

    /// Whether anything in this canvas is animating.
    ///
    /// A game that only redraws on change asks this; a game that draws every
    /// frame anyway can ignore it.
    pub fn is_animating(&self) -> bool {
        !self.ui.draw_ctx.motion.is_idle()
    }

    pub fn font_id(&self) -> u8 {
        self.font_id
    }

    pub fn add_font(&mut self, bytes: &[u8]) -> Result<u8, &'static str> {
        let id = self.ui.add_font(bytes)?;
        self.font_id = id;
        Ok(id)
    }

    pub fn process_os_event(&mut self, event: &WindowEvent) -> bool {
        self.ui.process_os_event(event)
    }

    /// Layout + draw. Creates the GPU pass on first call.
    ///
    /// A game canvas lays out in logical units exactly like the editor shell
    /// does, so the HUD keeps its apparent size on a HiDPI display. The scale
    /// is read from the window each frame rather than cached, because a canvas
    /// is cheap to lay out and a game may be dragged between monitors.
    pub fn render(
        &mut self,
        window: &Window,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        output_format: wgpu::TextureFormat,
    ) {
        let ui_scale = window.scale_factor() as f32;
        // MORROWIND-E: the canvas decides the logical size, not the window.
        // A `ConstantPixel` canvas resolves to exactly the window's logical
        // size, so this is a no-op for every pre-E caller.
        let physical_size = window.inner_size();
        self.apply_canvas(glam::Vec2::new(
            (physical_size.width.max(1) as f32) / ui_scale,
            (physical_size.height.max(1) as f32) / ui_scale,
        ));
        self.ui.set_ui_scale(ui_scale);
        self.ui.draw_ctx.font_atlas.set_render_scale(ui_scale);
        self.ui.draw_ctx.icon_atlas.set_render_scale(ui_scale);

        // MORROWIND-H. Advance motion before layout, so a track that finished
        // this frame settles into the layout it produced rather than one frame
        // late. A stalled frame — a breakpoint, a minimised window — must not
        // teleport every track to its end state, hence the 100 ms clamp, which
        // is the same number the editor shell uses.
        let now = std::time::Instant::now();
        if let Some(previous) = self.last_frame_at {
            let dt_ms = now.duration_since(previous).as_secs_f32() * 1000.0;
            self.ui.draw_ctx.motion.tick(dt_ms.min(100.0));
        }
        self.last_frame_at = Some(now);

        let _ = self.ui.update();
        self.ui.perform_layout();
        self.ui.draw();

        let logical_w = self.ui.screen_size.x.max(1.0);
        let logical_h = self.ui.screen_size.y.max(1.0);
        let physical = window.inner_size();
        let (phys_w, phys_h) = (physical.width.max(1), physical.height.max(1));

        let pass = self
            .pass
            .get_or_insert_with(|| UiPass::new(device, queue, output_format));
        pass.prepare(
            device,
            queue,
            &mut self.ui.draw_ctx,
            crate::pass::UiSurface::new((logical_w, logical_h), (phys_w, phys_h)),
        );
        pass.render(encoder, view);
    }

    /// Optional pause/HUD stub: a centred label on a dim panel.
    pub fn add_pause_banner(&mut self, text: &str) -> NodeHandle {
        let root = self.ui.root();
        let panel = BorderBuilder::new(
            WidgetBuilder::new()
                .with_width(280.0)
                .with_height(64.0)
                .with_desired_position(Vec2::new(40.0, 40.0))
                .with_background(crate::theme::BG_HEADER)
                .with_foreground(crate::theme::BORDER_DARK),
        )
        .with_stroke_thickness(crate::types::Thickness::uniform(1.0))
        .build();
        let panel_h = self.ui.add_node(panel, root);
        let label = TextBuilder::new(
            WidgetBuilder::new().with_margin(crate::types::Thickness::uniform(16.0)),
        )
        .with_text(text)
        .with_font_id(self.font_id)
        .with_font_size(16.0)
        .with_color(crate::theme::TEXT_PRIMARY)
        .build();
        self.ui.add_node(label, panel_h);
        panel_h
    }
}

// ── MORROWIND-E2: the hook ───────────────────────────────────────────────────
//
// MORROWIND-D, -E, -F and -G built a runtime UI a game could not reach.
// `UiCanvas::render` needs a window, a device, a queue, an encoder, a swapchain
// view and a surface format, and `EngineContext` hands a `GameApp` none of
// them — so `examples/vvardenfell` computed its HUD layout and `println!`d it.
// Four sub-phases of paint layer, canvas, navigation and text, and not one
// pixel a game could put on screen.
//
// The fix is deliberately not "put a `UiCanvas` in `EngineContext`". A game
// owns its canvases — a HUD, a pause menu, a world-space name-plate are three
// of them and the engine has no business knowing how many there are. What the
// engine owns is the *moment*: the point in the frame after the world and
// before the editor's chrome. So the engine hands the game that moment, with
// the GPU state already open, and the game hands back whichever canvases it
// wants drawn into it.

/// The open frame a game draws its UI into.
///
/// Handed to `GameApp::on_render_ui` at pass 9 of the renderer, after the world
/// and before the editor shell. Holds the encoder, so a canvas drawn through it
/// lands in the same submission as everything else in the frame rather than in
/// one of its own.
///
/// **Build in `on_render`, draw in `on_render_ui`.** The split is not
/// bureaucratic: `on_render` has the whole `EngineContext` and is where a
/// widget tree is mutated, and this type deliberately carries no world, no
/// physics and no time — a frame that could mutate the world halfway through
/// recording it is a frame with a hazard in it.
pub struct GameUiFrame<'a> {
    window: &'a Window,
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    encoder: &'a mut wgpu::CommandEncoder,
    view: &'a wgpu::TextureView,
    format: wgpu::TextureFormat,
    drawn: u32,
}

impl<'a> GameUiFrame<'a> {
    /// Open a frame. Called by the renderer, not by a game.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        window: &'a Window,
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        encoder: &'a mut wgpu::CommandEncoder,
        view: &'a wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) -> Self {
        Self {
            window,
            device,
            queue,
            encoder,
            view,
            format,
            drawn: 0,
        }
    }

    /// Lay out and draw one canvas into this frame.
    ///
    /// Call it once per canvas, in back-to-front order. Ordering *within* a
    /// canvas is [`Layer`]'s job; ordering *between* canvases is call order,
    /// because two canvases are two trees and nothing can sort across trees
    /// without merging them — which is what a single canvas with layers already
    /// is, and is the reason to prefer one.
    pub fn draw(&mut self, canvas: &mut UiCanvas) {
        canvas.render(
            self.window,
            self.device,
            self.queue,
            self.encoder,
            self.view,
            self.format,
        );
        self.drawn += 1;
    }

    /// How many canvases have been drawn into this frame so far.
    ///
    /// Exists so "the hook ran" is a checkable claim rather than an impression
    /// — the engine logs a one-time warning when a frame closes at zero and the
    /// game implements `on_render_ui`, which is the shape of the bug this whole
    /// sub-phase exists to fix.
    pub fn drawn(&self) -> u32 {
        self.drawn
    }

    /// The window, for a game that needs the scale factor or the inner size to
    /// decide *what* to draw before drawing it.
    pub fn window(&self) -> &Window {
        self.window
    }

    /// The surface format, for a game rendering its own offscreen pass.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }
}

/// The renderer's view of "there is a game with a UI".
///
/// `somnium_renderer` cannot depend on `somnium_core`, so the callback that
/// reaches a `GameApp` cannot be typed as one. This trait is the seam: the
/// renderer calls it, `somnium_core` implements it with a one-line adapter over
/// the boxed game, and neither crate learns about the other.
pub trait GameUi {
    /// Draw the game's UI into the open frame.
    fn draw_ui(&mut self, frame: &mut GameUiFrame);
}

impl<F: FnMut(&mut GameUiFrame)> GameUi for F {
    fn draw_ui(&mut self, frame: &mut GameUiFrame) {
        self(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::{border::BorderBuilder, text::TextBuilder};

    #[test]
    fn canvas_builds_a_widget_tree_without_editor_chrome() {
        let mut canvas = UiCanvas::new(640.0, 360.0);
        let root = canvas.ui().root();
        let t = TextBuilder::new(WidgetBuilder::new())
            .with_text("HUD")
            .build();
        let h = canvas.ui_mut().add_node(t, root);
        assert!(h.is_some());
        canvas.ui_mut().perform_layout();
        canvas.ui_mut().draw();
    }

    // ── MORROWIND-E2 ────────────────────────────────────────────────────────

    /// The bug this sub-phase is named after, as a test: an `Anchoring`
    /// resolved to a rectangle and no widget ever moved.
    #[test]
    fn place_anchored_moves_the_widget_and_not_only_the_rectangle() {
        let mut canvas = UiCanvas::with_canvas(
            Canvas::screen().on_layer(Layer::HUD),
            Vec2::new(1280.0, 720.0),
        );
        canvas.apply_canvas(Vec2::new(1280.0, 720.0));
        let root = canvas.ui().root();
        let node = TextBuilder::new(WidgetBuilder::new())
            .with_text("minimap")
            .build();
        let handle = canvas.ui_mut().add_node(node, root);

        let anchoring = Anchoring::pinned(
            Anchors::TOP_RIGHT,
            Vec2::new(-176.0, 16.0),
            Vec2::splat(160.0),
        );
        let rect = canvas.place_anchored(handle, &anchoring);
        canvas.ui_mut().perform_layout();

        let bounds = canvas.ui().screen_bounds(handle);
        assert!(
            (bounds.x - rect.x).abs() < 0.5 && (bounds.y - rect.y).abs() < 0.5,
            "anchoring resolved to {rect:?} but the widget laid out at {bounds:?}"
        );
        assert!(
            (bounds.w - 160.0).abs() < 0.5,
            "width not applied: {bounds:?}"
        );
        // Top-right on a 1280-wide canvas: 1280 - 176 = 1104.
        assert!(bounds.x > 1000.0, "not on the right: {bounds:?}");
    }

    /// A node placed at a rectangle fills it rather than centring inside it.
    #[test]
    fn place_node_pins_both_alignments() {
        let mut canvas = UiCanvas::new(800.0, 600.0);
        let root = canvas.ui().root();
        let node = BorderBuilder::new(WidgetBuilder::new()).build();
        let handle = canvas.ui_mut().add_node(node, root);
        canvas.place_node(
            handle,
            crate::types::Rect {
                x: 10.0,
                y: 20.0,
                w: 300.0,
                h: 40.0,
            },
        );
        canvas.ui_mut().perform_layout();
        let b = canvas.ui().screen_bounds(handle);
        assert!(
            (b.x - 10.0).abs() < 0.5 && (b.y - 20.0).abs() < 0.5,
            "{b:?}"
        );
        assert!(
            (b.w - 300.0).abs() < 0.5 && (b.h - 40.0).abs() < 0.5,
            "{b:?}"
        );
    }

    /// Re-placing on a resize keeps the widget on the anchor, which is the
    /// whole reason anchors exist and the thing absolute pixels get wrong.
    #[test]
    fn a_pinned_widget_survives_a_resize() {
        let anchoring = Anchoring::pinned(
            Anchors::TOP_RIGHT,
            Vec2::new(-100.0, 8.0),
            Vec2::splat(80.0),
        );
        let mut right_edges = Vec::new();
        for viewport in [Vec2::new(1280.0, 720.0), Vec2::new(3840.0, 2160.0)] {
            let mut canvas = UiCanvas::with_canvas(Canvas::screen().on_layer(Layer::HUD), viewport);
            canvas.apply_canvas(viewport);
            let root = canvas.ui().root();
            let h = canvas
                .ui_mut()
                .add_node(BorderBuilder::new(WidgetBuilder::new()).build(), root);
            canvas.place_anchored(h, &anchoring);
            canvas.ui_mut().perform_layout();
            let b = canvas.ui().screen_bounds(h);
            right_edges.push(canvas.ui().screen_size.x - (b.x + b.w));
        }
        assert!(
            (right_edges[0] - right_edges[1]).abs() < 1.0,
            "distance from the right edge changed with resolution: {right_edges:?}"
        );
    }

    // ── MORROWIND-H ────────────────────────────────────────────────────────

    /// The bug: a game canvas laid out and drew and never ticked, so a game's
    /// tweens sat at their origin forever.
    #[test]
    fn a_game_canvas_advances_its_own_motion() {
        use crate::motion::{Easing, Motion, MotionKey, MotionProperty};

        let mut canvas = UiCanvas::new(640.0, 360.0);
        let key = MotionKey::new(1, MotionProperty::HoverWash);
        canvas
            .motion_mut()
            .start_with(key, 0.0, 1.0, Motion::timed(100.0, Easing::Linear));
        assert!(canvas.is_animating());
        assert_eq!(canvas.motion().value_or(key, -1.0), 0.0);

        canvas.tick_motion(50.0);
        let half = canvas.motion().value_or(key, -1.0);
        assert!(
            (half - 0.5).abs() < 1e-3,
            "half way should be 0.5, got {half}"
        );

        canvas.tick_motion(60.0);
        assert_eq!(canvas.motion().value_or(key, -1.0), 1.0);
        assert!(!canvas.is_animating(), "a finished track was left running");
    }

    /// A spring on a game canvas, through the public surface only — the shape
    /// a pause menu sliding in actually has.
    #[test]
    fn a_game_canvas_can_drive_a_spring_through_its_public_surface() {
        use crate::motion::{Motion, MotionKey, MotionProperty, Spring};

        let mut canvas = UiCanvas::new(640.0, 360.0);
        let key = MotionKey::new(2, MotionProperty::Scale);
        canvas
            .motion_mut()
            .start_with(key, 0.0, 1.0, Motion::Spring(Spring::SNAPPY));
        let mut ticks = 0;
        while canvas.is_animating() && ticks < 1000 {
            canvas.tick_motion(1000.0 / 120.0);
            ticks += 1;
        }
        assert!(
            ticks > 1,
            "a spring that finished in one tick is not a spring"
        );
        assert_eq!(canvas.motion().value_or(key, -1.0), 1.0);
    }

    /// `GameUiFrame` counts what it drew, so "the hook ran" is checkable. The
    /// GPU half needs a device; this is the half that does not.
    #[test]
    fn a_game_ui_closure_satisfies_the_seam() {
        fn takes_a_game_ui(_: &mut dyn GameUi) {}
        let mut called = false;
        let mut f = |_: &mut GameUiFrame| called = true;
        takes_a_game_ui(&mut f);
        // Not called — nothing opened a frame. The assertion is that a plain
        // closure *is* a `GameUi`, so a game never writes an adapter type.
        assert!(!called);
    }
}
