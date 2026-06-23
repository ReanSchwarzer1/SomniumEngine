//! Entity handles and allocation.
//!
//! An [`Entity`] is a lightweight, copyable handle that uniquely
//! identifies a game object within the ECS [`World`](crate::World).
//! Internally it packs a 32-bit **index** (slot in the allocator) and
//! a 32-bit **generation** counter that is bumped every time a slot is
//! recycled, preventing ABA problems.
//!
//! ## Reference Architecture
//!
//! The generation-counter approach is inspired by Unreal Engine 5's
//! `FMassEntityHandle` (© Epic Games, Inc.) — see
//! `example_repo/UnrealEngine-release/.../MassEntity/Public/MassEntityHandle.h`.

use std::fmt;

/// A unique handle to an entity in the ECS world.
///
/// `Entity` is `Copy` and cheap to pass around. It is only valid in the
/// context of the [`World`](crate::World) that created it — using an
/// entity from one world in another is undefined (but memory-safe).
///
/// # Layout
///
/// | Bits   | Field        | Purpose                          |
/// |--------|--------------|----------------------------------|
/// | 0–31   | `index`      | Slot in the entity allocator     |
/// | 32–63  | `generation` | Recycling guard (monotonic bump) |
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entity {
    /// Index into the allocator's slot array.
    index: u32,
    /// Generation counter — incremented each time this slot is freed,
    /// so that stale handles are detected.
    generation: u32,
}

impl Entity {
    /// A sentinel value used to fill unused slots in fixed-size child arrays.
    /// Never a valid entity (generation u32::MAX is never issued by the allocator).
    pub const DANGLING: Self = Self { index: u32::MAX, generation: u32::MAX };

    /// Create an entity with the given index and generation.
    ///
    /// This is `pub(crate)` — only the allocator should mint entities.
    #[must_use]
    pub(crate) const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    /// Raw index into the allocator. Useful for dense-array lookups.
    #[inline]
    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }

    /// Generation counter for this entity slot.
    #[inline]
    #[must_use]
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

impl fmt::Debug for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Entity({}v{})", self.index, self.generation)
    }
}

impl fmt::Display for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}v{}", self.index, self.generation)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Entity Allocator
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Entry in the allocator's slot array.
#[derive(Debug)]
struct AllocatorEntry {
    /// Current generation for this slot.
    generation: u32,
    /// `true` if the slot is currently in use.
    is_alive: bool,
}

/// Free-list entity allocator.
///
/// Allocates entity handles with O(1) amortised `allocate` and O(1)
/// `free`. Freed indices are pushed onto a free-list and recycled with
/// an incremented generation.
#[derive(Debug)]
pub struct EntityAllocator {
    /// One entry per entity index ever allocated.
    entries: Vec<AllocatorEntry>,
    /// Indices available for reuse.
    free_list: Vec<u32>,
}

impl EntityAllocator {
    /// Create a new, empty allocator.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            free_list: Vec::new(),
        }
    }

    /// Allocate a new entity handle.
    ///
    /// If a previously freed slot is available it is recycled (with an
    /// incremented generation). Otherwise a fresh slot is appended.
    ///
    /// # Panics
    ///
    /// Panics if the number of allocated entities exceeds `u32::MAX`.
    pub fn allocate(&mut self) -> Entity {
        if let Some(index) = self.free_list.pop() {
            let entry = &mut self.entries[index as usize];
            entry.is_alive = true;
            Entity::new(index, entry.generation)
        } else {
            let index = u32::try_from(self.entries.len())
                .expect("entity index overflow (>4 billion entities)");
            self.entries.push(AllocatorEntry {
                generation: 0,
                is_alive: true,
            });
            Entity::new(index, 0)
        }
    }

    /// Free an entity, returning `true` if it was alive and is now freed.
    ///
    /// The slot's generation is bumped so that any outstanding handles
    /// with the old generation become stale.
    pub fn free(&mut self, entity: Entity) -> bool {
        let idx = entity.index() as usize;
        if idx >= self.entries.len() {
            return false;
        }
        let entry = &mut self.entries[idx];
        if !entry.is_alive || entry.generation != entity.generation() {
            return false;
        }
        entry.is_alive = false;
        entry.generation = entry.generation.wrapping_add(1);
        self.free_list.push(entity.index());
        true
    }

    /// Check whether an entity handle is still valid (alive).
    #[must_use]
    pub fn is_alive(&self, entity: Entity) -> bool {
        let idx = entity.index() as usize;
        if idx >= self.entries.len() {
            return false;
        }
        let entry = &self.entries[idx];
        entry.is_alive && entry.generation == entity.generation()
    }

    /// Number of currently alive entities.
    #[must_use]
    pub fn alive_count(&self) -> usize {
        self.entries.len() - self.free_list.len()
    }
}

impl Default for EntityAllocator {
    fn default() -> Self {
        Self::new()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_returns_sequential_indices() {
        let mut alloc = EntityAllocator::new();
        let a = alloc.allocate();
        let b = alloc.allocate();
        assert_eq!(a.index(), 0);
        assert_eq!(b.index(), 1);
        assert_eq!(a.generation(), 0);
    }

    #[test]
    fn free_and_reuse_bumps_generation() {
        let mut alloc = EntityAllocator::new();
        let a = alloc.allocate();
        assert!(alloc.free(a));
        let b = alloc.allocate();
        assert_eq!(b.index(), a.index()); // same slot reused
        assert_eq!(b.generation(), 1); // generation bumped
        assert!(!alloc.is_alive(a)); // old handle is stale
        assert!(alloc.is_alive(b)); // new handle is valid
    }

    #[test]
    fn double_free_is_noop() {
        let mut alloc = EntityAllocator::new();
        let a = alloc.allocate();
        assert!(alloc.free(a));
        assert!(!alloc.free(a)); // already freed
    }

    #[test]
    fn alive_count_tracks_correctly() {
        let mut alloc = EntityAllocator::new();
        assert_eq!(alloc.alive_count(), 0);
        let a = alloc.allocate();
        let b = alloc.allocate();
        assert_eq!(alloc.alive_count(), 2);
        alloc.free(a);
        assert_eq!(alloc.alive_count(), 1);
        alloc.free(b);
        assert_eq!(alloc.alive_count(), 0);
    }

    #[test]
    fn entity_debug_display() {
        let e = Entity::new(42, 3);
        assert_eq!(format!("{e:?}"), "Entity(42v3)");
        assert_eq!(format!("{e}"), "42v3");
    }
}
