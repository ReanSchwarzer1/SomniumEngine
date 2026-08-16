//! Phase 16-A: durable entity identity.
//!
//! [`Entity`] is an index plus a generation. It is the right runtime
//! handle — small, copyable, and able to detect a stale reference — and
//! the wrong durable name, because both halves are allocator state. Save
//! a scene, reload it, and the same game object is almost certainly a
//! different index.
//!
//! [`PersistentId`] is the name that survives. It is what a scene file
//! writes for a parent reference, what a script attachment records for
//! the entity it belongs to, and what an ordering key sorts on so a
//! frame's script execution order does not depend on allocator history.
//!
//! Two identifiers, two jobs — the same split as [`ComponentId`] versus
//! [`StableId`](crate::reflect::StableId).
//!
//! # Uniqueness
//!
//! An id is a 64-bit per-process session seed in the high half and a
//! monotonic counter in the low half. Ids minted in one editing session
//! therefore cannot collide with each other at all, and cannot collide
//! with another session's without a 64-bit seed collision. That is a
//! weaker guarantee than a cryptographic UUID and a much stronger one
//! than the engine needs, and it costs no dependency.
//!
//! [`ComponentId`]: crate::component::ComponentId

use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::component::Component;
use crate::entity::Entity;
use crate::world::{EcsError, World};

/// A durable identifier for an entity, stable across save and load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PersistentId(u128);

impl Component for PersistentId {}

impl PersistentId {
    /// The id that means "no entity". Never minted.
    pub const NONE: Self = Self(0);

    /// Mint a fresh id. Never returns [`Self::NONE`].
    #[must_use]
    pub fn mint() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        let low = COUNTER.fetch_add(1, Ordering::Relaxed);
        Self((u128::from(session_seed()) << 64) | u128::from(low))
    }

    /// Rebuild an id read from a file.
    #[inline]
    #[must_use]
    pub const fn from_raw(raw: u128) -> Self {
        Self(raw)
    }

    /// The raw value, for serialization only.
    #[inline]
    #[must_use]
    pub const fn raw(self) -> u128 {
        self.0
    }

    /// Whether this is the "no entity" id.
    #[inline]
    #[must_use]
    pub const fn is_none(self) -> bool {
        self.0 == 0
    }

    /// Parse the 32-character hexadecimal form written by [`fmt::Display`].
    #[must_use]
    pub fn parse_hex(text: &str) -> Option<Self> {
        u128::from_str_radix(text.trim(), 16).ok().map(Self)
    }
}

impl fmt::Display for PersistentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

impl Default for PersistentId {
    fn default() -> Self {
        Self::NONE
    }
}

/// A random-enough 64-bit value, fixed for the life of the process.
///
/// `RandomState` is seeded from the OS per process, so hashing anything
/// through it yields a value that differs between runs. Mixing in the
/// wall clock costs nothing and removes the dependence on that being
/// true of every platform's standard library.
fn session_seed() -> u64 {
    static SEED: OnceLock<u64> = OnceLock::new();
    *SEED.get_or_init(|| {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        hasher.write_u128(nanos);
        hasher.write_usize(std::ptr::from_ref(&SEED) as usize);
        // Never hand back 0: it would make every counter value collide
        // with the reserved `NONE` id in the high half.
        hasher.finish() | 1
    })
}

impl World {
    /// The durable id of an entity, if it has one.
    #[must_use]
    pub fn persistent_id(&self, entity: Entity) -> Option<PersistentId> {
        self.get::<PersistentId>(entity).copied()
    }

    /// The durable id of an entity, minting and attaching one if it does
    /// not have it yet.
    ///
    /// Editor-created and scene-loaded entities get an id; transient
    /// runtime entities that nothing needs to name across a save do not
    /// have to.
    ///
    /// # Errors
    ///
    /// [`EcsError::DeadEntity`] if the handle is stale.
    pub fn ensure_persistent_id(&mut self, entity: Entity) -> Result<PersistentId, EcsError> {
        if let Some(existing) = self.persistent_id(entity) {
            return Ok(existing);
        }
        let id = PersistentId::mint();
        self.insert_component(entity, id)?;
        Ok(id)
    }

    /// Attach a specific id — the scene loader's path, where the id comes
    /// from the file rather than from [`PersistentId::mint`].
    ///
    /// # Errors
    ///
    /// [`EcsError::DeadEntity`] if the handle is stale.
    pub fn set_persistent_id(
        &mut self,
        entity: Entity,
        id: PersistentId,
    ) -> Result<(), EcsError> {
        self.insert_component(entity, id)
    }

    /// Find the live entity carrying a durable id.
    ///
    /// Linear in the number of entities: this is a load-time and
    /// tooling-time operation, not a per-frame one. Per-frame code should
    /// hold the [`Entity`] handle and validate it, which is what the
    /// generation counter is for.
    #[must_use]
    pub fn entity_by_persistent_id(&self, id: PersistentId) -> Option<Entity> {
        if id.is_none() {
            return None;
        }
        self.entities()
            .find(|&e| self.get::<PersistentId>(e) == Some(&id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, Default, PartialEq)]
    struct Marker;
    impl Component for Marker {}

    #[test]
    fn minted_ids_are_unique_and_never_none() {
        let a = PersistentId::mint();
        let b = PersistentId::mint();
        assert_ne!(a, b);
        assert!(!a.is_none());
        assert!(!b.is_none());
        assert!(PersistentId::NONE.is_none());
    }

    #[test]
    fn display_round_trips_through_parse() {
        let id = PersistentId::mint();
        let text = id.to_string();
        assert_eq!(text.len(), 32);
        assert_eq!(PersistentId::parse_hex(&text), Some(id));
        assert_eq!(PersistentId::parse_hex("not hex"), None);
    }

    #[test]
    fn ensure_is_idempotent_and_attaches_the_component() {
        let mut world = World::new();
        let e = world.spawn((Marker,));
        assert_eq!(world.persistent_id(e), None);

        let first = world.ensure_persistent_id(e).unwrap();
        let second = world.ensure_persistent_id(e).unwrap();
        assert_eq!(first, second, "ensure must not re-mint");
        assert_eq!(world.persistent_id(e), Some(first));
        assert_eq!(world.get::<Marker>(e), Some(&Marker), "migration kept Marker");
    }

    #[test]
    fn lookup_by_persistent_id_finds_the_right_entity() {
        let mut world = World::new();
        let a = world.spawn((Marker,));
        let b = world.spawn((Marker,));
        let id_a = world.ensure_persistent_id(a).unwrap();
        let id_b = world.ensure_persistent_id(b).unwrap();

        assert_eq!(world.entity_by_persistent_id(id_a), Some(a));
        assert_eq!(world.entity_by_persistent_id(id_b), Some(b));
        assert_eq!(world.entity_by_persistent_id(PersistentId::NONE), None);

        world.despawn(a);
        assert_eq!(world.entity_by_persistent_id(id_a), None);
        assert_eq!(world.entity_by_persistent_id(id_b), Some(b));
    }

    #[test]
    fn a_loaded_id_can_be_set_explicitly() {
        let mut world = World::new();
        let e = world.spawn((Marker,));
        let from_file = PersistentId::from_raw(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
        world.set_persistent_id(e, from_file).unwrap();
        assert_eq!(world.persistent_id(e), Some(from_file));
        assert_eq!(world.entity_by_persistent_id(from_file), Some(e));
    }

    #[test]
    fn a_stale_handle_cannot_be_given_an_id() {
        let mut world = World::new();
        let e = world.spawn((Marker,));
        world.despawn(e);
        assert_eq!(world.ensure_persistent_id(e), Err(EcsError::DeadEntity));
        assert_eq!(world.persistent_id(e), None);
    }
}
