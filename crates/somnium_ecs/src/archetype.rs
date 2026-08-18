//! Archetype-based Struct-of-Arrays (`SoA`) component storage.
//!
//! An [`Archetype`] groups all entities that share the exact same set
//! of component types. Components are stored in parallel, contiguous
//! byte arrays — one [`ComponentColumn`] per component type — giving
//! optimal cache locality during iteration.
//!
//! ## Reference Architecture
//!
//! This design is directly informed by:
//!
//! - **Unreal Engine 5 `MassEntity`** (© Epic Games, Inc.) — see
//!   `example_repo/UnrealEngine-release/.../MassEntity/Public/MassArchetypeTypes.h`.
//!   UE5 stores entities in archetype "chunks" with contiguous memory
//!   layouts. Our `ComponentColumn` serves the same role as UE5's
//!   chunk-based fragment storage.
//!
//! - **The Forge `IVisibilityBuffer`** (© Confetti FX) — see
//!   `example_repo/The-Forge-master/Common_3/Renderer/Interfaces/IVisibilityBuffer.h`.
//!   The geometry-set batching model informs how we could later feed
//!   archetype data directly to GPU indirect draw calls.

use std::alloc::Layout;

use crate::component::{ComponentId, ComponentInfo, ComponentSet};
use crate::entity::Entity;

/// Unique identifier for an archetype.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArchetypeId(pub(crate) u32);

impl ArchetypeId {
    /// Raw index.
    #[inline]
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ComponentColumn — type-erased dense array
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A type-erased, dense array storing one component type per row.
///
/// Each row corresponds to exactly one entity in the archetype, with
/// the same index as the entity's position in [`Archetype::entities`].
#[derive(Debug)]
pub struct ComponentColumn {
    /// Component metadata (id, layout, drop fn).
    info: ComponentInfo,
    /// Raw byte storage. Length is always `item_size * len`.
    data: Vec<u8>,
    /// Number of items currently stored.
    len: usize,
}

impl ComponentColumn {
    /// Create an empty column for a given component type.
    #[must_use]
    pub fn new(info: ComponentInfo) -> Self {
        Self {
            info,
            data: Vec::new(),
            len: 0,
        }
    }

    /// Component ID this column stores.
    #[inline]
    #[must_use]
    pub fn component_id(&self) -> ComponentId {
        self.info.id
    }

    /// Type-erased metadata for the component this column stores.
    #[inline]
    #[must_use]
    pub fn info(&self) -> &ComponentInfo {
        &self.info
    }

    /// Size of one item in bytes.
    ///
    /// Zero-sized types report 1 so that row indexing stays uniform; the
    /// *payload* size (which is what gets copied) is `info.layout.size()`.
    #[inline]
    #[must_use]
    pub fn item_size(&self) -> usize {
        // Use at least 1 byte for ZSTs to keep indexing simple.
        self.info.layout.size().max(1)
    }

    /// Push a value of type `T` into this column.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `T` matches the component type this
    /// column was created for.
    pub unsafe fn push_raw(&mut self, ptr: *const u8, size: usize) {
        let start = self.data.len();
        self.data.resize(start + self.item_size(), 0);
        unsafe {
            std::ptr::copy_nonoverlapping(ptr, self.data.as_mut_ptr().add(start), size);
        }
        self.len += 1;
    }

    /// Push a typed value.
    ///
    /// # Panics
    ///
    /// Panics (debug) if `T`'s layout doesn't match this column's.
    pub fn push<T: 'static>(&mut self, value: T) {
        debug_assert_eq!(
            Layout::new::<T>().size(),
            self.info.layout.size(),
            "component size mismatch"
        );
        let ptr = std::ptr::from_ref(&value).cast::<u8>();
        unsafe {
            self.push_raw(ptr, std::mem::size_of::<T>());
        }
        std::mem::forget(value);
    }

    /// Get a reference to the item at `row`.
    ///
    /// # Safety
    ///
    /// The caller must ensure `T` matches the stored type and `row < len`.
    #[inline]
    #[must_use]
    pub unsafe fn get<T: 'static>(&self, row: usize) -> &T {
        debug_assert!(row < self.len);
        let offset = row * self.item_size();
        unsafe { &*self.data.as_ptr().add(offset).cast::<T>() }
    }

    /// Get a mutable reference to the item at `row`.
    ///
    /// # Safety
    ///
    /// The caller must ensure `T` matches the stored type and `row < len`.
    #[inline]
    pub unsafe fn get_mut<T: 'static>(&mut self, row: usize) -> &mut T {
        debug_assert!(row < self.len);
        let offset = row * self.item_size();
        unsafe { &mut *self.data.as_mut_ptr().add(offset).cast::<T>() }
    }

    /// Get a raw pointer to the item at `row`.
    #[inline]
    #[must_use]
    pub fn get_raw(&self, row: usize) -> *const u8 {
        debug_assert!(row < self.len);
        let offset = row * self.item_size();
        unsafe { self.data.as_ptr().add(offset) }
    }

    /// Get a raw mutable pointer to the item at `row`.
    #[inline]
    #[must_use]
    pub fn get_raw_mut(&mut self, row: usize) -> *mut u8 {
        debug_assert!(row < self.len);
        let offset = row * self.item_size();
        unsafe { self.data.as_mut_ptr().add(offset) }
    }

    /// Swap-remove the item at `row`, moving the last item into its
    /// place. Returns `true` if a swap occurred (i.e. row was not the
    /// last).
    ///
    /// Calls the drop function for the removed item.
    pub fn swap_remove(&mut self, row: usize) -> bool {
        debug_assert!(row < self.len);
        let last = self.len - 1;
        let item_size = self.item_size();

        // Drop the removed element.
        if let Some(drop_fn) = self.info.drop_fn {
            unsafe {
                drop_fn(self.data.as_mut_ptr().add(row * item_size));
            }
        }

        let swapped = row != last;
        if swapped {
            // Move last element into the vacated slot.
            let src = last * item_size;
            let dst = row * item_size;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    self.data.as_ptr().add(src),
                    self.data.as_mut_ptr().add(dst),
                    item_size,
                );
            }
        }
        self.data.truncate(last * item_size);
        self.len -= 1;
        swapped
    }

    /// Move the raw bytes at `row` into `dst`, then swap-remove the
    /// slot (without dropping). Used for archetype transitions.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `dst` is valid for writes of size
    /// `self.info.layout.size()`, and that `row < len`.
    ///
    /// Returns `true` if a swap occurred.
    pub unsafe fn move_out_and_swap_remove(&mut self, row: usize, dst: *mut u8) -> bool {
        debug_assert!(row < self.len);
        let last = self.len - 1;
        let item_size = self.item_size();

        // Copy data out (no drop — ownership transfers).
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.data.as_ptr().add(row * item_size),
                dst,
                self.info.layout.size(),
            );
        }

        let swapped = row != last;
        if swapped {
            let src = last * item_size;
            let dst_slot = row * item_size;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    self.data.as_ptr().add(src),
                    self.data.as_mut_ptr().add(dst_slot),
                    item_size,
                );
            }
        }
        self.data.truncate(last * item_size);
        self.len -= 1;
        swapped
    }

    /// Append one already-owned value, copying `info.layout.size()` bytes
    /// from `src` into a freshly allocated row.
    ///
    /// This is the archetype-migration counterpart of
    /// [`Self::move_out_and_swap_remove`]: ownership transfers into the
    /// column, so no drop runs on either side of the move.
    ///
    /// # Safety
    ///
    /// `src` must point to a valid, initialised value of this column's
    /// component type, and the caller must not use that value afterwards.
    pub unsafe fn push_moved(&mut self, src: *const u8) {
        let payload = self.info.layout.size();
        let start = self.data.len();
        self.data.resize(start + self.item_size(), 0);
        if payload > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(src, self.data.as_mut_ptr().add(start), payload);
            }
        }
        self.len += 1;
    }

    /// Number of items in this column.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether this column is empty.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Drop for ComponentColumn {
    fn drop(&mut self) {
        // Drop all remaining items.
        if let Some(drop_fn) = self.info.drop_fn {
            let item_size = self.item_size();
            for i in 0..self.len {
                unsafe {
                    drop_fn(self.data.as_mut_ptr().add(i * item_size));
                }
            }
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// MovedComponent — an owned value in transit between archetypes
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// One component value that has been moved out of an archetype and is not
/// yet in another one.
///
/// It owns the value. If it is dropped without being relinquished (via
/// [`Self::relinquish`]) the component's own destructor runs, so an early
/// return or a `?` on a migration path leaks nothing.
pub(crate) struct MovedComponent {
    /// Which component type these bytes hold.
    pub(crate) id: ComponentId,
    /// Destructor, if the type needs one.
    drop_fn: Option<unsafe fn(*mut u8)>,
    /// `item_size` bytes of storage holding one moved value.
    bytes: Vec<u8>,
    /// Set once ownership has been handed to a destination column.
    relinquished: bool,
}

impl MovedComponent {
    /// Hand ownership of the value to the caller and return a pointer to
    /// it. After this call the destructor will **not** run here, so the
    /// caller must move the value somewhere that owns it.
    pub(crate) fn relinquish(&mut self) -> *const u8 {
        self.relinquished = true;
        self.bytes.as_ptr()
    }
}

impl Drop for MovedComponent {
    fn drop(&mut self) {
        if !self.relinquished {
            if let Some(drop_fn) = self.drop_fn {
                // SAFETY: `bytes` holds one initialised value of the
                // component type `drop_fn` was built for, and this runs
                // exactly once because `Drop` runs once.
                unsafe { drop_fn(self.bytes.as_mut_ptr()) }
            }
        }
    }
}

impl std::fmt::Debug for MovedComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MovedComponent")
            .field("id", &self.id)
            .field("drop_fn", &self.drop_fn.is_some())
            .field("bytes", &self.bytes.len())
            .field("relinquished", &self.relinquished)
            .finish()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Archetype
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// An archetype: a group of entities sharing the same component set.
///
/// All component data is stored in parallel [`ComponentColumn`]s (`SoA`
/// layout). The `entities` vec maps each row index back to its `Entity`
/// handle.
#[derive(Debug)]
pub struct Archetype {
    /// Unique ID of this archetype.
    id: ArchetypeId,
    /// Sorted component signature.
    component_set: ComponentSet,
    /// Parallel columns — one per component, in the same order as
    /// `component_set.iter()`.
    columns: Vec<ComponentColumn>,
    /// Parallel array: `entities[row]` is the entity at that row.
    entities: Vec<Entity>,
}

impl Archetype {
    /// Create a new, empty archetype.
    #[must_use]
    pub fn new(id: ArchetypeId, component_set: ComponentSet, infos: Vec<ComponentInfo>) -> Self {
        let columns = infos.into_iter().map(ComponentColumn::new).collect();
        Self {
            id,
            component_set,
            columns,
            entities: Vec::new(),
        }
    }

    /// Archetype identifier.
    #[inline]
    #[must_use]
    pub fn id(&self) -> ArchetypeId {
        self.id
    }

    /// Component signature.
    #[inline]
    #[must_use]
    pub fn component_set(&self) -> &ComponentSet {
        &self.component_set
    }

    /// Number of entities in this archetype.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Whether this archetype has no entities.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Entities in this archetype.
    #[inline]
    #[must_use]
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    /// Find the column index for a given component ID.
    #[inline]
    #[must_use]
    pub fn column_index(&self, id: ComponentId) -> Option<usize> {
        self.component_set.position(id)
    }

    /// Get a reference to a column by its index.
    #[inline]
    #[must_use]
    pub fn column(&self, col_idx: usize) -> &ComponentColumn {
        &self.columns[col_idx]
    }

    /// Get a mutable reference to a column by its index.
    #[inline]
    pub fn column_mut(&mut self, col_idx: usize) -> &mut ComponentColumn {
        &mut self.columns[col_idx]
    }

    /// Type-erased metadata for every column, in sorted component order.
    pub fn column_infos(&self) -> impl Iterator<Item = &ComponentInfo> {
        self.columns.iter().map(ComponentColumn::info)
    }

    /// Move an entire row **out** of this archetype without dropping any
    /// component, leaving the archetype one row shorter.
    ///
    /// Returns one owned byte buffer per component (in sorted component
    /// order) and the entity that was swapped into `row`, if any. The
    /// caller now owns every value in those buffers and must either push
    /// them into another archetype or drop them explicitly — leaking them
    /// leaks whatever the components own.
    ///
    /// This is the primitive behind runtime component insert/remove: it
    /// exists so that migration never has to hold a mutable borrow of two
    /// archetypes at once.
    pub(crate) fn move_out_row(&mut self, row: usize) -> (Vec<MovedComponent>, Option<Entity>) {
        debug_assert!(row < self.entities.len());
        let mut moved = Vec::with_capacity(self.columns.len());
        for col in &mut self.columns {
            let mut bytes = vec![0_u8; col.item_size()];
            // SAFETY: `bytes` is at least `item_size` ≥ `layout.size()`
            // long, and `row` is in bounds for every parallel column.
            unsafe {
                col.move_out_and_swap_remove(row, bytes.as_mut_ptr());
            }
            moved.push(MovedComponent {
                id: col.component_id(),
                drop_fn: col.info().drop_fn,
                bytes,
                relinquished: false,
            });
        }

        let last = self.entities.len() - 1;
        let swapped = row != last;
        self.entities.swap_remove(row);
        if swapped {
            (moved, Some(self.entities[row]))
        } else {
            (moved, None)
        }
    }

    /// Add an entity to this archetype, returning its row index.
    ///
    /// The caller must push corresponding data into every column.
    pub(crate) fn allocate_row(&mut self, entity: Entity) -> usize {
        let row = self.entities.len();
        self.entities.push(entity);
        row
    }

    /// Remove an entity by row index using swap-remove.
    ///
    /// Returns the entity that was moved into `row` (if any — i.e. when
    /// the removed entity was not the last).
    pub(crate) fn swap_remove_row(&mut self, row: usize) -> Option<Entity> {
        let last = self.entities.len() - 1;
        let swapped = row != last;

        // Swap-remove from every column.
        for col in &mut self.columns {
            col.swap_remove(row);
        }

        self.entities.swap_remove(row);

        if swapped {
            Some(self.entities[row])
        } else {
            None
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::Component;

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Pos {
        x: f32,
        y: f32,
    }
    impl Component for Pos {}

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Vel {
        dx: f32,
        dy: f32,
    }
    impl Component for Vel {}

    fn make_archetype() -> Archetype {
        let set = ComponentSet::from_ids(vec![ComponentId::of::<Pos>(), ComponentId::of::<Vel>()]);
        let infos = vec![ComponentInfo::of::<Pos>(), ComponentInfo::of::<Vel>()];
        // Re-order infos to match sorted set order.
        let mut ordered_infos = Vec::new();
        for id in set.iter() {
            let info = infos.iter().find(|i| i.id == id).unwrap().clone();
            ordered_infos.push(info);
        }
        Archetype::new(ArchetypeId(0), set, ordered_infos)
    }

    #[test]
    fn push_and_read_components() {
        let mut arch = make_archetype();
        let entity = Entity::new(0, 0);
        let row = arch.allocate_row(entity);

        let pos_col = arch.column_index(ComponentId::of::<Pos>()).unwrap();
        let vel_col = arch.column_index(ComponentId::of::<Vel>()).unwrap();

        arch.column_mut(pos_col).push(Pos { x: 1.0, y: 2.0 });
        arch.column_mut(vel_col).push(Vel { dx: 3.0, dy: 4.0 });

        unsafe {
            assert_eq!(
                *arch.column(pos_col).get::<Pos>(row),
                Pos { x: 1.0, y: 2.0 }
            );
            assert_eq!(
                *arch.column(vel_col).get::<Vel>(row),
                Vel { dx: 3.0, dy: 4.0 }
            );
        }
        assert_eq!(arch.len(), 1);
    }

    #[test]
    fn swap_remove_preserves_data() {
        let mut arch = make_archetype();
        let e0 = Entity::new(0, 0);
        let e1 = Entity::new(1, 0);

        let pos_col = arch.column_index(ComponentId::of::<Pos>()).unwrap();
        let vel_col = arch.column_index(ComponentId::of::<Vel>()).unwrap();

        arch.allocate_row(e0);
        arch.column_mut(pos_col).push(Pos { x: 1.0, y: 1.0 });
        arch.column_mut(vel_col).push(Vel { dx: 10.0, dy: 10.0 });

        arch.allocate_row(e1);
        arch.column_mut(pos_col).push(Pos { x: 2.0, y: 2.0 });
        arch.column_mut(vel_col).push(Vel { dx: 20.0, dy: 20.0 });

        // Remove first entity — second entity should swap into row 0.
        let swapped = arch.swap_remove_row(0);
        assert_eq!(swapped, Some(e1));
        assert_eq!(arch.len(), 1);
        assert_eq!(arch.entities()[0], e1);

        unsafe {
            assert_eq!(*arch.column(pos_col).get::<Pos>(0), Pos { x: 2.0, y: 2.0 });
        }
    }
}
