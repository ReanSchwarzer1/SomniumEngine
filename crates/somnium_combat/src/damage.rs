//! Damage types, resistances and the hit packet every weapon, spell and surface
//! produces.
//!
//! Mitigation runs in one place ([`Resistances::mitigate`]) so a sword cut, an
//! arrow, a burning field and a Rime Bloom all answer the same armour the same
//! way.
use crate::CombatantId;
use crate::status::StatusKind;
use glam::Vec3;
use serde::{Deserialize, Serialize};

/// The eight kinds of harm. Physical kinds meet armour; the rest meet only
/// their resistance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DamageType {
    /// Edges: swords, axes, knives.
    Cut,
    /// Points: spears, arrows, Lattice rounds, lances.
    Pierce,
    /// Weight: hafts, falls, stone, a Water Bolt's slam.
    Blunt,
    /// Rime magic, cold ground.
    Frost,
    /// Flame magic, burning ground.
    Flame,
    /// Gale magic, wind blades.
    Gale,
    /// Stone magic.
    Stone,
    /// Raw mana, the Wheel's own force.
    Mana,
}

impl DamageType {
    /// Every kind, in index order.
    pub const ALL: [Self; 8] = [
        Self::Cut,
        Self::Pierce,
        Self::Blunt,
        Self::Frost,
        Self::Flame,
        Self::Gale,
        Self::Stone,
        Self::Mana,
    ];

    /// Index into per-kind tables.
    #[must_use]
    pub fn index(self) -> usize {
        self as usize
    }

    /// Whether armour applies.
    #[must_use]
    pub fn is_physical(self) -> bool {
        matches!(self, Self::Cut | Self::Pierce | Self::Blunt)
    }

    /// Display name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Cut => "Cut",
            Self::Pierce => "Pierce",
            Self::Blunt => "Blunt",
            Self::Frost => "Frost",
            Self::Flame => "Flame",
            Self::Gale => "Gale",
            Self::Stone => "Stone",
            Self::Mana => "Mana",
        }
    }
}

/// One component of a hit.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DamagePart {
    /// What kind of harm.
    pub kind: DamageType,
    /// How much, before mitigation.
    pub amount: f32,
}

impl DamagePart {
    /// A part.
    #[must_use]
    pub const fn new(kind: DamageType, amount: f32) -> Self {
        Self { kind, amount }
    }
}

/// Fractional resistance per kind and flat armour against physical harm.
///
/// A resistance of 0.5 halves that kind; -0.5 is a weakness taking half again.
/// Armour works with diminishing returns (`d * d / (d + armour)`), so it
/// shrugs off a knife's scratch but not a greataxe.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Resistances {
    /// Per [`DamageType::index`], clamped to `[-1, 0.9]` when applied.
    pub values: [f32; 8],
    /// Flat physical armour, in damage points.
    pub armor: f32,
}

impl Default for Resistances {
    fn default() -> Self {
        Self {
            values: [0.0; 8],
            armor: 0.0,
        }
    }
}

impl Resistances {
    /// Resistance to `kind`.
    #[must_use]
    pub fn get(&self, kind: DamageType) -> f32 {
        self.values[kind.index()]
    }

    /// Builder: set one kind.
    #[must_use]
    pub fn with(mut self, kind: DamageType, value: f32) -> Self {
        self.values[kind.index()] = value;
        self
    }

    /// Builder: set armour.
    #[must_use]
    pub fn with_armor(mut self, armor: f32) -> Self {
        self.armor = armor;
        self
    }

    /// Damage that gets through. Never negative, never NaN.
    #[must_use]
    pub fn mitigate(&self, kind: DamageType, amount: f32) -> f32 {
        if !amount.is_finite() || amount <= 0.0 {
            return 0.0;
        }
        let resist = self.get(kind);
        let resist = if resist.is_finite() {
            resist.clamp(-1.0, 0.9)
        } else {
            0.0
        };
        let after = amount * (1.0 - resist);
        let armor = if self.armor.is_finite() {
            self.armor.max(0.0)
        } else {
            0.0
        };
        if kind.is_physical() && armor > 0.0 {
            after * after / (after + armor)
        } else {
            after
        }
    }
}

/// What kind of delivery a hit was, and what may answer it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HitFlags(pub u16);

impl HitFlags {
    /// A weapon swing or thrust.
    pub const MELEE: Self = Self(1);
    /// An arrow, round, thrown knife or bolt.
    pub const PROJECTILE: Self = Self(1 << 1);
    /// Any spell.
    pub const SPELL: Self = Self(1 << 2);
    /// A counter after a parry.
    pub const RIPOSTE: Self = Self(1 << 3);
    /// Struck from behind.
    pub const BACKSTAB: Self = Self(1 << 4);
    /// Cannot be blocked.
    pub const UNBLOCKABLE: Self = Self(1 << 5);
    /// Cannot be parried.
    pub const UNPARRYABLE: Self = Self(1 << 6);
    /// A head strike or riposte.
    pub const CRITICAL: Self = Self(1 << 7);
    /// An area: wave, bloom, wall, burning ground.
    pub const AREA: Self = Self(1 << 8);
    /// From the ground itself (burning grass, cold water).
    pub const ENVIRONMENT: Self = Self(1 << 9);
    /// Knocks down on a poise break instead of staggering.
    pub const HEAVY: Self = Self(1 << 10);
    /// A status tick; never interrupts, never staggers.
    pub const TICK: Self = Self(1 << 11);

    /// Whether every bit of `other` is set.
    #[must_use]
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether any bit of `other` is set.
    #[must_use]
    pub fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

impl std::ops::BitOr for HitFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for HitFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// A request to harm one target. Weapons, spells and surfaces build these;
/// the world resolves them against guards, wards, dodges and resistances.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    /// Who struck, if anyone.
    pub source: Option<CombatantId>,
    /// The harm, by kind.
    pub parts: Vec<DamagePart>,
    /// Poise damage before hyper armour.
    pub poise: f32,
    /// Status build-up to add, by kind.
    pub status: Vec<(StatusKind, f32)>,
    /// Push in metres per second, applied as a short slide.
    pub knockback: Vec3,
    /// World contact point.
    pub point: Vec3,
    /// Direction of travel of the blow, for guard facing and ragdoll impulse.
    pub direction: Vec3,
    /// Delivery.
    pub flags: HitFlags,
    /// Multiplier for every part (criticals, ripostes, school bonuses).
    pub scale: f32,
}

impl Hit {
    /// A hit with no status, knockback or flags.
    #[must_use]
    pub fn new(source: Option<CombatantId>, parts: Vec<DamagePart>, poise: f32) -> Self {
        Self {
            source,
            parts,
            poise,
            status: Vec::new(),
            knockback: Vec3::ZERO,
            point: Vec3::ZERO,
            direction: Vec3::ZERO,
            flags: HitFlags::default(),
            scale: 1.0,
        }
    }

    /// Total raw damage, after `scale`.
    #[must_use]
    pub fn raw_total(&self) -> f32 {
        self.parts.iter().map(|p| p.amount.max(0.0)).sum::<f32>() * self.scale
    }

    /// The largest part's kind, for presentation.
    #[must_use]
    pub fn dominant(&self) -> Option<DamageType> {
        self.parts
            .iter()
            .max_by(|a, b| a.amount.total_cmp(&b.amount))
            .map(|p| p.kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resistance_scales_and_weakness_amplifies() {
        let r = Resistances::default()
            .with(DamageType::Flame, 0.5)
            .with(DamageType::Frost, -0.5);
        assert!((r.mitigate(DamageType::Flame, 20.0) - 10.0).abs() < 1e-5);
        assert!((r.mitigate(DamageType::Frost, 20.0) - 30.0).abs() < 1e-5);
        assert!((r.mitigate(DamageType::Gale, 20.0) - 20.0).abs() < 1e-5);
    }

    #[test]
    fn armour_shrugs_off_scratches_but_not_blows() {
        let r = Resistances::default().with_armor(20.0);
        let scratch = r.mitigate(DamageType::Cut, 5.0) / 5.0;
        let blow = r.mitigate(DamageType::Blunt, 80.0) / 80.0;
        assert!(scratch < 0.25, "a knife scratch mostly stops: {scratch}");
        assert!(blow > 0.75, "a greataxe mostly lands: {blow}");
        // Armour never touches magic.
        assert_eq!(r.mitigate(DamageType::Flame, 5.0), 5.0);
    }

    #[test]
    fn mitigation_rejects_nonsense() {
        let r = Resistances::default().with(DamageType::Cut, f32::NAN);
        assert_eq!(r.mitigate(DamageType::Cut, f32::NAN), 0.0);
        assert_eq!(r.mitigate(DamageType::Cut, -4.0), 0.0);
        assert_eq!(r.mitigate(DamageType::Cut, 4.0), 4.0);
        let immune = Resistances::default().with(DamageType::Mana, 5.0);
        assert!(
            immune.mitigate(DamageType::Mana, 10.0) >= 1.0 - 1e-5,
            "resistance caps at 90%"
        );
    }

    #[test]
    fn flags_combine() {
        let f = HitFlags::MELEE | HitFlags::RIPOSTE;
        assert!(f.contains(HitFlags::MELEE));
        assert!(f.intersects(HitFlags::RIPOSTE | HitFlags::SPELL));
        assert!(!f.contains(HitFlags::SPELL));
    }
}
