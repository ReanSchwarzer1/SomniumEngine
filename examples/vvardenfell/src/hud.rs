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

use somnium_ui::{
    runtime::{
        anchor::{Anchoring, Anchors, Offsets},
        canvas::{Canvas, CanvasLayout, Layer, SafeArea},
    },
    types::Rect,
};
use glam::{Mat4, Vec2, Vec3};

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
        assert!(centre.abs_diff_eq(expected, 0.01), "{centre:?} vs {expected:?}");
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
