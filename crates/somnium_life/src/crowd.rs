//! A crowd's bodies under a budget.
//!
//! Simulating a realm's villagers is cheap (thousands in a fraction of a
//! millisecond); drawing them is not: each body is an entity, draws, and an
//! animated rig. [`Bodies`] keeps a fixed number of slots and hands them to the
//! nearest agents worth drawing, with hysteresis so two agents at nearly the
//! same distance do not trade a body every frame. The host spawns one entity
//! per slot once, re-dresses a slot's entity when [`Bodies::assign`] says its
//! holder changed, and parks the entities of empty slots.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

/// Fixed body slots, handed to the nearest candidates.
#[derive(Clone, Debug)]
pub struct Bodies<K> {
    holders: Vec<Option<K>>,
    slot_of: HashMap<K, usize>,
    /// A holder keeps its body against a newcomer nearer by less than this
    /// (metres).
    pub keep: f32,
}

impl<K: Copy + Eq + Hash> Bodies<K> {
    /// `slots` bodies, empty.
    #[must_use]
    pub fn new(slots: usize) -> Self {
        Self {
            holders: vec![None; slots],
            slot_of: HashMap::new(),
            keep: 4.0,
        }
    }

    /// How many bodies there are.
    #[must_use]
    pub fn slots(&self) -> usize {
        self.holders.len()
    }

    /// Who holds `slot`.
    #[must_use]
    pub fn holder(&self, slot: usize) -> Option<K> {
        self.holders.get(slot).copied().flatten()
    }

    /// Which slot `key` holds.
    #[must_use]
    pub fn slot_of(&self, key: K) -> Option<usize> {
        self.slot_of.get(&key).copied()
    }

    /// How many slots are held.
    #[must_use]
    pub fn held(&self) -> usize {
        self.slot_of.len()
    }

    /// Give the bodies to the nearest `candidates` (each an agent worth
    /// drawing and its distance to the viewer; reordered in place). Agents not
    /// among them lose their bodies. Returns the slots whose holder changed,
    /// in order: re-dress or park those.
    pub fn assign(&mut self, candidates: &mut [(K, f32)]) -> Vec<usize> {
        let n = self.slots();
        let keep = self.keep;
        let held = &self.slot_of;
        let rank = |&(k, d): &(K, f32)| if held.contains_key(&k) { d - keep } else { d };
        if candidates.len() > n && n > 0 {
            candidates.select_nth_unstable_by(n - 1, |a, b| {
                rank(a).partial_cmp(&rank(b)).unwrap_or(Ordering::Equal)
            });
        }
        let wanted: HashSet<K> = candidates.iter().take(n).map(|(k, _)| *k).collect();
        let mut changed = Vec::new();
        for (slot, holder) in self.holders.iter_mut().enumerate() {
            if let Some(k) = *holder
                && !wanted.contains(&k)
            {
                *holder = None;
                self.slot_of.remove(&k);
                changed.push(slot);
            }
        }
        let free: Vec<usize> = (0..n).filter(|s| self.holders[*s].is_none()).collect();
        let mut free = free.into_iter();
        for (k, _) in candidates.iter().take(n) {
            if self.slot_of.contains_key(k) {
                continue;
            }
            let Some(slot) = free.next() else { break };
            self.holders[slot] = Some(*k);
            self.slot_of.insert(*k, slot);
            changed.push(slot);
        }
        changed.sort_unstable();
        changed.dedup();
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nearest_get_bodies_and_keep_them_against_near_ties() {
        let mut bodies = Bodies::new(2);
        let mut c = vec![(1, 50.0), (2, 10.0), (3, 30.0), (4, 5.0)];
        let changed = bodies.assign(&mut c);
        assert_eq!(changed, vec![0, 1]);
        assert!(bodies.slot_of(4).is_some() && bodies.slot_of(2).is_some());
        assert_eq!(bodies.held(), 2);
        // 3 comes a little nearer than 2: within `keep`, 2 keeps its body.
        let mut c = vec![(1, 50.0), (2, 10.0), (3, 8.0), (4, 5.0)];
        assert!(bodies.assign(&mut c).is_empty());
        // Much nearer: 3 takes the slot 2 had.
        let slot = bodies.slot_of(2).unwrap();
        let mut c = vec![(1, 50.0), (2, 10.0), (3, 2.0), (4, 5.0)];
        assert_eq!(bodies.assign(&mut c), vec![slot]);
        assert_eq!(bodies.holder(slot), Some(3));
        assert_eq!(bodies.slot_of(2), None);
        // Gone indoors (no longer a candidate): its body is freed.
        let mut c = vec![(4, 5.0)];
        assert_eq!(bodies.assign(&mut c), vec![slot]);
        assert_eq!(bodies.held(), 1);
        assert_eq!(bodies.holder(slot), None);
    }

    #[test]
    fn a_crowd_of_thousands_fills_every_body_with_its_nearest() {
        let mut bodies = Bodies::new(100);
        let mut c: Vec<(u32, f32)> = (0..5000).map(|i| (i, ((i * 7919) % 5000) as f32)).collect();
        bodies.assign(&mut c);
        assert_eq!(bodies.held(), 100);
        let worst = (0..100)
            .filter_map(|s| bodies.holder(s))
            .map(|k| ((k * 7919) % 5000) as f32)
            .fold(0.0, f32::max);
        assert!(worst < 100.0, "held the 100 nearest, worst {worst}");
    }
}
