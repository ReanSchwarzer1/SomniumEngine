//! Deterministic script ordering.
//!
//! Phase 16 promises deterministic scheduling on the same build and
//! platform. That promise is only worth anything if the order scripts run
//! in — and the order their commands are applied in — is computed from
//! authored data rather than observed from engine internals.
//!
//! The key is, in order of precedence:
//!
//! 1. `execution_order` — the author's explicit intent;
//! 2. the entity's [`PersistentId`] — durable across save and load;
//! 3. the attachment's [`InstanceUuid`] — durable, and unique per
//!    attachment, so the key is a total order with no ties.
//!
//! What it deliberately is **not**: archetype traversal order, lazily
//! assigned `ComponentId` order, hash-map iteration order, or directory
//! enumeration order. Every one of those varies between runs, between
//! machines, or with unrelated edits elsewhere in the engine.

use somnium_ecs::PersistentId;

use crate::ids::InstanceUuid;

/// The total order in which script attachments run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OrderKey {
    /// Author's explicit ordering. Lower runs first.
    pub execution_order: i32,
    /// Durable entity identity — the first tiebreak.
    pub entity: PersistentId,
    /// Durable attachment identity — the final tiebreak, and the reason
    /// this key is total rather than merely mostly-ordered.
    pub instance: InstanceUuid,
}

impl OrderKey {
    /// Build a key.
    #[must_use]
    pub const fn new(execution_order: i32, entity: PersistentId, instance: InstanceUuid) -> Self {
        Self {
            execution_order,
            entity,
            instance,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(order: i32, entity: u128, instance: u128) -> OrderKey {
        OrderKey::new(
            order,
            PersistentId::from_raw(entity),
            InstanceUuid::from_raw(instance),
        )
    }

    #[test]
    fn explicit_order_dominates_identity() {
        let early = key(-5, u128::MAX, u128::MAX);
        let late = key(0, 1, 1);
        assert!(early < late);
    }

    #[test]
    fn entity_identity_breaks_ties_before_attachment_identity() {
        let a = key(0, 1, 999);
        let b = key(0, 2, 1);
        assert!(a < b, "entity id is the earlier tiebreak");

        let c = key(0, 2, 2);
        assert!(b < c, "attachment id is the final tiebreak");
    }

    #[test]
    fn the_key_is_a_total_order_with_no_ties() {
        let mut keys = vec![key(1, 5, 5), key(0, 9, 1), key(1, 5, 4), key(0, 2, 7)];
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![key(0, 2, 7), key(0, 9, 1), key(1, 5, 4), key(1, 5, 5)]
        );
        // No two distinct attachments can compare equal, because
        // `instance` is unique.
        assert_ne!(key(0, 1, 1), key(0, 1, 2));
    }

    #[test]
    fn sorting_is_independent_of_input_order() {
        let all = vec![key(2, 3, 4), key(-1, 8, 2), key(2, 3, 1), key(0, 0, 9)];
        let mut forward = all.clone();
        let mut backward = all;
        backward.reverse();
        forward.sort_unstable();
        backward.sort_unstable();
        assert_eq!(forward, backward);
    }
}
