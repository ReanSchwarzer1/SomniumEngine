//! Canvas roots: what space a UI tree lives in (MORROWIND-E, Seam 4a).
//!
//! > *"A UI tree's root is a `Canvas` with a mode: `Screen { scaler }`,
//! > `World { transform, size, billboard }`, or `Overlay { camera }`. Widgets
//! > below it are unaware. This is what makes the editor's own chrome and a
//! > game's HUD the same code path."*
//!
//! The last sentence is the point. `somnium_ui` draws the editor shell today
//! and a `UiCanvas` for a game HUD, and they differ only in that one is wrapped
//! in editor chrome. Neither can be placed *in the world*, and neither has a
//! resolution policy beyond "logical pixels at the window's DPI scale".
//!
//! # The world-space decision, made here
//!
//! §8 asks MORROWIND-E to record the choice between two implementations, so it
//! is recorded rather than left to whoever gets there first.
//!
//! **Decision: render-to-texture, then draw the texture as world geometry.**
//! Not direct 3D submission of UI primitives.
//!
//! | | Render-to-texture | Direct 3D submission |
//! |---|---|---|
//! | Compositing with the visibility buffer | A textured quad. Nothing new. | Needs depth, ordering and a second projection path in both UI shaders. |
//! | Text crispness | Resampled once. Mitigated by sizing the target to its on-screen size. | Crisp at any angle. |
//! | Cost | One target per canvas. | None extra. |
//! | Blast radius | `UiPass` gains an offscreen target. The shaders are untouched. | **`ui_pass.wgsl` is the frozen Hades paint contract.** |
//!
//! The last row decides it. Direct submission means teaching the frozen quad
//! shader about a view-projection matrix and a depth test for a feature only
//! world-space canvases use, and Phase 27's contract is the thing this whole
//! track is built not to disturb. Resampled text on a floating name-plate is a
//! real cost and a small one; re-opening the paint contract is neither.
//!
//! The mechanism is the one MORROWIND-D deferred: an offscreen target is a
//! **registered texture** like any other
//! ([`crate::draw::DrawingContext::register_texture`]), and the world-space
//! quad samples it through the same bindless array a game sprite uses. That is
//! why D deferred `begin_layer` rather than half-building it — this decision
//! had not been made, and the two are one mechanism.
//!
//! Revisit if a game wants a wall-mounted terminal readable from across a room,
//! where the resampling shows. The fallback is per-canvas: raise that canvas's
//! target resolution before changing the architecture.

use crate::{runtime::anchor::Anchoring, types::Rect};
use glam::{Mat4, Vec2, Vec3};

/// How a canvas turns its logical coordinate space into pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CanvasScaler {
    /// One logical unit is one *logical* pixel, whatever the resolution.
    ///
    /// The editor's policy, and the right default: a 16 px label is 16 px at
    /// 1080p and at 4K, and DPI scaling is handled by the window's scale factor
    /// rather than by the canvas.
    ConstantPixel,
    /// The canvas is laid out at `reference` and scaled to fit the viewport.
    ///
    /// `match_width_or_height` blends between matching the width (`0.0`) and
    /// the height (`1.0`). The blend is **logarithmic**, which matters: a
    /// linear blend of two scale factors is not the geometric middle of them,
    /// and a 0.5 setting on an ultrawide monitor visibly favours one axis.
    ScaleWithResolution {
        reference: Vec2,
        match_width_or_height: f32,
    },
    /// One logical unit is a fixed physical size, so a button is the same
    /// number of millimetres on every display.
    ///
    /// The touch-target case. `dpi` is the display's real dots per inch and
    /// `reference_dpi` the density the UI was authored against.
    ConstantPhysicalSize { dpi: f32, reference_dpi: f32 },
}

impl Default for CanvasScaler {
    fn default() -> Self {
        Self::ConstantPixel
    }
}

impl CanvasScaler {
    /// The factor from logical units to logical pixels for `viewport`.
    #[must_use]
    pub fn factor(&self, viewport: Vec2) -> f32 {
        match *self {
            Self::ConstantPixel => 1.0,
            Self::ScaleWithResolution {
                reference,
                match_width_or_height,
            } => {
                let reference = reference.max(Vec2::splat(1.0));
                let by_width = viewport.x / reference.x;
                let by_height = viewport.y / reference.y;
                if by_width <= 0.0 || by_height <= 0.0 {
                    return 1.0;
                }
                // Logarithmic blend: the geometric mean at 0.5, which is what
                // "halfway between these two scale factors" actually means.
                let t = match_width_or_height.clamp(0.0, 1.0);
                (by_width.ln() * (1.0 - t) + by_height.ln() * t).exp()
            }
            Self::ConstantPhysicalSize { dpi, reference_dpi } => {
                if reference_dpi <= 0.0 {
                    return 1.0;
                }
                (dpi / reference_dpi).max(0.01)
            }
        }
    }

    /// The logical size a canvas lays out at, given the viewport.
    #[must_use]
    pub fn logical_size(&self, viewport: Vec2) -> Vec2 {
        let factor = self.factor(viewport);
        if factor <= 0.0 {
            return viewport;
        }
        (viewport / factor).max(Vec2::ONE)
    }
}

/// Insets that keep content clear of notches, rounded corners and system bars.
///
/// Nothing in the tree models this, and every shipped game needs it — the plan
/// says so in as many words. It is not only a phone concern: a TV's overscan
/// and a Steam Deck's rounded display are the same problem, and the failure
/// mode is a health bar that is fine on the developer's monitor and clipped on
/// the hardware somebody plays on.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SafeArea {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl SafeArea {
    /// No insets.
    pub const NONE: Self = Self {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    /// Uniform insets on every side.
    #[must_use]
    pub fn uniform(inset: f32) -> Self {
        Self {
            left: inset,
            top: inset,
            right: inset,
            bottom: inset,
        }
    }

    /// Whether anything is inset.
    #[must_use]
    pub fn is_none(&self) -> bool {
        *self == Self::NONE
    }

    /// Apply the insets to a rect, clamping rather than inverting.
    #[must_use]
    pub fn apply(&self, rect: Rect) -> Rect {
        let w = (rect.w - self.left - self.right).max(0.0);
        let h = (rect.h - self.top - self.bottom).max(0.0);
        Rect::new(rect.x + self.left, rect.y + self.top, w, h)
    }
}

/// A sorting layer. Higher draws later, so a tooltip sits above a panel sits
/// above a HUD without anyone computing a z by hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Layer(pub i32);

impl Layer {
    /// Game content: a HUD, a health bar, a minimap.
    pub const HUD: Self = Self(0);
    /// Menus and dialogs drawn over the HUD.
    pub const MENU: Self = Self(100);
    /// Tooltips and drag ghosts, above everything a menu contains.
    pub const OVERLAY: Self = Self(200);
    /// Debug overlays, above even those, because their whole job is to be
    /// visible over whatever is wrong.
    pub const DEBUG: Self = Self(1000);
}

/// Where a canvas lives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CanvasMode {
    /// Flat over the whole viewport. A HUD, a menu, the editor shell.
    Screen { scaler: CanvasScaler },
    /// A quad in the world: a name-plate, a wall terminal, a quest marker.
    ///
    /// `size` is in **world units** and `transform` places it. `billboard`
    /// turns it to face the camera every frame, which is what a name-plate
    /// wants and a wall terminal does not.
    World {
        transform: Mat4,
        size: Vec2,
        billboard: bool,
    },
    /// Flat over the viewport, but positioned by projecting a world point.
    ///
    /// The difference from `World` is that this stays screen-sized: a marker
    /// over a distant objective does not shrink to nothing. `world_anchor` is
    /// the point that is projected.
    Overlay { world_anchor: Vec3 },
}

impl Default for CanvasMode {
    fn default() -> Self {
        Self::Screen {
            scaler: CanvasScaler::ConstantPixel,
        }
    }
}

/// A UI tree's root.
///
/// Widgets below it are unaware of which mode it is: they lay out in the
/// canvas's logical space and the canvas decides what that space *is*.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Canvas {
    pub mode: CanvasMode,
    pub layer: Layer,
    pub safe_area: SafeArea,
    /// Whether the canvas is drawn at all. A pause menu exists between uses.
    pub visible: bool,
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            mode: CanvasMode::default(),
            layer: Layer::HUD,
            safe_area: SafeArea::NONE,
            visible: true,
        }
    }
}

/// What a canvas resolved to for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasLayout {
    /// The logical space widgets lay out in.
    pub logical_size: Vec2,
    /// The rect inside `logical_size` that is safe to place content in.
    ///
    /// Anchored widgets resolve against **this**, not against the full size,
    /// which is what makes a bottom-anchored health bar clear of a home
    /// indicator without every widget knowing about notches.
    pub safe_rect: Rect,
    /// Logical units to logical pixels.
    pub scale: f32,
}

impl Canvas {
    /// A screen canvas at one logical pixel per unit.
    #[must_use]
    pub fn screen() -> Self {
        Self::default()
    }

    /// A screen canvas laid out at `reference` and scaled to fit.
    #[must_use]
    pub fn scaled(reference: Vec2, match_width_or_height: f32) -> Self {
        Self {
            mode: CanvasMode::Screen {
                scaler: CanvasScaler::ScaleWithResolution {
                    reference,
                    match_width_or_height,
                },
            },
            ..Default::default()
        }
    }

    /// A billboarded world-space canvas of `size` world units at `transform`.
    #[must_use]
    pub fn world(transform: Mat4, size: Vec2) -> Self {
        Self {
            mode: CanvasMode::World {
                transform,
                size,
                billboard: true,
            },
            ..Default::default()
        }
    }

    /// Put this canvas on `layer`.
    #[must_use]
    pub fn on_layer(mut self, layer: Layer) -> Self {
        self.layer = layer;
        self
    }

    /// Keep content clear of `safe_area`.
    #[must_use]
    pub fn with_safe_area(mut self, safe_area: SafeArea) -> Self {
        self.safe_area = safe_area;
        self
    }

    /// Resolve the canvas against a viewport, in logical pixels.
    ///
    /// A world canvas does not consult the viewport at all: its logical space
    /// is its own, in world units scaled to a pixel density, and it renders to
    /// an offscreen target of that size. That independence is the reason a
    /// name-plate does not reflow when the window is resized.
    #[must_use]
    pub fn layout(&self, viewport: Vec2, world_pixels_per_unit: f32) -> CanvasLayout {
        match self.mode {
            CanvasMode::Screen { scaler } => {
                let logical_size = scaler.logical_size(viewport);
                let scale = scaler.factor(viewport);
                // The safe area arrives in *viewport* pixels from the platform,
                // so it is divided by the scale to land in the canvas's own
                // units. Skipping that division is a bug that only appears on a
                // scaled canvas, which is to say on somebody else's hardware.
                let safe = SafeArea {
                    left: self.safe_area.left / scale,
                    top: self.safe_area.top / scale,
                    right: self.safe_area.right / scale,
                    bottom: self.safe_area.bottom / scale,
                };
                CanvasLayout {
                    logical_size,
                    safe_rect: safe.apply(Rect::new(0.0, 0.0, logical_size.x, logical_size.y)),
                    scale,
                }
            }
            CanvasMode::World { size, .. } => {
                let logical_size = (size * world_pixels_per_unit.max(1.0)).max(Vec2::ONE);
                CanvasLayout {
                    logical_size,
                    // A world canvas has no notch to avoid; the safe area is
                    // still honoured because a caller may use it as padding.
                    safe_rect: self.safe_area.apply(Rect::new(
                        0.0,
                        0.0,
                        logical_size.x,
                        logical_size.y,
                    )),
                    scale: world_pixels_per_unit.max(1.0),
                }
            }
            CanvasMode::Overlay { .. } => {
                // Screen-sized, so a distant marker stays legible.
                let safe = self.safe_area;
                CanvasLayout {
                    logical_size: viewport.max(Vec2::ONE),
                    safe_rect: safe.apply(Rect::new(0.0, 0.0, viewport.x, viewport.y)),
                    scale: 1.0,
                }
            }
        }
    }

    /// Resolve a child's anchoring against this canvas's safe rect.
    #[must_use]
    pub fn place(&self, layout: &CanvasLayout, anchoring: &Anchoring) -> Rect {
        anchoring.resolve(layout.safe_rect)
    }

    /// Where an [`CanvasMode::Overlay`] canvas's anchor lands on screen.
    ///
    /// Returns `None` when the anchor is behind the camera. Callers **must**
    /// treat that as "do not draw": a point behind the camera projects to a
    /// mirrored position in front of it, so a marker for an objective behind
    /// the player appears ahead of them, pointing at nothing.
    #[must_use]
    pub fn project_overlay(&self, view_projection: Mat4, viewport: Vec2) -> Option<Vec2> {
        let CanvasMode::Overlay { world_anchor } = self.mode else {
            return None;
        };
        let clip = view_projection * world_anchor.extend(1.0);
        if clip.w <= 1e-6 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        if !(-1.0..=1.0).contains(&ndc.z) {
            return None;
        }
        Some(Vec2::new(
            (ndc.x * 0.5 + 0.5) * viewport.x,
            // NDC y is up, the UI's y is down.
            (0.5 - ndc.y * 0.5) * viewport.y,
        ))
    }

    /// The world transform a [`CanvasMode::World`] canvas's quad is drawn with.
    ///
    /// Billboarding replaces the stored rotation with one facing the camera,
    /// keeping the stored translation and scale — so a name-plate turns but
    /// does not drift or resize.
    #[must_use]
    pub fn world_transform(&self, camera_view: Mat4) -> Option<Mat4> {
        let CanvasMode::World {
            transform,
            billboard,
            ..
        } = self.mode
        else {
            return None;
        };
        if !billboard {
            return Some(transform);
        }
        let (scale, _, translation) = transform.to_scale_rotation_translation();
        // The inverse view's rotation is the camera's orientation; adopting it
        // makes the quad's plane parallel to the near plane, which is what
        // "faces the camera" means for a flat sprite and is stable when the
        // camera rolls.
        let camera_rotation = camera_view.inverse();
        let (_, rotation, _) = camera_rotation.to_scale_rotation_translation();
        Some(Mat4::from_scale_rotation_translation(
            scale,
            rotation,
            translation,
        ))
    }
}

/// Sort canvases into draw order.
///
/// Stable within a layer, so two HUD canvases keep the order they were
/// registered in and a frame does not shuffle when a `HashMap` iterates
/// differently.
pub fn sort_by_layer(canvases: &mut [(Layer, usize)]) {
    canvases.sort_by_key(|(layer, index)| (*layer, *index));
}

#[cfg(test)]
mod tests {
    use super::*;

    const HD: Vec2 = Vec2::new(1920.0, 1080.0);

    #[test]
    fn constant_pixel_lays_out_at_the_viewport_size() {
        let canvas = Canvas::screen();
        let layout = canvas.layout(HD, 1.0);
        assert_eq!(layout.logical_size, HD);
        assert_eq!(layout.scale, 1.0);
    }

    /// A reference-sized canvas keeps its layout at any resolution.
    #[test]
    fn scale_with_resolution_keeps_the_reference_layout() {
        let canvas = Canvas::scaled(Vec2::new(1920.0, 1080.0), 0.5);
        assert_eq!(canvas.layout(HD, 1.0).logical_size, HD);

        // 4K: the same layout, drawn twice as large.
        let uhd = canvas.layout(Vec2::new(3840.0, 2160.0), 1.0);
        assert!((uhd.scale - 2.0).abs() < 1e-4);
        assert!(uhd.logical_size.abs_diff_eq(HD, 0.5));
    }

    /// The blend is logarithmic, not linear.
    ///
    /// On an ultrawide the two axis factors differ a lot, and a linear blend at
    /// 0.5 is not the middle of them — it visibly favours the larger. The
    /// geometric mean is what "halfway" means for a ratio.
    #[test]
    fn the_match_blend_is_geometric() {
        let scaler = CanvasScaler::ScaleWithResolution {
            reference: Vec2::new(1000.0, 1000.0),
            match_width_or_height: 0.5,
        };
        // Width factor 4, height factor 1. Geometric mean 2; arithmetic 2.5.
        let factor = scaler.factor(Vec2::new(4000.0, 1000.0));
        assert!((factor - 2.0).abs() < 1e-4, "got {factor}");
    }

    #[test]
    fn matching_one_axis_ignores_the_other() {
        let by_width = CanvasScaler::ScaleWithResolution {
            reference: Vec2::new(1000.0, 1000.0),
            match_width_or_height: 0.0,
        };
        let by_height = CanvasScaler::ScaleWithResolution {
            reference: Vec2::new(1000.0, 1000.0),
            match_width_or_height: 1.0,
        };
        let viewport = Vec2::new(4000.0, 1000.0);
        assert!((by_width.factor(viewport) - 4.0).abs() < 1e-4);
        assert!((by_height.factor(viewport) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn constant_physical_size_follows_dpi() {
        let scaler = CanvasScaler::ConstantPhysicalSize {
            dpi: 192.0,
            reference_dpi: 96.0,
        };
        assert_eq!(scaler.factor(HD), 2.0);
    }

    /// A zero or absurd viewport must not produce a zero or NaN layout.
    #[test]
    fn a_degenerate_viewport_does_not_produce_a_degenerate_layout() {
        let canvas = Canvas::scaled(Vec2::new(1920.0, 1080.0), 0.5);
        let layout = canvas.layout(Vec2::ZERO, 1.0);
        assert!(layout.logical_size.x >= 1.0 && layout.logical_size.y >= 1.0);
        assert!(layout.logical_size.is_finite());

        let zero_reference = CanvasScaler::ScaleWithResolution {
            reference: Vec2::ZERO,
            match_width_or_height: 0.5,
        };
        assert!(zero_reference.factor(HD).is_finite());
    }

    /// The safe area arrives in viewport pixels and must land in canvas units.
    ///
    /// Skipping the division is a bug that only shows on a *scaled* canvas —
    /// which is to say, on somebody else's hardware.
    #[test]
    fn the_safe_area_is_converted_into_canvas_units() {
        let canvas = Canvas::scaled(Vec2::new(1920.0, 1080.0), 0.5).with_safe_area(SafeArea {
            top: 88.0,
            bottom: 68.0,
            ..SafeArea::NONE
        });
        // At 4K the scale is 2, so an 88 px notch is 44 canvas units.
        let layout = canvas.layout(Vec2::new(3840.0, 2160.0), 1.0);
        assert!(
            (layout.safe_rect.y - 44.0).abs() < 0.5,
            "{:?}",
            layout.safe_rect
        );
        assert!((layout.safe_rect.h - (1080.0 - 44.0 - 34.0)).abs() < 1.0);
    }

    /// Anchored widgets resolve against the safe rect, not the full canvas.
    #[test]
    fn a_bottom_anchored_widget_clears_the_safe_area() {
        use crate::runtime::anchor::{Anchoring, Anchors, Offsets};
        let canvas = Canvas::screen().with_safe_area(SafeArea {
            bottom: 40.0,
            ..SafeArea::NONE
        });
        let layout = canvas.layout(HD, 1.0);
        let bar = Anchoring {
            anchors: Anchors::BOTTOM_STRETCH,
            offsets: Offsets {
                left: 0.0,
                right: 0.0,
                top: -20.0,
                bottom: 20.0,
            },
            pivot: Vec2::ZERO,
        };
        let rect = canvas.place(&layout, &bar);
        assert_eq!(
            rect.y + rect.h,
            1040.0,
            "the bar sits above the home indicator, not under it"
        );
    }

    #[test]
    fn a_safe_area_larger_than_the_screen_collapses_rather_than_inverting() {
        let area = SafeArea::uniform(600.0);
        let r = area.apply(Rect::new(0.0, 0.0, 800.0, 600.0));
        assert_eq!((r.w, r.h), (0.0, 0.0));
    }

    /// A world canvas is sized in world units and ignores the viewport.
    ///
    /// That independence is why a name-plate does not reflow when the window is
    /// resized, which it would if it inherited the screen's logical size.
    #[test]
    fn a_world_canvas_does_not_depend_on_the_viewport() {
        let canvas = Canvas::world(Mat4::IDENTITY, Vec2::new(2.0, 0.5));
        let a = canvas.layout(HD, 100.0);
        let b = canvas.layout(Vec2::new(640.0, 480.0), 100.0);
        assert_eq!(a.logical_size, b.logical_size);
        assert_eq!(a.logical_size, Vec2::new(200.0, 50.0));
    }

    /// A point behind the camera does not project.
    ///
    /// It would project to a mirrored position *in front*, so a marker for an
    /// objective behind the player appears ahead of them, pointing at nothing.
    #[test]
    fn an_overlay_behind_the_camera_does_not_project() {
        let view = Mat4::look_at_rh(Vec3::ZERO, -Vec3::Z, Vec3::Y);
        let projection = Mat4::perspective_rh(1.0, 16.0 / 9.0, 0.1, 100.0);
        let vp = projection * view;

        let ahead = Canvas {
            mode: CanvasMode::Overlay {
                world_anchor: Vec3::new(0.0, 0.0, -10.0),
            },
            ..Default::default()
        };
        let behind = Canvas {
            mode: CanvasMode::Overlay {
                world_anchor: Vec3::new(0.0, 0.0, 10.0),
            },
            ..Default::default()
        };

        let hit = ahead.project_overlay(vp, HD).expect("in front projects");
        assert!((hit.x - 960.0).abs() < 1.0, "centred: {hit:?}");
        assert!(behind.project_overlay(vp, HD).is_none());
    }

    /// A point in front but past the far plane also does not project.
    #[test]
    fn an_overlay_past_the_far_plane_does_not_project() {
        let view = Mat4::look_at_rh(Vec3::ZERO, -Vec3::Z, Vec3::Y);
        let projection = Mat4::perspective_rh(1.0, 16.0 / 9.0, 0.1, 100.0);
        let canvas = Canvas {
            mode: CanvasMode::Overlay {
                world_anchor: Vec3::new(0.0, 0.0, -1000.0),
            },
            ..Default::default()
        };
        assert!(canvas.project_overlay(projection * view, HD).is_none());
    }

    /// Billboarding keeps the translation and the scale, changing only rotation.
    ///
    /// A billboard that also adopted the camera's translation would fly to the
    /// camera; one that adopted its scale would resize with the camera's own.
    #[test]
    fn billboarding_only_replaces_the_rotation() {
        let placed = Mat4::from_scale_rotation_translation(
            Vec3::new(2.0, 2.0, 1.0),
            glam::Quat::from_rotation_z(0.9),
            Vec3::new(5.0, 1.0, -3.0),
        );
        let canvas = Canvas::world(placed, Vec2::ONE);
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 8.0), Vec3::ZERO, Vec3::Y);
        let out = canvas.world_transform(view).expect("world canvas");

        let (scale, _, translation) = out.to_scale_rotation_translation();
        assert!(translation.abs_diff_eq(Vec3::new(5.0, 1.0, -3.0), 1e-4));
        assert!(scale.abs_diff_eq(Vec3::new(2.0, 2.0, 1.0), 1e-4));
    }

    #[test]
    fn a_non_billboard_world_canvas_keeps_its_transform() {
        let placed = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0));
        let canvas = Canvas {
            mode: CanvasMode::World {
                transform: placed,
                size: Vec2::ONE,
                billboard: false,
            },
            ..Default::default()
        };
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 8.0), Vec3::ZERO, Vec3::Y);
        assert_eq!(canvas.world_transform(view), Some(placed));
    }

    /// Layers sort, and ties keep registration order.
    #[test]
    fn canvases_sort_by_layer_then_registration() {
        let mut canvases = vec![
            (Layer::OVERLAY, 0),
            (Layer::HUD, 1),
            (Layer::HUD, 2),
            (Layer::MENU, 3),
        ];
        sort_by_layer(&mut canvases);
        assert_eq!(
            canvases,
            vec![
                (Layer::HUD, 1),
                (Layer::HUD, 2),
                (Layer::MENU, 3),
                (Layer::OVERLAY, 0)
            ]
        );
    }

    #[test]
    fn the_named_layers_are_in_the_order_they_read() {
        assert!(Layer::HUD < Layer::MENU);
        assert!(Layer::MENU < Layer::OVERLAY);
        assert!(Layer::OVERLAY < Layer::DEBUG);
    }
}
