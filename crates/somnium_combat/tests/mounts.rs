//! Riding: the mount key, mounted AI, a mount's footing, a mount left alone
//! (grazing, the whistle, edges), and falls.
mod common;
use common::*;
use glam::{Quat, Vec2, Vec3};
use somnium_combat::ai::{Archetype, BrainDesc};
use somnium_combat::environment::{EnvironmentField, Surface};
use somnium_combat::geometry::{Obb, flat};
use somnium_combat::mount::{Herding, MountKind, WANDER_RADIUS, WHISTLE_RANGE};
use somnium_combat::{
    BoxWorld, CombatEvent, CombatWorld, CombatantDesc, CombatantId, Command, weapon,
};

fn whistle(w: &mut CombatWorld, who: CombatantId) {
    w.command(
        who,
        Command {
            whistle: true,
            ..Command::default()
        },
    );
}

fn doing(w: &CombatWorld, id: CombatantId) -> Herding {
    w.combatant(id).unwrap().herd.unwrap().doing
}

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

#[test]
fn a_horse_left_alone_grazes_and_ambles_near_where_it_was_left() {
    let mut w = world();
    let home = Vec3::new(5.0, 0.0, -3.0);
    let horse = spawn(&mut w, MountKind::Horse.desc("Horse", PARTY, home));
    let (mut farthest, mut moved, mut grazed) = (0.0f32, 0.0f32, 0usize);
    let mut last = home;
    run(&mut w, 90.0, |w, _| {
        let at = w.combatant(horse).unwrap().position;
        farthest = farthest.max(flat(at - home).length());
        moved += flat(at - last).length();
        last = at;
        grazed += usize::from(doing(w, horse) == Herding::Grazing);
    });
    assert!(moved > 4.0, "it ambled: {moved} m");
    assert!(
        farthest <= WANDER_RADIUS + 0.75,
        "but stayed near home: {farthest} m"
    );
    assert!(grazed > 90 * 60 / 3, "and mostly grazed: {grazed} steps");
}

#[test]
fn a_whistle_brings_the_horse_but_not_one_from_the_next_village() {
    let mut w = world();
    let rider = fighter(&mut w, "Rider", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    let near = spawn(
        &mut w,
        MountKind::Horse.desc("Near", PARTY, Vec3::new(0.0, 0.0, 90.0)),
    );
    let far = spawn(
        &mut w,
        MountKind::Horse.desc("Far", PARTY, Vec3::new(0.0, 0.0, -(WHISTLE_RANGE + 400.0))),
    );
    let far_at = w.combatant(far).unwrap().position;
    whistle(&mut w, rider);
    let events = idle(&mut w, 0.1);
    assert!(events.iter().any(|e| matches!(
        e,
        CombatEvent::Whistled { who, mount: Some(m) } if *who == rider && *m == near
    )));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Noise { source, .. } if *source == rider)),
        "a whistle is heard"
    );
    let mut closest = f32::MAX;
    run(&mut w, 30.0, |w, _| {
        closest = closest.min(flat(w.combatant(near).unwrap().position).length());
    });
    assert!(closest < 3.5, "it came: {closest} m off");
    let gap = flat(w.combatant(near).unwrap().position).length();
    assert!(gap < 3.5 + WANDER_RADIUS, "and stays about: {gap} m off");
    assert!(flat(w.combatant(far).unwrap().position - far_at).length() < WANDER_RADIUS + 1.0);
    // It answers its last rider first, over a nearer horse.
    let at = w.combatant(near).unwrap().position;
    w.combatant_mut(rider).unwrap().position = at + Vec3::X * 1.5;
    assert_eq!(somnium_combat::mount::mount(&mut w, rider, near), Ok(()));
    somnium_combat::mount::dismount(&mut w, rider);
    w.combatant_mut(rider).unwrap().position = at + Vec3::X * 20.0;
    spawn(
        &mut w,
        MountKind::Horse.desc("Closer", PARTY, at + Vec3::X * 24.0),
    );
    let mine = somnium_combat::mount::whistle(&mut w, rider);
    assert_eq!(mine, Some(near), "the rider's own horse, not the nearest");
}

#[test]
fn a_grazing_horse_stops_at_the_edge_of_a_drop() {
    // A plateau 8 m across over a void, as the combat query sees it.
    let ground = BoxWorld(vec![Obb {
        center: Vec3::new(0.0, -0.5, 0.0),
        half: Vec3::new(4.0, 0.5, 4.0),
        rotation: Quat::IDENTITY,
    }]);
    let mut w = world();
    let horse = spawn(&mut w, MountKind::Horse.desc("Horse", PARTY, Vec3::ZERO));
    let caller = fighter(
        &mut w,
        "Caller",
        PARTY,
        Vec3::new(30.0, 0.0, 0.0),
        Vec3::Z,
        weapon::sword(),
    );
    let step = |w: &mut CombatWorld, seconds: f32| {
        let mut widest = 0.0f32;
        for _ in 0..(seconds / DT) as usize {
            w.step(DT, &ground, None);
            w.drain_events();
            let p = w.combatant(horse).unwrap().position;
            widest = widest.max(p.x.abs()).max(p.z.abs());
        }
        widest
    };
    let widest = step(&mut w, 60.0);
    assert!(
        widest < 4.0,
        "it ambled but never off the plateau: {widest}"
    );
    whistle(&mut w, caller);
    let widest = step(&mut w, 20.0);
    assert!(widest < 4.0, "it came to the edge and no further: {widest}");
    assert!(w.combatant(horse).unwrap().position.x > 2.0, "it did come");
    assert!(
        matches!(doing(&w, horse), Herding::Coming(c) if c == caller),
        "it waits there"
    );
}

#[test]
fn a_struck_horse_bolts_away_from_whoever_struck_it() {
    let mut w = world();
    let horse = spawn(&mut w, MountKind::Horse.desc("Horse", PARTY, Vec3::ZERO));
    let raider = fighter(
        &mut w,
        "Raider",
        RAIDERS,
        Vec3::new(0.0, 0.0, -2.0),
        Vec3::Z,
        weapon::axe(),
    );
    let now = w.time();
    if let Some(h) = w.combatant_mut(horse) {
        h.last_hurt = now;
        h.last_attacker = Some(raider);
    }
    idle(&mut w, 0.1);
    assert!(matches!(doing(&w, horse), Herding::Bolting(_)));
    idle(&mut w, 2.0);
    let fled = w.combatant(horse).unwrap().position;
    assert!(fled.z > 8.0, "galloped off, away from the raider: {fled}");
    idle(&mut w, 2.0);
    assert_eq!(
        doing(&w, horse),
        Herding::Grazing,
        "then settles where it got to"
    );
}

#[test]
fn a_hard_landing_hurts_the_mount_and_throws_its_rider() {
    let mut w = world();
    let horse = spawn(&mut w, MountKind::Horse.desc("Horse", PARTY, Vec3::ZERO));
    let rider = fighter(
        &mut w,
        "Rider",
        PARTY,
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::Z,
        weapon::sword(),
    );
    assert_eq!(somnium_combat::mount::mount(&mut w, rider, horse), Ok(()));
    let (horse_hp, rider_hp) = (health(&w, horse), health(&w, rider));
    // A 4 m drop (8.9 m/s) is within a horse's stride.
    w.landed(horse, 8.9);
    let events = idle(&mut w, 0.1);
    assert!(!events.iter().any(|e| matches!(e, CombatEvent::Fell { .. })));
    assert_eq!(health(&w, horse), horse_hp);
    // 7 m (11.7 m/s): it hurts; the rider holds on.
    w.landed(horse, 11.7);
    let events = idle(&mut w, 0.1);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Fell { who, .. } if *who == horse))
    );
    assert!(health(&w, horse) < horse_hp);
    assert_eq!(
        w.combatant(rider).unwrap().mount,
        Some(horse),
        "still riding"
    );
    assert_eq!(health(&w, rider), rider_hp);
    // 12 m (15.3 m/s): the rider is thrown and takes the fall.
    w.landed(horse, 15.3);
    let events = idle(&mut w, 0.1);
    assert_eq!(w.combatant(rider).unwrap().mount, None, "thrown");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Fell { who, .. } if *who == rider))
    );
    assert!(health(&w, rider) < rider_hp);
}

#[test]
fn a_person_walks_off_a_short_drop_but_not_a_tall_one() {
    let mut w = world();
    let walker = fighter(
        &mut w,
        "Walker",
        PARTY,
        Vec3::ZERO,
        Vec3::Z,
        weapon::sword(),
    );
    let hp = health(&w, walker);
    w.landed(walker, (2.0f32 * 9.81 * 3.0).sqrt());
    idle(&mut w, 0.1);
    assert_eq!(health(&w, walker), hp, "3 m is nothing");
    w.landed(walker, (2.0f32 * 9.81 * 6.0).sqrt());
    let events = idle(&mut w, 0.1);
    let lost = hp - health(&w, walker);
    assert!(lost > 5.0 && lost < hp, "6 m hurts: {lost}");
    assert!(events.iter().any(|e| matches!(e, CombatEvent::Fell { .. })));
}
