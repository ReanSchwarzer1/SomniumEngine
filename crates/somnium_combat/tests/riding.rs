//! Brains in the saddle: a patrol walks its horses, a lancer charges in passes
//! and wheels about, a swordsman rides up and gets down to fight.
mod common;
use common::*;
use glam::Vec3;
use somnium_combat::ai::{self, Archetype, BrainDesc, DISMOUNT_RANGE};
use somnium_combat::geometry::flat;
use somnium_combat::mount::{Herding, MountKind};
use somnium_combat::weapon::{self, WeaponDef};
use somnium_combat::{CombatEvent, CombatWorld, CombatantDesc, CombatantId};

/// A raider of `archetype` with `weapon`, on a horse at `at`, facing -Z.
fn rider(
    w: &mut CombatWorld,
    at: Vec3,
    archetype: Archetype,
    weapon: WeaponDef,
) -> (CombatantId, CombatantId) {
    let horse = spawn(w, MountKind::Horse.desc("Warhorse", RAIDERS, at));
    let mut desc = CombatantDesc::new("Rider", RAIDERS, at + Vec3::X, weapon);
    desc.facing = -Vec3::Z;
    let mut brain = BrainDesc::new(archetype);
    brain.home = Some(at);
    desc.brain = Some(brain);
    let id = spawn(w, desc);
    assert_eq!(somnium_combat::mount::mount(w, id, horse), Ok(()));
    (id, horse)
}

/// Seconds at which `source` landed a blow on `target`.
fn blows(events: &[(f32, CombatEvent)], source: CombatantId, target: CombatantId) -> Vec<f32> {
    events
        .iter()
        .filter_map(|(t, e)| match e {
            CombatEvent::Damaged {
                target: x,
                source: Some(s),
                amount,
                ..
            } if *x == target && *s == source && *amount > 0.0 => Some(*t),
            _ => None,
        })
        .collect()
}

/// Step `seconds`, with each event's time.
fn timed(
    w: &mut CombatWorld,
    seconds: f32,
    mut each: impl FnMut(&CombatWorld),
) -> Vec<(f32, CombatEvent)> {
    let mut out = Vec::new();
    let steps = (seconds / DT).round() as usize;
    for i in 0..steps {
        let events = run(w, DT, |_, _| {});
        each(w);
        out.extend(events.into_iter().map(|e| (i as f32 * DT, e)));
    }
    out
}

#[test]
fn a_lancer_charges_through_and_wheels_about_for_another_pass() {
    let mut w = world();
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    if let Some(c) = w.combatant_mut(hero) {
        c.vitals.health.max = 5000.0;
        c.vitals.health.current = 5000.0;
    }
    let (lancer, horse) = rider(
        &mut w,
        Vec3::new(0.0, 0.0, 30.0),
        Archetype::Spearman,
        weapon::lance_glaive(),
    );
    // Seen across the field (a patrol's shout does the same).
    ai::alert(&mut w, lancer, hero, Vec3::ZERO);
    let (mut through, mut gallop) = (false, 0.0f32);
    let events = timed(&mut w, 16.0, |w| {
        let h = w.combatant(horse).unwrap();
        // Past the hero: the far side of where it came from.
        through |= h.position.z < -4.0;
        gallop = gallop.max(flat(h.velocity).length());
    });
    let hits = blows(&events, lancer, hero);
    let passes =
        hits.windows(2).filter(|p| p[1] - p[0] > 2.0).count() + usize::from(!hits.is_empty());
    assert!(passes >= 2, "struck in two passes at least: {hits:?}");
    assert!(through, "rode on through");
    assert!(gallop > 8.0, "at the gallop: {gallop}");
    assert_eq!(
        w.combatant(lancer).unwrap().mount,
        Some(horse),
        "and never got down"
    );
}

#[test]
fn a_swordsman_rides_up_and_gets_down_to_fight() {
    let mut w = world();
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    let (swordsman, horse) = rider(
        &mut w,
        Vec3::new(0.0, 0.0, 30.0),
        Archetype::Duelist,
        weapon::sword(),
    );
    // Seen across the field (a patrol's shout does the same).
    ai::alert(&mut w, swordsman, hero, Vec3::ZERO);
    let mut got_down = None;
    let events = timed(&mut w, 10.0, |w| {
        let c = w.combatant(swordsman).unwrap();
        if got_down.is_none() && c.mount.is_none() {
            got_down = Some(flat(c.position).length());
        }
    });
    let at = got_down.expect("got down");
    assert!(
        at <= DISMOUNT_RANGE + 2.0 && at > 4.0,
        "got down short of the hero: {at} m"
    );
    assert!(
        !blows(&events, swordsman, hero).is_empty(),
        "and fought on foot"
    );
    let h = w.combatant(horse).unwrap();
    assert!(h.rider.is_none(), "the horse left");
    assert!(
        matches!(
            h.herd.map(|d| d.doing),
            Some(Herding::Grazing | Herding::Wandering(_))
        ),
        "grazing where it was left"
    );
}

#[test]
fn a_mounted_patrol_walks_its_horse_home() {
    let mut w = world();
    let (id, horse) = rider(
        &mut w,
        Vec3::new(0.0, 0.0, 30.0),
        Archetype::Spearman,
        weapon::spear(),
    );
    // Its home along the road, 40 m on (as a patrol's leader moves it).
    w.combatant_mut(id)
        .unwrap()
        .brain
        .as_mut()
        .unwrap()
        .mind
        .home = Some(Vec3::new(0.0, 0.0, -10.0));
    let start = w.combatant(horse).unwrap().position;
    let mut fastest = 0.0f32;
    idle(&mut w, 0.5);
    let _ = timed(&mut w, 6.0, |w| {
        fastest = fastest.max(flat(w.combatant(horse).unwrap().velocity).length())
    });
    let gone = start.z - w.combatant(horse).unwrap().position.z;
    assert!(gone > 6.0, "on its way: {gone} m");
    assert!(fastest < 2.5, "at a walk: {fastest} m/s");
}
