//! An isometric camera rig: a three-quarter view looking down at a focus point, rotated in steps or dragged, zoomed
//! with the wheel, smoothed, and the pieces a mouse-and-keyboard game needs around it (movement relative to the
//! view, the cursor's ray onto the ground).
//!
//! The rig is a component on the camera entity (Play possesses that entity's world transform) and a small runtime
//! state the game owns. It does not read input itself: a game feeds it wheel lines, rotation steps and drags from its
//! own bindings, which is what lets every key be rebound. A perspective view with a narrow field of view rather
//! than an orthographic one keeps every lighting, ray-tracing and post path the renderer already has.

use glam::{Quat, Vec2, Vec3};
use somnium_ecs::component_schema;
use somnium_ecs::reflect::TypeRegistry;
use somnium_ecs::Component;

/// Register the component. Called by [`component_registry`].
///
/// [`component_registry`]: crate::reflect_registry::component_registry
pub(crate) fn register(registry: &mut TypeRegistry) {
    registry.register(component_schema! {
        IsometricCameraComponent as "somnium.IsometricCamera", display "Isometric Camera", version 1,
        fields {
            enabled { group: "Rig" },
            pitch { min: 10.0, max: 85.0, step: 0.5, precision: 1, unit: "\u{b0}", group: "Rig", doc: "Degrees below the horizon the camera looks." },
            yaw { min: -360.0, max: 360.0, step: 1.0, precision: 1, unit: "\u{b0}", group: "Rig", doc: "Starting heading about +Y; 0 looks toward -Z." },
            distance { min: 1.0, max: 2000.0, step: 0.5, precision: 1, unit: "m", group: "Zoom", doc: "Starting distance from the focus." },
            min_distance { min: 1.0, max: 2000.0, step: 0.5, precision: 1, unit: "m", group: "Zoom" },
            max_distance { min: 1.0, max: 5000.0, step: 1.0, precision: 1, unit: "m", group: "Zoom" },
            fov { min: 5.0, max: 90.0, step: 0.5, precision: 1, unit: "\u{b0}", group: "Rig", display_name: "Field of View", doc: "Vertical. Narrow keeps the look close to isometric." },
            zoom_step { min: 0.01, max: 0.5, step: 0.01, precision: 2, group: "Zoom", doc: "Fraction of the distance one wheel line zooms." },
            rotate_step { min: 1.0, max: 180.0, step: 1.0, precision: 0, unit: "\u{b0}", group: "Turning", doc: "Degrees one rotation step turns." },
            drag_rate { min: 0.01, max: 2.0, step: 0.01, precision: 2, unit: "\u{b0}/px", group: "Turning", doc: "Degrees a pixel of drag turns." },
            smoothing { min: 0.0, max: 2.0, step: 0.01, precision: 2, unit: "s", group: "Rig", doc: "Seconds to settle most of the way; 0 snaps." },
            focus_height { min: 0.0, max: 10.0, step: 0.1, precision: 1, unit: "m", group: "Rig", doc: "Metres above the followed point the camera looks at." },
        }
    });
}

/// The rig's authored settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IsometricCameraComponent {
    /// Drive the camera from this rig.
    pub enabled: bool,
    /// Degrees below the horizon the camera looks.
    pub pitch: f32,
    /// Starting heading, degrees about +Y (0 looks toward -Z).
    pub yaw: f32,
    /// Starting distance from the focus, metres.
    pub distance: f32,
    /// Closest and farthest zoom, metres.
    pub min_distance: f32,
    /// Farthest zoom, metres.
    pub max_distance: f32,
    /// Vertical field of view, degrees. Narrow keeps the look close to isometric.
    pub fov: f32,
    /// Fraction of the distance one wheel line zooms.
    pub zoom_step: f32,
    /// Degrees one rotation step turns.
    pub rotate_step: f32,
    /// Degrees a pixel of drag turns.
    pub drag_rate: f32,
    /// Seconds the view takes to settle most of the way to where it is told to go.
    pub smoothing: f32,
    /// Metres above the followed point the camera looks at (a character's chest, not its feet).
    pub focus_height: f32,
}

impl Component for IsometricCameraComponent {}

impl Default for IsometricCameraComponent {
    fn default() -> Self {
        Self {
            enabled: true,
            pitch: 50.0,
            yaw: 45.0,
            distance: 24.0,
            min_distance: 6.0,
            max_distance: 120.0,
            fov: 30.0,
            zoom_step: 0.12,
            rotate_step: 45.0,
            drag_rate: 0.25,
            smoothing: 0.12,
            focus_height: 1.2,
        }
    }
}

/// Where the camera stands and how it is turned, in the scene's frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraPose {
    /// World position, metres.
    pub position: Vec3,
    /// World rotation (looks down -Z).
    pub rotation: Quat,
}

/// The rig's state between frames: what it shows now and where it has been told to go.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IsometricRig {
    /// Heading now, degrees.
    pub yaw: f32,
    /// Heading it eases toward, degrees.
    pub yaw_goal: f32,
    /// Distance from the focus now, metres.
    pub distance: f32,
    /// Distance it eases toward, metres.
    pub distance_goal: f32,
    /// The point looked at now.
    pub focus: Vec3,
    /// The point it eases toward.
    pub focus_goal: Vec3,
}

impl IsometricRig {
    /// A rig at the component's distance and heading, looking at `focus`.
    #[must_use]
    pub fn new(settings: &IsometricCameraComponent, focus: Vec3) -> Self {
        let distance = settings.distance.clamp(settings.min_distance, settings.max_distance);
        Self { yaw: settings.yaw, yaw_goal: settings.yaw, distance, distance_goal: distance, focus, focus_goal: focus }
    }

    /// Wheel lines: positive zooms in. Multiplicative, so a line feels the same near and far.
    pub fn zoom(&mut self, lines: f32, settings: &IsometricCameraComponent) {
        let factor = (1.0 - settings.zoom_step).powf(lines);
        self.distance_goal = (self.distance_goal * factor).clamp(settings.min_distance, settings.max_distance);
    }

    /// Turn by whole steps: positive turns the view to the right (the world to the left).
    pub fn rotate_steps(&mut self, steps: i32, settings: &IsometricCameraComponent) {
        #[allow(clippy::cast_precision_loss)]
        let turn = steps as f32 * settings.rotate_step;
        self.yaw_goal += turn;
    }

    /// Turn by a drag of `dx` pixels.
    pub fn drag(&mut self, dx: f32, settings: &IsometricCameraComponent) {
        self.yaw_goal += dx * settings.drag_rate;
    }

    /// Where to look: a followed point (feet), raised by the rig's focus height.
    pub fn follow(&mut self, point: Vec3, settings: &IsometricCameraComponent) {
        self.focus_goal = point + Vec3::Y * settings.focus_height;
    }

    /// Jump straight to the goals (a scene load, a teleport), without easing.
    pub fn snap(&mut self) {
        self.yaw = self.yaw_goal;
        self.distance = self.distance_goal;
        self.focus = self.focus_goal;
    }

    /// Ease toward the goals and return the camera's pose.
    pub fn update(&mut self, dt: f32, settings: &IsometricCameraComponent) -> CameraPose {
        let k = if settings.smoothing <= 0.0 { 1.0 } else { 1.0 - (-dt.max(0.0) / settings.smoothing).exp() };
        // The shortest way round, so a step past 360 does not spin the long way.
        let dyaw = (self.yaw_goal - self.yaw + 540.0).rem_euclid(360.0) - 180.0;
        let goal = self.yaw + dyaw; // the goal restated within half a turn of the view
        self.yaw += dyaw * k;
        self.yaw_goal = goal;
        self.distance += (self.distance_goal - self.distance) * k;
        self.focus += (self.focus_goal - self.focus) * k;
        pose(self.focus, self.yaw, settings.pitch, self.distance)
    }

    /// Ground-plane directions for movement relative to the view: forward (up the screen) and right.
    #[must_use]
    pub fn move_basis(&self) -> (Vec3, Vec3) {
        let rotation = Quat::from_rotation_y(self.yaw.to_radians());
        (rotation * Vec3::NEG_Z, rotation * Vec3::X)
    }
}

/// The pose looking at `focus` from `distance` metres, turned `yaw` degrees about +Y and `pitch` degrees down.
#[must_use]
pub fn pose(focus: Vec3, yaw: f32, pitch: f32, distance: f32) -> CameraPose {
    let rotation = Quat::from_rotation_y(yaw.to_radians()) * Quat::from_rotation_x(-pitch.clamp(1.0, 89.0).to_radians());
    let forward = rotation * Vec3::NEG_Z;
    CameraPose { position: focus - forward * distance, rotation }
}

/// The ray under a point of the screen: `ndc` in -1..1 with +y up, a vertical field of view in degrees, and the
/// view's width over its height. Returns the origin and a unit direction.
#[must_use]
pub fn screen_ray(camera: CameraPose, fov: f32, aspect: f32, ndc: Vec2) -> (Vec3, Vec3) {
    let half = (fov.to_radians() * 0.5).tan();
    let local = Vec3::new(ndc.x * half * aspect, ndc.y * half, -1.0).normalize();
    (camera.position, (camera.rotation * local).normalize())
}

/// Where a ray meets the horizontal plane at `height`, if it does in front of its origin.
#[must_use]
pub fn ray_plane(origin: Vec3, direction: Vec3, height: f32) -> Option<Vec3> {
    if direction.y.abs() < 1e-6 {
        return None;
    }
    let t = (height - origin.y) / direction.y;
    (t > 0.0).then(|| origin + direction * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_camera_looks_down_at_its_focus_from_its_distance() {
        let settings = IsometricCameraComponent::default();
        let focus = Vec3::new(100.0, 545.0, 250.0);
        let mut rig = IsometricRig::new(&settings, focus);
        let p = rig.update(1.0, &settings);
        let forward = p.rotation * Vec3::NEG_Z;
        assert!((p.position.distance(focus) - settings.distance).abs() < 1e-3);
        assert!(((focus - p.position).normalize()).dot(forward) > 0.9999);
        let down = (-forward.y).asin().to_degrees();
        assert!((down - settings.pitch).abs() < 1e-3, "{down}");
    }

    #[test]
    fn zoom_is_clamped_and_multiplicative() {
        let settings = IsometricCameraComponent::default();
        let mut rig = IsometricRig::new(&settings, Vec3::ZERO);
        rig.zoom(1.0, &settings);
        assert!((rig.distance_goal - settings.distance * (1.0 - settings.zoom_step)).abs() < 1e-4);
        rig.zoom(500.0, &settings);
        assert!((rig.distance_goal - settings.min_distance).abs() < 1e-4);
        rig.zoom(-500.0, &settings);
        assert!((rig.distance_goal - settings.max_distance).abs() < 1e-4);
    }

    #[test]
    fn steps_turn_the_short_way_and_settle() {
        let settings = IsometricCameraComponent { yaw: 350.0, ..IsometricCameraComponent::default() };
        let mut rig = IsometricRig::new(&settings, Vec3::ZERO);
        rig.rotate_steps(1, &settings); // 350 + 45 = 395, i.e. 35: forty-five degrees the short way
        let first = rig.update(0.016, &settings);
        assert!(rig.yaw > 350.0 && rig.yaw < 396.0, "turning the short way: {}", rig.yaw);
        for _ in 0..200 {
            rig.update(0.016, &settings);
        }
        assert!(((rig.yaw - 395.0).abs()) < 0.01, "{}", rig.yaw);
        assert_ne!(first, rig.update(0.016, &settings));
    }

    #[test]
    fn movement_follows_the_view_on_the_ground() {
        let settings = IsometricCameraComponent { yaw: 90.0, ..IsometricCameraComponent::default() };
        let rig = IsometricRig::new(&settings, Vec3::ZERO);
        let (forward, right) = rig.move_basis();
        assert!(forward.y.abs() < 1e-6 && right.y.abs() < 1e-6);
        assert!(forward.dot(right).abs() < 1e-6);
        // The basis and the camera agree about which way is up the screen.
        let p = pose(Vec3::ZERO, rig.yaw, settings.pitch, 10.0);
        let look = p.rotation * Vec3::NEG_Z;
        assert!(Vec3::new(look.x, 0.0, look.z).normalize().dot(forward) > 0.9999);
    }

    #[test]
    fn the_screen_centre_lands_on_the_focus() {
        let settings = IsometricCameraComponent::default();
        let focus = Vec3::new(10.0, 2.0, -4.0);
        let p = pose(focus, 30.0, settings.pitch, 20.0);
        let (origin, dir) = screen_ray(p, settings.fov, 16.0 / 9.0, Vec2::ZERO);
        let hit = ray_plane(origin, dir, focus.y).unwrap();
        assert!(hit.distance(focus) < 1e-3, "{hit}");
        // Up the screen is further from the camera on the ground.
        let (o2, d2) = screen_ray(p, settings.fov, 16.0 / 9.0, Vec2::new(0.0, 0.5));
        let far = ray_plane(o2, d2, focus.y).unwrap();
        assert!(far.distance(p.position) > hit.distance(p.position));
        assert!(ray_plane(origin, Vec3::Y, 0.0).is_none());
    }
}
