//! Ambient life for open worlds: villagers on daily routines between a place's
//! spots, livestock grazing in pens, carts on roads, and simulation LOD so
//! hundreds of them cost little.
//!
//! A host builds a [`LifeWorld`] from data: each [`Place`] (a farmstead, a
//! village, a quarter of a city) declares its [`Spot`]s (homes, workplaces, the
//! inn, the chapel, fields, pens, the green, the well), its lanes (the paths
//! people walk) and its roads (where carts go). [`Place::from_layout`] reads
//! the same building types and named areas a settlement generator writes, so a
//! town's own data feeds it. [`LifeWorld::populate`] then houses a household in
//! every home and gives each person a [`Role`] by the workplaces there.
//!
//! Each step the host hands it a [`Clock`] (the hour, and how far the Lid has
//! covered the sun: when it falls, everyone goes indoors) and the points that
//! matter (the player, the camera). Agents near them step every frame; further
//! out they step every few frames with the time owed; far away, rarely and
//! coarsely, so a village a kilometre off keeps its day without costing one.
//! The host reads back positions, facings, whether each is visible (not
//! indoors) and a [`Pose`] to animate.
//!
//! Deterministic: no clock is read, randomness is per agent and seeded.
#![deny(missing_docs)]

pub mod components;
pub mod crowd;
pub mod lod;
pub mod place;
pub mod routine;
mod sim;
pub mod table;

pub use crowd::Bodies;
pub use lod::{LodConfig, Tier};
pub use place::{LaneGraph, Place, Spot, SpotKind};
pub use routine::{Activity, Role, Routine};
pub use sim::{
    AgentId, Beast, BeastKind, Cart, DOOR_LEAD, Flat, Ground, LifeWorld, PersonAt, Schedule,
    ScheduleEntry, SpotRef, Stats, Villager, VillagerDesc,
};
pub use table::{GearPiece, LifeTable, TableLooks, TablePlace};

/// The time of day life runs on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clock {
    /// Local hour, `0..24`.
    pub hour: f32,
    /// How much of the sun the Lid covers, `0..1` (the scene's `TimeOfDay`
    /// eclipse). Past [`LID_CURFEW`] everyone goes indoors.
    pub eclipse: f32,
}

impl Clock {
    /// A day at `hour`, the sun uncovered.
    #[must_use]
    pub fn at(hour: f32) -> Self {
        Self { hour, eclipse: 0.0 }
    }

    /// Whether the Lid has fallen far enough to send everyone indoors.
    #[must_use]
    pub fn lid_fallen(&self) -> bool {
        self.eclipse >= LID_CURFEW
    }
}

/// Eclipse at which the Lid sends everyone indoors.
pub const LID_CURFEW: f32 = 0.5;

/// What a body is doing, for the host's animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Pose {
    /// Standing about.
    Idle,
    /// Walking (or a beast ambling).
    Walk,
    /// At a trade: hammering, hauling, sweeping.
    Work,
    /// In the fields: hoeing, scything.
    Farm,
    /// With the animals.
    Tend,
    /// Buying and selling, talking.
    Trade,
    /// Praying.
    Pray,
    /// Drinking, sitting.
    Sit,
    /// Playing (children).
    Play,
    /// Grazing (beasts).
    Graze,
    /// Driving a cart.
    Drive,
    /// Out of sight indoors.
    Inside,
    /// Running from danger.
    Run,
    /// Talking with a neighbour (facing them).
    Talk,
}

impl Pose {
    /// Its name, as a game's tables give poses (`Walk`, `Talk`, ...).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Walk => "Walk",
            Self::Work => "Work",
            Self::Farm => "Farm",
            Self::Tend => "Tend",
            Self::Trade => "Trade",
            Self::Pray => "Pray",
            Self::Sit => "Sit",
            Self::Play => "Play",
            Self::Graze => "Graze",
            Self::Drive => "Drive",
            Self::Inside => "Inside",
            Self::Run => "Run",
            Self::Talk => "Talk",
        }
    }
}

/// The world's random numbers: xorshift64*, seeded per agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        // Spread small seeds; xorshift cannot leave zero.
        let s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
        Self(if s == 0 { 1 } else { s })
    }

    pub(crate) fn unit(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let v = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (v >> 40) as f32 / (1u64 << 24) as f32
    }

    pub(crate) fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
}
