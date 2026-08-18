//! Ownership tokens: every engine resource a script acquires is tagged
//! with the attachment that asked for it.
//!
//! # Why teardown is not a checklist
//!
//! The alternative is discipline: whoever writes the audio binding
//! remembers to stop the voice in `onDestroy`, whoever writes the task
//! binding remembers to cancel it, and a reload that forgets one of them
//! leaks quietly until the hundredth cycle. Reload leaks are the failure
//! mode the plan's stress test exists to catch, and a checklist is not a
//! design.
//!
//! So acquisition goes through a ledger. Teardown asks the ledger what an
//! attachment owns and releases all of it, which means a new resource kind
//! is complete the moment it is recorded — there is no second place to
//! remember to edit.

use std::collections::BTreeMap;

use somnium_ecs::Entity;

use crate::ids::InstanceUuid;

/// Something an attachment holds that has to be given back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnedResource {
    /// A playing sound voice, by the host's handle index.
    Audio(u64),
    /// A subscription to a named event.
    Subscription(String),
    /// A deferred task or coroutine driven by the engine.
    Task(u64),
    /// An entity the attachment spawned and asked to own, so that
    /// destroying the script destroys what it made.
    Entity(Entity),
}

/// A receipt for one acquired resource.
///
/// Carries its owner so a release cannot be misattributed, and a sequence
/// so two resources of the same kind are distinguishable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OwnershipToken {
    owner: InstanceUuid,
    sequence: u64,
}

impl OwnershipToken {
    /// The attachment that holds it.
    #[must_use]
    pub const fn owner(self) -> InstanceUuid {
        self.owner
    }
}

/// Who owns what.
#[derive(Debug, Default)]
pub struct ResourceLedger {
    next: u64,
    /// `BTreeMap` and an ordered `Vec` per owner: release order is part of
    /// determinism, and a hash map's iteration order is not.
    owned: BTreeMap<InstanceUuid, Vec<(u64, OwnedResource)>>,
}

impl ResourceLedger {
    /// An empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `owner` now holds `resource`.
    pub fn acquire(&mut self, owner: InstanceUuid, resource: OwnedResource) -> OwnershipToken {
        self.next += 1;
        let sequence = self.next;
        self.owned
            .entry(owner)
            .or_default()
            .push((sequence, resource));
        OwnershipToken { owner, sequence }
    }

    /// Give one resource back.
    pub fn release(&mut self, token: OwnershipToken) -> Option<OwnedResource> {
        let held = self.owned.get_mut(&token.owner)?;
        let index = held.iter().position(|(seq, _)| *seq == token.sequence)?;
        let (_, resource) = held.remove(index);
        if held.is_empty() {
            self.owned.remove(&token.owner);
        }
        Some(resource)
    }

    /// Give everything one attachment holds back, in acquisition order.
    ///
    /// This is the whole point of the type: teardown is one call, and it
    /// cannot miss a resource kind that was added later.
    pub fn release_all(&mut self, owner: InstanceUuid) -> Vec<OwnedResource> {
        self.owned
            .remove(&owner)
            .map(|held| held.into_iter().map(|(_, resource)| resource).collect())
            .unwrap_or_default()
    }

    /// How many resources one attachment holds.
    #[must_use]
    pub fn count(&self, owner: InstanceUuid) -> usize {
        self.owned.get(&owner).map_or(0, Vec::len)
    }

    /// How many resources are held in total. A reload stress test asserts
    /// on this returning to where it started.
    #[must_use]
    pub fn total(&self) -> usize {
        self.owned.values().map(Vec::len).sum()
    }

    /// Whether nothing is held at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.owned.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(raw: u128) -> InstanceUuid {
        InstanceUuid::from_raw(raw)
    }

    #[test]
    fn teardown_releases_everything_one_attachment_holds() {
        let mut ledger = ResourceLedger::new();
        let a = owner(1);
        let b = owner(2);

        ledger.acquire(a, OwnedResource::Audio(7));
        ledger.acquire(a, OwnedResource::Subscription("door.opened".into()));
        ledger.acquire(b, OwnedResource::Task(1));

        assert_eq!(ledger.count(a), 2);
        assert_eq!(ledger.total(), 3);

        let released = ledger.release_all(a);
        assert_eq!(
            released,
            vec![
                OwnedResource::Audio(7),
                OwnedResource::Subscription("door.opened".into())
            ],
            "release order is acquisition order"
        );
        assert_eq!(ledger.count(a), 0);
        assert_eq!(ledger.count(b), 1, "a peer's resources are untouched");
    }

    #[test]
    fn a_token_releases_exactly_one_resource() {
        let mut ledger = ResourceLedger::new();
        let a = owner(1);
        let first = ledger.acquire(a, OwnedResource::Audio(1));
        let second = ledger.acquire(a, OwnedResource::Audio(2));

        assert_eq!(ledger.release(first), Some(OwnedResource::Audio(1)));
        assert_eq!(ledger.count(a), 1);
        assert_eq!(ledger.release(first), None, "a token is spent once");
        assert_eq!(ledger.release(second), Some(OwnedResource::Audio(2)));
        assert!(ledger.is_empty());
    }

    #[test]
    fn a_token_cannot_release_a_peers_resource() {
        let mut ledger = ResourceLedger::new();
        let a = owner(1);
        let b = owner(2);
        let token = ledger.acquire(a, OwnedResource::Audio(1));
        ledger.acquire(b, OwnedResource::Audio(2));

        assert_eq!(token.owner(), a);
        // The sequence is unique across owners, so even a forged token
        // scoped to `b` finds nothing.
        let forged = OwnershipToken {
            owner: b,
            sequence: 1,
        };
        assert_eq!(ledger.release(forged), None);
        assert_eq!(ledger.count(a), 1);
    }

    #[test]
    fn releasing_an_owner_that_holds_nothing_is_not_an_error() {
        let mut ledger = ResourceLedger::new();
        assert!(ledger.release_all(owner(9)).is_empty());
        assert!(ledger.is_empty());
    }

    #[test]
    fn a_hundred_acquire_release_cycles_leave_nothing_behind() {
        let mut ledger = ResourceLedger::new();
        for cycle in 0_u32..100 {
            let id = owner(u128::from(cycle));
            ledger.acquire(id, OwnedResource::Audio(u64::from(cycle)));
            ledger.acquire(id, OwnedResource::Task(u64::from(cycle)));
            assert_eq!(ledger.release_all(id).len(), 2);
        }
        assert_eq!(ledger.total(), 0, "no growth across a hundred cycles");
        assert!(ledger.is_empty());
    }
}
