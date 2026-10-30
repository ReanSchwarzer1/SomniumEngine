//! Enemies and companions on behaviour trees, perception and the navmesh.
mod common;
use common::*;
use glam::Vec3;
use somnium_ai::navigation::{BakeInput, BakeSettings, Bounds, NavWorld, Triangle, bake};
use somnium_combat::ai::{Archetype, BrainDesc};
use somnium_combat::geometry::{Obb, flat};
use somnium_combat::spell::{Discipline, Magic};
use somnium_combat::world::Phase;
use somnium_combat::{
    Action, BoxWorld, CombatEvent, CombatWorld, CombatantDesc, CombatantId, Command, Rank, TeamId,
    weapon,
};

fn ai(
    w: &mut CombatWorld,
    name: &str,
    team: TeamId,
    at: Vec3,
    toward: Vec3,
    kind: Archetype,
) -> CombatantId {
    let weapon = match kind {
        Archetype::Spearman => weapon::spear(),
        Archetype::Brute => weapon::axe(),
        Archetype::Skirmisher => weapon::knife(),
        Archetype::Archer => weapon::bow(),
        Archetype::Rifleman => weapon::lattice_rifle(),
        Archetype::Caster => weapon::unarmed(),
        _ => weapon::sword(),
    };
    let mut desc = CombatantDesc::new(name, team, at, weapon);
    desc.facing = toward - at;
    desc.brain = Some(BrainDesc::new(kind));
    w.spawn(desc)
}

fn target_of(w: &CombatWorld, id: CombatantId) -> Option<CombatantId> {
    w.combatant(id)?.brain.as_ref()?.mind.target
}

fn distance(w: &CombatWorld, a: CombatantId, b: CombatantId) -> f32 {
    flat(w.combatant(a).unwrap().position - w.combatant(b).unwrap().position).length()
}

#[test]
fn raiders_close_in_and_no_more_than_two_swing_at_once() {
    let mut w = world();
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    // Tough enough to outlast the test.
    w.combatant_mut(hero).unwrap().vitals = somnium_combat::Vitals::new(5000.0, 100.0, 5.0, 1000.0);
    let raiders: Vec<_> = [Vec3::Z, Vec3::X, -Vec3::Z, -Vec3::X]
        .into_iter()
        .enumerate()
        .map(|(i, d)| {
            ai(
                &mut w,
                &format!("Raider {i}"),
                RAIDERS,
                d * 8.0,
                Vec3::ZERO,
                Archetype::Duelist,
            )
        })
        .collect();
    let mut most = 0;
    let events = run(&mut w, 8.0, |w, _| {
        let swinging = raiders
            .iter()
            .filter(|r| matches!(&w.combatant(**r).unwrap().action, Action::Attack(a) if a.phase != Phase::Recovery))
            .count();
        most = most.max(swinging);
    });
    assert!(
        damage_to(&events, hero) > 0.0,
        "they reached him and struck"
    );
    assert!(most >= 1);
    assert!(
        most <= 2,
        "attack tokens hold the rest back: {most} at once"
    );
    for r in &raiders {
        assert_eq!(target_of(&w, *r), Some(hero));
    }
}

#[test]
fn an_archer_backs_off_to_its_range_then_looses() {
    let mut w = world();
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    let archer = ai(
        &mut w,
        "Archer",
        RAIDERS,
        Vec3::new(0.0, 0.0, 4.0),
        Vec3::ZERO,
        Archetype::Archer,
    );
    let events = run(&mut w, 6.0, |_, _| {});
    assert!(
        distance(&w, hero, archer) > 8.0,
        "kept its distance: {}",
        distance(&w, hero, archer)
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::ShotFired { shooter, .. } if *shooter == archer)),
        "and shot"
    );
}

#[test]
fn one_who_sees_shouts_and_one_who_does_not_comes_running() {
    let mut w = world();
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    let lookout = ai(
        &mut w,
        "Lookout",
        RAIDERS,
        Vec3::new(0.0, 0.0, 8.0),
        Vec3::ZERO,
        Archetype::Spearman,
    );
    let dozing = ai(
        &mut w,
        "Dozing",
        RAIDERS,
        Vec3::new(12.0, 0.0, 8.0),
        Vec3::new(30.0, 0.0, 8.0),
        Archetype::Duelist,
    );
    let start = distance(&w, hero, dozing);
    let events = run(&mut w, 2.0, |_, _| {});
    assert!(events.iter().any(|e| matches!(e, CombatEvent::Alerted { by, target, .. } if *by == lookout && *target == hero)));
    assert_eq!(target_of(&w, dozing), Some(hero), "the shout carried");
    assert!(distance(&w, hero, dozing) < start - 1.0, "and it came");
}

#[test]
fn a_broken_raider_runs() {
    let mut w = world();
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    let raider = ai(
        &mut w,
        "Raider",
        RAIDERS,
        Vec3::new(0.0, 0.0, 4.0),
        Vec3::ZERO,
        Archetype::Duelist,
    );
    w.combatant_mut(raider).unwrap().vitals.health.current = 15.0;
    let events = run(&mut w, 3.0, |_, _| {});
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Fled { who } if *who == raider))
    );
    assert!(
        distance(&w, hero, raider) > 8.0,
        "ran to {}",
        distance(&w, hero, raider)
    );
}

#[test]
fn a_companion_follows_and_turns_on_whoever_strikes_its_leader() {
    let mut w = world();
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    let mut desc = CombatantDesc::new(
        "Companion",
        PARTY,
        Vec3::new(6.0, 0.0, 0.0),
        weapon::spear(),
    );
    let mut brain = BrainDesc::new(Archetype::Companion);
    brain.leader = Some(hero);
    desc.brain = Some(brain);
    let friend = w.spawn(desc);
    run(&mut w, 4.0, |w, i| {
        let move_dir = if i < 150 { Vec3::Z } else { Vec3::ZERO };
        w.command(
            hero,
            Command {
                move_dir,
                ..Command::default()
            },
        );
    });
    assert!(
        distance(&w, hero, friend) < 5.0,
        "followed to {}",
        distance(&w, hero, friend)
    );
    let at = w.combatant(hero).unwrap().position + Vec3::new(0.0, 0.0, 1.8);
    let raider = fighter(&mut w, "Raider", RAIDERS, at, -Vec3::Z, weapon::axe());
    let events = run(&mut w, 4.0, |w, i| {
        if i % 50 == 0 {
            w.command(
                raider,
                Command {
                    attack: somnium_combat::Press::click(),
                    ..facing(w, hero)
                },
            );
        }
    });
    assert!(
        events.iter().any(|e| matches!(e, CombatEvent::Damaged { target, source: Some(s), .. } if *target == raider && *s == friend)),
        "the companion struck the raider"
    );
}

#[test]
fn a_caster_heals_its_hurt_friend_and_burns_its_enemy() {
    let mut w = world();
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    let mut desc = CombatantDesc::new(
        "Pyromancer",
        RAIDERS,
        Vec3::new(0.0, 0.0, 12.0),
        weapon::unarmed(),
    );
    desc.facing = -Vec3::Z;
    desc.vitals = somnium_combat::Vitals::new(80.0, 80.0, 120.0, 30.0);
    desc.magic = Magic::default()
        .with_rank(Discipline::Flame, Rank::Advanced)
        .with_rank(Discipline::Mending, Rank::Adept);
    desc.magic.speeds |= somnium_combat::spell::CastSpeed::ShortChant.bit();
    desc.brain = Some(BrainDesc::new(Archetype::Caster));
    let caster = w.spawn(desc);
    let hurt = ai(
        &mut w,
        "Hurt",
        RAIDERS,
        Vec3::new(2.0, 0.0, 14.0),
        Vec3::ZERO,
        Archetype::Archer,
    );
    w.combatant_mut(hurt).unwrap().vitals.health.current = 30.0;
    let events = run(&mut w, 8.0, |_, _| {});
    assert!(
        events.iter().any(|e| matches!(e, CombatEvent::Healed { target, source: Some(s), .. } if *target == hurt && *s == caster)),
        "healed the friend"
    );
    assert!(
        events.iter().any(|e| matches!(e, CombatEvent::SpellReleased { caster: c, spell, .. } if *c == caster && spell != "close_wound")),
        "and cast at the enemy"
    );
    assert!(damage_to(&events, hero) > 0.0);
}

#[test]
fn the_same_fight_plays_out_the_same_way_twice() {
    let battle = || {
        let mut w = world();
        ai(
            &mut w,
            "A1",
            PARTY,
            Vec3::new(-3.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 8.0),
            Archetype::Duelist,
        );
        ai(
            &mut w,
            "A2",
            PARTY,
            Vec3::new(3.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 8.0),
            Archetype::Spearman,
        );
        ai(
            &mut w,
            "B1",
            RAIDERS,
            Vec3::new(-3.0, 0.0, 8.0),
            Vec3::ZERO,
            Archetype::Brute,
        );
        ai(
            &mut w,
            "B2",
            RAIDERS,
            Vec3::new(3.0, 0.0, 8.0),
            Vec3::ZERO,
            Archetype::Skirmisher,
        );
        ai(
            &mut w,
            "B3",
            RAIDERS,
            Vec3::new(0.0, 0.0, 14.0),
            Vec3::ZERO,
            Archetype::Archer,
        );
        let events = run(&mut w, 10.0, |_, _| {});
        (format!("{events:?}"), events.len())
    };
    let (a, n) = battle();
    let (b, _) = battle();
    assert!(n > 20, "something happened: {n} events");
    assert!(a == b, "two runs diverged");
}

fn plane(x: f32, z: f32, w: f32, h: f32) -> Vec<Triangle> {
    vec![
        [[x, 0.0, z], [x + w, 0.0, z], [x + w, 0.0, z + h]],
        [[x, 0.0, z], [x + w, 0.0, z + h], [x, 0.0, z + h]],
    ]
}

#[test]
fn on_a_navmesh_a_raider_goes_round_the_wall() {
    let wall = Bounds {
        min: [-6.0, -0.5, -0.5],
        max: [6.0, 3.0, 0.5],
    };
    let tile = bake(&BakeInput {
        cell: [0, 0, 0],
        bounds: Bounds {
            min: [-15.0, -1.0, -15.0],
            max: [15.0, 6.0, 15.0],
        },
        triangles: plane(-15.0, -15.0, 30.0, 30.0),
        obstacles: vec![wall],
        settings: BakeSettings {
            voxel_size: 0.5,
            agent_radius: 0.4,
            ..BakeSettings::default()
        },
    })
    .unwrap();
    let mut nav = NavWorld::default();
    nav.install(tile);
    let blocks = BoxWorld(vec![Obb {
        center: Vec3::new(0.0, 1.5, 0.0),
        half: Vec3::new(6.0, 1.5, 0.5),
        rotation: glam::Quat::IDENTITY,
    }]);
    let mut w = world();
    let hero = fighter(
        &mut w,
        "Hero",
        PARTY,
        Vec3::new(0.0, 0.0, 8.0),
        -Vec3::Z,
        weapon::sword(),
    );
    let raider = ai(
        &mut w,
        "Raider",
        RAIDERS,
        Vec3::new(0.0, 0.0, -8.0),
        Vec3::new(0.0, 0.0, 8.0),
        Archetype::Duelist,
    );
    // The wall hides him; a shout says where he is, and it must find the way.
    somnium_combat::ai::alert(&mut w, raider, hero, Vec3::new(0.0, 0.0, 8.0));
    let mut events = Vec::new();
    for _ in 0..(14.0 / DT) as usize {
        w.step(DT, &blocks, Some(&nav));
        events.extend(w.drain_events());
    }
    assert!(
        distance(&w, hero, raider) < 4.0,
        "got round: {}",
        distance(&w, hero, raider)
    );
    assert!(damage_to(&events, hero) > 0.0);
}
