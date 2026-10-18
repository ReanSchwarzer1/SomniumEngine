//! Spells: the six parts, the three speeds, the shapes, and the ground's answer.
mod common;
use common::*;
use glam::{Vec2, Vec3};
use somnium_combat::environment::{EnvironmentField, Surface};
use somnium_combat::spell::{
    Address, Bearing, CastSpeed, Discipline, Magic, SpellError, SpellRequest, library,
};
use somnium_combat::{
    CombatEvent, CombatWorld, CombatantDesc, CombatantId, Command, Press, Rank, StatusKind, TeamId,
    Vitals, weapon,
};

fn mage(w: &mut CombatWorld, team: TeamId, at: Vec3, disciplines: &[Discipline]) -> CombatantId {
    let mut desc = CombatantDesc::new("Mage", team, at, weapon::unarmed());
    desc.vitals = Vitals::new(100.0, 100.0, 200.0, 40.0);
    let mut magic = Magic {
        speeds: 0b111,
        numbers: true,
        ..Magic::default()
    };
    for d in disciplines {
        magic = magic.with_rank(*d, Rank::Saint);
    }
    desc.magic = magic;
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

fn field() -> EnvironmentField {
    EnvironmentField::new(Vec2::splat(-20.0), Vec2::splat(20.0), 0.5).unwrap()
}

fn released(events: &[CombatEvent]) -> usize {
    count(events, |e| matches!(e, CombatEvent::SpellReleased { .. }))
}

#[test]
fn an_ice_lance_gathers_releases_strikes_chills_and_cools_down() {
    let mut w = world();
    let dummy = fighter(
        &mut w,
        "Dummy",
        RAIDERS,
        Vec3::ZERO,
        Vec3::Z,
        weapon::unarmed(),
    );
    let caster = mage(
        &mut w,
        PARTY,
        Vec3::new(0.0, 0.0, 10.0),
        &[Discipline::Rime],
    );
    // No refilling while we count what it cost.
    w.combatant_mut(caster).unwrap().vitals.mana_regen = 0.0;
    let mana = w.combatant(caster).unwrap().vitals.mana.current;
    assert_eq!(
        w.cast(
            caster,
            request("ice_lance", Bearing::Target(dummy), CastSpeed::ShortChant)
        ),
        Ok(())
    );
    let early = idle(&mut w, 0.45);
    assert_eq!(
        released(&early),
        0,
        "a short-chant still takes half a second"
    );
    let later = idle(&mut w, 0.5);
    assert_eq!(released(&later), 1);
    let harm = damage_to(&later, dummy);
    assert!(harm > 30.0 && harm < 50.0, "the lance did {harm}");
    assert!(w.combatant(dummy).unwrap().status.active(StatusKind::Chill));
    let spent = mana - w.combatant(caster).unwrap().vitals.mana.current;
    let expected = library::get("ice_lance")
        .unwrap()
        .cost(&request(
            "ice_lance",
            Bearing::Target(dummy),
            CastSpeed::ShortChant,
        ))
        .mana;
    assert!(
        (spent - expected).abs() < 0.2,
        "spent {spent}, cost {expected}"
    );
    // Past the release's recovery (0.35 s), inside its cooldown (0.8 s).
    idle(&mut w, 0.1);
    assert_eq!(
        w.cast(
            caster,
            request("ice_lance", Bearing::Target(dummy), CastSpeed::ShortChant)
        ),
        Err(SpellError::Cooldown)
    );
}

#[test]
fn rime_bloom_freezes_the_pond_and_the_wet_man_standing_in_it() {
    let mut w = world();
    let mut ground = field();
    ground.paint_circle(Vec2::ZERO, 3.0, Surface::Water);
    w.environment = Some(ground);
    let raider = fighter(
        &mut w,
        "Raider",
        RAIDERS,
        Vec3::ZERO,
        Vec3::Z,
        weapon::spear(),
    );
    idle(&mut w, 0.1);
    assert!(
        w.combatant(raider).unwrap().status.active(StatusKind::Wet),
        "standing in water soaks"
    );
    let caster = mage(
        &mut w,
        PARTY,
        Vec3::new(0.0, 0.0, 10.0),
        &[Discipline::Rime],
    );
    assert_eq!(
        w.cast(
            caster,
            request("rime_bloom", Bearing::Cursor(Vec3::ZERO), CastSpeed::Silent)
        ),
        Ok(())
    );
    let events = idle(&mut w, 1.0);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::BloomGathering { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Bloomed { .. }))
    );
    assert!(
        w.combatant(raider)
            .unwrap()
            .status
            .active(StatusKind::Freeze),
        "{events:#?}"
    );
    let field = w.environment.as_ref().unwrap();
    assert_eq!(field.surface_at(Vec3::new(1.0, 0.0, 1.0)), Surface::Ice);
    assert!(field.surface_at(Vec3::new(1.0, 0.0, 1.0)).bears_weight());
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Surface(c) if c.to == Surface::Ice))
    );
}

#[test]
fn fireburst_lights_the_grass_and_the_fire_keeps_burning_whoever_stands_in_it() {
    let mut w = world();
    let mut ground = field();
    ground.paint_rect(Vec2::splat(-20.0), Vec2::splat(20.0), Surface::Grass);
    w.environment = Some(ground);
    let raider = fighter(
        &mut w,
        "Raider",
        RAIDERS,
        Vec3::ZERO,
        Vec3::Z,
        weapon::axe(),
    );
    let caster = mage(
        &mut w,
        PARTY,
        Vec3::new(0.0, 0.0, 12.0),
        &[Discipline::Flame],
    );
    assert_eq!(
        w.cast(
            caster,
            request("fireburst", Bearing::Cursor(Vec3::ZERO), CastSpeed::Silent)
        ),
        Ok(())
    );
    let first = idle(&mut w, 0.6);
    let burst = damage_to(&first, raider);
    assert!(burst > 0.0);
    let lit = w.environment.as_ref().unwrap().census()[Surface::Burning as usize];
    assert!(lit > 50, "the bloom lit its circle: {lit}");
    let later = idle(&mut w, 3.0);
    assert!(
        damage_to(&later, raider) > 5.0,
        "the ground kept burning him"
    );
    let census = w.environment.as_ref().unwrap().census();
    assert!(
        census[Surface::Burning as usize] + census[Surface::Ash as usize] > lit,
        "and it spread"
    );
}

#[test]
fn a_water_bolt_puts_out_a_burning_man() {
    let mut w = world();
    let raider = fighter(
        &mut w,
        "Raider",
        RAIDERS,
        Vec3::ZERO,
        Vec3::Z,
        weapon::axe(),
    );
    w.apply_status(raider, StatusKind::Burn, 5.0);
    let caster = mage(&mut w, PARTY, Vec3::new(0.0, 0.0, 8.0), &[Discipline::Rime]);
    assert_eq!(
        w.cast(
            caster,
            request("water_bolt", Bearing::Target(raider), CastSpeed::Silent)
        ),
        Ok(())
    );
    let events = idle(&mut w, 0.6);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Extinguished { target } if *target == raider))
    );
    let c = w.combatant(raider).unwrap();
    assert!(!c.status.active(StatusKind::Burn));
    assert!(c.status.active(StatusKind::Wet));
}

#[test]
fn an_ice_wall_stops_a_runner() {
    let mut w = world();
    let runner = fighter(
        &mut w,
        "Runner",
        RAIDERS,
        Vec3::ZERO,
        Vec3::Z,
        weapon::spear(),
    );
    let caster = mage(
        &mut w,
        PARTY,
        Vec3::new(0.0, 0.0, -2.0),
        &[Discipline::Rime],
    );
    assert_eq!(
        w.cast(
            caster,
            request(
                "ice_wall",
                Bearing::Cursor(Vec3::new(0.0, 0.0, 6.0)),
                CastSpeed::Silent
            )
        ),
        Ok(())
    );
    let events = run(&mut w, 4.0, |w, _| {
        w.command(
            runner,
            Command {
                move_dir: Vec3::Z,
                ..Command::default()
            },
        );
    });
    assert!(events.iter().any(|e| matches!(
        e,
        CombatEvent::WallRaised {
            blocks_movement: true,
            ..
        }
    )));
    let z = w.combatant(runner).unwrap().position.z;
    assert!(z < 5.65 && z > 4.5, "the runner stopped at the ice: {z}");
}

/// A host's controller meets the same wall through `wall_contact`.
#[test]
fn a_host_controller_meets_the_ice_wall_too() {
    let mut w = world();
    let caster = mage(
        &mut w,
        PARTY,
        Vec3::new(0.0, 0.0, -2.0),
        &[Discipline::Rime],
    );
    let wall = request(
        "ice_wall",
        Bearing::Cursor(Vec3::new(0.0, 0.0, 6.0)),
        CastSpeed::Silent,
    );
    assert_eq!(w.cast(caster, wall), Ok(()));
    run(&mut w, 0.5, |_, _| {});
    let feet = Vec3::new(0.0, 0.0, 3.0);
    let hit = w
        .wall_contact(feet, Vec3::Z * 4.0, 0.35, 1.8)
        .expect("the wall is in the way");
    let stop = feet.z + 4.0 * hit.fraction;
    assert!(
        stop > 4.5 && stop < 5.8 && hit.normal.z < -0.9,
        "{stop} {hit:?}"
    );
    let beside = Vec3::new(9.0, 0.0, 3.0);
    assert!(w.wall_contact(beside, Vec3::Z * 4.0, 0.35, 1.8).is_none());
}

#[test]
fn a_hard_blow_breaks_a_short_chant_but_not_an_incantation() {
    let cast_while_struck = |release: CastSpeed| {
        let mut w = world();
        let hero = fighter(
            &mut w,
            "Hero",
            PARTY,
            Vec3::new(0.0, 0.0, -2.0),
            Vec3::Z,
            weapon::sword(),
        );
        let caster = mage(&mut w, RAIDERS, Vec3::ZERO, &[Discipline::Rime]);
        w.combatant_mut(caster).unwrap().facing = -Vec3::Z;
        assert_eq!(
            w.cast(caster, request("ice_lance", Bearing::Target(hero), release)),
            Ok(())
        );
        run(&mut w, 1.8, |w, i| {
            if i == 0 {
                w.command(
                    hero,
                    Command {
                        attack: Press::click(),
                        ..facing(w, caster)
                    },
                );
            }
        })
    };
    let chant = cast_while_struck(CastSpeed::ShortChant);
    assert!(
        chant
            .iter()
            .any(|e| matches!(e, CombatEvent::SpellInterrupted { .. })),
        "{chant:#?}"
    );
    assert_eq!(released(&chant), 0);
    let verse = cast_while_struck(CastSpeed::Incantation);
    assert!(
        !verse
            .iter()
            .any(|e| matches!(e, CombatEvent::SpellInterrupted { .. }))
    );
    assert_eq!(released(&verse), 1, "the full verse holds through a cut");
}

#[test]
fn silent_casting_is_instant_and_costs_focus() {
    let mut w = world();
    let dummy = fighter(
        &mut w,
        "Dummy",
        RAIDERS,
        Vec3::ZERO,
        Vec3::Z,
        weapon::unarmed(),
    );
    let caster = mage(
        &mut w,
        PARTY,
        Vec3::new(0.0, 0.0, 8.0),
        &[Discipline::Flame],
    );
    let focus = w.combatant(caster).unwrap().vitals.focus.current;
    assert_eq!(
        w.cast(
            caster,
            request("ember_bolt", Bearing::Target(dummy), CastSpeed::Silent)
        ),
        Ok(())
    );
    assert_eq!(released(w.events()), 1, "released the moment it was asked");
    assert!(w.combatant(caster).unwrap().vitals.focus.current < focus - 15.0);
}

#[test]
fn overdraw_casts_on_credit_only_when_allowed() {
    let mut w = world();
    let dummy = fighter(
        &mut w,
        "Dummy",
        RAIDERS,
        Vec3::ZERO,
        Vec3::Z,
        weapon::unarmed(),
    );
    let caster = mage(&mut w, PARTY, Vec3::new(0.0, 0.0, 8.0), &[Discipline::Rime]);
    w.combatant_mut(caster).unwrap().vitals.mana.current = 2.0;
    assert_eq!(
        w.cast(
            caster,
            request("ice_lance", Bearing::Target(dummy), CastSpeed::Silent)
        ),
        Err(SpellError::NoMana)
    );
    w.combatant_mut(caster).unwrap().magic.allow_overdraw = true;
    assert_eq!(
        w.cast(
            caster,
            request("ice_lance", Bearing::Target(dummy), CastSpeed::Silent)
        ),
        Ok(())
    );
    assert!(
        w.events()
            .iter()
            .any(|e| matches!(e, CombatEvent::Overdrawn { .. }))
    );
    let c = w.combatant(caster).unwrap();
    assert!(c.vitals.overdrawn());
    assert_eq!(c.vitals.overdraws, 1);
}

#[test]
fn a_chorus_shares_the_cost_by_capacity() {
    let mut w = world();
    fighter(
        &mut w,
        "Dummy",
        RAIDERS,
        Vec3::ZERO,
        Vec3::Z,
        weapon::unarmed(),
    );
    let lead = mage(
        &mut w,
        PARTY,
        Vec3::new(0.0, 0.0, 10.0),
        &[Discipline::Rime],
    );
    let a = mage(
        &mut w,
        PARTY,
        Vec3::new(1.5, 0.0, 10.0),
        &[Discipline::Rime],
    );
    let b = mage(
        &mut w,
        PARTY,
        Vec3::new(-1.5, 0.0, 10.0),
        &[Discipline::Rime],
    );
    w.combatant_mut(lead).unwrap().vitals.mana.current = 1.0;
    let mut r = request("rime_bloom", Bearing::Cursor(Vec3::ZERO), CastSpeed::Silent);
    r.address = Address::Chorus(vec![a, b]);
    assert_eq!(w.cast(lead, r), Ok(()));
    idle(&mut w, 1.0);
    let paid = |id| 200.0 - w.combatant(id).unwrap().vitals.mana.current;
    assert!(
        paid(a) > 1.0 && (paid(a) - paid(b)).abs() < 0.01,
        "both members paid alike"
    );
    // A chorus of nobody is refused.
    let mut r = request("ice_lance", Bearing::Cursor(Vec3::ZERO), CastSpeed::Silent);
    r.address = Address::Chorus(vec![CombatantId(999)]);
    assert_eq!(w.cast(lead, r), Err(SpellError::BadChorus));
}

#[test]
fn holding_a_spell_key_shapes_it_with_the_wheel_and_the_slot_remembers() {
    let shape = |numbers: bool| {
        let mut w = world();
        let dummy = fighter(
            &mut w,
            "Dummy",
            RAIDERS,
            Vec3::ZERO,
            Vec3::Z,
            weapon::unarmed(),
        );
        let caster = mage(
            &mut w,
            PARTY,
            Vec3::new(0.0, 0.0, 10.0),
            &[Discipline::Rime],
        );
        {
            let c = w.combatant_mut(caster).unwrap();
            c.magic.numbers = numbers;
            c.magic.hotbar[0] = Some(library::get("ice_lance").unwrap().slot());
        }
        let mut shaped = None;
        let events = run(&mut w, 2.4, |w, i| {
            let mut cmd = Command {
                aim: Some(Vec3::ZERO),
                ..Command::default()
            };
            cmd.spells[0] = match i {
                0 => Press::tap(),
                1..=5 => Press::hold(),
                6 => Press::release(),
                _ => Press::default(),
            };
            if i == 2 {
                cmd.measure_steps = 3.0;
            }
            if i == 5 {
                shaped = w.combatant(caster).unwrap().shaping.clone();
            }
            w.command(caster, cmd);
        });
        let shaping = shaped.expect("shaping while the key is held");
        assert!(shaping.preview.is_ok());
        assert_eq!(released(&events), 1);
        assert!(damage_to(&events, dummy) > 0.0);
        w.combatant(caster).unwrap().magic.hotbar[0]
            .clone()
            .unwrap()
            .measure
            .size
    };
    let numbered = shape(true);
    assert!(
        (numbered - 0.5 * 1.15f32.powi(3)).abs() < 1e-3,
        "numbers take the exact size: {numbered}"
    );
    let rote = shape(false);
    assert_eq!(rote, 1.0, "rote snaps to a span");
}

#[test]
fn frost_step_skates_across_water_and_leaves_ice() {
    let mut w = world();
    let mut ground = field();
    ground.paint_rect(Vec2::new(-1.0, 2.0), Vec2::new(1.0, 6.0), Surface::Water);
    w.environment = Some(ground);
    let caster = mage(&mut w, PARTY, Vec3::ZERO, &[Discipline::Rime]);
    assert_eq!(
        w.cast(
            caster,
            request(
                "frost_step",
                Bearing::Cursor(Vec3::new(0.0, 0.0, 10.0)),
                CastSpeed::Silent
            )
        ),
        Ok(())
    );
    let events = idle(&mut w, 1.2);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Stepped { .. }))
    );
    let z = w.combatant(caster).unwrap().position.z;
    assert!(z > 8.0, "skated to {z}");
    let field = w.environment.as_ref().unwrap();
    assert_eq!(field.surface_at(Vec3::new(0.0, 0.0, 4.0)), Surface::Ice);
}

#[test]
fn a_shield_ward_absorbs_until_it_breaks() {
    let mut w = world();
    let caster = mage(&mut w, PARTY, Vec3::ZERO, &[Discipline::Warding]);
    let raider = fighter(
        &mut w,
        "Raider",
        RAIDERS,
        Vec3::new(0.0, 0.0, 1.8),
        -Vec3::Z,
        weapon::sword(),
    );
    assert_eq!(
        w.cast(
            caster,
            request("shield", Bearing::AroundSelf, CastSpeed::Silent)
        ),
        Ok(())
    );
    assert!(
        w.events()
            .iter()
            .any(|e| matches!(e, CombatEvent::WardRaised { .. }))
    );
    let first = run(&mut w, 0.9, |w, i| {
        if i == 0 {
            w.command(
                raider,
                Command {
                    attack: Press::click(),
                    ..facing(w, caster)
                },
            );
        }
    });
    assert_eq!(damage_to(&first, caster), 0.0, "the ward took the cut");
    let second = run(&mut w, 0.9, |w, i| {
        if i == 0 {
            w.command(
                raider,
                Command {
                    attack: Press::click(),
                    ..facing(w, caster)
                },
            );
        }
    });
    assert!(
        second
            .iter()
            .any(|e| matches!(e, CombatEvent::WardBroken { .. }))
    );
    assert!(damage_to(&second, caster) > 0.0);
}

#[test]
fn close_wound_heals_an_ally() {
    let mut w = world();
    let friend = fighter(
        &mut w,
        "Friend",
        PARTY,
        Vec3::ZERO,
        Vec3::Z,
        weapon::spear(),
    );
    w.combatant_mut(friend).unwrap().vitals.health.current = 40.0;
    let mender = mage(
        &mut w,
        PARTY,
        Vec3::new(0.0, 0.0, 6.0),
        &[Discipline::Mending],
    );
    assert_eq!(
        w.cast(
            mender,
            request("close_wound", Bearing::Target(friend), CastSpeed::Silent)
        ),
        Ok(())
    );
    let events = idle(&mut w, 0.6);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Healed { target, .. } if *target == friend))
    );
    assert!(health(&w, friend) > 40.0);
}

#[test]
fn a_gust_wave_strikes_the_arc_in_front_and_not_behind() {
    let mut w = world();
    let caster = mage(&mut w, PARTY, Vec3::ZERO, &[Discipline::Gale]);
    let front = fighter(
        &mut w,
        "Front",
        RAIDERS,
        Vec3::new(0.0, 0.0, 4.0),
        -Vec3::Z,
        weapon::axe(),
    );
    let behind = fighter(
        &mut w,
        "Behind",
        RAIDERS,
        Vec3::new(0.0, 0.0, -4.0),
        Vec3::Z,
        weapon::axe(),
    );
    assert_eq!(
        w.cast(
            caster,
            request(
                "gust",
                Bearing::Cursor(Vec3::new(0.0, 0.0, 8.0)),
                CastSpeed::Silent
            )
        ),
        Ok(())
    );
    let events = idle(&mut w, 1.0);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::WaveSent { .. }))
    );
    assert!(damage_to(&events, front) > 0.0);
    assert_eq!(damage_to(&events, behind), 0.0);
    assert!(w.combatant(front).unwrap().position.z > 4.5, "pushed back");
}
