//! Riding: the mount key, mounted AI, and a mount's footing.
mod common;
use common::*;
use glam::{Vec2, Vec3};
use somnium_combat::ai::{Archetype, BrainDesc};
use somnium_combat::environment::{EnvironmentField, Surface};
use somnium_combat::geometry::flat;
use somnium_combat::mount::MountKind;
use somnium_combat::{CombatEvent, CombatantDesc, Command, weapon};

#[test]
fn the_mount_key_climbs_into_the_nearest_saddle_and_out_again() {
    let mut w = world();
    let far = spawn(
        &mut w,
        MountKind::Strider.desc("Strider", PARTY, Vec3::new(6.0, 0.0, 0.0)),
    );
    let near = spawn(&mut w, MountKind::Horse.desc("Horse", PARTY, Vec3::ZERO));
    let rider = fighter(
        &mut w,
        "Rider",
        PARTY,
        Vec3::new(1.2, 0.0, 0.0),
        Vec3::Z,
        weapon::sword(),
    );
    w.command(
        rider,
        Command {
            mount: true,
            ..Command::default()
        },
    );
    let events = idle(&mut w, 0.1);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Mounted { mount: Some(m), .. } if *m == near))
    );
    assert_eq!(w.combatant(rider).unwrap().mount, Some(near));
    assert_eq!(w.combatant(far).unwrap().rider, None);
    w.command(
        rider,
        Command {
            mount: true,
            ..Command::default()
        },
    );
    let events = idle(&mut w, 0.1);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Mounted { mount: None, .. }))
    );
    assert_eq!(w.combatant(rider).unwrap().mount, None);
}

#[test]
fn a_mounted_raider_rides_down_the_hero() {
    let mut w = world();
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    let horse = spawn(
        &mut w,
        MountKind::Horse.desc("Warhorse", RAIDERS, Vec3::new(0.0, 0.0, 18.0)),
    );
    let mut desc = CombatantDesc::new(
        "Lancer",
        RAIDERS,
        Vec3::new(1.0, 0.0, 18.0),
        weapon::lance_glaive(),
    );
    desc.facing = -Vec3::Z;
    desc.brain = Some(BrainDesc::new(Archetype::Duelist));
    let lancer = spawn(&mut w, desc);
    assert_eq!(somnium_combat::mount::mount(&mut w, lancer, horse), Ok(()));
    let start = flat(w.combatant(horse).unwrap().position).length();
    let events = run(&mut w, 8.0, |_, _| {});
    assert!(
        flat(w.combatant(horse).unwrap().position).length() < start - 10.0,
        "the horse carried it in"
    );
    assert!(
        damage_to(&events, hero) > 0.0,
        "and it struck from the saddle"
    );
    assert_eq!(
        w.combatant(lancer).unwrap().mount,
        Some(horse),
        "still riding"
    );
}

#[test]
fn a_frost_elk_keeps_its_feet_on_ice_where_a_horse_slides() {
    let mut w = world();
    let mut ground = EnvironmentField::new(Vec2::splat(-10.0), Vec2::splat(10.0), 0.5).unwrap();
    ground.paint_rect(Vec2::splat(-10.0), Vec2::splat(10.0), Surface::Ice);
    w.environment = Some(ground);
    let elk = spawn(
        &mut w,
        MountKind::FrostElk.desc("Elk", PARTY, Vec3::new(-3.0, 0.0, 0.0)),
    );
    let horse = spawn(
        &mut w,
        MountKind::Horse.desc("Horse", PARTY, Vec3::new(3.0, 0.0, 0.0)),
    );
    idle(&mut w, 0.05);
    assert_eq!(w.intent(elk).unwrap().traction, 1.0);
    assert!(w.intent(horse).unwrap().traction < 0.5);
}
