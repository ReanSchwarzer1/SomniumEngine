//! Bows, Lattice rifles, thrown knives, and what walls and the Mirror do to them.
mod common;
use common::*;
use glam::Vec3;
use somnium_combat::schools::{SwordSchool, Swordsmanship};
use somnium_combat::spell::{
    Address, Bearing, CastSpeed, Discipline, Magic, SpellRequest, library,
};
use somnium_combat::{
    Action, CombatEvent, CombatWorld, CombatantDesc, CombatantId, Command, HitFlags, Press, Rank,
    weapon,
};

fn shots(events: &[CombatEvent]) -> usize {
    count(events, |e| matches!(e, CombatEvent::ShotFired { .. }))
}

fn mage(
    w: &mut CombatWorld,
    team: somnium_combat::TeamId,
    at: Vec3,
    discipline: Discipline,
) -> CombatantId {
    let mut desc = CombatantDesc::new("Mage", team, at, weapon::unarmed());
    desc.vitals = somnium_combat::Vitals::new(100.0, 100.0, 200.0, 40.0);
    desc.magic = Magic {
        speeds: 0b111,
        numbers: true,
        ..Magic::default()
    }
    .with_rank(discipline, Rank::Saint);
    w.spawn(desc)
}

fn request(spell: &str, bearing: Bearing, release: CastSpeed) -> SpellRequest {
    let asset = library::get(spell).unwrap();
    SpellRequest {
        spell: asset.id.clone(),
        address: Address::Well,
        element: asset.element,
        shape: asset.default_shape,
        measure: asset.default_measure,
        bearing,
        release,
    }
}

#[test]
fn a_lattice_rifle_steadies_fires_hits_hard_and_reloads() {
    let mut w = world();
    let rifle = fighter(
        &mut w,
        "Rifleman",
        RAIDERS,
        Vec3::new(0.0, 0.0, 20.0),
        -Vec3::Z,
        weapon::lattice_rifle(),
    );
    let dummy = fighter(
        &mut w,
        "Dummy",
        PARTY,
        Vec3::ZERO,
        Vec3::Z,
        weapon::unarmed(),
    );
    let events = run(&mut w, 1.0, |w, i| {
        let cmd = Command {
            attack: if i == 0 {
                Press::click()
            } else {
                Press::default()
            },
            ..facing(w, dummy)
        };
        w.command(rifle, cmd);
    });
    assert_eq!(shots(&events), 1);
    assert!(
        (damage_to(&events, dummy) - 70.0).abs() < 0.01,
        "{events:#?}"
    );
    let c = w.combatant(rifle).unwrap();
    assert_eq!(c.ammo, 0);
    assert!(
        matches!(c.action, Action::Reload { .. }),
        "one round, then the long reload"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Noise { loudness, .. } if *loudness >= 1.0)),
        "everyone hears it"
    );
    idle(&mut w, 3.2);
    assert_eq!(w.combatant(rifle).unwrap().ammo, 1);
}

#[test]
fn a_full_draw_strikes_harder_than_a_snatched_shot() {
    let shoot = |hold: usize| {
        let mut w = world();
        let archer = fighter(
            &mut w,
            "Archer",
            RAIDERS,
            Vec3::new(0.0, 0.0, 15.0),
            -Vec3::Z,
            weapon::bow(),
        );
        let dummy = fighter(
            &mut w,
            "Dummy",
            PARTY,
            Vec3::ZERO,
            Vec3::Z,
            weapon::unarmed(),
        );
        let events = run(&mut w, 2.5, |w, i| {
            let attack = match i {
                0 => Press::tap(),
                i if i < hold => Press::hold(),
                i if i == hold => Press::release(),
                _ => Press::default(),
            };
            w.command(
                archer,
                Command {
                    attack,
                    ..facing(w, dummy)
                },
            );
        });
        assert_eq!(shots(&events), 1);
        damage_to(&events, dummy)
    };
    let full = shoot(60);
    let snatched = shoot(22);
    assert!((full - 30.0).abs() < 0.01, "full draw {full}");
    assert!(
        snatched > 0.0 && snatched < full * 0.75,
        "snatched {snatched}"
    );
}

#[test]
fn an_arrow_in_the_head_is_a_critical() {
    let mut w = world();
    let archer = fighter(
        &mut w,
        "Archer",
        RAIDERS,
        Vec3::new(0.0, 0.0, 12.0),
        -Vec3::Z,
        weapon::bow(),
    );
    let dummy = fighter(
        &mut w,
        "Dummy",
        PARTY,
        Vec3::ZERO,
        Vec3::Z,
        weapon::unarmed(),
    );
    let events = run(&mut w, 2.0, |w, i| {
        let attack = match i {
            0 => Press::tap(),
            1..=59 => Press::hold(),
            60 => Press::release(),
            _ => Press::default(),
        };
        // Aimed at the head: the cursor point plus the chest offset lands there.
        let aim = Some(Vec3::new(0.0, 0.55, 0.0));
        w.command(
            archer,
            Command {
                attack,
                aim,
                ..Command::default()
            },
        );
    });
    let (amount, flags) = events
        .iter()
        .find_map(|e| match e {
            CombatEvent::Damaged {
                target,
                amount,
                flags,
                ..
            } if *target == dummy => Some((*amount, *flags)),
            _ => None,
        })
        .expect("the arrow struck");
    assert!(flags.contains(HitFlags::CRITICAL));
    assert!((amount - 30.0 * 1.8).abs() < 0.01, "head shot {amount}");
}

#[test]
fn a_stone_wall_stops_arrows_and_a_gale_wall_throws_them_aside() {
    for (spell, discipline) in [
        ("raise_wall", Discipline::Stone),
        ("gale_wall", Discipline::Gale),
    ] {
        let mut w = world();
        let archer = fighter(
            &mut w,
            "Archer",
            RAIDERS,
            Vec3::new(0.0, 0.0, 16.0),
            -Vec3::Z,
            weapon::bow(),
        );
        let dummy = fighter(
            &mut w,
            "Dummy",
            PARTY,
            Vec3::ZERO,
            Vec3::Z,
            weapon::unarmed(),
        );
        let caster = mage(
            &mut w,
            somnium_combat::TeamId(0),
            Vec3::new(2.0, 0.0, 0.0),
            discipline,
        );
        let raised = w.cast(
            caster,
            request(
                spell,
                Bearing::Cursor(Vec3::new(0.0, 0.0, 6.0)),
                CastSpeed::Silent,
            ),
        );
        assert_eq!(raised, Ok(()));
        assert!(
            w.drain_events()
                .iter()
                .any(|e| matches!(e, CombatEvent::WallRaised { .. })),
            "{spell} stands"
        );
        let events = run(&mut w, 2.5, |w, i| {
            let attack = match i {
                0 => Press::tap(),
                1..=59 => Press::hold(),
                60 => Press::release(),
                _ => Press::default(),
            };
            w.command(
                archer,
                Command {
                    attack,
                    ..facing(w, dummy)
                },
            );
        });
        assert_eq!(shots(&events), 1);
        assert_eq!(damage_to(&events, dummy), 0.0, "{spell} kept the arrow off");
    }
}

#[test]
fn the_meridians_mirror_sends_a_bolt_back_at_its_caster() {
    let mut w = world();
    let mut desc = CombatantDesc::new("Hero", PARTY, Vec3::ZERO, weapon::sword());
    desc.swordsmanship = Some(Swordsmanship::with_forms(
        SwordSchool::Meridian,
        3,
        Rank::Adept,
    ));
    let hero = w.spawn(desc);
    let caster = mage(&mut w, RAIDERS, Vec3::new(0.0, 0.0, 12.0), Discipline::Rime);
    w.combatant_mut(caster).unwrap().facing = -Vec3::Z;
    assert_eq!(
        w.cast(
            caster,
            request("ice_lance", Bearing::Target(hero), CastSpeed::Silent)
        ),
        Ok(())
    );
    let events = run(&mut w, 0.8, |w, i| {
        let guard = match i {
            4 => Press::tap(),
            5..=30 => Press::hold(),
            _ => Press::default(),
        };
        w.command(
            hero,
            Command {
                guard,
                ..facing(w, caster)
            },
        );
    });
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Reflected { defender, .. } if *defender == hero)),
        "{events:#?}"
    );
    assert_eq!(damage_to(&events, hero), 0.0);
    assert!(
        damage_to(&events, caster) > 0.0,
        "the caster ate its own lance"
    );
}

#[test]
fn a_rifle_round_passes_through_the_first_body() {
    let mut w = world();
    let rifle = fighter(
        &mut w,
        "Rifleman",
        RAIDERS,
        Vec3::new(0.0, 0.0, 20.0),
        -Vec3::Z,
        weapon::lattice_rifle(),
    );
    let front = fighter(
        &mut w,
        "Front",
        PARTY,
        Vec3::new(0.0, 0.0, 6.0),
        Vec3::Z,
        weapon::unarmed(),
    );
    let back = fighter(
        &mut w,
        "Back",
        PARTY,
        Vec3::ZERO,
        Vec3::Z,
        weapon::unarmed(),
    );
    let events = run(&mut w, 1.0, |w, i| {
        let cmd = Command {
            attack: if i == 0 {
                Press::click()
            } else {
                Press::default()
            },
            ..facing(w, back)
        };
        w.command(rifle, cmd);
    });
    assert!(damage_to(&events, front) > 0.0);
    assert!(
        damage_to(&events, back) > 0.0,
        "pierce 1 reaches the man behind"
    );
}

#[test]
fn a_knife_fighter_throws_with_the_art_key() {
    let mut w = world();
    let knife = fighter(
        &mut w,
        "Cutthroat",
        RAIDERS,
        Vec3::new(0.0, 0.0, 8.0),
        -Vec3::Z,
        weapon::knife(),
    );
    let dummy = fighter(
        &mut w,
        "Dummy",
        PARTY,
        Vec3::ZERO,
        Vec3::Z,
        weapon::unarmed(),
    );
    let events = run(&mut w, 1.0, |w, i| {
        w.command(
            knife,
            Command {
                art: i == 0,
                ..facing(w, dummy)
            },
        );
    });
    assert_eq!(shots(&events), 1);
    assert!(damage_to(&events, dummy) > 0.0);
    assert_eq!(
        w.combatant(knife).unwrap().ammo,
        1,
        "one of the pair is gone"
    );
}
