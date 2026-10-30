//! Health, stamina, mana and poise.
//!
//! Mana is measured in **lamps**, the world's own unit (MAGIC 1.4): one lamp
//! keeps a Lamplighter's lamp lit for a Lid. A commoner holds about 5, a
//! trained mage 200, a Saint 2,000. Maximum mana *is* capacity.
//!
//! Casting past empty is **overdraw** (MAGIC 1.5): allowed, on the Wheel's
//! credit, down to a floor, and counted so the game can turn repeated overdraw
//! into mana-burn. The combat crate only counts; the cost is the game's rule.
use serde::{Deserialize, Serialize};

/// A bounded quantity.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pool {
    /// Current value. Mana may go below zero (overdraw); others stay at or above zero.
    pub current: f32,
    /// Maximum value.
    pub max: f32,
}

impl Pool {
    /// A full pool.
    #[must_use]
    pub fn full(max: f32) -> Self {
        let max = if max.is_finite() { max.max(0.0) } else { 0.0 };
        Self { current: max, max }
    }

    /// `current / max`, 0 when the pool is empty or has no size.
    #[must_use]
    pub fn fraction(&self) -> f32 {
        if self.max > 0.0 {
            (self.current / self.max).clamp(-1.0, 1.0)
        } else {
            0.0
        }
    }

    fn add(&mut self, amount: f32) {
        if amount.is_finite() {
            self.current = (self.current + amount).min(self.max);
        }
    }
}

/// What a mana spend did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ManaSpend {
    /// Paid from the well.
    Paid,
    /// Paid partly on credit; the value is how far below zero the well went.
    Overdrawn(f32),
    /// Not enough, and overdraw was not allowed or would pass the floor.
    Refused,
}

/// A combatant's pools and their regeneration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vitals {
    /// Hit points.
    pub health: Pool,
    /// Spent by swings, blocks and dodges.
    pub stamina: Pool,
    /// Lamps of mana; `max` is capacity.
    pub mana: Pool,
    /// Resistance to being staggered; broken at zero.
    pub poise: Pool,
    /// The meter silent casting and slow-time focus draw from.
    pub focus: Pool,
    /// Stamina per second once regeneration starts.
    pub stamina_regen: f32,
    /// Seconds after spending before stamina returns.
    pub stamina_delay: f32,
    /// Poise per second once regeneration starts.
    pub poise_regen: f32,
    /// Seconds after poise damage before poise returns.
    pub poise_delay: f32,
    /// Lamps per second. Wells refill slowly; most mana comes back with rest.
    pub mana_regen: f32,
    /// Focus per second.
    pub focus_regen: f32,
    /// How far below zero the well may go, as a fraction of capacity.
    pub overdraw_floor: f32,
    /// How many times this combatant has overdrawn (the game's mana-burn input).
    pub overdraws: u32,
    stamina_idle: f32,
    poise_idle: f32,
}

impl Vitals {
    /// Pools at the given maxima, everything full.
    #[must_use]
    pub fn new(health: f32, stamina: f32, capacity: f32, poise: f32) -> Self {
        Self {
            health: Pool::full(health),
            stamina: Pool::full(stamina),
            mana: Pool::full(capacity),
            poise: Pool::full(poise),
            focus: Pool::full(100.0),
            stamina_regen: 32.0,
            stamina_delay: 0.8,
            poise_regen: poise.max(1.0) * 0.35,
            poise_delay: 2.5,
            // A tenth of a percent a second: a fight barely refills a well.
            mana_regen: (capacity * 0.001).max(0.02),
            focus_regen: 6.0,
            overdraw_floor: 0.5,
            overdraws: 0,
            stamina_idle: 10.0,
            poise_idle: 10.0,
        }
    }

    /// Whether health is above zero.
    #[must_use]
    pub fn alive(&self) -> bool {
        self.health.current > 0.0
    }

    /// Take health. Returns the amount actually removed.
    pub fn damage(&mut self, amount: f32) -> f32 {
        if !amount.is_finite() || amount <= 0.0 || !self.alive() {
            return 0.0;
        }
        let taken = amount.min(self.health.current);
        self.health.current -= taken;
        taken
    }

    /// Restore health. Returns the amount actually restored.
    pub fn heal(&mut self, amount: f32) -> f32 {
        if !amount.is_finite() || amount <= 0.0 || !self.alive() {
            return 0.0;
        }
        let before = self.health.current;
        self.health.add(amount);
        self.health.current - before
    }

    /// Spend stamina. Any positive stamina allows the action (it may then go
    /// to zero), which is what keeps the last swing of a tired fighter possible.
    pub fn spend_stamina(&mut self, cost: f32) -> bool {
        if !cost.is_finite() || cost < 0.0 {
            return false;
        }
        if cost == 0.0 {
            return true;
        }
        if self.stamina.current <= 0.0 {
            return false;
        }
        self.stamina.current = (self.stamina.current - cost).max(0.0);
        self.stamina_idle = 0.0;
        true
    }

    /// Drain stamina without the "any positive" allowance (blocking a blow).
    /// Returns whether anything was left.
    pub fn drain_stamina(&mut self, cost: f32) -> bool {
        if !cost.is_finite() || cost <= 0.0 {
            return self.stamina.current > 0.0;
        }
        self.stamina.current -= cost;
        self.stamina_idle = 0.0;
        if self.stamina.current <= 0.0 {
            self.stamina.current = 0.0;
            false
        } else {
            true
        }
    }

    /// Spend mana, overdrawing if allowed.
    pub fn spend_mana(&mut self, cost: f32, allow_overdraw: bool) -> ManaSpend {
        if !cost.is_finite() || cost < 0.0 {
            return ManaSpend::Refused;
        }
        if cost <= self.mana.current {
            self.mana.current -= cost;
            return ManaSpend::Paid;
        }
        let floor = -self.mana.max * self.overdraw_floor.clamp(0.0, 1.0);
        if !allow_overdraw || self.mana.current - cost < floor {
            return ManaSpend::Refused;
        }
        self.mana.current -= cost;
        self.overdraws += 1;
        ManaSpend::Overdrawn(-self.mana.current)
    }

    /// Whether the well is below zero.
    #[must_use]
    pub fn overdrawn(&self) -> bool {
        self.mana.current < 0.0
    }

    /// Take poise. Returns true when this broke it (and resets it to full).
    pub fn break_poise(&mut self, amount: f32) -> bool {
        if !amount.is_finite() || amount <= 0.0 {
            return false;
        }
        self.poise_idle = 0.0;
        self.poise.current -= amount;
        if self.poise.current <= 0.0 {
            self.poise.current = self.poise.max;
            true
        } else {
            false
        }
    }

    /// Spend focus.
    pub fn spend_focus(&mut self, amount: f32) -> bool {
        if !amount.is_finite() || amount < 0.0 || self.focus.current < amount {
            return false;
        }
        self.focus.current -= amount;
        true
    }

    /// Regenerate. `exerting` (sprinting, blocking, channelling) holds stamina;
    /// `focusing` holds focus. An overdrawn well recovers at half rate, and
    /// stamina returns at half rate while it does (the shakes).
    pub fn tick(&mut self, dt: f32, exerting: bool, focusing: bool) {
        if !dt.is_finite() || dt <= 0.0 || !self.alive() {
            return;
        }
        self.stamina_idle += dt;
        self.poise_idle += dt;
        let shakes = if self.overdrawn() { 0.5 } else { 1.0 };
        if !exerting && self.stamina_idle >= self.stamina_delay {
            self.stamina.add(self.stamina_regen * shakes * dt);
        }
        if self.poise_idle >= self.poise_delay {
            self.poise.add(self.poise_regen * dt);
        }
        self.mana.add(self.mana_regen * shakes * dt);
        if !focusing {
            self.focus.add(self.focus_regen * dt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamina_allows_the_last_swing_then_waits_for_the_delay() {
        let mut v = Vitals::new(100.0, 30.0, 10.0, 20.0);
        assert!(v.spend_stamina(20.0));
        assert!(v.spend_stamina(20.0), "any stamina left allows the action");
        assert_eq!(v.stamina.current, 0.0);
        assert!(!v.spend_stamina(1.0));
        v.tick(0.5, false, false);
        assert_eq!(v.stamina.current, 0.0, "the delay holds regeneration");
        v.tick(0.5, false, false);
        assert!(v.stamina.current > 0.0);
    }

    #[test]
    fn overdraw_goes_on_credit_to_a_floor_and_is_counted() {
        let mut v = Vitals::new(100.0, 50.0, 10.0, 20.0);
        assert_eq!(v.spend_mana(8.0, false), ManaSpend::Paid);
        assert_eq!(v.spend_mana(5.0, false), ManaSpend::Refused);
        assert_eq!(v.spend_mana(5.0, true), ManaSpend::Overdrawn(3.0));
        assert!(v.overdrawn());
        assert_eq!(v.overdraws, 1);
        // The floor is half the capacity below zero.
        assert_eq!(v.spend_mana(3.0, true), ManaSpend::Refused);
        assert_eq!(v.spend_mana(2.0, true), ManaSpend::Overdrawn(5.0));
    }

    #[test]
    fn poise_breaks_and_resets() {
        let mut v = Vitals::new(100.0, 50.0, 10.0, 30.0);
        assert!(!v.break_poise(20.0));
        assert!(v.break_poise(15.0));
        assert_eq!(v.poise.current, 30.0);
    }

    #[test]
    fn damage_and_heal_clamp_and_the_dead_stay_dead() {
        let mut v = Vitals::new(50.0, 50.0, 10.0, 30.0);
        assert_eq!(v.damage(80.0), 50.0);
        assert!(!v.alive());
        assert_eq!(v.heal(10.0), 0.0);
        assert_eq!(v.damage(f32::NAN), 0.0);
    }
}
