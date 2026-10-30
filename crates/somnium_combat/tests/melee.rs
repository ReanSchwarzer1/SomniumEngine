//! Swords and everyone else's weapons, close in.
mod common;
use common::*;
use glam::Vec3;
use somnium_combat::schools::{SwordSchool, Swordsmanship};
use somnium_combat::weapon::{self, HitShape};
use somnium_combat::world::Phase;
use somnium_combat::{
    Action, CombatEvent, CombatantDesc, Command, HitFlags, Press, Rank, StatusKind,
};

/// A click aimed at `at` (the command persists, so it must not stay held).
fn attack(w: &somnium_combat::CombatWorld, at: somnium_combat::CombatantId) -> Command {
    Command {
        attack: Press::click(),
        ..facing(w, at)
    }
}

#[test]
fn a_three_cut_chain_lands_three_times_and_breaks_poise() {
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
    let stamina = w.combatant(hero).unwrap().vitals.stamina.current;
    let events = run(&mut w, 1.6, |w, i| {
        let cmd = if [0, 20, 45].contains(&i) {
            attack(w, dummy)
        } else {
            facing(w, dummy)
        };
        w.command(hero, cmd);
    });
    let hits = count(
        &events,
        |e| matches!(e, CombatEvent::Damaged { target, .. } if *target == dummy),
    );
    assert_eq!(hits, 3, "{events:#?}");
    assert!((damage_to(&events, dummy) - (18.0 + 20.0 + 28.0)).abs() < 0.01);
    let names: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            CombatEvent::AttackStarted { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, ["sword_cut_1", "sword_cut_2", "sword_thrust"]);
    assert!(events.iter().any(
        |e| matches!(e, CombatEvent::PoiseBroken { target, knockdown: false } if *target == dummy)
    ));
    assert!(w.combatant(hero).unwrap().vitals.stamina.current < stamina);
}

#[test]
fn holding_the_attack_turns_the_opener_into_the_heavy_and_it_knocks_down() {
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
    let events = run(&mut w, 1.4, |w, i| {
        let mut cmd = facing(w, dummy);
        cmd.attack = match i {
            0 => Press::tap(),
            1..=20 => Press::hold(),
            21 => Press::release(),
            _ => Press::default(),
        };
        w.command(hero, cmd);
    });
    assert!(
        events.iter().any(
            |e| matches!(e, CombatEvent::AttackStarted { name, .. } if name == "sword_overhead")
        )
    );
    assert!(
        (damage_to(&events, dummy) - 48.0).abs() < 0.01,
        "only the heavy landed"
    );
    assert!(events.iter().any(|e| matches!(
        e,
        CombatEvent::PoiseBroken {
            knockdown: true,
            ..
        }
    )));
    assert!(
        w.combatant(dummy)
            .unwrap()
            .status
            .active(StatusKind::Knockdown)
    );
}

#[test]
fn a_parry_staggers_the_attacker_and_the_riposte_is_a_critical() {
    let mut w = world();
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    let raider = fighter(
        &mut w,
        "Raider",
        RAIDERS,
        Vec3::new(0.0, 0.0, 2.0),
        -Vec3::Z,
        weapon::sword(),
    );
    let events = run(&mut w, 1.2, |w, i| {
        if i == 0 {
            w.command(raider, attack(w, hero));
        }
        let mut cmd = facing(w, raider);
        cmd.guard = match i {
            12 => Press::tap(),
            13..=20 => Press::hold(),
            _ => Press::default(),
        };
        if i == 40 {
            cmd.attack = Press::tap();
        }
        w.command(hero, cmd);
    });
    assert!(events.iter().any(|e| matches!(e, CombatEvent::Parried { defender, attacker, .. } if *defender == hero && *attacker == raider)));
    assert_eq!(damage_to(&events, hero), 0.0, "a parried blow does no harm");
    assert!(events.iter().any(|e| matches!(e, CombatEvent::StatusApplied { target, kind: StatusKind::Stagger } if *target == raider)));
    assert!(events.iter().any(|e| matches!(e, CombatEvent::AttackStarted { attacker, riposte: true, .. } if *attacker == hero)));
    let riposte = events.iter().find_map(|e| match e {
        CombatEvent::Damaged {
            target,
            amount,
            critical,
            flags,
            ..
        } if *target == raider => Some((*amount, *critical, *flags)),
        _ => None,
    });
    let (amount, critical, flags) = riposte.expect("the riposte landed");
    assert!(critical && flags.contains(HitFlags::RIPOSTE));
    assert!((amount - 18.0 * 2.5).abs() < 0.01, "riposte did {amount}");
}

#[test]
fn a_held_guard_blocks_for_stamina_until_it_breaks() {
    let mut w = world();
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    let raider = fighter(
        &mut w,
        "Raider",
        RAIDERS,
        Vec3::new(0.0, 0.0, 2.0),
        -Vec3::Z,
        weapon::sword(),
    );
    let before = w.combatant(hero).unwrap().vitals.stamina.current;
    let events = run(&mut w, 0.7, |w, i| {
        if i == 15 {
            w.command(raider, attack(w, hero));
        }
        let mut cmd = facing(w, raider);
        cmd.guard = if i == 0 { Press::tap() } else { Press::hold() };
        w.command(hero, cmd);
    });
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Blocked { target, .. } if *target == hero))
    );
    let chip = damage_to(&events, hero);
    assert!((chip - 18.0 * (1.0 - 0.72)).abs() < 0.01, "chip {chip}");
    assert!(w.combatant(hero).unwrap().vitals.stamina.current < before);

    // Spent: the next blow breaks the guard.
    w.combatant_mut(hero).unwrap().vitals.stamina.current = 3.0;
    let events = run(&mut w, 1.2, |w, i| {
        if i == 10 {
            w.command(raider, attack(w, hero));
        }
        let mut cmd = facing(w, raider);
        cmd.guard = Press::hold();
        w.command(hero, cmd);
    });
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::GuardBroken { target } if *target == hero)),
        "{events:#?}"
    );
    assert!(events.iter().any(|e| matches!(e, CombatEvent::StatusApplied { target, kind: StatusKind::Stagger } if *target == hero)));
}

#[test]
fn dodge_frames_let_a_blow_pass_through() {
    let mut w = world();
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    let raider = fighter(
        &mut w,
        "Raider",
        RAIDERS,
        Vec3::new(0.0, 0.0, 1.8),
        -Vec3::Z,
        weapon::sword(),
    );
    let events = run(&mut w, 0.8, |w, i| {
        if i == 0 {
            w.command(raider, attack(w, hero));
        }
        let mut cmd = facing(w, raider);
        if i == 13 {
            cmd.dodge = true;
            cmd.move_dir = Vec3::X;
        }
        w.command(hero, cmd);
    });
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::DodgeStarted { who, .. } if *who == hero))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Dodged { target, .. } if *target == hero)),
        "{events:#?}"
    );
    assert_eq!(damage_to(&events, hero), 0.0);
}

#[test]
fn a_knife_from_behind_is_a_backstab() {
    let mut w = world();
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    let knife = fighter(
        &mut w,
        "Cutthroat",
        RAIDERS,
        Vec3::new(0.0, 0.0, -1.2),
        Vec3::Z,
        weapon::knife(),
    );
    let events = run(&mut w, 0.5, |w, i| {
        if i == 0 {
            w.command(knife, attack(w, hero));
        }
    });
    let (amount, flags) = events
        .iter()
        .find_map(|e| match e {
            CombatEvent::Damaged {
                target,
                amount,
                flags,
                ..
            } if *target == hero => Some((*amount, *flags)),
            _ => None,
        })
        .expect("the knife landed");
    assert!(flags.contains(HitFlags::BACKSTAB));
    assert!((amount - 9.0 * 3.0).abs() < 0.01, "backstab did {amount}");
}

#[test]
fn night_fang_feints_by_raising_the_guard_mid_windup() {
    let mut w = world();
    let mut desc = CombatantDesc::new(
        "Nightblade",
        RAIDERS,
        Vec3::new(0.0, 0.0, 2.0),
        weapon::sword(),
    );
    desc.facing = -Vec3::Z;
    desc.swordsmanship = Some(Swordsmanship::with_forms(
        SwordSchool::NightFang,
        3,
        Rank::Adept,
    ));
    let feinter = spawn(&mut w, desc);
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    let events = run(&mut w, 0.6, |w, i| {
        let mut cmd = facing(w, hero);
        match i {
            0 => cmd.attack = Press::tap(),
            6 => cmd.guard = Press::tap(),
            _ => {}
        }
        w.command(feinter, cmd);
    });
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Feinted { attacker } if *attacker == feinter))
    );
    assert_eq!(
        damage_to(&events, hero),
        0.0,
        "the feinted blow never lands"
    );
}

#[test]
fn meridian_noon_counters_at_the_instant_of_the_strike() {
    let mut w = world();
    let mut desc = CombatantDesc::new("Hero", PARTY, Vec3::ZERO, weapon::sword());
    desc.swordsmanship = Some(Swordsmanship::with_forms(
        SwordSchool::Meridian,
        4,
        Rank::Adept,
    ));
    let hero = spawn(&mut w, desc);
    let raider = fighter(
        &mut w,
        "Raider",
        RAIDERS,
        Vec3::new(0.0, 0.0, 2.0),
        -Vec3::Z,
        weapon::sword(),
    );
    let events = run(&mut w, 0.8, |w, i| {
        if i == 0 {
            w.command(raider, attack(w, hero));
        }
        let mut cmd = facing(w, raider);
        cmd.guard = match i {
            13 => Press::tap(),
            14..=30 => Press::hold(),
            _ => Press::default(),
        };
        w.command(hero, cmd);
    });
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Parried { perfect: true, .. })),
        "{events:#?}"
    );
    let counter = damage_to(&events, raider);
    let expected = 18.0 * 2.5 * 1.5 * Rank::Adept.scale();
    assert!(
        (counter - expected).abs() < 0.05,
        "Noon countered for {counter}, expected {expected}"
    );
}

#[test]
fn the_aetherlance_charge_runs_down_the_first_body_and_floors_it() {
    let mut w = world();
    let lancer = fighter(
        &mut w,
        "Lancer",
        PARTY,
        Vec3::new(0.0, 0.0, -6.0),
        Vec3::Z,
        weapon::lance_glaive(),
    );
    let dummy = fighter(
        &mut w,
        "Dummy",
        RAIDERS,
        Vec3::ZERO,
        -Vec3::Z,
        weapon::unarmed(),
    );
    let events = run(&mut w, 1.6, |w, i| {
        let mut cmd = facing(w, dummy);
        cmd.move_dir = Vec3::Z;
        cmd.sprint = true;
        if i == 0 {
            cmd.attack = Press::tap();
        }
        w.command(lancer, cmd);
    });
    assert!(
        events.iter().any(
            |e| matches!(e, CombatEvent::AttackStarted { name, .. } if name == "glaive_charge")
        )
    );
    assert!(damage_to(&events, dummy) >= 45.0 - 0.01);
    assert!(events.iter().any(
        |e| matches!(e, CombatEvent::PoiseBroken { target, knockdown: true } if *target == dummy)
    ));
    assert!(
        w.combatant(lancer).unwrap().position.z > -3.0,
        "the lancer travelled"
    );
}

#[test]
fn a_clip_driven_move_strikes_only_between_its_events() {
    let mut w = world();
    let mut sword = weapon::sword();
    sword.light[0].driven_by_events = true;
    sword.light[0].next = None;
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, sword);
    let dummy = fighter(
        &mut w,
        "Dummy",
        RAIDERS,
        Vec3::new(0.0, 0.0, 2.0),
        -Vec3::Z,
        weapon::unarmed(),
    );
    let event = |name: &str| somnium_anim::AnimationEvent {
        time: 0.0,
        name: name.into(),
        payload: String::new(),
    };
    let quiet = run(&mut w, 0.5, |w, i| {
        if i == 0 {
            w.command(hero, attack(w, dummy));
        }
    });
    assert_eq!(
        damage_to(&quiet, dummy),
        0.0,
        "no hit before the clip says so"
    );
    assert!(
        matches!(&w.combatant(hero).unwrap().action, Action::Attack(a) if a.phase == Phase::Windup)
    );
    w.animation_event(hero, &event("hit_on"));
    let struck = idle(&mut w, 0.05);
    assert!((damage_to(&struck, dummy) - 18.0).abs() < 0.01);
    w.animation_event(hero, &event("hit_off"));
    w.animation_event(hero, &event("recover"));
    assert!(matches!(w.combatant(hero).unwrap().action, Action::Idle));
}

#[test]
fn a_blade_from_the_skeleton_is_swept_between_frames() {
    let mut w = world();
    let mut sword = weapon::sword();
    sword.light[0].shape = HitShape::Blade { radius: 0.08 };
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, sword);
    let dummy = fighter(
        &mut w,
        "Dummy",
        RAIDERS,
        Vec3::new(0.0, 0.0, 1.6),
        -Vec3::Z,
        weapon::unarmed(),
    );
    let events = run(&mut w, 0.6, |w, i| {
        if i == 0 {
            w.command(hero, attack(w, dummy));
        }
        // A fast cut: one frame on the left, the next on the right, never
        // touching the body on either frame.
        let x = if i % 2 == 0 { -1.6 } else { 1.6 };
        w.set_blade(hero, Vec3::new(x * 0.3, 1.2, 0.4), Vec3::new(x, 1.1, 1.6));
    });
    assert!(
        damage_to(&events, dummy) > 0.0,
        "the sweep between frames struck"
    );
}

#[test]
fn a_spear_keeps_reach_and_an_axe_opens_wounds() {
    let mut w = world();
    let spear = fighter(
        &mut w,
        "Spear",
        RAIDERS,
        Vec3::new(0.0, 0.0, 3.2),
        -Vec3::Z,
        weapon::spear(),
    );
    let axe = fighter(
        &mut w,
        "Axe",
        RAIDERS,
        Vec3::new(1.6, 0.0, 0.0),
        -Vec3::X,
        weapon::axe(),
    );
    let hero = fighter(&mut w, "Hero", PARTY, Vec3::ZERO, Vec3::Z, weapon::sword());
    let events = run(&mut w, 0.8, |w, i| {
        if i == 0 {
            w.command(spear, attack(w, hero));
            w.command(axe, attack(w, hero));
        }
    });
    let from = |who| {
        events
            .iter()
            .filter(|e| matches!(e, CombatEvent::Damaged { source: Some(s), target, .. } if *s == who && *target == hero))
            .count()
    };
    assert_eq!(from(spear), 1, "the spear reaches 3.2 m");
    assert_eq!(from(axe), 1);
    assert!(w.combatant(hero).unwrap().status.buildup(StatusKind::Bleed) > 0.0);
}
