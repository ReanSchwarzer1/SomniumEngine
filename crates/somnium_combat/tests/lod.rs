//! Brain LOD: far brains with nothing on their mind sleep, mid-range ones think
//! every few steps, near ones as before; a fight near the focus plays out
//! exactly as it would without one.
mod common;
use common::*;
use glam::Vec3;
use somnium_combat::ai::{Archetype, BrainDesc};
use somnium_combat::{CombatWorld, CombatantDesc, CombatantId, OpenField, weapon};

fn raider(w: &mut CombatWorld, at: Vec3) -> CombatantId {
    let mut desc = CombatantDesc::new("Raider", RAIDERS, at, weapon::axe());
    desc.facing = Vec3::X;
    desc.brain = Some(BrainDesc::new(Archetype::Brute));
    w.spawn(desc)
}

fn step(w: &mut CombatWorld) {
    w.step(DT, &OpenField, None);
    w.drain_events();
}

#[test]
fn with_no_focus_every_brain_thinks() {
    let mut w = world();
    for i in 0..6 {
        raider(&mut w, Vec3::new(i as f32 * 100.0, 0.0, 0.0));
    }
    step(&mut w);
    let s = w.lod_stats();
    assert_eq!((s.near, s.thought), (6, 6));
}

#[test]
fn far_idle_brains_sleep_and_mid_ones_think_on_their_stride() {
    let mut w = world();
    w.set_focus(&[Vec3::ZERO]);
    for i in 0..12 {
        raider(&mut w, Vec3::new(300.0 + i as f32 * 3.0, 0.0, 0.0));
    }
    let mid: Vec<CombatantId> = (0..8)
        .map(|i| raider(&mut w, Vec3::new(80.0, 0.0, i as f32 * 3.0)))
        .collect();
    let mut thought = 0;
    for _ in 0..w.lod.far_stride {
        step(&mut w);
        let s = w.lod_stats();
        assert_eq!((s.far, s.mid), (12, 8));
        thought += s.thought;
    }
    let (mid_strides, far_strides) = (w.lod.far_stride / w.lod.mid_stride, 1);
    let expected = mid.len() * mid_strides as usize + 12 * far_strides;
    assert!(
        thought.abs_diff(expected) <= mid.len(),
        "each mid brain once a mid stride, each far one once a far stride: {thought} vs {expected}"
    );
    // Every brain is back after the step.
    assert!(w.combatants().all(|c| c.brain.is_some()));
}

#[test]
fn a_far_brain_in_a_fight_keeps_thinking() {
    let mut w = world();
    w.set_focus(&[Vec3::ZERO]);
    let hero = fighter(
        &mut w,
        "Hero",
        PARTY,
        Vec3::new(500.0, 0.0, 0.0),
        Vec3::Z,
        weapon::sword(),
    );
    let r = raider(&mut w, Vec3::new(505.0, 0.0, 0.0));
    somnium_combat::ai::alert(&mut w, r, hero, Vec3::new(500.0, 0.0, 0.0));
    // Within a far stride it hears the alert; then it thinks at the mid rate.
    for _ in 0..w.lod.far_stride {
        step(&mut w);
    }
    let mind = &w.combatant(r).unwrap().brain.as_ref().unwrap().mind;
    assert!(mind.target.is_some() || mind.lead.is_some(), "it heard");
    let mut thought = 0;
    for _ in 0..w.lod.mid_stride * 3 {
        step(&mut w);
        thought += w.lod_stats().thought;
    }
    assert_eq!(thought, 3, "a fight far off thinks at the mid-range rate");
}

#[test]
fn a_fight_near_the_focus_is_the_same_fight() {
    let fight = |focus: bool| {
        let mut w = world();
        if focus {
            w.set_focus(&[Vec3::ZERO]);
        }
        let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
        for i in 0..3 {
            let r = raider(&mut w, Vec3::new(-3.0 + 3.0 * i as f32, 0.0, 6.0));
            somnium_combat::ai::alert(&mut w, r, hero, Vec3::ZERO);
        }
        run(&mut w, 6.0, |_, _| {})
    };
    assert_eq!(fight(false), fight(true));
}

/// `cargo test -p somnium_combat --release --test lod -- --ignored --nocapture`:
/// 400 brains over two kilometres, the focus in the middle of one camp.
#[test]
#[ignore]
fn brain_cost() {
    for focus in [false, true] {
        let mut w = world();
        if focus {
            w.set_focus(&[Vec3::ZERO]);
        }
        for i in 0..400 {
            let camp = (i / 20) as f32;
            let at = Vec3::new(
                camp * 100.0 + (i % 5) as f32 * 2.0,
                0.0,
                (i % 20 / 5) as f32 * 2.0,
            );
            raider(&mut w, at);
        }
        let start = std::time::Instant::now();
        let steps = 600;
        let mut thought = 0;
        for _ in 0..steps {
            w.step(DT, &OpenField, None);
            w.drain_events();
            thought += w.lod_stats().thought;
        }
        let per = start.elapsed().as_secs_f64() * 1000.0 / f64::from(steps);
        println!(
            "focus {focus}: 400 brains, {per:.3} ms a step, {:.0} thought a step",
            f64::from(thought as u32) / f64::from(steps)
        );
    }
}
