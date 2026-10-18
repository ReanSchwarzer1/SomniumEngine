//! Teams and who fights whom.
//!
//! Attitudes are declared between teams; a personal grudge (someone neutral
//! struck you) overrides them for that pair alone, which is what lets a guard
//! answer the one citizen who swung at him without the whole street turning.
use crate::CombatantId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A team number. Team 0 is conventionally the protagonist's party.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TeamId(pub u16);

/// How one team regards another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attitude {
    /// Fights on sight.
    Hostile,
    /// Ignores unless provoked.
    Neutral,
    /// Fights beside; never a target.
    Allied,
}

/// The attitude table and personal grudges.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Relations {
    table: BTreeMap<(TeamId, TeamId), Attitude>,
    grudges: BTreeSet<(CombatantId, CombatantId)>,
}

impl Relations {
    /// Declare a symmetric attitude between two teams.
    pub fn set(&mut self, a: TeamId, b: TeamId, attitude: Attitude) {
        let key = if a <= b { (a, b) } else { (b, a) };
        self.table.insert(key, attitude);
    }

    /// The declared attitude: allied with itself, neutral by default.
    #[must_use]
    pub fn between(&self, a: TeamId, b: TeamId) -> Attitude {
        if a == b {
            return self.table.get(&(a, a)).copied().unwrap_or(Attitude::Allied);
        }
        let key = if a <= b { (a, b) } else { (b, a) };
        self.table.get(&key).copied().unwrap_or(Attitude::Neutral)
    }

    /// Record that `aggressor` attacked `victim`: the pair are now enemies
    /// whatever their teams say.
    pub fn provoke(&mut self, aggressor: CombatantId, victim: CombatantId) {
        if aggressor != victim {
            let key = if aggressor <= victim {
                (aggressor, victim)
            } else {
                (victim, aggressor)
            };
            self.grudges.insert(key);
        }
    }

    /// Forget every grudge involving `id` (it died, or was pardoned).
    pub fn forgive(&mut self, id: CombatantId) {
        self.grudges.retain(|(a, b)| *a != id && *b != id);
    }

    /// Whether `a` should fight `b`.
    #[must_use]
    pub fn hostile(&self, a: (CombatantId, TeamId), b: (CombatantId, TeamId)) -> bool {
        if a.0 == b.0 {
            return false;
        }
        let key = if a.0 <= b.0 { (a.0, b.0) } else { (b.0, a.0) };
        if self.grudges.contains(&key) {
            return true;
        }
        self.between(a.1, b.1) == Attitude::Hostile
    }

    /// Whether `a` counts `b` a friend (never a direct target).
    #[must_use]
    pub fn allied(&self, a: (CombatantId, TeamId), b: (CombatantId, TeamId)) -> bool {
        a.0 == b.0 || (!self.hostile(a, b) && self.between(a.1, b.1) == Attitude::Allied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_attitudes_and_grudges() {
        let mut r = Relations::default();
        let (party, raiders, town) = (TeamId(0), TeamId(1), TeamId(2));
        r.set(party, raiders, Attitude::Hostile);
        let hero = (CombatantId(1), party);
        let raider = (CombatantId(2), raiders);
        let citizen = (CombatantId(3), town);
        let friend = (CombatantId(4), party);
        assert!(r.hostile(hero, raider));
        assert!(r.hostile(raider, hero), "symmetric");
        assert!(!r.hostile(hero, citizen));
        assert!(r.allied(hero, friend));
        assert!(!r.hostile(hero, hero));
        r.provoke(citizen.0, hero.0);
        assert!(r.hostile(hero, citizen));
        assert!(!r.allied(hero, citizen));
        r.forgive(citizen.0);
        assert!(!r.hostile(hero, citizen));
    }
}
