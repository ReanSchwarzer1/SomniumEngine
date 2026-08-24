//! The slice's HUD and name-plate (Phase MORROWIND, MORROWIND-E).
//!
//! `phase_MORROWIND.md` §8, MORROWIND-E: *"Slice: `vvardenfell` gets a HUD and
//! a floating name-plate over an object."*
//!
//! # What this file is really testing
//!
//! The preamble's second-example rule: *"If a track cannot be exercised from
//! this example without reaching into engine internals, the track's API is
//! wrong."* So everything below goes through `somnium_ui`'s public surface —
//! `Canvas`, `Anchoring`, `UiCanvas` — and nothing reaches for a `pub(crate)`.
//!
//! It also exercises the two cases a HUD gets wrong in ways that only appear on
//! somebody else's hardware:
//!
//! - **A different resolution.** A health bar positioned in absolute pixels is
//!   correct at 1080p and wrong everywhere else. Anchors fix it, and the tests
//!   below check 1080p and 4K produce the same *relative* layout.
//! - **A notch.** Content under a home indicator or behind a rounded corner is
//!   invisible on the hardware somebody actually plays on, and on no developer
//!   monitor.

use glam::{Mat4, Vec2, Vec3};
use somnium_ui::{
    runtime::{
        anchor::{Anchoring, Anchors, Offsets},
        canvas::{Canvas, CanvasLayout, Layer, SafeArea},
    },
    types::Rect,
};

/// The HUD's parts, each with the anchoring that survives a resize.
///
/// Kept as data rather than as widget-construction calls so the layout can be
/// asserted without a window — and so the reason each one is anchored the way
/// it is has somewhere to live.
pub struct Hud {
    pub canvas: Canvas,
    /// Stretched across the bottom, inset from both sides. The case plain
    /// alignment cannot express at all.
    pub health_bar: Anchoring,
    /// Pinned to the top-right corner, so it stays there at any resolution.
    pub minimap: Anchoring,
    /// Centred by its own middle. A top-left pivot would leave it half its own
    /// size off centre, which looks like a rounding bug and is not one.
    pub crosshair: Anchoring,
}

impl Default for Hud {
    fn default() -> Self {
        Self::new(SafeArea::NONE)
    }
}

impl Hud {
    /// A HUD that keeps clear of `safe_area`.
    #[must_use]
    pub fn new(safe_area: SafeArea) -> Self {
        Self {
            // 1920x1080 reference, blended evenly between matching width and
            // height: the layout holds its proportions on an ultrawide instead
            // of stretching to fill it.
            canvas: Canvas::scaled(Vec2::new(1920.0, 1080.0), 0.5)
                .on_layer(Layer::HUD)
                .with_safe_area(safe_area),
            health_bar: Anchoring {
                anchors: Anchors::BOTTOM_STRETCH,
                offsets: Offsets {
                    left: 48.0,
                    right: 48.0,
                    // Pinned on y: 40 up from the bottom, 18 tall.
                    top: -40.0,
                    bottom: 18.0,
                },
                pivot: Vec2::ZERO,
            },
            minimap: Anchoring::pinned(
                Anchors::TOP_RIGHT,
                Vec2::new(-176.0, 16.0),
                Vec2::splat(160.0),
            ),
            crosshair: Anchoring::centred(Anchors::CENTRE, Vec2::splat(24.0)),
        }
    }

    /// Resolve every part against `viewport`.
    #[must_use]
    pub fn layout(&self, viewport: Vec2) -> HudLayout {
        let canvas = self.canvas.layout(viewport, 1.0);
        HudLayout {
            health_bar: self.canvas.place(&canvas, &self.health_bar),
            minimap: self.canvas.place(&canvas, &self.minimap),
            crosshair: self.canvas.place(&canvas, &self.crosshair),
            canvas,
        }
    }
}

/// Where every part landed this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudLayout {
    pub canvas: CanvasLayout,
    pub health_bar: Rect,
    pub minimap: Rect,
    pub crosshair: Rect,
}

/// A name-plate floating over a world position.
///
/// World-space rather than an overlay, because a name-plate should shrink with
/// distance: an overlay marker stays screen-sized, which is right for a quest
/// objective and wrong for a label attached to a thing.
#[must_use]
pub fn name_plate(at: Vec3, width_metres: f32) -> Canvas {
    Canvas::world(
        Mat4::from_translation(at),
        Vec2::new(width_metres, width_metres * 0.25),
    )
    .on_layer(Layer::HUD)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HD: Vec2 = Vec2::new(1920.0, 1080.0);
    const UHD: Vec2 = Vec2::new(3840.0, 2160.0);
    const ULTRAWIDE: Vec2 = Vec2::new(3440.0, 1440.0);

    /// The reason anchors exist: the same layout at four times the pixels.
    #[test]
    fn the_hud_is_the_same_layout_at_1080p_and_4k() {
        let hud = Hud::default();
        let hd = hud.layout(HD);
        let uhd = hud.layout(UHD);

        // The canvas is laid out at its reference size either way, so the parts
        // land in the same logical place and only the scale differs.
        assert_eq!(hd.health_bar, uhd.health_bar);
        assert_eq!(hd.minimap, uhd.minimap);
        assert_eq!(hd.crosshair, uhd.crosshair);
        assert!((uhd.canvas.scale / hd.canvas.scale - 2.0).abs() < 1e-3);
    }

    /// An absolute-pixel HUD would put the minimap off-screen on a small
    /// display and marooned in the middle of a large one.
    #[test]
    fn the_minimap_stays_in_its_corner_on_an_ultrawide() {
        let hud = Hud::default();
        let layout = hud.layout(ULTRAWIDE);
        let right_edge = layout.canvas.safe_rect.x + layout.canvas.safe_rect.w;
        assert!(
            (right_edge - (layout.minimap.x + layout.minimap.w) - 16.0).abs() < 0.5,
            "minimap {:?} against a canvas ending at {right_edge}",
            layout.minimap
        );
    }

    /// The crosshair's centre is the canvas's centre, not its corner.
    #[test]
    fn the_crosshair_is_actually_centred() {
        let layout = Hud::default().layout(HD);
        let centre = Vec2::new(
            layout.crosshair.x + layout.crosshair.w * 0.5,
            layout.crosshair.y + layout.crosshair.h * 0.5,
        );
        let expected = Vec2::new(
            layout.canvas.safe_rect.x + layout.canvas.safe_rect.w * 0.5,
            layout.canvas.safe_rect.y + layout.canvas.safe_rect.h * 0.5,
        );
        assert!(
            centre.abs_diff_eq(expected, 0.01),
            "{centre:?} vs {expected:?}"
        );
    }

    /// The health bar spans the width whatever the width is.
    #[test]
    fn the_health_bar_stretches_and_keeps_its_insets() {
        let hud = Hud::default();
        for viewport in [HD, UHD, ULTRAWIDE, Vec2::new(1280.0, 720.0)] {
            let layout = hud.layout(viewport);
            let safe = layout.canvas.safe_rect;
            assert!(
                (layout.health_bar.x - (safe.x + 48.0)).abs() < 0.5,
                "left inset at {viewport:?}"
            );
            assert!(
                ((safe.x + safe.w) - (layout.health_bar.x + layout.health_bar.w) - 48.0).abs()
                    < 0.5,
                "right inset at {viewport:?}"
            );
            assert_eq!(layout.health_bar.h, 18.0);
        }
    }

    /// A notch moves the HUD in, and nothing else has to know about it.
    ///
    /// This is the case that is invisible on every developer monitor and
    /// obvious on the hardware somebody plays on.
    #[test]
    fn a_safe_area_moves_the_hud_clear_of_the_notch() {
        let plain = Hud::default().layout(HD);
        let notched = Hud::new(SafeArea {
            top: 88.0,
            bottom: 68.0,
            left: 0.0,
            right: 0.0,
        })
        .layout(HD);

        assert!(
            notched.minimap.y > plain.minimap.y,
            "the minimap moved down, clear of the notch"
        );
        let plain_bottom = plain.health_bar.y + plain.health_bar.h;
        let notched_bottom = notched.health_bar.y + notched.health_bar.h;
        assert!(
            notched_bottom < plain_bottom,
            "the health bar moved up, clear of the home indicator"
        );
    }

    /// A world-space name-plate is sized in metres and ignores the viewport.
    #[test]
    fn the_name_plate_is_world_sized() {
        let plate = name_plate(Vec3::new(3.0, 1.8, -12.0), 1.2);
        let near = plate.layout(HD, 100.0);
        let far = plate.layout(Vec2::new(640.0, 480.0), 100.0);
        assert_eq!(near.logical_size, far.logical_size);
        // Compared with a tolerance, not for equality: 1.2 metres times 100
        // pixels is 120.00001 in f32, and a slice that asserts float equality
        // on derived sizes fails for a reason that has nothing to do with the
        // layout it is testing.
        assert!(near.logical_size.abs_diff_eq(Vec2::new(120.0, 30.0), 1e-3));
    }

    /// Raising the pixel density is the per-canvas mitigation for resampled
    /// text that `canvas.rs`'s world-space decision names.
    #[test]
    fn a_name_plate_can_trade_memory_for_crispness() {
        let plate = name_plate(Vec3::ZERO, 2.0);
        let coarse = plate.layout(HD, 50.0).logical_size;
        let crisp = plate.layout(HD, 200.0).logical_size;
        assert_eq!(crisp, coarse * 4.0);
    }

    /// The whole HUD builds from the public API of one crate.
    ///
    /// The second-example rule, checked rather than asserted: if this file ever
    /// needs a `pub(crate)` or a second engine crate to lay out a HUD, the API
    /// is wrong and this is where that shows.
    #[test]
    fn the_slice_uses_only_public_ui_api() {
        let hud = Hud::default();
        let layout = hud.layout(HD);
        assert!(layout.canvas.logical_size.x > 0.0);
        assert!(layout.health_bar.w > 0.0);
        assert!(layout.minimap.w > 0.0);
        assert!(layout.crosshair.w > 0.0);
    }
}

// ── MORROWIND-E2: the HUD as a widget tree, not as a table of rectangles ─────
//
// Everything above resolves anchoring into rectangles and, until MORROWIND-E2,
// that is all it did — `main.rs` printed them. Three sub-phases of paint layer
// and canvas, and the slice's evidence that any of it worked was a `println!`.
//
// `HudTree` is the part that makes the rectangles into pixels. It is
// deliberately built from the same `Anchoring` values above rather than from a
// second set of numbers, so the layout tests keep testing what is on screen.

use somnium_ui::{
    UiCanvas,
    message::NodeHandle,
    theme,
    widget::WidgetBuilder,
    widgets::{border::BorderBuilder, text::TextBuilder},
};

/// The HUD's widget tree, and the handles needed to re-place it on a resize.
pub struct HudTree {
    canvas: UiCanvas,
    hud: Hud,
    health_bar: NodeHandle,
    health_fill: NodeHandle,
    minimap: NodeHandle,
    crosshair: NodeHandle,
    /// Last viewport the tree was placed against. A HUD is re-anchored when the
    /// window changes and not once per frame: `place_anchored` invalidates the
    /// layout of every ancestor, and doing that sixty times a second to answer
    /// "no, nothing moved" is how a HUD becomes a frame cost.
    placed_for: Vec2,
    /// 0..=1. The only piece of game state the slice has, and it exists so the
    /// health bar is observably *driven* rather than observably drawn.
    pub health: f32,
}

impl HudTree {
    /// Build the tree. One canvas, four nodes, no editor chrome.
    #[must_use]
    pub fn new(hud: Hud) -> Self {
        let mut canvas = UiCanvas::with_canvas(hud.canvas, Vec2::new(1920.0, 1080.0));
        let root = canvas.ui().root();

        // The bar is two nodes: a track and a fill. One node with a background
        // cannot show a value, and a HUD whose health bar cannot show a value
        // is a rectangle with a name.
        let fill = BorderBuilder::new(
            WidgetBuilder::new()
                .with_background(theme::ACCENT)
                .with_name("health-fill"),
        )
        .build();
        let fill = canvas.ui_mut().add_node(fill, root);

        let track = BorderBuilder::new(
            WidgetBuilder::new()
                .with_background(theme::ACCENT_DIM)
                .with_name("health-track"),
        )
        .build();
        let track = canvas.ui_mut().add_node(track, root);

        let minimap = BorderBuilder::new(
            WidgetBuilder::new()
                .with_background(theme::ACCENT_DIM)
                .with_name("minimap")
                .with_children([canvas.ui_mut().add_node(
                    TextBuilder::new(WidgetBuilder::new())
                        .with_text("Seyda Neen")
                        .with_color(theme::TEXT_PRIMARY)
                        .build(),
                    root,
                )]),
        )
        .build();
        let minimap = canvas.ui_mut().add_node(minimap, root);

        let crosshair = BorderBuilder::new(
            WidgetBuilder::new()
                .with_background(theme::TEXT_PRIMARY)
                .with_name("crosshair"),
        )
        .build();
        let crosshair = canvas.ui_mut().add_node(crosshair, root);

        Self {
            canvas,
            hud,
            health_bar: track,
            health_fill: fill,
            minimap,
            crosshair,
            placed_for: Vec2::ZERO,
            health: 1.0,
        }
    }

    /// Re-anchor against `viewport` if it changed, then size the fill to
    /// `health`. Call from `on_render`; draw from `on_render_ui`.
    pub fn update(&mut self, viewport: Vec2) {
        let resized = (viewport - self.placed_for).abs().max_element() > 0.5;
        if resized {
            self.canvas.apply_canvas(viewport);
            self.canvas
                .place_anchored(self.health_bar, &self.hud.health_bar);
            self.canvas.place_anchored(self.minimap, &self.hud.minimap);
            self.canvas
                .place_anchored(self.crosshair, &self.hud.crosshair);
            self.placed_for = viewport;
        }
        // The fill is placed every frame, because the value moves even when the
        // window does not.
        let track = self.canvas.place(viewport, &self.hud.health_bar);
        self.canvas.place_node(
            self.health_fill,
            Rect {
                w: track.w * self.health.clamp(0.0, 1.0),
                ..track
            },
        );
    }

    /// The canvas, for `GameUiFrame::draw`.
    pub fn canvas_mut(&mut self) -> &mut UiCanvas {
        &mut self.canvas
    }

    /// How wide the health fill actually drew.
    ///
    /// `cfg(test)`-only for now: nothing in the slice reads its own HUD back
    /// yet. When something does — a tutorial that points at the health bar, a
    /// screenshot annotator — the gate comes off and the accessor is already
    /// the right shape. The value the bar claims to
    /// show, read back from the tree that shows it.
    #[cfg(test)]
    #[must_use]
    pub fn fill_width(&self) -> f32 {
        self.canvas.ui().screen_bounds(self.health_fill).w
    }

    /// The canvas's logical size after its scaler ran.
    #[cfg(test)]
    #[must_use]
    pub fn logical_size(&self) -> Vec2 {
        self.canvas.ui().screen_size
    }

    /// Lay out, without needing a GPU. `render` does this too; a test does not
    /// have a window to hang one off.
    #[cfg(test)]
    pub fn layout_now(&mut self) {
        self.canvas.ui_mut().perform_layout();
    }

    /// Where the parts landed. Used by the tests, and by anything that wants to
    /// assert the tree agrees with the anchoring rather than assume it.
    #[must_use]
    pub fn bounds(&self) -> [Rect; 3] {
        [
            self.canvas.ui().screen_bounds(self.health_bar),
            self.canvas.ui().screen_bounds(self.minimap),
            self.canvas.ui().screen_bounds(self.crosshair),
        ]
    }
}

#[cfg(test)]
mod tree_tests {
    use super::*;

    /// MORROWIND-E2's acceptance test, stated as the thing that was false for
    /// four sub-phases: the HUD is a tree of widgets that land where the
    /// anchoring says, not a table of rectangles a `println!` reports.
    #[test]
    fn the_tree_lands_where_the_anchoring_says() {
        let hud = Hud::new(SafeArea::NONE);
        let viewport = Vec2::new(1920.0, 1080.0);
        let expected = hud.layout(viewport);
        let mut tree = HudTree::new(hud);
        tree.update(viewport);
        tree.layout_now();

        let [bar, map, cross] = tree.bounds();
        for (got, want, name) in [
            (bar, expected.health_bar, "health bar"),
            (map, expected.minimap, "minimap"),
            (cross, expected.crosshair, "crosshair"),
        ] {
            assert!(
                (got.x - want.x).abs() < 1.0
                    && (got.y - want.y).abs() < 1.0
                    && (got.w - want.w).abs() < 1.0
                    && (got.h - want.h).abs() < 1.0,
                "{name}: anchoring says {want:?}, the widget is at {got:?}"
            );
        }
    }

    /// The fill tracks the value. A health bar that does not is a rectangle.
    #[test]
    fn the_fill_follows_the_value() {
        let viewport = Vec2::new(1920.0, 1080.0);
        let mut tree = HudTree::new(Hud::new(SafeArea::NONE));
        tree.health = 1.0;
        tree.update(viewport);
        tree.layout_now();
        let full = tree.fill_width();

        tree.health = 0.25;
        tree.update(viewport);
        tree.layout_now();
        let quarter = tree.fill_width();

        assert!(full > 0.0, "the bar has no width at all");
        assert!(
            (quarter - full * 0.25).abs() < 2.0,
            "quarter health drew {quarter} of a {full}-wide bar"
        );
    }

    /// A value outside 0..=1 clamps rather than drawing past the track — the
    /// bug every health bar has once, when a heal overshoots the maximum.
    #[test]
    fn an_overfull_bar_clamps() {
        let viewport = Vec2::new(1920.0, 1080.0);
        let mut tree = HudTree::new(Hud::new(SafeArea::NONE));
        tree.health = 1.0;
        tree.update(viewport);
        tree.layout_now();
        let full = tree.fill_width();

        tree.health = 3.0;
        tree.update(viewport);
        tree.layout_now();
        let over = tree.fill_width();
        assert!(
            (over - full).abs() < 1.0,
            "{over} exceeded the track's {full}"
        );
    }

    /// The tree survives a resize with its anchors intact, at 1080p and 4K.
    #[test]
    fn the_tree_re_anchors_on_a_resize() {
        let mut tree = HudTree::new(Hud::new(SafeArea::NONE));
        let mut insets = Vec::new();
        for viewport in [Vec2::new(1920.0, 1080.0), Vec2::new(3840.0, 2160.0)] {
            tree.update(viewport);
            tree.layout_now();
            let [_, map, _] = tree.bounds();
            let logical = tree.logical_size();
            insets.push(logical.x - (map.x + map.w));
        }
        assert!(
            (insets[0] - insets[1]).abs() < 2.0,
            "the minimap left the top-right corner on resize: {insets:?}"
        );
    }
}
