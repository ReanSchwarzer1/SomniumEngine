//! Status effects and how they meet each other.
//!
//! The interactions are the systemic part of Rimeborn's combat (after
//! Divinity: Original Sin): water puts out fire, fire dries water and thaws
//! ice, cold on a wet body freezes it twice as fast, and a blunt blow shatters
//! the frozen. Each rule lives in [`StatusSet::apply`] so a spell, a weapon and
//! a burning field all obey the same ones.
use serde::{Deserialize, Serialize};

/// The status effects. Chill and Bleed build up to a threshold; the rest are
/// timed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum StatusKind {
    /// Cold build-up: slows movement and attacks; at 100 it becomes Freeze.
    Chill,
    /// Frozen solid: cannot act; a heavy blunt blow shatters it for extra harm.
    Freeze,
    /// On fire: Flame damage over time; put out by water and frost.
    Burn,
    /// Soaked: frost builds faster, flame does less, fire will not catch.
    Wet,
    /// Reeling: cannot act for a moment; interrupts swings and spells.
    Stagger,
    /// On the ground: cannot act; then gets up.
    Knockdown,
    /// Blood loss build-up: at 100 it bursts for a share of maximum health.
    Bleed,
}

impl StatusKind {
    /// Every kind, in index order.
    pub const ALL: [Self; 7] = [
        Self::Chill,
        Self::Freeze,
        Self::Burn,
        Self::Wet,
        Self::Stagger,
        Self::Knockdown,
        Self::Bleed,
    ];

    /// Index into per-kind tables.
    #[must_use]
    pub fn index(self) -> usize {
        self as usize
    }

    /// Display name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Chill => "Chill",
            Self::Freeze => "Freeze",
            Self::Burn => "Burn",
            Self::Wet => "Wet",
            Self::Stagger => "Stagger",
            Self::Knockdown => "Knockdown",
            Self::Bleed => "Bleed",
        }
    }
}

/// Build-up at which Chill freezes and Bleed bursts.
pub const THRESHOLD: f32 = 100.0;
/// Seconds frozen when Chill reaches the threshold.
pub const FREEZE_SECONDS: f32 = 2.5;
/// Seconds a Burn lasts after its last application.
pub const BURN_SECONDS: f32 = 4.0;
/// Chill build-up lost per second.
pub const CHILL_DECAY: f32 = 12.0;
/// Bleed build-up lost per second.
pub const BLEED_DECAY: f32 = 6.0;
/// How much faster frost builds on a wet body.
pub const WET_FROST_MULTIPLIER: f32 = 1.75;
/// How much of a Flame hit a wet body takes.
pub const WET_FLAME_MULTIPLIER: f32 = 0.6;
/// Share of maximum health a bleed burst takes (plus [`BLEED_FLAT`]).
pub const BLEED_FRACTION: f32 = 0.12;
/// Flat part of a bleed burst.
pub const BLEED_FLAT: f32 = 10.0;

/// Something a status change wants the world to know or do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StatusChange {
    /// A status became active.
    Applied(StatusKind),
    /// A status ended on its own or was removed.
    Ended(StatusKind),
    /// Fire met water or frost and went out.
    Extinguished,
    /// Fire met wet: the water went to steam instead.
    Steam,
    /// Blood burst: this much damage, to apply as true damage.
    BleedBurst(f32),
}

/// One combatant's statuses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatusSet {
    buildup: [f32; 7],
    remaining: [f32; 7],
    burn_dps: f32,
    /// Per kind, 0 to 1: reduces build-up and durations; 1 is immune.
    pub resist: [f32; 7],
}

impl Default for StatusSet {
    fn default() -> Self {
        Self {
            buildup: [0.0; 7],
            remaining: [0.0; 7],
            burn_dps: 0.0,
            resist: [0.0; 7],
        }
    }
}

impl StatusSet {
    fn scale(&self, kind: StatusKind) -> f32 {
        let r = self.resist[kind.index()];
        if r.is_finite() {
            (1.0 - r).clamp(0.0, 1.0)
        } else {
            1.0
        }
    }

    /// Whether `kind` is in effect now.
    #[must_use]
    pub fn active(&self, kind: StatusKind) -> bool {
        match kind {
            StatusKind::Chill => self.buildup[kind.index()] > 0.0,
            _ => self.remaining[kind.index()] > 0.0,
        }
    }

    /// Current build-up for Chill or Bleed, 0 to 100.
    #[must_use]
    pub fn buildup(&self, kind: StatusKind) -> f32 {
        self.buildup[kind.index()]
    }

    /// Seconds left on a timed status.
    #[must_use]
    pub fn remaining(&self, kind: StatusKind) -> f32 {
        self.remaining[kind.index()]
    }

    /// Flame damage per second while burning.
    #[must_use]
    pub fn burn_dps(&self) -> f32 {
        if self.active(StatusKind::Burn) {
            self.burn_dps
        } else {
            0.0
        }
    }

    /// Chill as a fraction, 0 to 1.
    #[must_use]
    pub fn chill_level(&self) -> f32 {
        (self.buildup[StatusKind::Chill.index()] / THRESHOLD).clamp(0.0, 1.0)
    }

    /// Whether the body can act at all (attack, cast, block, dodge).
    #[must_use]
    pub fn can_act(&self) -> bool {
        !(self.active(StatusKind::Freeze)
            || self.active(StatusKind::Stagger)
            || self.active(StatusKind::Knockdown))
    }

    /// Movement speed multiplier.
    #[must_use]
    pub fn move_scale(&self) -> f32 {
        if self.active(StatusKind::Freeze) || self.active(StatusKind::Knockdown) {
            0.0
        } else if self.active(StatusKind::Stagger) {
            0.15
        } else {
            1.0 - 0.4 * self.chill_level()
        }
    }

    /// Attack and casting speed multiplier.
    #[must_use]
    pub fn action_scale(&self) -> f32 {
        1.0 - 0.3 * self.chill_level()
    }

    /// Multiplier for incoming damage of a kind, from statuses alone.
    #[must_use]
    pub fn incoming_scale(&self, kind: crate::DamageType) -> f32 {
        match kind {
            crate::DamageType::Flame if self.active(StatusKind::Wet) => WET_FLAME_MULTIPLIER,
            _ => 1.0,
        }
    }

    /// Remove a status outright.
    pub fn clear(&mut self, kind: StatusKind, out: &mut Vec<StatusChange>) {
        let i = kind.index();
        if self.active(kind) {
            out.push(StatusChange::Ended(kind));
        }
        self.buildup[i] = 0.0;
        self.remaining[i] = 0.0;
        if kind == StatusKind::Burn {
            self.burn_dps = 0.0;
        }
    }

    /// Apply `amount` of `kind`. The meaning of `amount` is per kind: build-up
    /// points for Chill and Bleed, flame damage per second for Burn, seconds
    /// for Freeze, Wet, Stagger and Knockdown. `max_health` sizes a bleed burst.
    pub fn apply(
        &mut self,
        kind: StatusKind,
        amount: f32,
        max_health: f32,
        out: &mut Vec<StatusChange>,
    ) {
        if !amount.is_finite() || amount <= 0.0 {
            return;
        }
        let amount = amount * self.scale(kind);
        if amount <= 0.0 {
            return;
        }
        let i = kind.index();
        let was = self.active(kind);
        match kind {
            StatusKind::Chill => {
                let mut points = amount;
                if self.active(StatusKind::Wet) {
                    points *= WET_FROST_MULTIPLIER;
                }
                if self.active(StatusKind::Burn) {
                    // Frost puts the fire out and spends itself doing it.
                    self.clear(StatusKind::Burn, out);
                    out.push(StatusChange::Extinguished);
                    points *= 0.5;
                }
                self.buildup[i] += points;
                if !was {
                    out.push(StatusChange::Applied(kind));
                }
                if self.buildup[i] >= THRESHOLD {
                    self.buildup[i] = 0.0;
                    out.push(StatusChange::Ended(StatusKind::Chill));
                    self.apply(StatusKind::Freeze, FREEZE_SECONDS, max_health, out);
                }
            }
            StatusKind::Freeze => {
                // The water on the body is the ice now.
                if self.active(StatusKind::Wet) {
                    self.clear(StatusKind::Wet, out);
                }
                if self.active(StatusKind::Burn) {
                    self.clear(StatusKind::Burn, out);
                    out.push(StatusChange::Extinguished);
                }
                self.remaining[i] = self.remaining[i].max(amount);
                if !was {
                    out.push(StatusChange::Applied(kind));
                }
            }
            StatusKind::Burn => {
                if self.active(StatusKind::Wet) {
                    // Fire dries the body instead of catching.
                    self.clear(StatusKind::Wet, out);
                    out.push(StatusChange::Steam);
                    return;
                }
                if self.active(StatusKind::Freeze) {
                    self.clear(StatusKind::Freeze, out);
                }
                if self.active(StatusKind::Chill) {
                    self.clear(StatusKind::Chill, out);
                }
                self.remaining[i] = BURN_SECONDS * self.scale(kind).max(0.25);
                self.burn_dps = self.burn_dps.max(amount);
                if !was {
                    out.push(StatusChange::Applied(kind));
                }
            }
            StatusKind::Wet => {
                if self.active(StatusKind::Burn) {
                    self.clear(StatusKind::Burn, out);
                    out.push(StatusChange::Extinguished);
                }
                self.remaining[i] = self.remaining[i].max(amount);
                if !was {
                    out.push(StatusChange::Applied(kind));
                }
            }
            StatusKind::Stagger => {
                if self.active(StatusKind::Knockdown) {
                    return;
                }
                self.remaining[i] = self.remaining[i].max(amount);
                if !was {
                    out.push(StatusChange::Applied(kind));
                }
            }
            StatusKind::Knockdown => {
                if self.active(StatusKind::Stagger) {
                    self.clear(StatusKind::Stagger, out);
                }
                self.remaining[i] = self.remaining[i].max(amount);
                if !was {
                    out.push(StatusChange::Applied(kind));
                }
            }
            StatusKind::Bleed => {
                self.buildup[i] += amount;
                if self.buildup[i] >= THRESHOLD {
                    self.buildup[i] = 0.0;
                    self.remaining[i] = 0.4;
                    let max = if max_health.is_finite() {
                        max_health.max(0.0)
                    } else {
                        0.0
                    };
                    out.push(StatusChange::BleedBurst(max * BLEED_FRACTION + BLEED_FLAT));
                }
            }
        }
    }

    /// Advance timers and build-up decay. Returns the flame damage the burn
    /// dealt over `dt` (before resistances; the world mitigates it).
    pub fn tick(&mut self, dt: f32, out: &mut Vec<StatusChange>) -> f32 {
        if !dt.is_finite() || dt <= 0.0 {
            return 0.0;
        }
        let mut burn = 0.0;
        if self.active(StatusKind::Burn) {
            burn = self.burn_dps * dt.min(self.remaining[StatusKind::Burn.index()]);
        }
        for kind in StatusKind::ALL {
            let i = kind.index();
            match kind {
                StatusKind::Chill => {
                    if self.buildup[i] > 0.0 {
                        self.buildup[i] = (self.buildup[i] - CHILL_DECAY * dt).max(0.0);
                        if self.buildup[i] == 0.0 {
                            out.push(StatusChange::Ended(kind));
                        }
                    }
                }
                StatusKind::Bleed => {
                    self.buildup[i] = (self.buildup[i] - BLEED_DECAY * dt).max(0.0);
                    self.remaining[i] = (self.remaining[i] - dt).max(0.0);
                }
                _ => {
                    if self.remaining[i] > 0.0 {
                        self.remaining[i] = (self.remaining[i] - dt).max(0.0);
                        if self.remaining[i] == 0.0 {
                            if kind == StatusKind::Burn {
                                self.burn_dps = 0.0;
                            }
                            out.push(StatusChange::Ended(kind));
                        }
                    }
                }
            }
        }
        burn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(set: &mut StatusSet, kind: StatusKind, amount: f32) -> Vec<StatusChange> {
        let mut out = Vec::new();
        set.apply(kind, amount, 200.0, &mut out);
        out
    }

    #[test]
    fn chill_builds_to_freeze_and_wet_freezes_faster() {
        let mut dry = StatusSet::default();
        apply(&mut dry, StatusKind::Chill, 60.0);
        assert!(!dry.active(StatusKind::Freeze));
        assert!(dry.move_scale() < 1.0 && dry.move_scale() > 0.0);

        let mut wet = StatusSet::default();
        apply(&mut wet, StatusKind::Wet, 8.0);
        let out = apply(&mut wet, StatusKind::Chill, 60.0);
        assert!(wet.active(StatusKind::Freeze), "60 x 1.75 crosses 100");
        assert!(!wet.active(StatusKind::Wet), "the water became the ice");
        assert!(out.contains(&StatusChange::Applied(StatusKind::Freeze)));
        assert!(!wet.can_act());
        assert_eq!(wet.move_scale(), 0.0);
    }

    #[test]
    fn water_and_fire_cancel() {
        let mut s = StatusSet::default();
        apply(&mut s, StatusKind::Burn, 5.0);
        assert!(s.active(StatusKind::Burn));
        let out = apply(&mut s, StatusKind::Wet, 6.0);
        assert!(!s.active(StatusKind::Burn));
        assert!(out.contains(&StatusChange::Extinguished));

        // Fire on a wet body steams it dry and does not catch.
        let out = apply(&mut s, StatusKind::Burn, 5.0);
        assert!(!s.active(StatusKind::Burn));
        assert!(!s.active(StatusKind::Wet));
        assert!(out.contains(&StatusChange::Steam));
        // The next flame catches.
        apply(&mut s, StatusKind::Burn, 5.0);
        assert!(s.active(StatusKind::Burn));
    }

    #[test]
    fn fire_thaws_and_frost_smothers() {
        let mut s = StatusSet::default();
        apply(&mut s, StatusKind::Freeze, 3.0);
        apply(&mut s, StatusKind::Burn, 4.0);
        assert!(!s.active(StatusKind::Freeze), "fire thaws");
        let out = apply(&mut s, StatusKind::Chill, 20.0);
        assert!(!s.active(StatusKind::Burn), "frost smothers the fire");
        assert!(out.contains(&StatusChange::Extinguished));
        assert!((s.buildup(StatusKind::Chill) - 10.0).abs() < 1e-4);
    }

    #[test]
    fn burn_deals_damage_over_its_duration_then_ends() {
        let mut s = StatusSet::default();
        apply(&mut s, StatusKind::Burn, 6.0);
        let mut out = Vec::new();
        let mut total = 0.0;
        for _ in 0..600 {
            total += s.tick(1.0 / 60.0, &mut out);
        }
        assert!(
            (total - 6.0 * BURN_SECONDS).abs() < 0.2,
            "burned for {total}"
        );
        assert!(out.contains(&StatusChange::Ended(StatusKind::Burn)));
        assert_eq!(s.burn_dps(), 0.0);
    }

    #[test]
    fn bleed_bursts_at_the_threshold() {
        let mut s = StatusSet::default();
        assert!(apply(&mut s, StatusKind::Bleed, 70.0).is_empty());
        let out = apply(&mut s, StatusKind::Bleed, 40.0);
        let burst = out.iter().find_map(|c| match c {
            StatusChange::BleedBurst(d) => Some(*d),
            _ => None,
        });
        assert_eq!(burst, Some(200.0 * BLEED_FRACTION + BLEED_FLAT));
        assert_eq!(s.buildup(StatusKind::Bleed), 0.0);
    }

    #[test]
    fn resistance_reduces_and_immunity_blocks() {
        let mut s = StatusSet::default();
        s.resist[StatusKind::Stagger.index()] = 1.0;
        apply(&mut s, StatusKind::Stagger, 1.0);
        assert!(!s.active(StatusKind::Stagger));
        s.resist[StatusKind::Chill.index()] = 0.5;
        apply(&mut s, StatusKind::Chill, 40.0);
        assert!((s.buildup(StatusKind::Chill) - 20.0).abs() < 1e-4);
    }

    #[test]
    fn knockdown_overrides_stagger_and_chill_decays() {
        let mut s = StatusSet::default();
        apply(&mut s, StatusKind::Stagger, 0.5);
        apply(&mut s, StatusKind::Knockdown, 1.5);
        assert!(!s.active(StatusKind::Stagger));
        apply(&mut s, StatusKind::Stagger, 0.5);
        assert!(!s.active(StatusKind::Stagger), "no stagger while down");
        apply(&mut s, StatusKind::Chill, 24.0);
        let mut out = Vec::new();
        for _ in 0..180 {
            s.tick(1.0 / 60.0, &mut out);
        }
        assert!(!s.active(StatusKind::Chill));
        assert!(!s.active(StatusKind::Knockdown));
        assert!(s.can_act());
    }
}
