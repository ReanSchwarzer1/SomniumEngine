//! Anchors, offsets and pivots (Phase MORROWIND, MORROWIND-E).
//!
//! `phase_MORROWIND.md` §8 item 2: *"min/max anchor, offsets, pivot, stretch —
//! the RectTransform vocabulary, **without discarding Fyrox's arrange pass**."*
//!
//! # What this is layered on, and what it is not
//!
//! `somnium_ui` already has a measure/arrange core: a widget declares a desired
//! size, its parent arranges it into a rect, and `HorizontalAlignment` /
//! `VerticalAlignment` / `Thickness` decide where inside that rect it sits.
//! That core is not being replaced. **Anchors are a different question**, and
//! the two answer different halves of "where does this go":
//!
//! - Alignment says *"put me at the top-right of whatever space I was given"*.
//!   It is a widget's preference, resolved during arrange, and it is what a
//!   stack panel or a grid cell wants.
//! - An anchor says *"my top-right corner is 20 px in from the parent's
//!   top-right corner, and stays there when the parent resizes"*. It is a
//!   *relationship to the parent's rect*, and it is what a HUD wants — a
//!   minimap pinned to a corner, a health bar stretched across the bottom, a
//!   crosshair that stays centred at every resolution.
//!
//! Alignment cannot express the stretch case at all: "16 px from the left edge
//! and 16 px from the right edge, whatever the width" has no alignment value.
//! That is the gap this module fills, and it fills it by computing a rect that
//! the existing arrange pass then uses, rather than by replacing arrange.
//!
//! # The vocabulary
//!
//! Unity's `RectTransform`, because it is the one every UI author already knows
//! and because its degenerate cases are the useful ones: equal min and max on
//! an axis is "pin", unequal is "stretch", and the offsets mean different
//! things in each — which sounds like a wart and is actually the whole
//! expressiveness of the system.

use crate::types::Rect;
use glam::Vec2;

/// Normalised anchor corners within the parent's rect.
///
/// `(0, 0)` is the parent's top-left and `(1, 1)` its bottom-right, matching
/// the y-down convention the rest of `somnium_ui` uses. When `min` and `max`
/// are equal on an axis the child is **pinned** on that axis and its size comes
/// from `offsets`; when they differ it **stretches** and `offsets` become
/// insets from the two anchored edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchors {
    pub min: Vec2,
    pub max: Vec2,
}

impl Default for Anchors {
    /// Top-left pin, which is what a widget with no opinion gets and which
    /// reproduces the pre-anchor behaviour of `desired_local_position`.
    fn default() -> Self {
        Self::TOP_LEFT
    }
}

impl Anchors {
    /// Pinned to the parent's top-left.
    pub const TOP_LEFT: Self = Self {
        min: Vec2::ZERO,
        max: Vec2::ZERO,
    };
    /// Pinned to the parent's top-right.
    pub const TOP_RIGHT: Self = Self {
        min: Vec2::new(1.0, 0.0),
        max: Vec2::new(1.0, 0.0),
    };
    /// Pinned to the parent's bottom-left.
    pub const BOTTOM_LEFT: Self = Self {
        min: Vec2::new(0.0, 1.0),
        max: Vec2::new(0.0, 1.0),
    };
    /// Pinned to the parent's bottom-right.
    pub const BOTTOM_RIGHT: Self = Self {
        min: Vec2::ONE,
        max: Vec2::ONE,
    };
    /// Pinned to the parent's centre. A crosshair.
    pub const CENTRE: Self = Self {
        min: Vec2::splat(0.5),
        max: Vec2::splat(0.5),
    };
    /// Stretched across the parent in both axes. A full-screen dim overlay.
    pub const STRETCH: Self = Self {
        min: Vec2::ZERO,
        max: Vec2::ONE,
    };
    /// Stretched horizontally, pinned to the top. A title bar.
    pub const TOP_STRETCH: Self = Self {
        min: Vec2::ZERO,
        max: Vec2::new(1.0, 0.0),
    };
    /// Stretched horizontally, pinned to the bottom. A health bar.
    pub const BOTTOM_STRETCH: Self = Self {
        min: Vec2::new(0.0, 1.0),
        max: Vec2::ONE,
    };

    /// Whether the child stretches on x.
    #[must_use]
    pub fn stretches_x(&self) -> bool {
        (self.max.x - self.min.x).abs() > f32::EPSILON
    }

    /// Whether the child stretches on y.
    #[must_use]
    pub fn stretches_y(&self) -> bool {
        (self.max.y - self.min.y).abs() > f32::EPSILON
    }
}

/// Offsets from the anchors, in logical pixels.
///
/// The meaning depends on whether the axis stretches:
///
/// - **Pinned** (`min == max`): `left`/`top` position the child's pivot
///   relative to the anchor point, and `right`/`bottom` are the child's size.
/// - **Stretched** (`min != max`): all four are insets from the anchored edges,
///   so `left: 16, right: 16` means "16 px in from both sides, whatever the
///   parent's width".
///
/// That dual meaning is Unity's and it reads as a wart until the alternative is
/// tried: two separate structs make every caller pick one before knowing
/// whether the designer will later want to stretch, and converting between them
/// loses information.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Offsets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Offsets {
    /// A pinned child at `position` with `size`.
    #[must_use]
    pub fn pinned(position: Vec2, size: Vec2) -> Self {
        Self {
            left: position.x,
            top: position.y,
            right: size.x,
            bottom: size.y,
        }
    }

    /// A stretched child inset by `inset` on every side.
    #[must_use]
    pub fn inset(inset: f32) -> Self {
        Self {
            left: inset,
            top: inset,
            right: inset,
            bottom: inset,
        }
    }
}

/// The point within the child that the anchor positions, normalised.
///
/// `(0, 0)` is the child's top-left, `(0.5, 0.5)` its centre. Only meaningful
/// on a pinned axis: a stretched axis has no free position for a pivot to move.
pub type Pivot = Vec2;

/// The whole placement of one child within its parent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Anchoring {
    pub anchors: Anchors,
    pub offsets: Offsets,
    /// Defaults to `(0, 0)` — top-left — so an unset pivot reproduces the
    /// behaviour of a plain `desired_local_position`.
    pub pivot: Pivot,
}

impl Anchoring {
    /// A pinned child.
    #[must_use]
    pub fn pinned(anchors: Anchors, position: Vec2, size: Vec2) -> Self {
        Self {
            anchors,
            offsets: Offsets::pinned(position, size),
            pivot: Vec2::ZERO,
        }
    }

    /// A pinned child positioned by its own centre.
    ///
    /// The crosshair case, and the one where a top-left pivot is always wrong:
    /// centring by the top-left corner leaves the widget half its own size off
    /// centre, which looks like a rounding bug and is not one.
    #[must_use]
    pub fn centred(anchors: Anchors, size: Vec2) -> Self {
        Self {
            anchors,
            offsets: Offsets::pinned(Vec2::ZERO, size),
            pivot: Vec2::splat(0.5),
        }
    }

    /// A child stretched across `anchors`, inset by `offsets`.
    #[must_use]
    pub fn stretched(anchors: Anchors, offsets: Offsets) -> Self {
        Self {
            anchors,
            offsets,
            pivot: Vec2::ZERO,
        }
    }

    /// Resolve to a rect in the parent's coordinate space.
    ///
    /// The one function this module exists for. Everything else is vocabulary.
    #[must_use]
    pub fn resolve(&self, parent: Rect) -> Rect {
        let anchor_min = Vec2::new(
            parent.x + parent.w * self.anchors.min.x,
            parent.y + parent.h * self.anchors.min.y,
        );
        let anchor_max = Vec2::new(
            parent.x + parent.w * self.anchors.max.x,
            parent.y + parent.h * self.anchors.max.y,
        );

        let (x, w) = if self.anchors.stretches_x() {
            let left = anchor_min.x + self.offsets.left;
            let right = anchor_max.x - self.offsets.right;
            // A negative width is what happens when the insets exceed the
            // available space — a sidebar at 300 px each side in a 400 px
            // window. Clamping to zero renders nothing, which is honest;
            // letting it go negative flips the rect inside out and the widget
            // reappears mirrored somewhere unexpected.
            (left, (right - left).max(0.0))
        } else {
            let w = self.offsets.right;
            (anchor_min.x + self.offsets.left - w * self.pivot.x, w)
        };

        let (y, h) = if self.anchors.stretches_y() {
            let top = anchor_min.y + self.offsets.top;
            let bottom = anchor_max.y - self.offsets.bottom;
            (top, (bottom - top).max(0.0))
        } else {
            let h = self.offsets.bottom;
            (anchor_min.y + self.offsets.top - h * self.pivot.y, h)
        };

        Rect::new(x, y, w, h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARENT: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 800.0,
        h: 600.0,
    };

    #[test]
    fn a_top_left_pin_reproduces_a_plain_position() {
        let a = Anchoring::pinned(
            Anchors::TOP_LEFT,
            Vec2::new(10.0, 20.0),
            Vec2::new(100.0, 40.0),
        );
        assert_eq!(a.resolve(PARENT), Rect::new(10.0, 20.0, 100.0, 40.0));
    }

    /// A bottom-right pin measures from the far corner, so it survives a resize.
    #[test]
    fn a_bottom_right_pin_stays_in_its_corner() {
        let a = Anchoring::pinned(
            Anchors::BOTTOM_RIGHT,
            Vec2::new(-120.0, -60.0),
            Vec2::new(100.0, 40.0),
        );
        assert_eq!(a.resolve(PARENT), Rect::new(680.0, 540.0, 100.0, 40.0));

        let wider = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        assert_eq!(a.resolve(wider), Rect::new(1800.0, 1020.0, 100.0, 40.0));
    }

    /// Centring by the top-left corner leaves the widget half its own size off
    /// centre. This is the bug `Anchoring::centred` exists to make unwritable.
    #[test]
    fn centring_uses_the_pivot() {
        let size = Vec2::new(64.0, 64.0);
        let top_left = Anchoring::pinned(Anchors::CENTRE, Vec2::ZERO, size);
        let centred = Anchoring::centred(Anchors::CENTRE, size);

        assert_eq!(
            top_left.resolve(PARENT),
            Rect::new(400.0, 300.0, 64.0, 64.0)
        );
        assert_eq!(centred.resolve(PARENT), Rect::new(368.0, 268.0, 64.0, 64.0));

        let r = centred.resolve(PARENT);
        assert_eq!(
            r.x + r.w * 0.5,
            400.0,
            "the centre of the child is the centre"
        );
        assert_eq!(r.y + r.h * 0.5, 300.0);
    }

    /// The case alignment cannot express: an inset from *both* edges.
    #[test]
    fn a_stretch_insets_from_both_edges() {
        let a = Anchoring::stretched(Anchors::STRETCH, Offsets::inset(16.0));
        assert_eq!(a.resolve(PARENT), Rect::new(16.0, 16.0, 768.0, 568.0));

        let narrow = Rect::new(0.0, 0.0, 400.0, 300.0);
        assert_eq!(a.resolve(narrow), Rect::new(16.0, 16.0, 368.0, 268.0));
    }

    /// One axis pinned, one stretched. A health bar.
    #[test]
    fn mixed_axes_stretch_one_way_and_pin_the_other() {
        let a = Anchoring {
            anchors: Anchors::BOTTOM_STRETCH,
            offsets: Offsets {
                left: 40.0,
                right: 40.0,
                // Pinned on y: `top` is the offset from the anchor, `bottom` the height.
                top: -32.0,
                bottom: 12.0,
            },
            pivot: Vec2::ZERO,
        };
        assert_eq!(a.resolve(PARENT), Rect::new(40.0, 568.0, 720.0, 12.0));
    }

    /// Insets that exceed the parent clamp to zero rather than inverting.
    ///
    /// A rect with negative width flips inside out and the widget reappears
    /// mirrored somewhere unexpected, which reads as a layout bug several
    /// panels away from the one that is actually too small.
    #[test]
    fn over_inset_stretch_collapses_rather_than_inverting() {
        let a = Anchoring::stretched(Anchors::STRETCH, Offsets::inset(300.0));
        let narrow = Rect::new(0.0, 0.0, 400.0, 400.0);
        let r = a.resolve(narrow);
        assert_eq!(r.w, 0.0);
        assert_eq!(r.h, 0.0);
        assert!(r.w >= 0.0 && r.h >= 0.0);
    }

    /// A non-zero parent origin offsets everything, so nesting works.
    #[test]
    fn anchoring_is_relative_to_the_parent_rect_not_the_screen() {
        let panel = Rect::new(100.0, 50.0, 200.0, 100.0);
        let a = Anchoring::pinned(
            Anchors::TOP_RIGHT,
            Vec2::new(-10.0, 10.0),
            Vec2::splat(20.0),
        );
        assert_eq!(a.resolve(panel), Rect::new(290.0, 60.0, 20.0, 20.0));
    }

    #[test]
    fn stretch_detection_matches_the_constants() {
        assert!(!Anchors::CENTRE.stretches_x() && !Anchors::CENTRE.stretches_y());
        assert!(Anchors::STRETCH.stretches_x() && Anchors::STRETCH.stretches_y());
        assert!(Anchors::BOTTOM_STRETCH.stretches_x());
        assert!(!Anchors::BOTTOM_STRETCH.stretches_y());
    }

    /// The default is the pre-anchor behaviour, so adding the field to a widget
    /// changes nothing until somebody sets it.
    #[test]
    fn the_default_is_a_plain_top_left_position() {
        let a = Anchoring {
            offsets: Offsets::pinned(Vec2::new(7.0, 9.0), Vec2::new(30.0, 30.0)),
            ..Default::default()
        };
        assert_eq!(a.resolve(PARENT), Rect::new(7.0, 9.0, 30.0, 30.0));
    }
}
