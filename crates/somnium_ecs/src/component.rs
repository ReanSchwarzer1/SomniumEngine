//! Component trait and type-level metadata.
//!
//! A **component** is a plain data struct attached to entities. The ECS
//! stores components in dense, cache-friendly arrays grouped by
//! archetype, so component access during iteration is as fast as
//! iterating a `Vec<T>`.
//!
//! ## Reference Architecture
//!
//! The component-as-plain-data design is informed by Unreal Engine 5's
//! `MassEntity` (© Epic Games, Inc.) where "fragments" are POD structs
//! stored contiguously in archetype chunks, and by The Forge's
//! (© Confetti FX) data-oriented `IVisibilityBuffer` geometry-set
//! approach.

use std::alloc::Layout;
use std::any::TypeId;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

/// Marker trait for ECS components.
///
/// Any type that is `Send + Sync + 'static` can be a component.
/// Implement this on your structs to use them with the ECS.
///
/// # Example
///
/// ```
/// use somnium_ecs::Component;
///
/// #[derive(Debug, Clone, Copy)]
/// struct Position { x: f32, y: f32 }
/// impl Component for Position {}
///
/// #[derive(Debug, Clone, Copy)]
/// struct Velocity { dx: f32, dy: f32 }
/// impl Component for Velocity {}
/// ```
pub trait Component: Send + Sync + 'static {}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Component ID
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Runtime-assigned unique identifier for a component type.
///
/// `ComponentId` values are stable for the lifetime of the process but
/// **not** across runs. They are assigned lazily on first use via
/// [`ComponentId::of::<T>()`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ComponentId(u32);

impl ComponentId {
    /// Get (or lazily assign) the `ComponentId` for type `T`.
    #[must_use]
    pub fn of<T: Component>() -> Self {
        Self(inner_id::<T>())
    }

    /// Raw numeric value.
    #[inline]
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl PartialOrd for ComponentId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ComponentId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

/// Inner helper: assigns a unique u32 to each monomorphised `T`.
fn inner_id<T: 'static>() -> u32 {
    use std::sync::OnceLock;
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    // Per-T static, created via monomorphisation.
    static ID_MAP: OnceLock<std::sync::Mutex<HashMap<TypeId, u32>>> = OnceLock::new();
    let map = ID_MAP.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = map
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *guard
        .entry(TypeId::of::<T>())
        .or_insert_with(|| COUNTER.fetch_add(1, AtomicOrdering::Relaxed))
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Component Info
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Type-erased metadata about a component, sufficient for managing
/// storage without knowing the concrete type.
#[derive(Debug, Clone)]
pub struct ComponentInfo {
    /// Unique runtime identifier.
    pub id: ComponentId,
    /// Memory layout (size + alignment).
    pub layout: Layout,
    /// Name of the type (for debug/logging).
    pub name: &'static str,
    /// Function pointer to drop a single instance at a given pointer.
    /// `None` for types that don't need dropping (e.g. `Copy` types).
    pub drop_fn: Option<unsafe fn(*mut u8)>,
}

impl ComponentInfo {
    /// Build `ComponentInfo` for a concrete component type.
    #[must_use]
    pub fn of<T: Component>() -> Self {
        Self {
            id: ComponentId::of::<T>(),
            layout: Layout::new::<T>(),
            name: std::any::type_name::<T>(),
            drop_fn: if std::mem::needs_drop::<T>() {
                Some(Self::drop_impl::<T>)
            } else {
                None
            },
        }
    }

    /// Drop implementation for type `T`.
    ///
    /// # Safety
    ///
    /// `ptr` must point to a valid, initialised `T`.
    unsafe fn drop_impl<T>(ptr: *mut u8) {
        unsafe {
            std::ptr::drop_in_place(ptr.cast::<T>());
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Component Set
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A **sorted** set of component IDs that defines an archetype's
/// signature. Two archetypes with the same `ComponentSet` store
/// exactly the same component types.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ComponentSet {
    /// Sorted by `ComponentId` for deterministic comparison and hashing.
    ids: Vec<ComponentId>,
}

impl ComponentSet {
    /// Create a component set from an unsorted list of IDs.
    ///
    /// The IDs are sorted and deduplicated.
    #[must_use]
    pub fn from_ids(mut ids: Vec<ComponentId>) -> Self {
        ids.sort();
        ids.dedup();
        Self { ids }
    }

    /// Create an empty component set.
    #[must_use]
    pub fn empty() -> Self {
        Self { ids: Vec::new() }
    }

    /// Whether this set contains a given component ID.
    #[must_use]
    pub fn contains(&self, id: ComponentId) -> bool {
        self.ids.binary_search(&id).is_ok()
    }

    /// Whether this set is a superset of `other`.
    #[must_use]
    pub fn contains_all(&self, other: &Self) -> bool {
        other.ids.iter().all(|id| self.contains(*id))
    }

    /// Whether this set contains *none* of the IDs in `other`.
    #[must_use]
    pub fn contains_none(&self, other: &Self) -> bool {
        !other.ids.iter().any(|id| self.contains(*id))
    }

    /// Return a new set with `id` added.
    #[must_use]
    pub fn with(&self, id: ComponentId) -> Self {
        let mut ids = self.ids.clone();
        if let Err(pos) = ids.binary_search(&id) {
            ids.insert(pos, id);
        }
        Self { ids }
    }

    /// Return a new set with `id` removed.
    #[must_use]
    pub fn without(&self, id: ComponentId) -> Self {
        let mut ids = self.ids.clone();
        if let Ok(pos) = ids.binary_search(&id) {
            ids.remove(pos);
        }
        Self { ids }
    }

    /// Number of component types in this set.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Whether this set is empty.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Iterate over the component IDs in sorted order.
    pub fn iter(&self) -> impl Iterator<Item = ComponentId> + '_ {
        self.ids.iter().copied()
    }

    /// Find the position of a component ID within the sorted set.
    ///
    /// This position corresponds to the column index in the archetype.
    #[must_use]
    pub fn position(&self, id: ComponentId) -> Option<usize> {
        self.ids.binary_search(&id).ok()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    struct Pos;
    impl Component for Pos {}

    struct Vel;
    impl Component for Vel {}

    struct Health;
    impl Component for Health {}

    #[test]
    fn component_id_is_stable() {
        let a = ComponentId::of::<Pos>();
        let b = ComponentId::of::<Pos>();
        assert_eq!(a, b);
    }

    #[test]
    fn different_types_get_different_ids() {
        let a = ComponentId::of::<Pos>();
        let b = ComponentId::of::<Vel>();
        assert_ne!(a, b);
    }

    #[test]
    fn component_set_contains() {
        let set = ComponentSet::from_ids(vec![ComponentId::of::<Pos>(), ComponentId::of::<Vel>()]);
        assert!(set.contains(ComponentId::of::<Pos>()));
        assert!(set.contains(ComponentId::of::<Vel>()));
        assert!(!set.contains(ComponentId::of::<Health>()));
    }

    #[test]
    fn component_set_with_without() {
        let set = ComponentSet::from_ids(vec![ComponentId::of::<Pos>()]);
        let extended = set.with(ComponentId::of::<Vel>());
        assert_eq!(extended.len(), 2);

        let reduced = extended.without(ComponentId::of::<Pos>());
        assert_eq!(reduced.len(), 1);
        assert!(reduced.contains(ComponentId::of::<Vel>()));
    }

    #[test]
    fn component_set_deduplicates() {
        let id = ComponentId::of::<Pos>();
        let set = ComponentSet::from_ids(vec![id, id, id]);
        assert_eq!(set.len(), 1);
    }
}
