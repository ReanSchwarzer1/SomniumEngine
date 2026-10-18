//! Death into the ragdoll blend, slow time, mounts, grudges, breakable parts,
//! the Kindled Body, lock-on and Shadow-step.
mod common;
use common::*;
use glam::{Mat4, Quat, Vec3};
use somnium_anim::{NO_PARENT, Pose, Skeleton, SkeletonId, Transform, blend_ragdoll};
use somnium_combat::mount::{self, MountKind};
use somnium_combat::schools::{SwordSchool, Swordsmanship};
use somnium_combat::weapon::ProjectileKind;
use somnium_combat::world::HurtPart;
use somnium_combat::{
    CombatEvent, CombatantDesc, Command, Press, Rank, StatusKind, TeamId, Vitals, weapon,
};

fn strike(w: &somnium_combat::CombatWorld, at: somnium_combat::CombatantId) -> Command {
    Command {
        attack: Press::click(),
        ..facing(w, at)
    }
}

#[test]
fn death_hands_the_ragdoll_its_impulse_and_a_blend_ramp() {
    let mut w = world();
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    let dummy = fighter(
        &mut w,
        "Dummy",
        RAIDERS,
        Vec3::new(0.0, 0.0, 2.0),
        -Vec3::Z,
        weapon::unarmed(),
    );
    w.combatant_mut(dummy).unwrap().vitals.health.current = 10.0;
    let events = run(&mut w, 0.6, |w, i| {
        if i == 0 {
            w.command(hero, strike(w, dummy));
        }
    });
    let (killer, impulse) = events
        .iter()
        .find_map(|e| match e {
            CombatEvent::Died {
                target,
                killer,
                impulse,
                ..
            } if *target == dummy => Some((*killer, *impulse)),
            _ => None,
        })
        .expect("it died");
    assert_eq!(killer, Some(hero));
    assert!(impulse.z > 0.0, "thrown away from the blow: {impulse}");
    let death = w.combatant(dummy).unwrap().death.expect("a ragdoll ramp");
    assert!(death.weight() > 0.99, "fully physical after the blend-in");

    // The ramp drives the engine's own ragdoll blend: at weight 0 the
    // animation stands, at weight 1 the physics snapshot has it.
    let (skeleton, _) = Skeleton::new(
        SkeletonId(1),
        vec!["pelvis".into(), "spine".into(), "head".into()],
        vec![NO_PARENT, 0, 1],
        vec![Mat4::IDENTITY; 3],
        vec![
            Transform {
                translation: Vec3::new(0.0, 0.9, 0.0),
                ..Transform::IDENTITY
            },
            Transform {
                translation: Vec3::new(0.0, 0.5, 0.0),
                ..Transform::IDENTITY
            },
            Transform {
                translation: Vec3::new(0.0, 0.3, 0.0),
                ..Transform::IDENTITY
            },
        ],
    )
    .unwrap();
    let animation = Pose::rest(&skeleton);
    let fallen = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let physics = vec![
        Some(Mat4::from_rotation_translation(
            fallen,
            Vec3::new(0.0, 0.2, 0.0),
        )),
        None,
        None,
    ];
    let mut start = death;
    start.t = 0.0;
    let standing = blend_ragdoll(&animation, &skeleton, &physics, start.weight()).unwrap();
    assert!(standing.local[0].rotation.abs_diff_eq(Quat::IDENTITY, 1e-5));
    let lying = blend_ragdoll(&animation, &skeleton, &physics, death.weight()).unwrap();
    assert!(lying.local[0].rotation.abs_diff_eq(fallen, 1e-3));
}

#[test]
fn focus_slows_the_world_more_than_the_one_who_focuses_and_runs_out() {
    let mut w = world();
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    fighter(
        &mut w,
        "Raider",
        RAIDERS,
        Vec3::new(0.0, 0.0, 6.0),
        -Vec3::Z,
        weapon::axe(),
    );
    w.command(
        hero,
        Command {
            focus: true,
            ..Command::default()
        },
    );
    let events = idle(&mut w, 1.0);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Focus { active: true, .. }))
    );
    assert!(
        (w.time() - 0.35).abs() < 0.02,
        "the world ran a third as fast: {}",
        w.time()
    );
    assert_eq!(w.time_scale_for(hero), 0.7);
    let events = idle(&mut w, 9.0);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Focus { active: false, .. })),
        "focus ran out"
    );
    assert_eq!(w.focusing(), None);
}

#[test]
fn a_rider_gallops_tramples_cuts_wind_from_the_saddle_and_can_be_thrown() {
    let mut w = world();
    let horse = spawn(&mut w, MountKind::Horse.desc("Horse", PARTY, Vec3::ZERO));
    mount::make_mount(&mut w, horse, MountKind::Horse);
    let mut desc = CombatantDesc::new("Rider", PARTY, Vec3::new(1.0, 0.0, 0.0), weapon::sword());
    desc.swordsmanship = Some(Swordsmanship::with_forms(
        SwordSchool::GaleEdge,
        3,
        Rank::Advanced,
    ));
    let rider = spawn(&mut w, desc);
    assert_eq!(mount::mount(&mut w, rider, horse), Ok(()));
    assert_eq!(
        mount::mount(&mut w, rider, horse),
        Err(mount::MountError::Taken)
    );
    let seat = w.combatant(rider).unwrap().position;
    assert!(seat.y > 0.5, "the rider sits up in the saddle: {seat}");

    let raider = fighter(
        &mut w,
        "Raider",
        RAIDERS,
        Vec3::new(0.0, 0.0, 9.0),
        -Vec3::Z,
        weapon::spear(),
    );
    let events = run(&mut w, 2.0, |w, _| {
        w.command(
            rider,
            Command {
                move_dir: Vec3::Z,
                sprint: true,
                ..Command::default()
            },
        );
    });
    let horse_z = w.combatant(horse).unwrap().position.z;
    let rider_at = w.combatant(rider).unwrap().position;
    assert!(horse_z > 5.0, "the horse galloped: {horse_z}");
    assert!((rider_at.z - horse_z).abs() < 0.01, "and carried the rider");
    assert!(
        events.iter().any(|e| matches!(e, CombatEvent::StatusApplied { target, kind: StatusKind::Knockdown } if *target == raider)),
        "the raider was run down"
    );

    let events = run(&mut w, 0.8, |w, i| {
        let cmd = if i == 0 {
            strike(w, raider)
        } else {
            facing(w, raider)
        };
        w.command(rider, cmd);
    });
    assert!(
        events.iter().any(|e| matches!(e, CombatEvent::ShotFired { shooter, kind: ProjectileKind::WindCut, .. } if *shooter == rider)),
        "every cut from the saddle flies"
    );

    w.apply_status(rider, StatusKind::Knockdown, 1.6);
    let events = idle(&mut w, 0.1);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Mounted { rider: r, mount: None } if *r == rider))
    );
    assert_eq!(w.combatant(rider).unwrap().mount, None);
    assert_eq!(w.combatant(horse).unwrap().rider, None);
}

#[test]
fn striking_a_bystander_makes_an_enemy_of_him() {
    let mut w = world();
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    let citizen = fighter(
        &mut w,
        "Citizen",
        TeamId(5),
        Vec3::new(0.0, 0.0, 2.0),
        -Vec3::Z,
        weapon::unarmed(),
    );
    assert!(!w.hostile(citizen, hero));
    run(&mut w, 0.6, |w, i| {
        if i == 0 {
            w.command(hero, strike(w, citizen));
        }
    });
    assert!(w.hostile(citizen, hero), "a grudge both ways");
}

#[test]
fn a_breakable_part_takes_more_and_breaks() {
    let mut w = world();
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    let mut desc = CombatantDesc::new(
        "Grieving giant",
        RAIDERS,
        Vec3::new(0.0, 0.0, 2.2),
        weapon::unarmed(),
    );
    desc.facing = -Vec3::Z;
    desc.vitals = Vitals::new(600.0, 200.0, 0.0, 300.0);
    desc.parts = vec![HurtPart {
        name: "lantern".into(),
        a: Vec3::new(0.0, 1.0, 0.4),
        b: Vec3::new(0.0, 1.3, 0.4),
        radius: 0.3,
        damage_scale: 1.5,
        health: Some(20.0),
    }];
    let giant = spawn(&mut w, desc);
    let events = run(&mut w, 0.6, |w, i| {
        if i == 0 {
            w.command(hero, strike(w, giant));
        }
    });
    assert!((damage_to(&events, giant) - 27.0).abs() < 0.01);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::PartBroken { part, .. } if part == "lantern"))
    );
}

#[test]
fn the_kindled_body_strikes_by_capacity_and_burns_mana() {
    let mut w = world();
    let mut desc = CombatantDesc::new("Hero", PARTY, Vec3::ZERO, weapon::sword());
    desc.vitals = Vitals::new(150.0, 100.0, 2000.0, 50.0);
    let hero = spawn(&mut w, desc);
    let dummy = fighter(
        &mut w,
        "Dummy",
        RAIDERS,
        Vec3::new(0.0, 0.0, 2.0),
        -Vec3::Z,
        weapon::unarmed(),
    );
    w.command(
        hero,
        Command {
            kindle: true,
            ..Command::default()
        },
    );
    let events = idle(&mut w, 0.1);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Kindled { on: true, .. }))
    );
    let mana = w.combatant(hero).unwrap().vitals.mana.current;
    let events = run(&mut w, 0.6, |w, i| {
        if i == 0 {
            w.command(hero, strike(w, dummy));
        }
    });
    let strength = 1.0 + 0.18 * (1.0f32 + 2000.0 / 50.0).ln();
    assert!((damage_to(&events, dummy) - 18.0 * strength).abs() < 0.05);
    idle(&mut w, 1.5);
    assert!(
        w.combatant(hero).unwrap().vitals.mana.current < mana - 1.0,
        "the aura burns the well"
    );
}

#[test]
fn lock_on_takes_the_enemy_nearest_the_cursor_and_shadow_step_goes_behind_it() {
    let mut w = world();
    let mut desc = CombatantDesc::new("Hollowborn", PARTY, Vec3::ZERO, weapon::sword());
    desc.swordsmanship = Some(Swordsmanship::with_forms(
        SwordSchool::NightFang,
        4,
        Rank::Saint,
    ));
    desc.hollowborn = true;
    let hero = spawn(&mut w, desc);
    let left = fighter(
        &mut w,
        "Left",
        RAIDERS,
        Vec3::new(-3.0, 0.0, 4.0),
        -Vec3::Z,
        weapon::axe(),
    );
    fighter(
        &mut w,
        "Right",
        RAIDERS,
        Vec3::new(3.0, 0.0, 4.0),
        -Vec3::Z,
        weapon::axe(),
    );
    w.command(
        hero,
        Command {
            lock: true,
            aim: Some(Vec3::new(-2.5, 0.0, 4.0)),
            ..Command::default()
        },
    );
    let events = idle(&mut w, 0.05);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::LockChanged { target: Some(t), .. } if *t == left))
    );
    w.command(
        hero,
        Command {
            dodge: true,
            ..Command::default()
        },
    );
    idle(&mut w, 0.4);
    let at = w.combatant(hero).unwrap().position;
    let behind = Vec3::new(-3.0, 0.0, 4.0) + Vec3::Z * 1.2;
    assert!(at.distance(behind) < 0.8, "stepped behind the lock: {at}");
}
