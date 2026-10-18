//! AI on the ground: cover for the ranged, formations, and what a host's
//! controller hears from a brain (a jump, "I am climbing").
mod common;
use common::*;
use glam::{Quat, Vec3};
use somnium_combat::ai::{self, Archetype, BrainDesc};
use somnium_combat::geometry::Obb;
use somnium_combat::{
    Action, CombatEvent, CombatQuery, CombatWorld, CombatantDesc, CombatantId, TeamId, Vitals,
    WorldHit, weapon,
};

/// A world of boxes for the brain's sight lines and the projectiles.
struct Boxes(Vec<Obb>);

impl CombatQuery for Boxes {
    fn sweep(&self, from: Vec3, to: Vec3, radius: f32) -> Option<WorldHit> {
        let len = (to - from).length();
        let dir = (to - from).try_normalize()?;
        self.0
            .iter()
            .filter_map(|b| b.ray(from, dir, len, radius))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(t, normal)| WorldHit {
                fraction: t / len,
                normal,
            })
    }
}

fn pillar(at: Vec3) -> Obb {
    Obb {
        center: at + Vec3::Y * 1.5,
        half: Vec3::new(0.6, 1.5, 0.6),
        rotation: Quat::IDENTITY,
    }
}

fn brain(
    w: &mut CombatWorld,
    name: &str,
    team: TeamId,
    at: Vec3,
    weapon: weapon::WeaponDef,
    kind: Archetype,
) -> CombatantId {
    let mut desc = CombatantDesc::new(name, team, at, weapon);
    desc.facing = Vec3::Z;
    desc.brain = Some(BrainDesc::new(kind));
    w.spawn(desc)
}

/// Someone who stands and takes it.
fn post(w: &mut CombatWorld, team: TeamId, at: Vec3) -> CombatantId {
    let mut desc = CombatantDesc::new("Post", team, at, weapon::sword());
    desc.facing = -Vec3::Z;
    desc.vitals = Vitals::new(100_000.0, 100.0, 0.0, 10_000.0);
    w.spawn(desc)
}

#[test]
fn a_rifleman_reloads_behind_a_pillar_and_steps_out_to_fire_again() {
    let mut w = world();
    let target = post(&mut w, PARTY, Vec3::ZERO);
    let rifle = brain(
        &mut w,
        "Rifleman",
        RAIDERS,
        Vec3::new(0.0, 0.0, -15.0),
        weapon::lattice_rifle(),
        Archetype::Rifleman,
    );
    // A pillar a little to the rifleman's right, between it and nothing.
    let world_boxes = Boxes(vec![pillar(Vec3::new(2.5, 0.0, -13.0))]);
    ai::alert(&mut w, rifle, target, Vec3::ZERO);
    let (mut shots, mut hidden) = (0, false);
    for _ in 0..(10.0 / DT) as usize {
        w.step(DT, &world_boxes, None);
        shots += w
            .drain_events()
            .iter()
            .filter(|e| matches!(e, CombatEvent::ShotFired { shooter, .. } if *shooter == rifle))
            .count();
        let (r, t) = (w.combatant(rifle).unwrap(), w.combatant(target).unwrap());
        if matches!(r.action, Action::Reload { .. }) && world_boxes.blocked(t.eye(), r.chest()) {
            hidden = true;
        }
    }
    assert!(hidden, "reloaded out of the target's sight");
    assert!(shots >= 2, "stepped out and fired again: {shots} shots");
}

#[test]
fn spears_hold_a_line_and_the_archers_stand_behind_it() {
    let mut w = world();
    let enemy = post(&mut w, PARTY, Vec3::new(0.0, 0.0, 20.0));
    let spears: Vec<CombatantId> = [
        Vec3::new(-3.0, 0.0, 0.5),
        Vec3::new(0.5, 0.0, -1.0),
        Vec3::new(3.5, 0.0, 0.2),
    ]
    .into_iter()
    .map(|at| {
        brain(
            &mut w,
            "Spear",
            RAIDERS,
            at,
            weapon::spear(),
            Archetype::Spearman,
        )
    })
    .collect();
    let archers: Vec<CombatantId> = [Vec3::new(-1.0, 0.0, -2.0), Vec3::new(2.0, 0.0, 1.5)]
        .into_iter()
        .map(|at| {
            brain(
                &mut w,
                "Archer",
                RAIDERS,
                at,
                weapon::bow(),
                Archetype::Archer,
            )
        })
        .collect();
    let group: Vec<CombatantId> = spears.iter().chain(&archers).copied().collect();
    ai::form_up(&mut w, &group);
    for &id in &group {
        ai::alert(&mut w, id, enemy, Vec3::new(0.0, 0.0, 20.0));
    }
    let start = w.combatant(spears[1]).unwrap().position.z;
    idle(&mut w, 5.0);
    let at = |id: CombatantId| w.combatant(id).unwrap().position;
    let line: Vec<Vec3> = spears.iter().map(|id| at(*id)).collect();
    let front = line.iter().map(|p| p.z).fold(f32::INFINITY, f32::min);
    let mean = line.iter().map(|p| p.z).sum::<f32>() / 3.0;
    assert!(
        line.iter().all(|p| (p.z - mean).abs() < 0.7),
        "abreast: {line:?}"
    );
    let mut xs: Vec<f32> = line.iter().map(|p| p.x).collect();
    xs.sort_by(f32::total_cmp);
    assert!(xs.windows(2).all(|p| p[1] - p[0] > 1.2), "spaced: {xs:?}");
    assert!(
        mean > start + 1.0,
        "the line walked toward the enemy: {mean}"
    );
    for id in &archers {
        assert!(
            at(*id).z < front - 2.5,
            "behind the line: {:?} {line:?}",
            at(*id)
        );
    }
    // Near enough, the line breaks and fights.
    w.combatant_mut(enemy).unwrap().position = Vec3::new(0.0, 0.0, mean + 3.0);
    let events = idle(&mut w, 3.0);
    assert!(
        events.iter().any(|e| matches!(
            e,
            CombatEvent::AttackStarted { attacker, .. } if spears.contains(attacker)
        )),
        "a spear struck"
    );
}

#[test]
fn a_climbing_brain_holds_still_and_a_host_reads_its_jump() {
    let mut w = world();
    let enemy = post(&mut w, PARTY, Vec3::new(0.0, 0.0, 1.5));
    let axe = brain(
        &mut w,
        "Axe",
        RAIDERS,
        Vec3::ZERO,
        weapon::axe(),
        Archetype::Brute,
    );
    ai::alert(&mut w, axe, enemy, Vec3::new(0.0, 0.0, 1.5));
    let hanging = ai::Climb {
        progress: 0.3,
        vulnerable: true,
    };
    ai::set_climbing(&mut w, axe, Some(hanging));
    let events = idle(&mut w, 2.0);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, CombatEvent::AttackStarted { attacker, .. } if *attacker == axe)),
        "no swing mid-climb"
    );
    assert!(!ai::wants_jump(&w, axe), "no route, no jump");
    ai::set_climbing(&mut w, axe, None);
    let events = idle(&mut w, 2.0);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::AttackStarted { attacker, .. } if *attacker == axe)),
        "swings once up"
    );
}

/// A brute on a ledge cannot reach someone below it, but strikes at one
/// whose hands are on the edge.
#[test]
fn a_climber_hanging_on_the_edge_is_struck_from_the_top() {
    let mut w = world();
    let climber = post(&mut w, PARTY, Vec3::new(0.0, 0.6, 1.2));
    let axe = brain(
        &mut w,
        "Axe",
        RAIDERS,
        Vec3::new(0.0, 2.0, 0.0),
        weapon::axe(),
        Archetype::Brute,
    );
    ai::alert(&mut w, axe, climber, Vec3::new(0.0, 0.6, 1.2));
    let swings = |events: &[CombatEvent]| {
        events
            .iter()
            .filter(
                |e| matches!(e, CombatEvent::AttackStarted { attacker, .. } if *attacker == axe),
            )
            .count()
    };
    let hold = |w: &mut CombatWorld, _: usize| {
        // The ledge holds the brute up; the climber hangs where it is.
        w.combatant_mut(axe).unwrap().position.y = 2.0;
        w.combatant_mut(climber).unwrap().position = Vec3::new(0.0, 0.6, 1.2);
    };
    assert_eq!(
        swings(&run(&mut w, 1.5, hold)),
        0,
        "out of reach below the ledge"
    );
    let hanging = ai::Climb {
        progress: 0.3,
        vulnerable: true,
    };
    ai::set_climbing(&mut w, climber, Some(hanging));
    assert!(
        swings(&run(&mut w, 1.5, hold)) > 0,
        "struck while its hands are on the edge"
    );
}
