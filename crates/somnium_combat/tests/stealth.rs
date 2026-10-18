//! Stealth on perception: awareness that fills with sight (slower far off and
//! in the dark), footsteps that are heard but not seen, quiet sounds that draw
//! a look, and blows on the unaware.
mod common;
use common::*;
use glam::Vec3;
use somnium_combat::ai::{self, Archetype, Awareness, BrainDesc, Clue};
use somnium_combat::{
    CombatEvent, CombatWorld, CombatantDesc, CombatantId, Command, Press, weapon,
};

/// A guard at `at` looking along `facing`.
fn guard(w: &mut CombatWorld, at: Vec3, facing: Vec3) -> CombatantId {
    let mut desc = CombatantDesc::new("Guard", RAIDERS, at, weapon::sword());
    desc.facing = facing;
    desc.vitals = somnium_combat::Vitals::new(500.0, 100.0, 0.0, 9999.0);
    desc.brain = Some(BrainDesc::new(Archetype::Duelist));
    w.spawn(desc)
}

fn sneak(w: &mut CombatWorld, at: Vec3) -> CombatantId {
    let mut desc = CombatantDesc::new("Sneak", PARTY, at, weapon::sword());
    desc.vitals = somnium_combat::Vitals::new(5000.0, 100.0, 0.0, 9999.0);
    w.spawn(desc)
}

/// Seconds until `watcher` reaches `level` of `target`, if within `limit`.
fn time_to(
    w: &mut CombatWorld,
    watcher: CombatantId,
    target: CombatantId,
    level: Awareness,
    limit: f32,
) -> Option<f32> {
    let steps = (limit / DT) as usize;
    for i in 0..steps {
        w.command(target, Command::default());
        w.step(DT, &somnium_combat::OpenField, None);
        let noticed = w.drain_events().into_iter().any(|e| {
            matches!(e, CombatEvent::Noticed { watcher: by, target: t, level: l, .. }
                if by == watcher && t == target && l == level)
        });
        if noticed {
            return Some(i as f32 * DT);
        }
    }
    None
}

#[test]
fn sight_fills_awareness_quickly_up_close_and_slowly_far_off() {
    let mut near = world();
    let g = guard(&mut near, Vec3::ZERO, Vec3::Z);
    let s = sneak(&mut near, Vec3::new(0.0, 0.0, 4.0));
    let close = time_to(&mut near, g, s, Awareness::Alerted, 5.0).expect("seen up close");

    let mut far = world();
    let g = guard(&mut far, Vec3::ZERO, Vec3::Z);
    let s = sneak(&mut far, Vec3::new(0.0, 0.0, 19.0));
    let wary = time_to(&mut far, g, s, Awareness::Suspicious, 5.0).expect("glimpsed far off");
    let known = time_to(&mut far, g, s, Awareness::Alerted, 5.0).expect("then known");
    assert!(close < 0.8, "up close: {close} s");
    assert!(
        wary + known > 1.0 && wary + known > close * 2.0,
        "far off it takes longer: {wary} + {known} s"
    );
}

#[test]
fn in_the_dark_under_the_lid_a_guard_misses_what_it_sees_by_day() {
    let mut day = world();
    let g = guard(&mut day, Vec3::ZERO, Vec3::Z);
    let s = sneak(&mut day, Vec3::new(0.0, 0.0, 12.0));
    assert!(
        time_to(&mut day, g, s, Awareness::Alerted, 4.0).is_some(),
        "by day"
    );

    let mut night = world();
    night.darkness = 1.0;
    let g = guard(&mut night, Vec3::ZERO, Vec3::Z);
    let s = sneak(&mut night, Vec3::new(0.0, 0.0, 12.0));
    assert!(
        time_to(&mut night, g, s, Awareness::Suspicious, 6.0).is_none(),
        "under the Lid, 12 m off, unseen"
    );
    assert_eq!(ai::awareness(&night, g, s), 0.0);
    // Close in, the dark does not hide you.
    night.combatant_mut(s).unwrap().position = Vec3::new(0.0, 0.0, 5.0);
    assert!(
        time_to(&mut night, g, s, Awareness::Alerted, 4.0).is_some(),
        "5 m off, seen"
    );
}

#[test]
fn footsteps_behind_are_heard_not_seen_and_a_walk_is_quiet() {
    let mut w = world();
    let g = guard(&mut w, Vec3::ZERO, Vec3::Z);
    let s = sneak(&mut w, Vec3::new(-3.0, 0.0, -5.0));
    // Walking back and forth behind it, five metres off.
    let pace = |w: &mut CombatWorld, i: usize, sprint: bool| {
        let dir = if (i / 60) % 2 == 0 { Vec3::X } else { -Vec3::X };
        w.command(
            s,
            Command {
                move_dir: dir,
                sprint,
                ..Command::default()
            },
        );
    };
    let events = run(&mut w, 4.0, |w, i| pace(w, i, false));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, CombatEvent::Noticed { .. })),
        "a walk goes unheard"
    );
    // Running: heard, not seen, so it goes to look; turning, it sees.
    let mut heard = None;
    let mut seen = false;
    for i in 0..4 * 60 {
        pace(&mut w, i, true);
        w.step(DT, &somnium_combat::OpenField, None);
        for e in w.drain_events() {
            match e {
                CombatEvent::Noticed {
                    watcher,
                    level: Awareness::Suspicious,
                    clue: Some(Clue::Heard),
                    ..
                } if watcher == g && heard.is_none() => {
                    let mind = &w.combatant(g).unwrap().brain.as_ref().unwrap().mind;
                    heard = Some((mind.target, mind.lead));
                }
                CombatEvent::Noticed {
                    watcher,
                    level: Awareness::Alerted,
                    ..
                } if watcher == g => seen = true,
                _ => {}
            }
        }
    }
    let (target, lead) = heard.expect("running feet are heard");
    assert!(target.is_none(), "heard, not seen: no target yet");
    assert!(lead.is_some(), "it goes to look");
    assert!(seen, "and, turning, it sees");
}

#[test]
fn a_blow_on_the_unaware_lands_hard() {
    // The same cut: on an unaware guard from behind, and on a dummy that faces
    // it (no brain, so never caught unawares; no back turned either).
    let strike = |brain: bool| {
        let mut w = world();
        let target = if brain {
            guard(&mut w, Vec3::ZERO, Vec3::Z)
        } else {
            let mut desc = CombatantDesc::new("Dummy", RAIDERS, Vec3::ZERO, weapon::sword());
            desc.vitals = somnium_combat::Vitals::new(500.0, 100.0, 0.0, 9999.0);
            desc.facing = -Vec3::Z;
            w.spawn(desc)
        };
        let s = sneak(&mut w, Vec3::new(0.0, 0.0, -1.9));
        w.combatant_mut(s).unwrap().facing = Vec3::Z;
        w.command(
            s,
            Command {
                attack: Press::click(),
                aim: Some(Vec3::ZERO),
                ..Command::default()
            },
        );
        let events = idle(&mut w, 1.0);
        events
            .iter()
            .find_map(|e| match e {
                CombatEvent::Damaged {
                    target: t,
                    amount,
                    critical,
                    ..
                } if *t == target => Some((*amount, *critical)),
                _ => None,
            })
            .expect("the cut landed")
    };
    let (unaware, crit) = strike(true);
    let (aware, _) = strike(false);
    assert!(crit, "a sneak attack is a critical");
    // At least the sneak multiplier (more if its back was still turned).
    let ratio = unaware / aware;
    assert!(
        ratio >= ai::SNEAK_ATTACK - 0.05,
        "{unaware} vs {aware}: x{ratio}"
    );
}

#[test]
fn a_whistle_draws_a_look_but_not_the_alarm() {
    let mut w = world();
    let g = guard(&mut w, Vec3::ZERO, Vec3::Z);
    let s = sneak(&mut w, Vec3::new(0.0, 0.0, -12.0));
    w.command(
        s,
        Command {
            whistle: true,
            ..Command::default()
        },
    );
    let events = idle(&mut w, 0.5);
    assert!(events.iter().any(|e| matches!(
        e,
        CombatEvent::Noticed { watcher, level: Awareness::Suspicious, clue: Some(Clue::Heard), .. } if *watcher == g
    )));
    let mind = &w.combatant(g).unwrap().brain.as_ref().unwrap().mind;
    assert!(mind.target.is_none() && mind.lead.is_some());
}
