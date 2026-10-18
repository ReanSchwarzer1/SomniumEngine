//! Tyffir's enemies in the open: rime wolf packs and bandit bands.
//!
//! A **rime wolf** is a [`crate::ai::Archetype::Wolf`] with [`crate::weapon::fangs`]: it
//! circles its prey a few metres out, darts in when the pack's two attack
//! tokens give it an opening (from behind, it leaps), bites, and breaks away
//! to the ring again. The pack follows its alpha, howls far (a wolf's shout
//! carries 40 m), and breaks and runs when the alpha falls.
//!
//! A **bandit band** is a chief and its spears, axes and bows on the ordinary
//! styles (spearman, brute, archer), less skilled than soldiers and with less
//! nerve: every friend that falls near one shakes it, and when the chief falls
//! the band scatters.
//!
//! The bodies are placeholders' numbers for workstream 3's characters: a wolf
//! is long and low (0.42 m radius, 0.95 m tall), frost slides off its coat and
//! fire bites it.
use crate::ai::{Archetype, BrainDesc};
use crate::weapon::{self, WeaponDef};
use crate::world::{Body, CombatWorld, CombatantDesc};
use crate::{CombatantId, DamageType, Resistances, TeamId, Vitals};
use glam::Vec3;

/// A rime wolf's body, pools and coat, on a description (the editor's
/// Combatant with the Wolf style uses this).
pub fn wolf_body(desc: &mut CombatantDesc, alpha: bool) {
    desc.body = Body {
        radius: if alpha { 0.48 } else { 0.42 },
        height: if alpha { 1.05 } else { 0.95 },
        mass: if alpha { 70.0 } else { 55.0 },
        speed: 5.0,
    };
    desc.vitals = if alpha {
        Vitals::new(110.0, 160.0, 0.0, 45.0)
    } else {
        Vitals::new(75.0, 140.0, 0.0, 30.0)
    };
    desc.resist = Resistances::default()
        .with(DamageType::Frost, 0.6)
        .with(DamageType::Flame, -0.25);
}

/// One rime wolf, its brain at home where it stands.
#[must_use]
pub fn rime_wolf(name: &str, team: TeamId, at: Vec3, alpha: bool) -> CombatantDesc {
    let mut desc = CombatantDesc::new(name, team, at, weapon::fangs());
    wolf_body(&mut desc, alpha);
    let mut brain = BrainDesc::new(Archetype::Wolf);
    brain.home = Some(at);
    desc.brain = Some(brain);
    desc
}

/// A pack of `count` (at least one) round `centre`: the alpha first, the
/// rest following it. Returns their ids, alpha first.
pub fn wolf_pack(
    w: &mut CombatWorld,
    team: TeamId,
    centre: Vec3,
    count: usize,
) -> Vec<CombatantId> {
    let alpha = w.spawn(rime_wolf("Rime wolf (alpha)", team, centre, true));
    let mut pack = vec![alpha];
    let others = count.max(1) - 1;
    for i in 0..others {
        let a = i as f32 / others.max(1) as f32 * std::f32::consts::TAU;
        let at = centre + Vec3::new(a.cos(), 0.0, a.sin()) * 2.5;
        let mut desc = rime_wolf(&format!("Rime wolf {}", i + 1), team, at, false);
        if let Some(brain) = &mut desc.brain {
            brain.leader = Some(alpha);
        }
        pack.push(w.spawn(desc));
    }
    pack
}

/// What a bandit carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BanditKind {
    /// The chief: an axe, more health, more skill; the band follows it.
    Chief,
    /// A spear, kept at reach.
    Spear,
    /// An axe, walked through blows.
    Axe,
    /// A bow, from a distance.
    Bow,
}

impl BanditKind {
    /// Every kind.
    pub const ALL: [Self; 4] = [Self::Chief, Self::Spear, Self::Axe, Self::Bow];

    fn kit(self) -> (Archetype, WeaponDef, &'static str) {
        match self {
            Self::Chief => (Archetype::Duelist, weapon::axe(), "Bandit chief"),
            Self::Spear => (Archetype::Spearman, weapon::spear(), "Bandit (spear)"),
            Self::Axe => (Archetype::Brute, weapon::axe(), "Bandit (axe)"),
            Self::Bow => (Archetype::Archer, weapon::bow(), "Bandit (bow)"),
        }
    }
}

/// One bandit of `kind`, at home where it stands.
#[must_use]
pub fn bandit(kind: BanditKind, team: TeamId, at: Vec3) -> CombatantDesc {
    let (archetype, weapon, name) = kind.kit();
    let mut desc = CombatantDesc::new(name, team, at, weapon);
    let chief = kind == BanditKind::Chief;
    desc.vitals = if chief {
        Vitals::new(160.0, 120.0, 0.0, 60.0)
    } else {
        Vitals::new(100.0, 100.0, 0.0, 40.0)
    };
    desc.resist = Resistances::default().with_armor(if chief { 4.0 } else { 2.0 });
    let mut brain = BrainDesc::new(archetype);
    let mut profile = archetype.profile();
    // Rougher than soldiers, and with less nerve.
    profile.skill = if chief { 0.5 } else { 0.3 };
    profile.morale = if chief { 0.9 } else { 0.7 };
    profile.flee_below = profile.flee_below.max(0.3);
    profile.alert_radius = 25.0;
    profile.breaks_with_leader = !chief;
    brain.profile = Some(profile);
    brain.home = Some(at);
    desc.brain = Some(brain);
    desc
}

/// A band round `centre` (its camp): the first chief in `kinds` leads the
/// rest. Returns their ids in `kinds` order.
pub fn bandit_band(
    w: &mut CombatWorld,
    team: TeamId,
    centre: Vec3,
    kinds: &[BanditKind],
) -> Vec<CombatantId> {
    let spot = |i: usize| {
        let a = i as f32 / kinds.len().max(1) as f32 * std::f32::consts::TAU;
        centre + Vec3::new(a.cos(), 0.0, a.sin()) * 3.0
    };
    let mut ids = vec![None; kinds.len()];
    let chief = kinds.iter().position(|k| *k == BanditKind::Chief);
    if let Some(i) = chief {
        ids[i] = Some(w.spawn(bandit(BanditKind::Chief, team, spot(i))));
    }
    let leader = chief.and_then(|i| ids[i]);
    for (i, kind) in kinds.iter().enumerate() {
        if ids[i].is_some() {
            continue;
        }
        let mut desc = bandit(*kind, team, spot(i));
        if let Some(brain) = &mut desc.brain {
            brain.leader = leader;
        }
        ids[i] = Some(w.spawn(desc));
    }
    ids.into_iter().flatten().collect()
}
