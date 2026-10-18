//! Real-time combat and abilities.
//!
//! A deterministic, headless combat simulation that a host drives one fixed
//! step at a time. The host owns bodies, animation, rendering and the camera;
//! this crate owns who hits whom, for how much, and what the world does about
//! it.
//!
//! ```text
//! host controller ── set_pose ──▶ ┌──────────────┐ ── intents ──▶ host controller
//! input / AI ─────── command ───▶ │ CombatWorld  │ ── events ───▶ animation, VFX,
//! animation clips ── anim events ▶│  step(dt)    │                decals, audio,
//! physics ────────── CombatQuery ▶└──────────────┘                ragdolls, nav
//! ```
//!
//! | Module | Owns |
//! |---|---|
//! | [`damage`] | the eight damage types, resistances, armour, the [`Hit`] packet |
//! | [`status`] | chill, freeze, burn, wet, stagger, knockdown, bleed and their interactions |
//! | [`vitals`] | health, stamina, mana in lamps (capacity), poise, focus, overdraw |
//! | [`team`] | teams, attitudes, personal grudges |
//! | [`geometry`] | the exact shape tests hit volumes use |
//! | [`weapon`] | weapons as data: moves, guards, ranged kits, the library |
//! | [`schools`] | the four sword schools and their forms |
//! | [`spell`] | the six-part spell request, assets, measures, costs, the spell library |
//! | [`environment`] | surface tags on a grid and the reactions spells cause |
//! | [`world`] | the simulation: melee, projectiles, casting, effects, resolution |
//! | [`ai`] | combat brains on `somnium_ai` behaviour trees, perception and navigation |
//! | [`input`] | the rebindable mouse-and-keyboard combat map and the cursor aim |
//! | [`components`] | authored ECS components and their Details schemas |
//! | [`mount`] | mounts, riders and mounted combat |
//!
//! Determinism: every collection iterates in id order, randomness comes from
//! one seeded generator, and nothing reads the clock. Two worlds fed the same
//! commands produce the same events.
#![deny(missing_docs)]

pub mod ai;
pub mod components;
pub mod damage;
pub mod environment;
pub mod geometry;
pub mod input;
pub mod mount;
pub mod schools;
pub mod spell;
pub mod status;
pub mod team;
pub mod vitals;
pub mod weapon;
pub mod wilds;
pub mod world;

pub use damage::{DamagePart, DamageType, Hit, HitFlags, Resistances};
pub use status::{StatusKind, StatusSet};
pub use team::{Attitude, Relations, TeamId};
pub use vitals::{Pool, Vitals};
pub use world::{
    Action, Body, CombatEvent, CombatRules, CombatWorld, Combatant, CombatantDesc, Command,
    MoveIntent, Press,
};

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// A combatant's identity inside one [`CombatWorld`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CombatantId(pub u32);

/// The seven ranks shared by magic and the sword (MAGIC 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Rank {
    /// First spells by full incantation.
    Novice,
    /// Reliable in the field.
    Adept,
    /// Can fight with it.
    Advanced,
    /// A career.
    Saint,
    /// A legend in one country.
    King,
    /// A name in the history books.
    Emperor,
    /// None recorded for three centuries.
    Sovereign,
}

impl Rank {
    /// Every rank, lowest first.
    pub const ALL: [Self; 7] = [
        Self::Novice,
        Self::Adept,
        Self::Advanced,
        Self::Saint,
        Self::King,
        Self::Emperor,
        Self::Sovereign,
    ];

    /// How many ranks `self` is above `other` (zero when not above).
    #[must_use]
    pub fn steps_above(self, other: Self) -> u32 {
        (self as u32).saturating_sub(other as u32)
    }

    /// Damage multiplier a fighter of this rank brings to a sword.
    #[must_use]
    pub fn scale(self) -> f32 {
        [1.0, 1.08, 1.16, 1.25, 1.36, 1.48, 1.6][self as usize]
    }
}

/// `glam::Vec3` as `[x, y, z]`, without turning on glam's serde feature for
/// the whole workspace.
pub(crate) mod serde_vec3 {
    use glam::Vec3;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(v: &Vec3, s: S) -> Result<S::Ok, S::Error> {
        v.to_array().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec3, D::Error> {
        <[f32; 3]>::deserialize(d).map(Vec3::from_array)
    }
}

/// A hit against static world geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldHit {
    /// Fraction of the swept segment travelled before contact, 0 to 1.
    pub fraction: f32,
    /// Outward surface normal.
    pub normal: Vec3,
}

/// The static world, as combat needs to ask about it. The host answers from
/// physics (a thin capsule sweep through Jolt); tests answer from a few boxes.
pub trait CombatQuery {
    /// Sweep a sphere of `radius` from `from` to `to`; the first contact.
    fn sweep(&self, from: Vec3, to: Vec3, radius: f32) -> Option<WorldHit>;

    /// Whether the line between two points is blocked (sight, line of fire).
    fn blocked(&self, from: Vec3, to: Vec3) -> bool {
        self.sweep(from, to, 0.0).is_some()
    }
}

/// An empty world: nothing blocks anything.
#[derive(Clone, Copy, Debug, Default)]
pub struct OpenField;

impl CombatQuery for OpenField {
    fn sweep(&self, _: Vec3, _: Vec3, _: f32) -> Option<WorldHit> {
        None
    }
}

/// Static boxes, for tests and for hosts without physics.
#[derive(Clone, Debug, Default)]
pub struct BoxWorld(pub Vec<geometry::Obb>);

impl CombatQuery for BoxWorld {
    fn sweep(&self, from: Vec3, to: Vec3, radius: f32) -> Option<WorldHit> {
        let delta = to - from;
        let len = delta.length();
        let dir = delta.try_normalize()?;
        self.0
            .iter()
            .filter_map(|b| b.ray(from, dir, len, radius))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(t, normal)| WorldHit {
                fraction: if len > 0.0 { t / len } else { 0.0 },
                normal,
            })
    }
}

/// The world's one random stream: xorshift64*, seeded per world, so AI
/// choices, spreads and fire spread replay exactly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rng(u64);

impl Rng {
    /// Seed; zero is replaced (xorshift cannot leave zero).
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in `[-1, 1)`.
    pub fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_repeatable_and_uniform_enough() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        let xs: Vec<f32> = (0..1000).map(|_| a.unit()).collect();
        let ys: Vec<f32> = (0..1000).map(|_| b.unit()).collect();
        assert_eq!(xs, ys);
        let mean = xs.iter().sum::<f32>() / 1000.0;
        assert!((mean - 0.5).abs() < 0.05);
        assert!(xs.iter().all(|x| (0.0..1.0).contains(x)));
    }

    #[test]
    fn ranks_order_and_scale() {
        assert!(Rank::Saint > Rank::Adept);
        assert_eq!(Rank::Saint.steps_above(Rank::Adept), 2);
        assert_eq!(Rank::Novice.steps_above(Rank::King), 0);
        assert!(Rank::Sovereign.scale() > Rank::Novice.scale());
    }

    #[test]
    fn box_world_reports_the_nearest_wall() {
        let world = BoxWorld(vec![
            geometry::Obb::wall(Vec3::new(0.0, 0.0, 6.0), Vec3::Z, 4.0, 3.0, 0.5),
            geometry::Obb::wall(Vec3::new(0.0, 0.0, 3.0), Vec3::Z, 4.0, 3.0, 0.5),
        ]);
        let hit = world
            .sweep(Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 1.0, 10.0), 0.0)
            .unwrap();
        assert!((hit.fraction - 0.275).abs() < 1e-4);
        assert!(world.blocked(Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 1.0, 10.0)));
        assert!(!world.blocked(Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 1.0, 2.0)));
    }
}
