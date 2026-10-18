//! Simulation LOD: how often an agent steps, by how far it is from what
//! matters.
//!
//! Near agents step every tick. Mid-range ones step every `mid_stride` ticks
//! and far ones every `far_stride`, each with the time it is owed, so a far
//! village keeps its day at a sliver of the cost. Agents are spread across
//! ticks by id, so no tick takes all the mid-range ones at once; and a budget
//! caps how many out-of-sight steps one tick may take (the rest wait a tick,
//! still owed their time).

use glam::{Vec2, Vec3};

/// How far each tier reaches and how often it steps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodConfig {
    /// Within this (metres, on the ground plane), every tick.
    pub near: f32,
    /// Within this, every `mid_stride` ticks; beyond, every `far_stride`.
    pub far: f32,
    /// Ticks between mid-range steps.
    pub mid_stride: u32,
    /// Ticks between far steps.
    pub far_stride: u32,
    /// Most mid and far steps one tick may take.
    pub budget: usize,
}

impl Default for LodConfig {
    fn default() -> Self {
        Self {
            near: 60.0,
            far: 220.0,
            mid_stride: 4,
            far_stride: 30,
            budget: 256,
        }
    }
}

/// How much an agent matters now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tier {
    /// Near what matters: every tick, smooth.
    #[default]
    Near,
    /// In view but not close: every few ticks.
    Mid,
    /// Far off: rarely, coarsely. A host need not draw it.
    Far,
}

impl LodConfig {
    /// The tier of a body at `position` given the points that matter. With
    /// none, everything is near.
    #[must_use]
    pub fn tier(&self, position: Vec3, focus: &[Vec3]) -> Tier {
        let Some(d2) = focus
            .iter()
            .map(|f| Vec2::new(f.x - position.x, f.z - position.z).length_squared())
            .reduce(f32::min)
        else {
            return Tier::Near;
        };
        if d2 <= self.near * self.near {
            Tier::Near
        } else if d2 <= self.far * self.far {
            Tier::Mid
        } else {
            Tier::Far
        }
    }

    /// Whether an agent of `tier` with `id` steps on `tick`.
    #[must_use]
    pub fn due(&self, tier: Tier, tick: u64, id: u32) -> bool {
        let stride = match tier {
            Tier::Near => return true,
            Tier::Mid => self.mid_stride,
            Tier::Far => self.far_stride,
        };
        let stride = u64::from(stride.max(1));
        (tick + u64::from(id)) % stride == 0
    }
}
