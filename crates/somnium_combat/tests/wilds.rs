//! Tyffir's enemies: a rime wolf pack (circling, darting in two at a time,
//! leaping from behind, breaking when the alpha falls) and a bandit band
//! (spear, axe, bow; scattering when the chief falls).
mod common;
use common::*;
use glam::Vec3;
use somnium_combat::geometry::flat;
use somnium_combat::wilds::{BanditKind, bandit_band, wolf_pack};
use somnium_combat::{
    CombatEvent, CombatWorld, CombatantDesc, CombatantId, Command, TeamId, weapon,
};

const WILD: TeamId = TeamId(2);

/// A hero who stands and takes it, facing +Z.
fn hero(w: &mut CombatWorld) -> CombatantId {
    w.relations
        .set(PARTY, WILD, somnium_combat::Attitude::Hostile);
    let mut desc = CombatantDesc::new("Hero", PARTY, Vec3::ZERO, weapon::sword());
    desc.vitals = somnium_combat::Vitals::new(5000.0, 100.0, 0.0, 9999.0);
    w.spawn(desc)
}

fn kill(w: &mut CombatWorld, id: CombatantId) {
    // A fall from a great height does it.
    w.landed(id, 200.0);
}

fn doing(w: &CombatWorld, id: CombatantId) -> &'static str {
    w.combatant(id)
        .and_then(|c| c.brain.as_ref())
        .map_or("", |b| b.mind.doing)
}

#[test]
fn a_wolf_pack_circles_darts_in_two_at_a_time_and_bites() {
    let mut w = world();
    let hero = hero(&mut w);
    let pack = wolf_pack(&mut w, WILD, Vec3::new(0.0, 0.0, 14.0), 5);
    // The alpha catches the scent (they face away); its howl calls the rest.
    somnium_combat::ai::alert(&mut w, pack[0], hero, Vec3::ZERO);
    let (mut crowded, mut steps, mut circled, mut behind) = (0usize, 0usize, 0usize, 0usize);
    let mut bites = Vec::new();
    let mut swings: std::collections::BTreeMap<CombatantId, u32> = Default::default();
    let events = run(&mut w, 25.0, |w, _| {
        w.command(hero, Command::default());
        let at = w.combatant(hero).unwrap().position;
        let close = pack
            .iter()
            .filter(|&&p| {
                w.combatant(p)
                    .is_some_and(|c| flat(c.position - at).length() < 1.7)
            })
            .count();
        crowded += usize::from(close > 2);
        steps += 1;
        circled += pack.iter().filter(|&&p| doing(w, p) == "Circling").count();
        // A move just begun: where did it come from?
        for &p in &pack {
            if let Some(c) = w.combatant(p)
                && swings.insert(p, c.swings).is_some_and(|s| s != c.swings)
                && let somnium_combat::Action::Attack(a) = &c.action
            {
                bites.push((p, a.def.name.clone()));
                behind += usize::from(c.position.z < at.z - 0.3);
            }
        }
    });
    let bitten: f32 = events
        .iter()
        .filter_map(|e| match e {
            CombatEvent::Damaged {
                target,
                source: Some(s),
                amount,
                ..
            } if *target == hero && pack.contains(s) => Some(*amount),
            _ => None,
        })
        .sum::<f32>();
    assert!(bitten > 40.0, "the pack drew blood: {bitten}");
    assert!(
        (crowded as f32) < steps as f32 * 0.05,
        "rarely more than two at the throat: {crowded} of {steps} steps"
    );
    assert!(circled > steps, "the rest circled: {circled} wolf-steps");
    assert!(
        events.iter().any(|e| matches!(
            e,
            CombatEvent::AttackStarted { name, .. } if name == "wolf_bite" || name == "wolf_leap"
        )),
        "{bites:?}"
    );
    assert!(behind > 0, "some came from behind: {bites:?}");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::Alerted { by, .. } if pack.contains(by))),
        "a howl called the pack"
    );
}

#[test]
fn the_pack_breaks_and_runs_when_the_alpha_falls() {
    let mut w = world();
    let hero = hero(&mut w);
    let pack = wolf_pack(&mut w, WILD, Vec3::new(0.0, 0.0, 8.0), 4);
    somnium_combat::ai::alert(&mut w, pack[0], hero, Vec3::ZERO);
    run(&mut w, 4.0, |w, _| w.command(hero, Command::default()));
    let alpha = pack[0];
    kill(&mut w, alpha);
    let events = run(&mut w, 6.0, |w, _| w.command(hero, Command::default()));
    for &wolf in &pack[1..] {
        assert!(
            events
                .iter()
                .any(|e| matches!(e, CombatEvent::Fled { who } if *who == wolf)),
            "{wolf:?} ran"
        );
        let gap = flat(w.combatant(wolf).unwrap().position).length();
        assert!(gap > 12.0, "{wolf:?} is away: {gap} m");
    }
}

#[test]
fn a_bandit_band_fights_with_spear_axe_and_bow_and_scatters_without_its_chief() {
    let mut w = world();
    w.relations
        .set(PARTY, RAIDERS, somnium_combat::Attitude::Hostile);
    let hero = hero(&mut w);
    let band = bandit_band(&mut w, RAIDERS, Vec3::new(0.0, 0.0, 16.0), &BanditKind::ALL);
    let [chief, spear, axe, bow] = band[..] else {
        panic!("four bandits");
    };
    // The chief spots the hero; the shout turns the band.
    somnium_combat::ai::alert(&mut w, chief, hero, Vec3::ZERO);
    let mut bow_gap = f32::MAX;
    let events = run(&mut w, 12.0, |w, _| {
        w.command(hero, Command::default());
        let at = w.combatant(hero).unwrap().position;
        bow_gap = bow_gap.min(flat(w.combatant(bow).unwrap().position - at).length());
    });
    let from = |who: CombatantId| {
        events
            .iter()
            .filter(|e| matches!(e, CombatEvent::Damaged { target, source: Some(s), .. } if *target == hero && *s == who))
            .count()
    };
    assert!(from(spear) > 0 && from(axe) > 0, "spear and axe struck");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CombatEvent::ShotFired { shooter, .. } if *shooter == bow)),
        "the bow loosed"
    );
    assert!(bow_gap > 6.0, "the bow kept its distance: {bow_gap} m");
    let morale = |w: &CombatWorld, id: CombatantId| {
        w.combatant(id)
            .and_then(|c| c.brain.as_ref())
            .map_or(0.0, |b| b.mind.morale)
    };
    // One falls: the rest are shaken but hold.
    let before = morale(&w, bow);
    kill(&mut w, axe);
    idle(&mut w, 0.1);
    assert!(morale(&w, bow) < before - 0.15, "shaken");
    assert!(
        !w.combatant(spear)
            .unwrap()
            .brain
            .as_ref()
            .unwrap()
            .mind
            .fleeing,
        "but holding"
    );
    // The chief falls: they scatter.
    kill(&mut w, chief);
    let events = run(&mut w, 5.0, |w, _| w.command(hero, Command::default()));
    for who in [spear, bow] {
        assert!(
            events
                .iter()
                .any(|e| matches!(e, CombatEvent::Fled { who: f } if *f == who)),
            "{who:?} ran"
        );
    }
}

#[test]
fn a_wolf_from_the_editor_has_a_wolfs_body_and_coat() {
    let c = somnium_combat::components::CombatantComponent {
        archetype: somnium_combat::ai::Archetype::Wolf as u32,
        weapon: somnium_combat::weapon::WeaponKind::Fangs as u32,
        ..Default::default()
    };
    let desc = c.desc("Wolf", Vec3::ZERO, Vec3::Z, 0);
    assert!(desc.body.radius < 0.5 && desc.body.height < 1.1);
    let frost = desc.resist.values[somnium_combat::DamageType::Frost as usize];
    assert!(frost > 0.5, "frost slides off a rime wolf: {frost}");
    assert_eq!(desc.weapon.kind, somnium_combat::weapon::WeaponKind::Fangs);
}
