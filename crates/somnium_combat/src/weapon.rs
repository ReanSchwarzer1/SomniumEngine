//! Weapons as data: moves with timings, hit shapes, guards and ranged kits.
//!
//! Every number a designer tunes lives in a [`WeaponDef`], which round-trips
//! through JSON. The library functions are Rimeborn's defaults, not special
//! cases: the protagonist's sword, the Aetherlance's lance-glaive, and the
//! spears, axes, knives, bows and Lattice rifles the world's other fighters
//! carry (GAMEPLAY 5: he uses only swords and magic; everyone else uses the
//! rest).
use crate::damage::{DamagePart, DamageType};
use crate::status::StatusKind;
use serde::{Deserialize, Serialize};

/// The families of weapon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum WeaponKind {
    /// Longsword or sabre: the protagonist's weapon.
    Sword,
    /// The Aetherlance's lance-glaive: thrusts and the Charge.
    LanceGlaive,
    /// Infantry spear: reach.
    Spear,
    /// Axe: slow, heavy, bleeding.
    Axe,
    /// Knife: fast, bleeding, deadly from behind; throws.
    Knife,
    /// Bow: draw and loose.
    Bow,
    /// Lattice rifle (4W 1150): one round, long reload, kills Saints.
    LatticeRifle,
    /// Fists.
    Unarmed,
}

impl WeaponKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 8] = [
        Self::Sword,
        Self::LanceGlaive,
        Self::Spear,
        Self::Axe,
        Self::Knife,
        Self::Bow,
        Self::LatticeRifle,
        Self::Unarmed,
    ];

    /// Display names, in [`Self::ALL`] order.
    pub const NAMES: [&'static str; 8] = [
        "Sword",
        "Lance-glaive",
        "Spear",
        "Axe",
        "Knife",
        "Bow",
        "Lattice rifle",
        "Unarmed",
    ];

    /// The library weapon of this kind.
    #[must_use]
    pub fn library(self) -> WeaponDef {
        match self {
            Self::Sword => sword(),
            Self::LanceGlaive => lance_glaive(),
            Self::Spear => spear(),
            Self::Axe => axe(),
            Self::Knife => knife(),
            Self::Bow => bow(),
            Self::LatticeRifle => lattice_rifle(),
            Self::Unarmed => unarmed(),
        }
    }
}

/// The shape of a move.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MoveKind {
    /// A horizontal cut.
    Slash,
    /// A straight point.
    Thrust,
    /// A downward blow.
    Overhead,
    /// A low sweep at the legs.
    Sweep,
    /// A running charge (the Aetherlance's): travels, hits the first in its way.
    Charge,
}

/// Where a move's hit volume is, relative to the attacker.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum HitShape {
    /// A sector in front: `radius` metres, `half_angle` degrees, from `low` to
    /// `high` metres relative to the attacker's chest.
    Arc {
        /// Reach.
        radius: f32,
        /// Half the swept angle, degrees.
        half_angle: f32,
        /// Lowest reach relative to the chest.
        low: f32,
        /// Highest reach relative to the chest.
        high: f32,
    },
    /// A point: a capsule from the chest along the facing.
    Thrust {
        /// Length from the chest.
        length: f32,
        /// Radius of the point's path.
        radius: f32,
    },
    /// A ball in front (an overhead's landing).
    Sphere {
        /// Distance in front.
        forward: f32,
        /// Ball radius.
        radius: f32,
    },
    /// Follow the blade the host reports from the skeleton (`set_blade`),
    /// swept between frames. Falls back to an arc of the weapon's reach when
    /// the host reports nothing.
    Blade {
        /// Radius around the hilt-to-tip segment.
        radius: f32,
    },
}

/// One attack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoveDef {
    /// Name, for events and debugging.
    pub name: String,
    /// What kind of motion.
    pub kind: MoveKind,
    /// Clip the host plays; empty plays nothing.
    pub animation: String,
    /// Seconds before the blow lands (telegraph).
    pub windup: f32,
    /// Seconds the hit volume is live.
    pub active: f32,
    /// Seconds after.
    pub recovery: f32,
    /// Harm by kind.
    pub damage: Vec<DamagePart>,
    /// Poise damage.
    pub poise: f32,
    /// Stamina to start the move.
    pub stamina: f32,
    /// Hit volume.
    pub shape: HitShape,
    /// Metres the attacker travels during windup and active.
    pub lunge: f32,
    /// Fraction of recovery, from and to, in which the next move may chain.
    pub combo_window: [f32; 2],
    /// Index of the next light move in the chain.
    pub next: Option<u8>,
    /// Poise multiplier while winding up and striking (hyper armour).
    pub hyper_armor: f32,
    /// Status build-up on a hit.
    pub status: Vec<(StatusKind, f32)>,
    /// Push on the target, metres per second.
    pub knockback: f32,
    /// A poise break from this knocks down rather than staggers.
    pub heavy: bool,
    /// Fraction of recovery after which a dodge cancels it.
    pub dodge_cancel: f32,
    /// Phases come from the clip's events (`hit_on`, `hit_off`, `recover`)
    /// rather than from the timings above; the timings become a safety limit.
    pub driven_by_events: bool,
}

impl MoveDef {
    /// Seconds from start to end when run by timings.
    #[must_use]
    pub fn duration(&self) -> f32 {
        self.windup + self.active + self.recovery
    }

    fn slash(name: &str, windup: f32, active: f32, recovery: f32, cut: f32, poise: f32) -> Self {
        Self {
            name: name.into(),
            kind: MoveKind::Slash,
            animation: name.into(),
            windup,
            active,
            recovery,
            damage: vec![DamagePart::new(DamageType::Cut, cut)],
            poise,
            stamina: 12.0,
            shape: HitShape::Arc {
                radius: 2.1,
                half_angle: 70.0,
                low: -1.0,
                high: 0.7,
            },
            lunge: 0.4,
            combo_window: [0.0, 0.85],
            next: None,
            hyper_armor: 0.0,
            status: Vec::new(),
            knockback: 1.0,
            heavy: false,
            dodge_cancel: 0.35,
            driven_by_events: false,
        }
    }

    fn with_next(mut self, next: u8) -> Self {
        self.next = Some(next);
        self
    }
}

/// How a weapon guards.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GuardDef {
    /// Fraction of physical harm a block stops (magic: half of this).
    pub stability: f32,
    /// Stamina lost per point of harm blocked.
    pub stamina_per_damage: f32,
    /// Seconds after raising the guard in which a blow is parried.
    pub parry_window: f32,
    /// Full angle in front that the guard covers, degrees.
    pub angle: f32,
    /// Whether this weapon can parry at all.
    pub can_parry: bool,
}

/// What a projectile is, for presentation and for the rules that read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectileKind {
    /// Bow arrow.
    Arrow,
    /// Lattice round.
    Round,
    /// Thrown knife.
    Knife,
    /// A spell's bolt or lance.
    Spell,
    /// A cut that flies (Gale Edge's Wind Blade, Sky-Shrine).
    WindCut,
}

/// A projectile's ballistics and harm.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectileDef {
    /// Kind.
    pub kind: ProjectileKind,
    /// Muzzle speed, metres per second.
    pub speed: f32,
    /// Fraction of gravity that acts on it.
    pub gravity: f32,
    /// Radius for hit tests.
    pub radius: f32,
    /// Maximum travel, metres.
    pub range: f32,
    /// Harm by kind.
    pub damage: Vec<DamagePart>,
    /// Poise damage.
    pub poise: f32,
    /// Status build-up on a hit.
    pub status: Vec<(StatusKind, f32)>,
    /// Bodies it passes through before stopping.
    pub pierce: u8,
    /// Push on the target, metres per second.
    pub knockback: f32,
    /// Damage multiplier for a strike in the head zone.
    pub head_multiplier: f32,
    /// How strongly wind (gale volumes) bends it, 0 to 1.
    pub wind_factor: f32,
}

/// A ranged kit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RangedDef {
    /// The projectile.
    pub projectile: ProjectileDef,
    /// Seconds to full draw (bow) or to steady the aim (rifle).
    pub draw: f32,
    /// Fraction of draw below which a release is a weak shot.
    pub min_draw: f32,
    /// Damage multiplier at minimum draw (scales up to 1 at full).
    pub weak_shot: f32,
    /// Seconds to reload once the magazine is empty.
    pub reload: f32,
    /// Shots before a reload.
    pub magazine: u8,
    /// Spread at full draw, degrees.
    pub spread: f32,
    /// How far the shot is heard, as a fraction of the loudest sound (1).
    pub loudness: f32,
    /// Stamina per shot.
    pub stamina: f32,
    /// Seconds after a shot before anything else.
    pub recovery: f32,
    /// Whether the attack button releases the shot (bow) or presses it (rifle).
    pub release_to_fire: bool,
}

/// A weapon.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WeaponDef {
    /// Family.
    pub kind: WeaponKind,
    /// Display name.
    pub name: String,
    /// The light chain; index 0 starts it.
    pub light: Vec<MoveDef>,
    /// Hold-to-charge heavy blow.
    pub heavy: Option<MoveDef>,
    /// Running charge (lance-glaive; the Aetherlance form unlocks it for swords).
    pub charge: Option<MoveDef>,
    /// Guard.
    pub guard: GuardDef,
    /// Ranged kit, if this weapon shoots or throws.
    pub ranged: Option<RangedDef>,
    /// Typical striking distance, for AI spacing.
    pub reach: f32,
    /// Damage multiplier from behind an unaware target.
    pub backstab: f32,
}

fn guard(stability: f32, parry_window: f32) -> GuardDef {
    GuardDef {
        stability,
        stamina_per_damage: 0.6,
        parry_window,
        angle: 120.0,
        can_parry: parry_window > 0.0,
    }
}

/// The protagonist's sword: a three-cut chain, an overhead heavy, a sure guard.
#[must_use]
pub fn sword() -> WeaponDef {
    let mut thrust = MoveDef::slash("sword_thrust", 0.3, 0.1, 0.55, 0.0, 26.0);
    thrust.kind = MoveKind::Thrust;
    thrust.damage = vec![DamagePart::new(DamageType::Pierce, 28.0)];
    thrust.shape = HitShape::Thrust {
        length: 2.5,
        radius: 0.25,
    };
    thrust.lunge = 0.8;
    thrust.stamina = 15.0;
    thrust.knockback = 2.5;
    let mut heavy = MoveDef::slash("sword_overhead", 0.6, 0.14, 0.7, 40.0, 48.0);
    heavy.kind = MoveKind::Overhead;
    heavy.damage.push(DamagePart::new(DamageType::Blunt, 8.0));
    heavy.shape = HitShape::Sphere {
        forward: 1.4,
        radius: 0.9,
    };
    heavy.stamina = 26.0;
    heavy.hyper_armor = 1.0;
    heavy.heavy = true;
    heavy.knockback = 3.0;
    heavy.dodge_cancel = 0.6;
    WeaponDef {
        kind: WeaponKind::Sword,
        name: "Longsword".into(),
        light: vec![
            MoveDef::slash("sword_cut_1", 0.26, 0.12, 0.38, 18.0, 14.0).with_next(1),
            MoveDef::slash("sword_cut_2", 0.22, 0.12, 0.36, 20.0, 15.0).with_next(2),
            thrust,
        ],
        heavy: Some(heavy),
        charge: None,
        guard: guard(0.72, 0.18),
        ranged: None,
        reach: 2.0,
        backstab: 1.5,
    }
}

/// The Aetherlance's lance-glaive: reaching thrusts and the Charge.
#[must_use]
pub fn lance_glaive() -> WeaponDef {
    let thrust = |name: &str, windup: f32, pierce: f32| {
        let mut m = MoveDef::slash(name, windup, 0.12, 0.45, 0.0, 18.0);
        m.kind = MoveKind::Thrust;
        m.damage = vec![DamagePart::new(DamageType::Pierce, pierce)];
        m.shape = HitShape::Thrust {
            length: 3.2,
            radius: 0.25,
        };
        m.lunge = 0.6;
        m.stamina = 14.0;
        m
    };
    let mut sweep = MoveDef::slash("glaive_sweep", 0.4, 0.16, 0.6, 26.0, 30.0);
    sweep.shape = HitShape::Arc {
        radius: 3.0,
        half_angle: 85.0,
        low: -1.1,
        high: 0.6,
    };
    sweep.stamina = 22.0;
    sweep.hyper_armor = 0.6;
    let mut charge = MoveDef::slash("glaive_charge", 0.35, 0.9, 0.7, 0.0, 70.0);
    charge.kind = MoveKind::Charge;
    charge.damage = vec![DamagePart::new(DamageType::Pierce, 45.0)];
    charge.shape = HitShape::Thrust {
        length: 2.0,
        radius: 0.5,
    };
    charge.lunge = 7.0;
    charge.stamina = 30.0;
    charge.hyper_armor = 2.0;
    charge.heavy = true;
    charge.knockback = 6.0;
    charge.status = vec![(StatusKind::Chill, 35.0)];
    WeaponDef {
        kind: WeaponKind::LanceGlaive,
        name: "Lance-glaive".into(),
        light: vec![
            thrust("glaive_thrust_1", 0.3, 20.0).with_next(1),
            thrust("glaive_thrust_2", 0.26, 22.0).with_next(2),
            sweep,
        ],
        heavy: None,
        charge: Some(charge),
        guard: guard(0.6, 0.12),
        ranged: None,
        reach: 2.9,
        backstab: 1.5,
    }
}

/// Infantry spear: keep them at the point, sweep the legs.
#[must_use]
pub fn spear() -> WeaponDef {
    let mut t1 = MoveDef::slash("spear_thrust", 0.34, 0.12, 0.42, 0.0, 14.0);
    t1.kind = MoveKind::Thrust;
    t1.damage = vec![DamagePart::new(DamageType::Pierce, 16.0)];
    t1.shape = HitShape::Thrust {
        length: 3.0,
        radius: 0.22,
    };
    t1.lunge = 0.3;
    t1.stamina = 11.0;
    let t2 = MoveDef {
        name: "spear_thrust_2".into(),
        animation: "spear_thrust_2".into(),
        ..t1.clone()
    };
    let mut sweep = MoveDef::slash("spear_sweep", 0.55, 0.16, 0.65, 0.0, 34.0);
    sweep.kind = MoveKind::Sweep;
    sweep.damage = vec![DamagePart::new(DamageType::Blunt, 12.0)];
    sweep.shape = HitShape::Arc {
        radius: 2.8,
        half_angle: 100.0,
        low: -1.3,
        high: -0.4,
    };
    sweep.heavy = true;
    sweep.stamina = 20.0;
    WeaponDef {
        kind: WeaponKind::Spear,
        name: "Spear".into(),
        light: vec![t1.with_next(1), t2],
        heavy: Some(sweep),
        charge: None,
        guard: guard(0.55, 0.0),
        ranged: None,
        reach: 2.7,
        backstab: 1.5,
    }
}

/// Axe: slow cuts that open wounds.
#[must_use]
pub fn axe() -> WeaponDef {
    let mut c1 = MoveDef::slash("axe_cut_1", 0.42, 0.14, 0.5, 26.0, 28.0);
    c1.status = vec![(StatusKind::Bleed, 18.0)];
    c1.stamina = 16.0;
    c1.hyper_armor = 0.5;
    let mut c2 = MoveDef::slash("axe_cut_2", 0.4, 0.14, 0.55, 28.0, 30.0);
    c2.status = vec![(StatusKind::Bleed, 18.0)];
    c2.stamina = 16.0;
    c2.hyper_armor = 0.5;
    let mut heavy = MoveDef::slash("axe_overhead", 0.75, 0.15, 0.8, 50.0, 58.0);
    heavy.kind = MoveKind::Overhead;
    heavy.shape = HitShape::Sphere {
        forward: 1.3,
        radius: 0.85,
    };
    heavy.status = vec![(StatusKind::Bleed, 35.0)];
    heavy.stamina = 28.0;
    heavy.hyper_armor = 1.5;
    heavy.heavy = true;
    WeaponDef {
        kind: WeaponKind::Axe,
        name: "Axe".into(),
        light: vec![c1.with_next(1), c2],
        heavy: Some(heavy),
        charge: None,
        guard: guard(0.5, 0.0),
        ranged: None,
        reach: 1.8,
        backstab: 1.5,
    }
}

/// Knife: quick, bleeding, terrible from behind; a pair to throw.
#[must_use]
pub fn knife() -> WeaponDef {
    let stab = |name: &str| {
        let mut m = MoveDef::slash(name, 0.14, 0.08, 0.22, 9.0, 5.0);
        m.shape = HitShape::Arc {
            radius: 1.4,
            half_angle: 50.0,
            low: -0.8,
            high: 0.6,
        };
        m.status = vec![(StatusKind::Bleed, 22.0)];
        m.stamina = 7.0;
        m.lunge = 0.5;
        m.knockback = 0.3;
        m.dodge_cancel = 0.1;
        m
    };
    WeaponDef {
        kind: WeaponKind::Knife,
        name: "Knife".into(),
        light: vec![
            stab("knife_1").with_next(1),
            stab("knife_2").with_next(2),
            stab("knife_3"),
        ],
        heavy: None,
        charge: None,
        guard: guard(0.3, 0.1),
        ranged: Some(RangedDef {
            projectile: ProjectileDef {
                kind: ProjectileKind::Knife,
                speed: 24.0,
                gravity: 0.6,
                radius: 0.06,
                range: 18.0,
                damage: vec![DamagePart::new(DamageType::Pierce, 12.0)],
                poise: 6.0,
                status: vec![(StatusKind::Bleed, 25.0)],
                pierce: 0,
                knockback: 0.2,
                head_multiplier: 1.5,
                wind_factor: 0.3,
            },
            draw: 0.2,
            min_draw: 1.0,
            weak_shot: 1.0,
            reload: 0.7,
            magazine: 2,
            spread: 2.0,
            loudness: 0.1,
            stamina: 6.0,
            recovery: 0.25,
            release_to_fire: false,
        }),
        reach: 1.3,
        backstab: 3.0,
    }
}

/// Bow: draw longer, strike harder.
#[must_use]
pub fn bow() -> WeaponDef {
    let mut bash = MoveDef::slash("bow_bash", 0.3, 0.1, 0.4, 0.0, 10.0);
    bash.damage = vec![DamagePart::new(DamageType::Blunt, 6.0)];
    bash.shape = HitShape::Arc {
        radius: 1.3,
        half_angle: 60.0,
        low: -0.6,
        high: 0.6,
    };
    WeaponDef {
        kind: WeaponKind::Bow,
        name: "Hunting bow".into(),
        light: vec![bash],
        heavy: None,
        charge: None,
        guard: guard(0.2, 0.0),
        ranged: Some(RangedDef {
            projectile: ProjectileDef {
                kind: ProjectileKind::Arrow,
                speed: 48.0,
                gravity: 0.5,
                radius: 0.04,
                range: 70.0,
                damage: vec![DamagePart::new(DamageType::Pierce, 30.0)],
                poise: 16.0,
                status: Vec::new(),
                pierce: 0,
                knockback: 0.5,
                head_multiplier: 1.8,
                wind_factor: 0.6,
            },
            draw: 0.9,
            min_draw: 0.3,
            weak_shot: 0.35,
            reload: 0.0,
            magazine: 1,
            spread: 1.0,
            loudness: 0.15,
            stamina: 8.0,
            recovery: 0.35,
            release_to_fire: true,
        }),
        reach: 1.2,
        backstab: 1.5,
    }
}

/// Lattice rifle: one round that any farmer can kill a Saint with, then a long
/// reload (MAGIC 10.3).
#[must_use]
pub fn lattice_rifle() -> WeaponDef {
    let mut butt = MoveDef::slash("rifle_butt", 0.32, 0.1, 0.45, 0.0, 18.0);
    butt.damage = vec![DamagePart::new(DamageType::Blunt, 10.0)];
    butt.shape = HitShape::Sphere {
        forward: 1.0,
        radius: 0.6,
    };
    WeaponDef {
        kind: WeaponKind::LatticeRifle,
        name: "Lattice rifle".into(),
        light: vec![butt],
        heavy: None,
        charge: None,
        guard: guard(0.25, 0.0),
        ranged: Some(RangedDef {
            projectile: ProjectileDef {
                kind: ProjectileKind::Round,
                speed: 320.0,
                gravity: 0.1,
                radius: 0.02,
                range: 160.0,
                damage: vec![DamagePart::new(DamageType::Pierce, 70.0)],
                poise: 60.0,
                status: Vec::new(),
                pierce: 1,
                knockback: 2.0,
                head_multiplier: 2.0,
                wind_factor: 0.02,
            },
            draw: 0.45,
            min_draw: 1.0,
            weak_shot: 1.0,
            reload: 2.8,
            magazine: 1,
            spread: 0.6,
            loudness: 1.0,
            stamina: 0.0,
            recovery: 0.3,
            release_to_fire: false,
        }),
        reach: 1.0,
        backstab: 1.5,
    }
}

/// Fists.
#[must_use]
pub fn unarmed() -> WeaponDef {
    let mut jab = MoveDef::slash("punch_1", 0.16, 0.08, 0.26, 0.0, 8.0);
    jab.damage = vec![DamagePart::new(DamageType::Blunt, 6.0)];
    jab.shape = HitShape::Arc {
        radius: 1.1,
        half_angle: 45.0,
        low: -0.4,
        high: 0.6,
    };
    jab.stamina = 6.0;
    let cross = MoveDef {
        name: "punch_2".into(),
        animation: "punch_2".into(),
        ..jab.clone()
    };
    WeaponDef {
        kind: WeaponKind::Unarmed,
        name: "Unarmed".into(),
        light: vec![jab.with_next(1), cross],
        heavy: None,
        charge: None,
        guard: guard(0.25, 0.1),
        ranged: None,
        reach: 1.0,
        backstab: 1.5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_library_weapon_is_coherent() {
        for kind in WeaponKind::ALL {
            let w = kind.library();
            assert_eq!(w.kind, kind);
            assert!(!w.light.is_empty(), "{kind:?} has a light move");
            for (i, m) in w.light.iter().enumerate() {
                assert!(m.windup > 0.0 && m.active > 0.0 && m.recovery > 0.0);
                if let Some(next) = m.next {
                    assert!((next as usize) < w.light.len(), "{kind:?} move {i} chains");
                    assert_ne!(next as usize, i);
                }
                assert!(m.combo_window[0] <= m.combo_window[1]);
            }
            if let Some(r) = &w.ranged {
                assert!(r.magazine >= 1 && r.projectile.speed > 0.0);
            }
        }
    }

    #[test]
    fn weapons_round_trip_as_data() {
        for kind in WeaponKind::ALL {
            let w = kind.library();
            let json = serde_json::to_string(&w).unwrap();
            let back: WeaponDef = serde_json::from_str(&json).unwrap();
            assert_eq!(w, back);
        }
    }

    #[test]
    fn the_rifle_outhits_the_bow_and_the_knife_outbleeds_the_sword() {
        let rifle = lattice_rifle().ranged.unwrap().projectile.damage[0].amount;
        let arrow = bow().ranged.unwrap().projectile.damage[0].amount;
        assert!(rifle > arrow * 2.0);
        let knife_bleed: f32 = knife().light[0].status.iter().map(|s| s.1).sum();
        let sword_bleed: f32 = sword().light[0].status.iter().map(|s| s.1).sum();
        assert!(knife_bleed > sword_bleed);
    }
}
