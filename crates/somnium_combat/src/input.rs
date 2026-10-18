//! Mouse-and-keyboard combat, through `somnium_input` (so every binding is
//! data and rebindable).
//!
//! | Action | Default | Does |
//! |---|---|---|
//! | `CombatMove` | W A S D | move, relative to the camera's heading |
//! | `CombatSprint` | hold Left Shift | run (after a fifth of a second) |
//! | `Attack` | left mouse | swing; keep pressing to chain; hold for the heavy; draw and loose a bow |
//! | `Guard` | right mouse | raise the guard: the first instant parries, holding blocks |
//! | `Dodge` | Space, or tap Left Shift | dodge (invulnerable for most of it) |
//! | `Spell1`..`Spell10` | 1 to 0 | tap to cast the slot; hold to shape it with the mouse, release to cast |
//! | `MeasureWheel` | mouse wheel | while shaping: grow or shrink the measure |
//! | `MeasureNext` | R | while shaping: which measure the wheel edits (size, range, speed, count, duration) |
//! | `Art` | F | the school's art (the Charge, Lantern-out, Wind Blade) or a throw |
//! | `Kindle` | C | the Kindled Body on or off |
//! | `LockOn` | Q | lock onto the enemy nearest the cursor, or let go |
//! | `Focus` | Tab | slow time while focus lasts |
//! | `Mount` | E | climb into the nearest saddle, or out of it |
//!
//! Action names are distinct from the default gameplay map's (`Move`,
//! `Sprint`, ...) because `somnium_input` keeps one state per action name across
//! every enabled map. The camera is not here: the host supplies the cursor's
//! ray and the camera's heading.
use crate::world::{Command, Press};
use glam::{Mat4, Vec2, Vec3, Vec4};
use somnium_input::{Action, ActionMap, Binding, ControlPath, InputSystem, Interaction, Processor};

/// The map's name.
pub const MAP: &str = "combat";
/// Hotbar action names, keys 1 to 0.
pub const SPELL_ACTIONS: [&str; 10] = [
    "Spell1", "Spell2", "Spell3", "Spell4", "Spell5", "Spell6", "Spell7", "Spell8", "Spell9",
    "Spell10",
];
/// Seconds Left Shift must be held before it sprints (a shorter press is a dodge).
pub const SPRINT_HOLD: f32 = 0.2;

/// The default combat map.
#[must_use]
pub fn combat_map() -> ActionMap {
    let mut map = ActionMap::new(MAP)
        .action(Action::vector2("CombatMove").bind(Binding::Vector2 {
            up: ControlPath::keyboard("w"),
            down: ControlPath::keyboard("s"),
            left: ControlPath::keyboard("a"),
            right: ControlPath::keyboard("d"),
            processors: vec![Processor::Normalize],
        }))
        .action(
            Action::digital("CombatSprint")
                .bind(Binding::single(ControlPath::keyboard("shiftleft"))),
        )
        .action(Action::digital("Attack").bind(Binding::single(ControlPath::mouse("leftbutton"))))
        .action(Action::digital("Guard").bind(Binding::single(ControlPath::mouse("rightbutton"))))
        .action(
            Action::digital("Dodge")
                .bind(Binding::single(ControlPath::keyboard("space")))
                .bind(
                    Binding::single(ControlPath::keyboard("shiftleft")).with_interaction(
                        Interaction::Tap {
                            seconds: SPRINT_HOLD,
                        },
                    ),
                ),
        )
        .action(Action::axis("MeasureWheel").bind(Binding::single(ControlPath::mouse("scroll"))))
        .action(Action::digital("MeasureNext").bind(Binding::single(ControlPath::keyboard("r"))))
        .action(Action::digital("Art").bind(Binding::single(ControlPath::keyboard("f"))))
        .action(Action::digital("Kindle").bind(Binding::single(ControlPath::keyboard("c"))))
        .action(Action::digital("LockOn").bind(Binding::single(ControlPath::keyboard("q"))))
        .action(Action::digital("Focus").bind(Binding::single(ControlPath::keyboard("tab"))))
        .action(Action::digital("Mount").bind(Binding::single(ControlPath::keyboard("e"))));
    for (i, name) in SPELL_ACTIONS.iter().enumerate() {
        let key = if i == 9 {
            "0".to_string()
        } else {
            (i + 1).to_string()
        };
        map = map.action(Action::digital(*name).bind(Binding::single(ControlPath::keyboard(&key))));
    }
    map
}

/// The ray under a cursor: origin and unit direction, from the camera's
/// view and projection (depth 0 to 1, as `glam::Mat4::perspective_rh` makes).
#[must_use]
pub fn cursor_ray(
    cursor: Vec2,
    viewport: Vec2,
    view: Mat4,
    projection: Mat4,
) -> Option<(Vec3, Vec3)> {
    if viewport.x <= 0.0 || viewport.y <= 0.0 || !cursor.is_finite() {
        return None;
    }
    let ndc = Vec2::new(
        cursor.x / viewport.x * 2.0 - 1.0,
        1.0 - cursor.y / viewport.y * 2.0,
    );
    let inverse = (projection * view).inverse();
    if !inverse.is_finite() {
        return None;
    }
    let unproject = |z: f32| {
        let p = inverse * Vec4::new(ndc.x, ndc.y, z, 1.0);
        (p.w.abs() > 1e-8).then(|| p.truncate() / p.w)
    };
    let near = unproject(0.0)?;
    let far = unproject(1.0)?;
    Some((near, (far - near).try_normalize()?))
}

/// Where a ray meets the horizontal plane at `height`.
#[must_use]
pub fn ray_plane(origin: Vec3, dir: Vec3, height: f32) -> Option<Vec3> {
    if dir.y.abs() < 1e-6 {
        return None;
    }
    let t = (height - origin.y) / dir.y;
    (t >= 0.0).then(|| origin + dir * t)
}

/// Where a ray meets terrain given by `ground(x, z)`: marched in half-metre
/// steps, then bisected. `None` if it never comes down within `max` metres.
#[must_use]
pub fn ray_ground(
    origin: Vec3,
    dir: Vec3,
    max: f32,
    ground: impl Fn(f32, f32) -> Option<f32>,
) -> Option<Vec3> {
    let below = |p: Vec3| ground(p.x, p.z).is_some_and(|h| p.y <= h);
    let step = 0.5;
    let mut t0 = 0.0;
    let mut t = step;
    while t <= max {
        if below(origin + dir * t) {
            let (mut lo, mut hi) = (t0, t);
            for _ in 0..12 {
                let mid = (lo + hi) * 0.5;
                if below(origin + dir * mid) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            return Some(origin + dir * hi);
        }
        t0 = t;
        t += step;
    }
    None
}

/// What the host tells the input reader about the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AimView {
    /// The point on the ground under the cursor, if any.
    pub aim: Option<Vec3>,
    /// The camera's heading on the ground plane (W walks this way).
    pub camera_forward: Vec3,
}

/// Turns the combat map's state into [`Command`]s.
#[derive(Clone, Debug, Default)]
pub struct CombatInput {
    shift_held: f32,
}

fn press(input: &InputSystem, name: &str) -> Press {
    let a = input.actions();
    Press {
        pressed: a.just_activated(name),
        held: a.is_active(name),
        released: a.just_deactivated(name),
    }
}

impl CombatInput {
    /// A reader.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// This frame's command. Call after `InputSystem::update`.
    pub fn command(&mut self, input: &InputSystem, dt: f32, view: AimView) -> Command {
        let forward = crate::geometry::planar_dir(view.camera_forward, Vec3::Z);
        let right = forward.cross(Vec3::Y);
        let m = input.vec2("CombatMove");
        // Screen space is y-down: W reads as negative y.
        let move_dir = (right * m.x + forward * -m.y).clamp_length_max(1.0);
        if input.is_active("CombatSprint") {
            self.shift_held += dt;
        } else {
            self.shift_held = 0.0;
        }
        let mut spells = [Press::default(); 10];
        for (slot, name) in spells.iter_mut().zip(SPELL_ACTIONS) {
            *slot = press(input, name);
        }
        Command {
            move_dir,
            sprint: self.shift_held >= SPRINT_HOLD,
            aim: view.aim,
            attack: press(input, "Attack"),
            guard: press(input, "Guard"),
            // A tap performs on release, when the key's value is already
            // zero: it shows in the phase, not the value.
            dodge: input.just_activated("Dodge")
                || (input.actions().phase("Dodge").performed() && !input.is_active("Dodge")),
            art: input.just_activated("Art"),
            kindle: input.just_activated("Kindle"),
            lock: input.just_activated("LockOn"),
            focus: input.just_activated("Focus"),
            mount: input.just_activated("Mount"),
            spells,
            measure_steps: input.axis("MeasureWheel"),
            measure_cycle: input.just_activated("MeasureNext"),
            cast: None,
            channel: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system() -> InputSystem {
        let mut input = InputSystem::new();
        input.add_map(combat_map());
        input
    }

    fn view() -> AimView {
        AimView {
            aim: Some(Vec3::new(3.0, 0.0, 4.0)),
            camera_forward: Vec3::new(0.0, -1.0, -1.0),
        }
    }

    #[test]
    fn wasd_moves_relative_to_the_camera() {
        let mut input = system();
        let mut reader = CombatInput::new();
        input.devices_mut().set_key("w", true);
        input.update(0.016);
        let cmd = reader.command(&input, 0.016, view());
        assert!(
            cmd.move_dir.distance(Vec3::new(0.0, 0.0, -1.0)) < 1e-4,
            "{}",
            cmd.move_dir
        );
        input.devices_mut().set_key("w", false);
        input.devices_mut().set_key("d", true);
        input.update(0.016);
        let cmd = reader.command(&input, 0.016, view());
        assert!(
            cmd.move_dir.distance(Vec3::new(1.0, 0.0, 0.0)) < 1e-4,
            "{}",
            cmd.move_dir
        );
    }

    #[test]
    fn clicks_and_keys_become_presses_and_edges() {
        let mut input = system();
        let mut reader = CombatInput::new();
        input.devices_mut().set_mouse_button("leftbutton", true);
        input.devices_mut().set_key("3", true);
        input.update(0.016);
        let cmd = reader.command(&input, 0.016, view());
        assert!(cmd.attack.pressed && cmd.attack.held);
        assert!(cmd.spells[2].pressed);
        assert_eq!(cmd.aim, Some(Vec3::new(3.0, 0.0, 4.0)));
        input.devices_mut().set_mouse_button("leftbutton", false);
        input.update(0.016);
        let cmd = reader.command(&input, 0.016, view());
        assert!(cmd.attack.released && !cmd.attack.held);
        assert!(cmd.spells[2].held && !cmd.spells[2].pressed);
    }

    #[test]
    fn tapping_shift_dodges_and_holding_it_sprints() {
        let mut input = system();
        let mut reader = CombatInput::new();
        input.devices_mut().set_key("shiftleft", true);
        input.update(0.05);
        let cmd = reader.command(&input, 0.05, view());
        assert!(!cmd.dodge && !cmd.sprint);
        input.devices_mut().set_key("shiftleft", false);
        input.update(0.05);
        let cmd = reader.command(&input, 0.05, view());
        assert!(cmd.dodge, "a short press dodges on release");
        input.devices_mut().set_key("shiftleft", true);
        for _ in 0..6 {
            input.update(0.05);
            reader.command(&input, 0.05, view());
        }
        input.update(0.05);
        let cmd = reader.command(&input, 0.05, view());
        assert!(cmd.sprint, "a held shift sprints");
        input.devices_mut().set_key("shiftleft", false);
        input.update(0.05);
        let cmd = reader.command(&input, 0.05, view());
        assert!(!cmd.dodge, "releasing a long hold is not a dodge");
    }

    #[test]
    fn the_map_is_data_and_only_shift_is_shared_on_purpose() {
        let map = combat_map();
        let json = serde_json::to_string(&map).unwrap();
        let back: ActionMap = serde_json::from_str(&json).unwrap();
        assert_eq!(map, back);
        let maps = [map];
        for action in &maps[0].actions {
            for binding in &action.bindings {
                for path in binding.paths() {
                    let clashes: Vec<_> =
                        somnium_input::conflicts_for(&maps, MAP, &action.name, path)
                            .into_iter()
                            .map(|c| c.action)
                            .collect();
                    let expected: &[&str] = match (action.name.as_str(), path.control.as_str()) {
                        ("Dodge", "shiftleft") => &["CombatSprint"],
                        ("CombatSprint", "shiftleft") => &["Dodge"],
                        _ => &[],
                    };
                    assert_eq!(clashes, expected, "{} on {}", action.name, path.control);
                }
            }
        }
        // And none of its names collide with the default gameplay map's.
        let gameplay = somnium_input::default_gameplay_map();
        for action in &maps[0].actions {
            assert!(
                gameplay.find(&action.name).is_none(),
                "{} shadows gameplay",
                action.name
            );
        }
    }

    #[test]
    fn the_cursor_ray_finds_the_ground() {
        let eye = Vec3::new(0.0, 12.0, 12.0);
        let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
        let projection = Mat4::perspective_rh(45f32.to_radians(), 16.0 / 9.0, 0.1, 500.0);
        let (origin, dir) = cursor_ray(
            Vec2::new(960.0, 540.0),
            Vec2::new(1920.0, 1080.0),
            view,
            projection,
        )
        .unwrap();
        let hit = ray_plane(origin, dir, 0.0).unwrap();
        assert!(
            hit.length() < 1e-3,
            "the centre of the screen looks at the origin: {hit}"
        );
        let bumpy = ray_ground(origin, dir, 100.0, |x, _| Some(x * 0.1 + 0.5)).unwrap();
        assert!((bumpy.y - (bumpy.x * 0.1 + 0.5)).abs() < 0.01);
        assert!(
            ray_plane(origin, -dir, 0.0).is_none(),
            "the sky has no ground"
        );
    }
}
