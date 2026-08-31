//! One view of the scene, and the frame's list of them.
//!
//! MORROWIND-J step 3. Before this, "the view" was renderer state — a view
//! matrix, a projection and a camera position that whatever ran last had set —
//! and the frame drew exactly one of them, full-window. A four-up editor needs
//! four, each with its own camera and its own rectangle, and it needs them to
//! be *arguments* rather than state, because state read off `self` in a loop is
//! read from the previous iteration.

use glam::{Mat4, Vec3};

/// A rectangle of the swapchain, in physical pixels.
pub type ViewRect = (u32, u32, u32, u32);

/// Everything a single pass over the scene needs that is not the scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneView {
    /// Where the finished image lands, in physical pixels.
    ///
    /// `None` means the whole surface, and that is not merely the common case:
    /// it is the *pre-existing* case. A one-view frame writes straight to the
    /// swapchain with a clear, exactly as it did before this type existed, and
    /// pays nothing for the blit a tiled frame needs.
    pub rect: Option<ViewRect>,
    /// World-to-camera.
    pub view: Mat4,
    /// Camera projection.
    pub proj: Mat4,
    pub camera_pos: Vec3,
    /// The debug visualisation this view shows, or `None` for the lit image.
    ///
    /// Per view rather than per frame is the whole point of a second viewport
    /// in an editor: the interesting arrangement is the lit image beside the
    /// overdraw, not the same picture twice.
    pub debug_view: Option<u32>,
    /// Whether editor overlays — grid, gizmos, selection outline — draw here.
    pub overlays: bool,
}

impl SceneView {
    /// The whole surface, with the camera the renderer already has.
    #[must_use]
    pub fn full(view: Mat4, proj: Mat4, camera_pos: Vec3) -> Self {
        Self {
            rect: None,
            view,
            proj,
            camera_pos,
            debug_view: None,
            overlays: true,
        }
    }

    /// The same camera in a given rectangle.
    #[must_use]
    pub fn in_rect(self, rect: ViewRect) -> Self {
        Self {
            rect: Some(rect),
            ..self
        }
    }

    #[must_use]
    pub fn with_debug_view(self, debug_view: Option<u32>) -> Self {
        Self { debug_view, ..self }
    }

    #[must_use]
    pub fn with_overlays(self, overlays: bool) -> Self {
        Self { overlays, ..self }
    }
}

/// How far ahead to look when the camera's ray never meets the ground.
///
/// A focus point has to come from somewhere, and the editor camera does not
/// carry one — it is a free-fly camera, not an orbit camera. Looking *up* at
/// the sky is the case with no better answer than a fixed distance.
pub const FALLBACK_FOCUS_DISTANCE: f32 = 10.0;

/// The furthest the ground-plane intersection is believed.
///
/// A camera a degree above the horizon meets `y = 0` kilometres away, and an
/// orthographic view framed on that point shows a strip of haze. Past this the
/// fallback is the better answer.
const MAX_GROUND_FOCUS: f32 = 2_000.0;

impl SceneView {
    /// Rebuild a standard projection for a different aspect ratio.
    ///
    /// A tile is not the shape of the window, and a perspective matrix built
    /// for the window and used in a half-width tile squashes everything
    /// horizontally. `glam`'s `perspective_rh` puts `f / aspect` in `x_axis.x`
    /// and `f` in `y_axis.y`, so the correction is exact rather than a
    /// reconstruction — and for an orthographic matrix, whose `x_axis.x` is
    /// `2 / width`, it is the same operation with the same meaning.
    #[must_use]
    pub fn with_aspect(mut self, aspect: f32) -> Self {
        if aspect > 0.0 && self.proj.y_axis.y != 0.0 {
            // Sign is carried through: `perspective_rh` and `orthographic_rh`
            // differ in it, and dropping it flips the image left to right.
            let magnitude = self.proj.y_axis.y.abs() / aspect;
            self.proj.x_axis.x = magnitude.copysign(self.proj.x_axis.x);
        }
        self
    }

    /// Which way this view is looking, in world space.
    #[must_use]
    pub fn forward(&self) -> Vec3 {
        // Row 2 of a world-to-camera matrix is the camera's backward axis in
        // world space, so the forward direction is its negation.
        (-Vec3::new(self.view.x_axis.z, self.view.y_axis.z, self.view.z_axis.z)).normalize_or_zero()
    }

    /// What this view is looking *at*, and how far away it is.
    ///
    /// Where the camera's ray meets the ground plane, which for an editor
    /// camera above an outdoor scene is what the person flying it means by
    /// "here". The first version of this used a fixed ten metres ahead and it
    /// was wrong in the way that is easy to miss in a unit test and impossible
    /// to miss on screen: with the camera a hundred and fifty metres up, the
    /// top view framed a twenty-metre cube of empty air and rendered black.
    #[must_use]
    pub fn focus_and_distance(&self) -> (Vec3, f32) {
        let forward = self.forward();
        let ground = if forward.y < -1e-4 {
            -self.camera_pos.y / forward.y
        } else {
            -1.0
        };
        let distance = if (0.1..MAX_GROUND_FOCUS).contains(&ground) {
            ground
        } else {
            FALLBACK_FOCUS_DISTANCE
        };
        (self.camera_pos + forward * distance, distance)
    }

    /// What this view is looking at.
    #[must_use]
    pub fn focus(&self) -> Vec3 {
        self.focus_and_distance().0
    }

    /// Half the height this view frames at `distance`, in world units.
    ///
    /// Recovered from the projection rather than assumed, so the orthographic
    /// elevations frame **exactly what the perspective view frames** — which is
    /// the only framing that makes them useful for judging a placement.
    /// `perspective_rh` puts `1 / tan(fov_y / 2)` in `y_axis.y`.
    #[must_use]
    pub fn half_height_at(&self, distance: f32) -> f32 {
        let f = self.proj.y_axis.y.abs();
        if f > 1e-6 { distance / f } else { distance }
    }
}

/// The views a layout draws, given the primary camera and the region to fill.
///
/// The secondaries are the three orthographic elevations every DCC tool ships —
/// top, front, side — because the useful second viewport is a *different* way
/// of looking at the same thing, and a second perspective camera nobody has
/// aimed yet is a second picture of nothing.
#[must_use]
pub fn standard_views(tiles: &[ViewRect], primary: SceneView) -> Vec<SceneView> {
    let (focus, distance) = primary.focus_and_distance();
    let extent = primary.half_height_at(distance).max(0.5);
    // Stand back far enough that nothing between the subject and the camera is
    // clipped away, and keep the far plane behind it by the same margin.
    let standoff = (distance + extent * 2.0).max(1.0);
    // Right-handed, y up. Each looks at the focus down one axis, and the up
    // vector for the top view has to be −Z or the projection is degenerate.
    let elevations: [(Vec3, Vec3); 3] = [
        (Vec3::new(0.0, standoff, 0.0), Vec3::NEG_Z), // Top
        (Vec3::new(0.0, 0.0, standoff), Vec3::Y),     // Front
        (Vec3::new(standoff, 0.0, 0.0), Vec3::Y),     // Side
    ];
    tiles
        .iter()
        .copied()
        .enumerate()
        .map(|(index, tile)| {
            let aspect = tile.2 as f32 / tile.3.max(1) as f32;
            if index == 0 {
                return primary
                    .in_rect(tile)
                    .with_aspect(aspect)
                    .with_overlays(true);
            }
            let (offset, up) = elevations[(index - 1) % elevations.len()];
            let eye = focus + offset;
            let half_h = extent;
            let half_w = half_h * aspect.max(0.01);
            SceneView {
                rect: Some(tile),
                view: Mat4::look_at_rh(eye, focus, up),
                proj: Mat4::orthographic_rh(
                    -half_w,
                    half_w,
                    -half_h,
                    half_h,
                    0.1,
                    standoff * 2.0 + extent * 4.0,
                ),
                camera_pos: eye,
                debug_view: None,
                // Gizmos and the selection outline are drawn once, over the
                // whole surface, from the primary camera. Claiming them here
                // would put the primary view's gizmo in the top view's tile.
                overlays: false,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_view_asks_for_no_rectangle_at_all() {
        // Not a detail: `None` is what keeps a one-viewport frame on the path
        // it was on before this module existed — a clear and a direct write,
        // with no blit.
        let view = SceneView::full(Mat4::IDENTITY, Mat4::IDENTITY, Vec3::ZERO);
        assert!(view.rect.is_none());
    }
    #[test]
    fn a_single_layout_asks_for_the_primary_camera_unchanged() {
        // The floor under the whole feature: a one-viewport editor must record
        // exactly the frame it recorded before views existed.
        let primary = SceneView::full(
            Mat4::look_at_rh(Vec3::new(0.0, 2.0, 8.0), Vec3::ZERO, Vec3::Y),
            Mat4::perspective_rh(1.0, 16.0 / 9.0, 0.1, 100.0),
            Vec3::new(0.0, 2.0, 8.0),
        );
        let views = standard_views(&[(0, 0, 1600, 900)], primary);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].view, primary.view);
        assert_eq!(views[0].camera_pos, primary.camera_pos);
        assert!(views[0].overlays);
    }

    #[test]
    fn a_tile_narrower_than_the_window_does_not_squash_the_picture() {
        // A perspective matrix built for a 16:9 window and used in a
        // half-width tile squashes everything horizontally, and it looks like a
        // modelling error rather than a projection one.
        let proj = Mat4::perspective_rh(1.0, 16.0 / 9.0, 0.1, 100.0);
        let primary = SceneView::full(Mat4::IDENTITY, proj, Vec3::ZERO);
        let views = standard_views(&[(0, 0, 800, 900), (800, 0, 800, 900)], primary);
        let tile = views[0].rect.expect("tiled");
        let aspect = tile.2 as f32 / tile.3 as f32;
        // x_axis.x is f / aspect for `perspective_rh`, so recovering the aspect
        // from the matrix is exact.
        let recovered = views[0].proj.y_axis.y / views[0].proj.x_axis.x;
        assert!(
            (recovered - aspect).abs() < 1e-4,
            "tile {tile:?} has aspect {aspect}, projection says {recovered}"
        );
    }

    #[test]
    fn the_elevations_look_at_what_the_primary_camera_is_looking_at() {
        // A second viewport aimed somewhere else is a second picture of
        // nothing. All three orthographic views orbit the point ahead of the
        // primary camera, and each looks down a different axis.
        let eye = Vec3::new(5.0, 3.0, 5.0);
        let primary = SceneView::full(
            Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y),
            Mat4::perspective_rh(1.0, 1.0, 0.1, 100.0),
            eye,
        );
        let focus = primary.focus();
        let views = standard_views(
            &[
                (0, 0, 800, 450),
                (800, 0, 800, 450),
                (0, 450, 800, 450),
                (800, 450, 800, 450),
            ],
            primary,
        );
        assert_eq!(views.len(), 4);

        let mut axes = Vec::new();
        for view in &views[1..] {
            let to_focus = (focus - view.camera_pos).normalize();
            axes.push(to_focus);
            assert!(!view.overlays, "only the primary view draws gizmos");
            // Orthographic: the projection has no perspective divide.
            assert_eq!(view.proj.w_axis.w, 1.0);
        }
        // Three different directions, one per axis.
        for (i, a) in axes.iter().enumerate() {
            for b in &axes[i + 1..] {
                assert!(a.dot(*b).abs() < 0.01, "{a:?} and {b:?} are not orthogonal");
            }
        }
    }

    #[test]
    fn the_focus_is_ahead_of_the_camera_and_not_behind_it() {
        // The sign of the forward axis, which is the kind of thing that is
        // invisible until the top view shows the wall behind you.
        let eye = Vec3::new(0.0, 0.0, 10.0);
        let primary = SceneView::full(
            Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y),
            Mat4::perspective_rh(1.0, 1.0, 0.1, 100.0),
            eye,
        );
        let focus = primary.focus();
        assert!(
            focus.z < eye.z,
            "looking at the origin from +Z, the focus must be nearer the origin: {focus:?}"
        );
    }

    #[test]
    fn the_focus_is_where_the_camera_meets_the_ground() {
        // The bug this replaced, stated. A camera 150 m up looking down at the
        // coast has to focus on the coast; ten metres ahead of it is empty air,
        // and the top view of empty air renders black.
        let eye = Vec3::new(0.0, 150.0, 0.0);
        let target = Vec3::new(0.0, 0.0, -300.0);
        let primary = SceneView::full(
            Mat4::look_at_rh(eye, target, Vec3::Y),
            Mat4::perspective_rh(45.0f32.to_radians(), 1.6, 0.1, 1000.0),
            eye,
        );
        let (focus, distance) = primary.focus_and_distance();
        assert!(
            focus.y.abs() < 0.5,
            "the focus should be on the ground: {focus:?}"
        );
        assert!((focus.z - target.z).abs() < 1.0, "{focus:?}");
        assert!(
            distance > 300.0,
            "and hundreds of metres away, not ten: {distance}"
        );

        // And the elevations frame what the perspective view frames.
        let extent = primary.half_height_at(distance);
        assert!(
            extent > 100.0,
            "a 45-degree view 335 m from its subject frames ~139 m: {extent}"
        );
    }

    #[test]
    fn a_camera_pointed_at_the_sky_falls_back_rather_than_diverging() {
        // `-y / forward.y` with a positive `forward.y` is a point *behind* the
        // camera, and with a near-zero one it is thousands of kilometres away.
        for target in [
            Vec3::new(0.0, 500.0, -10.0),
            Vec3::new(0.0, 10.0, -10_000.0),
        ] {
            let eye = Vec3::new(0.0, 10.0, 0.0);
            let primary = SceneView::full(
                Mat4::look_at_rh(eye, target, Vec3::Y),
                Mat4::perspective_rh(1.0, 1.0, 0.1, 100.0),
                eye,
            );
            let (focus, distance) = primary.focus_and_distance();
            assert!(distance <= MAX_GROUND_FOCUS, "{target:?} gave {distance}");
            assert!(focus.is_finite(), "{focus:?}");
        }
    }
}
