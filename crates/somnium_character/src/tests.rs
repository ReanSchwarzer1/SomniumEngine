use glam::Vec3;

use crate::boxes::{Boxes, Solid};
use crate::{Controller, ControllerConfig, LocomotionEvent, MantleKind, Mode, MoveInput};

const DT: f32 = 1.0 / 60.0;

fn at(x: f32, y: f32, z: f32) -> Controller {
    Controller::new(ControllerConfig::default(), Vec3::new(x, y, z))
}

fn run(c: &mut Controller, t: &Boxes, input: MoveInput, seconds: f32) -> Vec<LocomotionEvent> {
    let mut events = Vec::new();
    for _ in 0..(seconds / DT).round() as usize {
        events.extend(c.step(DT, &input, t));
    }
    events
}

fn walk(z: f32) -> MoveInput {
    MoveInput {
        velocity: Vec3::new(0.0, 0.0, z),
        ..MoveInput::default()
    }
}

fn press_jump(c: &mut Controller, t: &Boxes, velocity: Vec3) -> Vec<LocomotionEvent> {
    c.step(
        DT,
        &MoveInput {
            velocity,
            jump: true,
            climb: false,
        },
        t,
    )
}

fn started(events: &[LocomotionEvent]) -> Option<MantleKind> {
    events.iter().find_map(|e| match e {
        LocomotionEvent::MantleStarted { kind, .. } => Some(*kind),
        _ => None,
    })
}

#[test]
fn settles_and_walks_at_the_wanted_speed() {
    let t = Boxes::floor(0.0);
    let mut c = at(0.0, 0.3, 0.0);
    let events = run(&mut c, &t, MoveInput::default(), 0.5);
    assert!(c.grounded(), "{:?}", c.mode);
    assert!(matches!(
        events.last(),
        Some(LocomotionEvent::Landed { .. })
    ));
    assert!(c.feet.y.abs() < 0.05, "{}", c.feet.y);
    run(&mut c, &t, walk(4.0), 2.0);
    assert!(c.grounded());
    assert!((7.6..8.01).contains(&c.feet.z), "{}", c.feet.z);
    assert!(c.feet.y.abs() < 0.05);
}

#[test]
fn slides_along_a_wall() {
    let t = Boxes::floor(0.0).block(Vec3::new(1.0, 0.0, -20.0), Vec3::new(2.0, 3.0, 20.0));
    let mut c = at(0.0, 0.02, 0.0);
    let input = MoveInput {
        velocity: Vec3::new(3.0, 0.0, 3.0),
        ..MoveInput::default()
    };
    run(&mut c, &t, input, 2.0);
    assert!(c.feet.x < 1.0 - 0.34 && c.feet.x > 0.5, "{}", c.feet.x);
    assert!(c.feet.z > 4.0, "slid along: {}", c.feet.z);
    assert!(c.grounded());
}

#[test]
fn steps_up_a_low_step_but_not_a_high_one() {
    let low = Boxes::floor(0.0).block(Vec3::new(-5.0, 0.0, 2.0), Vec3::new(5.0, 0.3, 20.0));
    let mut c = at(0.0, 0.02, 0.0);
    run(&mut c, &low, walk(3.0), 2.0);
    assert!(
        c.feet.z > 4.0 && (c.feet.y - 0.3).abs() < 0.05,
        "{:?}",
        c.feet
    );
    assert!(c.grounded());

    let high = Boxes::floor(0.0).block(Vec3::new(-5.0, 0.0, 2.0), Vec3::new(5.0, 0.8, 20.0));
    let mut c = at(0.0, 0.02, 0.0);
    let events = run(&mut c, &high, walk(3.0), 2.0);
    assert!(c.feet.z < 2.0 - 0.3 && c.feet.y < 0.05, "{:?}", c.feet);
    assert!(started(&events).is_none(), "walking alone never mantles");
}

#[test]
fn walks_down_stairs_without_leaving_the_ground() {
    let mut t = Boxes::floor(0.0);
    for i in 0..6 {
        let top = 1.5 - i as f32 * 0.25;
        t = t.block(
            Vec3::new(-3.0, 0.0, -10.0),
            Vec3::new(3.0, top, 1.0 + i as f32 * 0.4),
        );
    }
    let mut c = at(0.0, 1.52, 0.0);
    run(&mut c, &t, MoveInput::default(), 0.2);
    let mut airborne = 0;
    for _ in 0..120 {
        c.step(DT, &walk(2.0), &t);
        airborne += usize::from(!c.grounded());
    }
    assert!(c.feet.z > 3.5 && c.feet.y < 0.05, "{:?}", c.feet);
    assert_eq!(airborne, 0, "snapped down every 0.25 m step");
}

#[test]
fn vaults_a_crate_when_jumping_into_it() {
    let t = Boxes::floor(0.0).block(Vec3::new(-1.0, 0.0, 1.0), Vec3::new(1.0, 1.0, 2.5));
    let mut c = at(0.0, 0.02, 0.0);
    run(&mut c, &t, walk(2.0), 0.6);
    let events = press_jump(&mut c, &t, Vec3::Z * 2.0);
    assert_eq!(started(&events), Some(MantleKind::Vault));
    let events = run(&mut c, &t, MoveInput::default(), 1.0);
    assert!(events.contains(&LocomotionEvent::MantleEnded));
    assert!(c.grounded());
    assert!(
        (c.feet.y - 1.0).abs() < 0.05 && c.feet.z > 1.3,
        "on top: {:?}",
        c.feet
    );
}

#[test]
fn vaults_over_a_thin_fence_and_lands_beyond() {
    let t = Boxes::floor(0.0).block(Vec3::new(-3.0, 0.0, 1.0), Vec3::new(3.0, 1.0, 1.1));
    let mut c = at(0.0, 0.02, 0.0);
    run(&mut c, &t, walk(2.0), 0.6);
    let events = press_jump(&mut c, &t, Vec3::Z * 2.0);
    assert_eq!(started(&events), Some(MantleKind::Vault));
    let events = run(&mut c, &t, MoveInput::default(), 1.5);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, LocomotionEvent::Landed { .. }))
    );
    assert!(
        c.grounded() && c.feet.z > 1.1 + 0.3 && c.feet.y < 0.05,
        "{:?}",
        c.feet
    );
}

#[test]
fn climbs_a_high_ledge_but_not_a_wall_too_tall() {
    let ledge = Boxes::floor(0.0).block(Vec3::new(-3.0, 0.0, 1.0), Vec3::new(3.0, 2.0, 4.0));
    let mut c = at(0.0, 0.02, 0.0);
    let hold = MoveInput {
        velocity: Vec3::Z * 2.0,
        climb: true,
        ..MoveInput::default()
    };
    let events = run(&mut c, &ledge, hold, 0.6);
    assert_eq!(started(&events), Some(MantleKind::Climb));
    let edge = events.iter().find_map(|e| match e {
        LocomotionEvent::MantleStarted {
            edge, wall_normal, ..
        } => Some((*edge, *wall_normal)),
        _ => None,
    });
    let (edge, normal) = edge.expect("an edge for the hands");
    assert!(
        (edge.y - 2.0).abs() < 0.05 && (edge.z - 1.0).abs() < 0.1,
        "{edge}"
    );
    assert!(normal.abs_diff_eq(Vec3::NEG_Z, 1e-4));
    run(&mut c, &ledge, MoveInput::default(), 1.5);
    assert!(
        c.grounded() && (c.feet.y - 2.0).abs() < 0.05,
        "{:?}",
        c.feet
    );

    let wall = Boxes::floor(0.0).block(Vec3::new(-3.0, 0.0, 1.0), Vec3::new(3.0, 3.0, 4.0));
    let mut c = at(0.0, 0.02, 0.0);
    let events = run(&mut c, &wall, hold, 1.0);
    assert!(started(&events).is_none());
    assert!(c.feet.y < 0.05 && c.feet.z < 1.0, "{:?}", c.feet);
}

#[test]
fn no_climb_without_room_to_rise() {
    // A roof over the climber's head: the ledge is in reach, the space is not.
    let t = Boxes::floor(0.0)
        .block(Vec3::new(-3.0, 0.0, 1.0), Vec3::new(3.0, 1.9, 4.0))
        .block(Vec3::new(-3.0, 2.6, -3.0), Vec3::new(3.0, 3.0, 0.95));
    let mut c = at(0.0, 0.02, 0.0);
    let hold = MoveInput {
        velocity: Vec3::Z * 2.0,
        climb: true,
        ..MoveInput::default()
    };
    let events = run(&mut c, &t, hold, 1.0);
    assert!(started(&events).is_none(), "{events:?}");
    // Nor onto a top with no headroom to stand on.
    let t = Boxes::floor(0.0)
        .block(Vec3::new(-3.0, 0.0, 1.0), Vec3::new(3.0, 1.0, 4.0))
        .block(Vec3::new(-3.0, 2.0, 0.9), Vec3::new(3.0, 3.0, 4.0));
    let mut c = at(0.0, 0.02, 0.0);
    let events = run(&mut c, &t, hold, 1.0);
    assert!(started(&events).is_none(), "{events:?}");
}

#[test]
fn grabs_a_ledge_mid_jump() {
    // 2.9 m: out of reach from the ground, in reach at the top of a jump.
    let t = Boxes::floor(0.0).block(Vec3::new(-3.0, 0.0, 1.0), Vec3::new(3.0, 2.9, 4.0));
    let mut c = at(0.0, 0.02, 0.0);
    let hold = MoveInput {
        velocity: Vec3::Z * 2.0,
        climb: true,
        ..MoveInput::default()
    };
    let events = run(&mut c, &t, hold, 0.6);
    assert!(started(&events).is_none(), "too high from the ground");
    let mut events = press_jump(&mut c, &t, Vec3::Z * 2.0);
    events.extend(run(&mut c, &t, hold, 0.8));
    assert_eq!(started(&events), Some(MantleKind::Climb), "{events:?}");
    run(&mut c, &t, MoveInput::default(), 1.5);
    assert!(
        c.grounded() && (c.feet.y - 2.9).abs() < 0.05,
        "{:?}",
        c.feet
    );
}

#[test]
fn walks_up_a_gentle_ramp_but_not_a_steep_one() {
    let gentle = Boxes::floor(0.0).with(Solid::ramp(Vec3::new(0.0, 0.0, 2.0), 4.0, 8.0, 25.0));
    let mut c = at(0.0, 0.02, 0.0);
    let mut airborne = 0;
    for _ in 0..150 {
        c.step(DT, &walk(3.0), &gentle);
        airborne += usize::from(!c.grounded());
    }
    let expected = (c.feet.z - 2.0) * 25f32.to_radians().tan();
    assert!(
        c.feet.z > 5.0 && (c.feet.y - expected).abs() < 0.08,
        "{:?} vs {expected}",
        c.feet
    );
    assert_eq!(airborne, 0);

    let steep = Boxes::floor(0.0).with(Solid::ramp(Vec3::new(0.0, 0.0, 2.0), 4.0, 8.0, 60.0));
    let mut c = at(0.0, 0.02, 0.0);
    run(&mut c, &steep, walk(3.0), 2.5);
    assert!(c.feet.y < 0.45 && c.feet.z < 2.5, "{:?}", c.feet);
}

#[test]
fn slides_off_ground_too_steep_to_stand_on() {
    let t = Boxes::floor(0.0).with(Solid::ramp(Vec3::new(0.0, 0.0, 0.0), 4.0, 6.0, 60.0));
    let mut c = at(0.0, 3.0, 1.75);
    run(&mut c, &t, MoveInput::default(), 3.0);
    assert!(
        c.grounded() && c.feet.y < 0.05 && c.feet.z < 0.0,
        "{:?}",
        c.feet
    );
}

#[test]
fn jumps_to_its_height_and_lands() {
    let t = Boxes::floor(0.0);
    let mut c = at(0.0, 0.02, 0.0);
    run(&mut c, &t, MoveInput::default(), 0.1);
    let mut events = press_jump(&mut c, &t, Vec3::ZERO);
    assert!(events.contains(&LocomotionEvent::Jumped));
    let mut peak: f32 = 0.0;
    for _ in 0..90 {
        events.extend(c.step(DT, &MoveInput::default(), &t));
        peak = peak.max(c.feet.y);
    }
    let ideal = 4.6f32.powi(2) / (2.0 * 9.81);
    assert!((peak - ideal).abs() < 0.08, "{peak} vs {ideal}");
    let landed = events.iter().find_map(|e| match e {
        LocomotionEvent::Landed { speed } => Some(*speed),
        _ => None,
    });
    let speed = landed.expect("landed");
    assert!((speed - 4.6).abs() < 0.4, "{speed}");
    assert!(c.grounded());
}

#[test]
fn walks_off_a_ledge_and_lands_hard() {
    let t = Boxes::floor(0.0).block(Vec3::new(-3.0, 0.0, -3.0), Vec3::new(3.0, 3.0, 1.0));
    let mut c = at(0.0, 3.02, 0.0);
    let events = run(&mut c, &t, walk(2.0), 1.6);
    let speed = events
        .iter()
        .rev()
        .find_map(|e| match e {
            LocomotionEvent::Landed { speed } => Some(*speed),
            _ => None,
        })
        .expect("landed");
    // The capsule rolls off the edge (dropping up to its radius) before it falls.
    let (low, high) = ((2.0f32 * 9.81 * 2.6).sqrt(), (2.0f32 * 9.81 * 3.0).sqrt());
    assert!(
        speed > low - 0.2 && speed < high + 0.2,
        "{speed} vs {low}..{high}"
    );
    assert!(c.grounded() && c.feet.y < 0.05);
}

#[test]
fn a_low_ceiling_stops_the_jump_and_the_walk() {
    let t = Boxes::floor(0.0).block(Vec3::new(-5.0, 2.4, -5.0), Vec3::new(5.0, 3.0, 5.0));
    let mut c = at(0.0, 0.02, 0.0);
    run(&mut c, &t, MoveInput::default(), 0.1);
    press_jump(&mut c, &t, Vec3::ZERO);
    let mut peak: f32 = 0.0;
    for _ in 0..60 {
        c.step(DT, &MoveInput::default(), &t);
        peak = peak.max(c.feet.y);
    }
    assert!(peak <= 0.6 && peak > 0.5, "{peak}");
    assert!(c.grounded());

    let overhang = Boxes::floor(0.0).block(Vec3::new(-5.0, 1.5, 2.0), Vec3::new(5.0, 3.0, 6.0));
    let mut c = at(0.0, 0.02, 0.0);
    run(&mut c, &overhang, walk(3.0), 2.0);
    assert!(c.feet.z < 2.0 && c.feet.y < 0.05, "{:?}", c.feet);
}

/// A pool 2 m deep, sheer sides, from z 2 to z 10.
fn pool() -> Boxes {
    Boxes::floor(-2.0)
        .block(Vec3::new(-10.0, -2.0, -10.0), Vec3::new(10.0, 0.0, 2.0))
        .block(Vec3::new(-10.0, -2.0, 10.0), Vec3::new(10.0, 0.0, 20.0))
        .pool([-10.0, 2.0], [10.0, 10.0], -0.1)
}

#[test]
fn swims_across_deep_water_and_climbs_out() {
    let t = pool();
    let mut c = at(0.0, 0.02, 0.0);
    let events = run(&mut c, &t, walk(3.0), 2.0);
    assert!(
        events.contains(&LocomotionEvent::EnteredWater),
        "{events:?}"
    );
    assert_eq!(c.mode, Mode::Swim);
    let float = -0.1 - (c.config.swim_depth - 0.1);
    assert!((c.feet.y - float).abs() < 0.1, "afloat: {}", c.feet.y);

    // Swimming is slower than walking.
    let z = c.feet.z;
    run(&mut c, &t, walk(5.0), 1.0);
    assert!(
        c.feet.z - z <= c.config.swim_speed + 0.01,
        "{}",
        c.feet.z - z
    );

    let out = MoveInput {
        velocity: Vec3::Z * 2.0,
        climb: true,
        ..MoveInput::default()
    };
    let events = run(&mut c, &t, out, 6.0);
    assert!(events.contains(&LocomotionEvent::LeftWater), "{events:?}");
    assert!(started(&events).is_some());
    assert!(
        c.grounded() && c.feet.y.abs() < 0.05 && c.feet.z > 10.0,
        "{:?}",
        c.feet
    );
}

#[test]
fn wades_shallow_water_and_swims_from_a_sloping_shore() {
    // Shallow water: still walking.
    let shallow = Boxes::floor(0.0).pool([-10.0, -10.0], [10.0, 10.0], 0.8);
    let mut c = at(0.0, 0.02, 0.0);
    let events = run(&mut c, &shallow, walk(2.0), 2.0);
    assert!(c.grounded() && !events.contains(&LocomotionEvent::EnteredWater));

    // A beach shelving 15 degrees into a lake 3 m deep (water at z < 2).
    let run_out = 3.0 / 15f32.to_radians().tan();
    let t = Boxes::floor(-3.0)
        .block(Vec3::new(-6.0, -3.0, 2.0), Vec3::new(6.0, 0.0, 20.0))
        .with(Solid::ramp(
            Vec3::new(0.0, -3.0, 2.0 - run_out),
            12.0,
            3.0 / 15f32.to_radians().sin(),
            15.0,
        ))
        .pool([-6.0, -40.0], [6.0, 2.0], 0.0);
    let mut c = at(0.0, 0.02, 5.0);
    let events = run(&mut c, &t, walk(-2.0), 5.0);
    assert!(
        events.contains(&LocomotionEvent::EnteredWater),
        "{events:?}"
    );
    assert_eq!(c.mode, Mode::Swim);
    assert!(c.feet.z < -3.0, "{:?}", c.feet);
    let events = run(&mut c, &t, walk(2.0), 6.0);
    assert!(events.contains(&LocomotionEvent::LeftWater), "{events:?}");
    assert!(started(&events).is_none(), "walked out, no climb");
    assert!(
        c.grounded() && c.feet.z > 3.0 && c.feet.y.abs() < 0.05,
        "{:?}",
        c.feet
    );
}

#[test]
fn mantle_runs_from_start_to_top() {
    let t = Boxes::floor(0.0).block(Vec3::new(-1.0, 0.0, 1.0), Vec3::new(1.0, 1.6, 3.0));
    let c = at(0.0, 0.02, 0.62);
    let m = c.find_ledge(&t, c.feet, Vec3::Z).expect("ledge");
    assert_eq!(m.kind, MantleKind::Climb);
    let mut probe = m;
    probe.t = 0.0;
    assert!(probe.position().abs_diff_eq(m.from, 1e-5));
    probe.t = m.duration;
    assert!(probe.position().abs_diff_eq(m.to, 1e-5));
    let mut last = m.from.y;
    for i in 0..=20 {
        probe.t = m.duration * i as f32 / 20.0;
        let y = probe.position().y;
        assert!(y >= last - 1e-5, "rises monotonically");
        last = y;
    }
}

#[test]
fn bad_input_changes_nothing() {
    let t = Boxes::floor(0.0);
    let mut c = at(0.0, 0.02, 0.0);
    run(&mut c, &t, MoveInput::default(), 0.1);
    let before = c.clone();
    c.step(DT, &walk(f32::NAN), &t);
    assert!(c.feet.is_finite() && c.feet.abs_diff_eq(before.feet, 1e-4));
    assert!(c.step(f32::NAN, &walk(1.0), &t).is_empty());
    assert!(c.step(-1.0, &walk(1.0), &t).is_empty());
    assert_eq!(c.feet, before.feet);
}

#[test]
fn a_sledge_runs_downhill_steers_and_brakes() {
    // 30 m of 20 degree snow descending toward -Z, then the flat.
    let t = Boxes::floor(0.0).with(Solid::ramp(Vec3::new(0.0, 0.0, 0.0), 12.0, 30.0, 20.0));
    let mut c = at(0.0, 27.0 * 20f32.to_radians().tan() + 0.05, 27.0);
    c.sledging = true;
    run(&mut c, &t, MoveInput::default(), 0.2);
    assert!(c.grounded());
    run(&mut c, &t, MoveInput::default(), 2.0);
    let downhill = -c.velocity.z;
    assert!(downhill > 5.0 && c.velocity.x.abs() < 0.1, "{}", c.velocity);
    // Steering toward +X turns the run without killing it.
    run(
        &mut c,
        &t,
        MoveInput {
            velocity: Vec3::new(1.0, 0.0, -1.0),
            ..MoveInput::default()
        },
        0.6,
    );
    assert!(
        c.velocity.x > 1.0 && c.velocity.length() > 5.0,
        "{}",
        c.velocity
    );
    // Pulling back brakes hard.
    let (before, back) = (c.velocity.length(), -c.velocity);
    run(
        &mut c,
        &t,
        MoveInput {
            velocity: back,
            ..MoveInput::default()
        },
        0.5,
    );
    assert!(
        c.velocity.length() < before * 0.6,
        "{} vs {before}",
        c.velocity.length()
    );
    assert!(c.grounded());
    // On foot again, the same slope is walked at walking pace.
    let mut w = at(0.0, 27.0 * 20f32.to_radians().tan() + 0.05, 27.0);
    run(&mut w, &t, walk(-1.5), 2.0);
    assert!((w.velocity.z + 1.5).abs() < 0.05, "{}", w.velocity);
}
